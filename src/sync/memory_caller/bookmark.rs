//! Task bookmarks. Mirrors the module's `bookmark.rs`.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::TaskId;

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables, DONE};

impl MemoryReducerCaller {
    /// End every person's bookmark on `task_id`.
    pub(super) fn clear_bookmarks_for_task(&self, tables: &mut Tables, task_id: i64) {
        let gone: Vec<String> = tables
            .bookmarks
            .values()
            .filter(|b| b.task_id == task_id)
            .map(|b| b.id.clone())
            .collect();
        for id in gone {
            tables.bookmarks.remove(&id);
            self.rows.remove_bookmark(id);
        }
    }

    /// `tasks.allium: SetBookmark`. Mirrors the module's `set_bookmark`.
    pub(super) fn apply_set_bookmark(
        &self,
        subscriber: String,
        task_id: TaskId,
        bookmarked: bool,
    ) -> Result<ReducerOutcome> {
        if subscriber.trim().is_empty() {
            return Ok(ReducerOutcome::Refused(
                "cannot bookmark without an identity".into(),
            ));
        }
        let scope = self.scope();
        if let Err(why) = Tables::require_own_identity(&scope, &subscriber) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let mut tables = self.lock();
        let Some(task) = tables.visible_task(&scope, task_id.0) else {
            return Ok(ReducerOutcome::Refused(format!(
                "task {} does not exist",
                task_id.0
            )));
        };
        let id = module::bookmark_id(&subscriber, task_id.0);
        if !bookmarked {
            if tables.bookmarks.remove(&id).is_some() {
                self.rows.remove_bookmark(id);
            }
            return Ok(ReducerOutcome::Applied(vec![]));
        }
        if task.status == DONE {
            return Ok(ReducerOutcome::Refused(format!(
                "Task {} is done and cannot be bookmarked",
                task_id.0
            )));
        }
        if let std::collections::btree_map::Entry::Vacant(entry) =
            tables.bookmarks.entry(id.clone())
        {
            let row = module::Bookmark {
                id,
                task_id: task_id.0,
                subscriber,
            };
            entry.insert(row.clone());
            self.rows.upsert_bookmark(&row.into());
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }
}
