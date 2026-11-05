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

    /// Mirrors the module's `clear_task_marks`.
    pub(super) fn clear_task_marks(&self, tables: &mut Tables, task_id: i64) {
        self.clear_unseen_for_task(tables, task_id);
        self.clear_bookmarks_for_task(tables, task_id);
    }

    /// Mirrors the module's `end_marks_on_finish`.
    pub(super) fn end_marks_on_finish(
        &self,
        tables: &mut Tables,
        prior: &module::Task,
        next: &module::Task,
    ) {
        if module::stamps_completion(&prior.status, &next.status) {
            self.clear_task_marks(tables, next.id);
        }
    }

    /// Mirrors the module's `drop_marks_where`; `lost_sight` reads the tables
    /// as they were before any mark is dropped.
    pub(super) fn drop_marks_where(
        &self,
        tables: &mut Tables,
        lost_sight: impl Fn(&Tables, i64, &str) -> bool,
    ) {
        let unseen: Vec<String> = tables
            .unseen_tasks
            .values()
            .filter(|u| lost_sight(tables, u.task_id, &u.subscriber))
            .map(|u| u.id.clone())
            .collect();
        let bookmarks: Vec<String> = tables
            .bookmarks
            .values()
            .filter(|b| lost_sight(tables, b.task_id, &b.subscriber))
            .map(|b| b.id.clone())
            .collect();
        for id in unseen {
            tables.unseen_tasks.remove(&id);
            self.rows.remove_unseen_task(id);
        }
        for id in bookmarks {
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
