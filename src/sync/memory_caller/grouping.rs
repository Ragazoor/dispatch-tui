//! Repo grouping, mirroring `spacetime/module/src/grouping.rs`: the routing of
//! a grouped epic's tasks into per-repo `repo-group` sub-epics, and the two
//! reducers that migrate a whole epic.

use anyhow::Result;

use super::bindings;

use dispatch_spacetime_module as module;

use crate::models::EpicId;

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

impl MemoryReducerCaller {
    fn auto_groups(epic: &module::Epic) -> bool {
        module::can_auto_group(epic.group_by_repo, &epic.feed_command)
    }

    /// Mirrors `route_to_repo_group`: where a task of `repo_path` assigned to
    /// `epic_id` lives. `0` stays `0`; a grouped non-feed epic answers its
    /// per-repo sub-epic (created when absent); anything else is unchanged.
    pub(super) fn route_to_repo_group(
        &self,
        tables: &mut Tables,
        epic_id: i64,
        repo_path: &str,
    ) -> i64 {
        if epic_id == 0 {
            return 0;
        }
        match tables.epics.get(&epic_id) {
            Some(root) if Self::auto_groups(root) => {
                let title = module::repo_name_from_path(repo_path);
                self.find_or_create_repo_group(tables, epic_id, &title, self.sender.clone())
            }
            _ => epic_id,
        }
    }

    /// Mirrors the module's `find_or_create_repo_group`, shared by manual
    /// routing and feed ingestion as the module shares it.
    pub(super) fn find_or_create_repo_group(
        &self,
        tables: &mut Tables,
        root: i64,
        title: &str,
        created_by: String,
    ) -> i64 {
        let existing = tables
            .epics
            .values()
            .find(|e| {
                e.parent_epic_id == root
                    && e.title == title
                    && e.origin == module::REPO_GROUP_ORIGIN
            })
            .map(|e| e.id);
        if let Some(id) = existing {
            return id;
        }
        let now = self.now();
        self.write_epic(
            tables,
            module::Epic {
                title: title.to_string(),
                parent_epic_id: root,
                origin: module::REPO_GROUP_ORIGIN.to_string(),
                created_by,
                created_at: now.clone(),
                updated_at: now,
                ..module::blank_epic()
            },
        )
    }

    fn grouping_root_of(tables: &Tables, epic_id: i64) -> Option<i64> {
        let epic = tables.epics.get(&epic_id)?;
        if Self::auto_groups(epic) {
            return Some(epic.id);
        }
        if epic.origin == module::REPO_GROUP_ORIGIN {
            let parent = tables.epics.get(&epic.parent_epic_id)?;
            if Self::auto_groups(parent) {
                return Some(parent.id);
            }
        }
        None
    }

    fn delete_if_empty_repo_group(&self, tables: &mut Tables, epic_id: i64) {
        let Some(epic) = tables.epics.get(&epic_id).cloned() else {
            return;
        };
        if epic.origin != module::REPO_GROUP_ORIGIN {
            return;
        }
        let has_tasks = tables.tasks.values().any(|t| t.epic_id == epic_id);
        let has_children = tables.epics.values().any(|e| e.parent_epic_id == epic_id);
        if has_tasks || has_children {
            self.recalculate_epic_chain(tables, epic_id);
            return;
        }
        if epic.parent_epic_id != 0 {
            self.rescope_learnings(tables, epic_id, epic.parent_epic_id);
        }
        let doomed = Self::collect_epic_subtree_ids(tables, epic_id);
        self.delete_epic_subtree_with_feed_cleanup(tables, epic_id, &doomed);
    }

    /// Mirrors the module's `relink_task`: the write, no recalculation.
    fn relink_task(
        &self,
        tables: &mut Tables,
        task: module::Task,
        target: i64,
    ) -> std::result::Result<(), String> {
        let id = task.id;
        let row = module::Task {
            epic_id: target,
            owner: String::new(),
            updated_at: self.now(),
            ..task
        };
        self.write_task(tables, row)?;
        self.clear_unseen_for_task(tables, id);
        Ok(())
    }

    /// Mirrors `reroute_on_repo_change`.
    pub(super) fn reroute_on_repo_change(
        &self,
        tables: &mut Tables,
        task_id: i64,
    ) -> std::result::Result<(), String> {
        let Some(task) = tables.tasks.get(&task_id).cloned() else {
            return Ok(());
        };
        let Some(root) = Self::grouping_root_of(tables, task.epic_id) else {
            return Ok(());
        };
        let current = task.epic_id;
        let target = self.route_to_repo_group(tables, root, &task.repo_path);
        if target == current {
            return Ok(());
        }
        self.relink_task(tables, task, target)?;
        if current != root {
            self.delete_if_empty_repo_group(tables, current);
        }
        self.recalculate_epic_chain(tables, target);
        Ok(())
    }

