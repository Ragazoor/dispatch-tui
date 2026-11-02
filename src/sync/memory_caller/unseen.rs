//! The new-task signal's rows. Mirrors the module's `unseen.rs`.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::TaskId;
use crate::spacetime::bindings;

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

impl MemoryReducerCaller {
    pub(super) fn apply_create_task_signalled(&self, row: bindings::Task) -> Result<TaskId> {
        let id = self.apply_create_task(row)?;
        let mut tables = self.lock();
        let epic_id = tables.tasks.get(&id.0).map_or(0, |t| t.epic_id);
        self.mark_task_unseen_for_viewers(&mut tables, id.0, epic_id);
        Ok(id)
    }

    pub(super) fn mark_task_unseen_for_viewers(
        &self,
        tables: &mut Tables,
        task_id: i64,
        epic_id: i64,
    ) {
        if epic_id == 0 {
            return;
        }
        let viewers = tables.epic_viewers(&self.scope(), epic_id);
        for subscriber in viewers {
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

    /// `tasks.allium: SeeTaskByLandingOnIt`. Mirrors the module's
    /// `mark_task_seen`.
    pub(super) fn apply_mark_task_seen(
        &self,
        subscriber: String,
        task_id: TaskId,
    ) -> Result<ReducerOutcome> {
        if subscriber.trim().is_empty() {
            return Ok(ReducerOutcome::Refused(
                "cannot mark seen without an identity".into(),
            ));
        }
        let scope = self.scope();
        if let Err(why) = Tables::require_own_identity(&scope, &subscriber) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let mut tables = self.lock();
        if tables.visible_task(&scope, task_id.0).is_none() {
            return Ok(ReducerOutcome::Applied(vec![]));
        }
        let id = module::unseen_id(&subscriber, task_id.0);
        if tables.unseen_tasks.remove(&id).is_some() {
            self.rows.remove_unseen_task(id);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }
}
