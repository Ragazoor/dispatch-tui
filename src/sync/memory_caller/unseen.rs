//! The new-task signal's rows. Mirrors the module's `unseen.rs`.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{EpicId, TaskId};
use crate::spacetime::bindings;

use super::super::encode;
use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

impl MemoryReducerCaller {
    pub(super) fn apply_create_task_signalled(&self, row: bindings::Task) -> Result<TaskId> {
        let id = self.apply_create_task(row)?;
        let mut tables = self.lock();
        let epic_id = tables.tasks.get(&id.0).map_or(0, |t| t.epic_id);
        self.mark_task_unseen_for_followers(&mut tables, id.0, epic_id);
        Ok(id)
    }

    pub(super) fn mark_task_unseen_for_followers(
        &self,
        tables: &mut Tables,
        task_id: i64,
        epic_id: i64,
    ) {
        if epic_id == 0 {
            return;
        }
        let mut chain = Vec::new();
        let mut next = epic_id;
        while next != 0 && chain.len() < 1024 && !chain.contains(&next) {
            chain.push(next);
            next = tables.epics.get(&next).map_or(0, |e| e.parent_epic_id);
        }
        let followers: std::collections::BTreeSet<String> = tables
            .subscriptions
            .values()
            .filter(|s| chain.contains(&s.epic_id))
            .map(|s| s.subscriber.clone())
            .collect();
        for subscriber in followers {
            let id = module::unseen_id(&subscriber, task_id);
            if let std::collections::btree_map::Entry::Vacant(entry) =
                tables.unseen_tasks.entry(id.clone())
            {
                let row = module::UnseenTask {
                    id,
                    task_id,
                    subscriber,
                };
                entry.insert(row.clone());
                self.rows.upsert_unseen_task(&row.into());
            }
        }
    }

    /// End every person's mark on `task_id`.
    pub(super) fn clear_unseen_for_task(&self, tables: &mut Tables, task_id: i64) {
        let gone: Vec<String> = tables
            .unseen_tasks
            .values()
            .filter(|u| u.task_id == task_id)
            .map(|u| u.id.clone())
            .collect();
        for id in gone {
            tables.unseen_tasks.remove(&id);
            self.rows.remove_unseen_task(id);
        }
    }

    pub(super) fn apply_mark_epic_seen(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> Result<ReducerOutcome> {
        if subscriber.trim().is_empty() {
            return Ok(ReducerOutcome::Refused(
                "cannot mark seen without an identity".into(),
            ));
        }
        let epic_id = encode::epic_ref(Some(epic_id));
        let scope = self.scope();
        let mut tables = self.lock();
        if tables.visible_epic(&scope, epic_id).is_none() {
            return Ok(ReducerOutcome::Applied(vec![]));
        }
        let in_epic: Vec<i64> = tables
            .tasks
            .values()
            .filter(|t| t.epic_id == epic_id)
            .map(|t| t.id)
            .collect();
        for task_id in in_epic {
            let id = module::unseen_id(&subscriber, task_id);
            if tables.unseen_tasks.remove(&id).is_some() {
                self.rows.remove_unseen_task(id);
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }
}