    /// Mirrors `regroup_epic`.
    pub(super) fn apply_regroup_epic(&self, root: EpicId) -> Result<ReducerOutcome> {
        let scope = self.scope();
        let mut tables = self.lock();
        if tables.visible_epic(&scope, root.0).is_none() {
            return Ok(ReducerOutcome::Applied(vec![]));
        }
        Ok(match self.regroup_epic_rows(&mut tables, root.0) {
            Ok(()) => ReducerOutcome::Applied(vec![]),
            Err(why) => ReducerOutcome::Refused(why),
        })
    }

    /// The body of `regroup_epic`, shared with `set_epic_group_by_repo`.
    pub(super) fn regroup_epic_rows(&self, tables: &mut Tables, root: i64) -> Result<(), String> {
        let tasks: Vec<module::Task> = tables
            .tasks
            .values()
            .filter(|t| t.epic_id == root)
            .cloned()
            .collect();
        let mut joined = std::collections::BTreeSet::new();
        for task in tasks {
            let target = self.route_to_repo_group(tables, root, &task.repo_path);
            if target != root {
                self.relink_task(tables, task, target)?;
                joined.insert(target);
            }
        }
        for sub in joined {
            if let Some(epic) = tables.epics.get(&sub).cloned() {
                self.recalculate_one(tables, &epic);
            }
        }
        self.recalculate_epic_chain(tables, root);
        Ok(())
    }

    /// Mirrors `set_epic_group_by_repo`: the flag write and, for a non-feed
    /// epic, the regroup or flatten, whole or not at all.
    ///
    /// The module leans on its rolled-back transaction; this double has none.
    /// The flag patch is dry-run first (`check_only`), so its refusals (the
    /// append-only one) leave nothing written; if the move then refuses, the
    /// epic row is put back as it was. Tasks the move had already relinked
    /// stay moved, which only a store-side refusal of `relink_task` could
    /// cause, and the conformance scenario does not reach one.
    pub(super) fn apply_set_epic_group_by_repo(
        &self,
        id: EpicId,
        group_by_repo: bool,
    ) -> Result<ReducerOutcome> {
        {
            let scope = self.scope();
            let tables = self.lock();
            if tables.visible_epic(&scope, id.0).is_none() {
                return Ok(ReducerOutcome::Refused(format!(
                    "epic {} does not exist",
                    id.0
                )));
            }
        }
        let patch = || bindings::EpicPatch {
            group_by_repo: Some(group_by_repo),
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            auto_dispatch: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            feed_role: None,
            origin: None,
            feed_append_only: None,
            completed_at: None,
        };
        let dry = self.patch_epic_row(id, patch(), true)?;
        if matches!(dry, ReducerOutcome::Refused(_)) {
            return Ok(dry);
        }
        let original = self.lock().epics.get(&id.0).cloned();
        let written = self.patch_epic_row(id, patch(), false)?;
        if matches!(written, ReducerOutcome::Refused(_)) {
            return Ok(written);
        }
        let scope = self.scope();
        let mut tables = self.lock();
        let Some(epic) = tables.visible_epic(&scope, id.0) else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        if !epic.feed_command.is_empty() {
            return Ok(ReducerOutcome::Applied(vec![]));
        }
        let moved = if group_by_repo {
            self.regroup_epic_rows(&mut tables, id.0)
        } else {
            self.flatten_epic_rows(&mut tables, id.0)
        };
        Ok(match moved {
            Ok(()) => ReducerOutcome::Applied(vec![]),
            Err(why) => {
                if let Some(original) = original {
                    tables.epics.insert(id.0, original);
                }
                ReducerOutcome::Refused(why)
            }
        })
    }

    /// Mirrors `flatten_epic`.
    pub(super) fn apply_flatten_epic(&self, root: EpicId) -> Result<ReducerOutcome> {
        let scope = self.scope();
        let mut tables = self.lock();
        if tables.visible_epic(&scope, root.0).is_none() {
            return Ok(ReducerOutcome::Applied(vec![]));
        }
        Ok(match self.flatten_epic_rows(&mut tables, root.0) {
            Ok(()) => ReducerOutcome::Applied(vec![]),
            Err(why) => ReducerOutcome::Refused(why),
        })
    }

    /// The body of `flatten_epic`, shared with `ingest_feed`'s flat path.
    pub(super) fn flatten_epic_rows(&self, tables: &mut Tables, root: i64) -> Result<(), String> {
        let subs: Vec<i64> = tables
            .epics
            .values()
            .filter(|e| e.parent_epic_id == root && e.origin == module::REPO_GROUP_ORIGIN)
            .map(|e| e.id)
            .collect();
        for sub in subs {
            let tasks: Vec<module::Task> = tables
                .tasks
                .values()
                .filter(|t| t.epic_id == sub)
                .cloned()
                .collect();
            for task in tasks {
                self.relink_task(tables, task, root)?;
            }
            self.delete_if_empty_repo_group(tables, sub);
        }
        self.recalculate_epic_chain(tables, root);
        Ok(())
    }
}
