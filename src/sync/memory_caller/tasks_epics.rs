//! Tasks and epics: row storage, the epic status chain, and the reducers that
//! create, patch, move and delete them.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{EpicId, TaskId};
use crate::spacetime::bindings;

use super::super::encode;
use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables, BACKLOG, DONE, MAX_EPIC_DEPTH};

impl MemoryReducerCaller {
    /// Mirrors the module's `write_task`: validates ownership, stamps
    /// `completed_at` on a row born `done`, assigns an id if none was given,
    /// and pushes the result to `SharedRows`. Held under `tables`'s lock by
    /// every caller.
    pub(super) fn write_task(
        &self,
        tables: &mut Tables,
        mut row: module::Task,
    ) -> Result<(), String> {
        module::validate_task_ownership(row.epic_id, &row.owner)
            .map_err(|why| format!("task {}: {why}", row.id))?;
        if row.status == DONE && row.completed_at.is_empty() {
            row.completed_at = self.now();
        }
        let id = Self::assign_id(row.id, &mut tables.next_task_id);
        row.id = id;
        tables.tasks.insert(id, row.clone());
        self.rows.upsert_task(&row.into());
        Ok(())
    }

    pub(super) fn delete_task_row(&self, tables: &mut Tables, id: i64) {
        if tables.tasks.remove(&id).is_some() {
            self.rows.remove_task(TaskId(id));
        }
    }

    pub(super) fn write_epic(&self, tables: &mut Tables, mut row: module::Epic) -> i64 {
        let id = Self::assign_id(row.id, &mut tables.next_epic_id);
        row.id = id;
        tables.epics.insert(id, row.clone());
        self.rows.upsert_epic(&row.into());
        id
    }

    pub(super) fn delete_epic_row(&self, tables: &mut Tables, id: i64) {
        if tables.epics.remove(&id).is_some() {
            self.rows.remove_epic(crate::models::EpicId(id));
        }
    }

    /// Mirrors the module's `recalculate_epic_chain`: walk from `epic_id` up
    /// through parents, recalculating each one from its own children, until
    /// the root or [`MAX_EPIC_DEPTH`].
    pub(super) fn recalculate_epic_chain(&self, tables: &mut Tables, epic_id: i64) {
        let mut next = epic_id;
        for _ in 0..MAX_EPIC_DEPTH {
            if next == 0 {
                return;
            }
            let Some(epic) = tables.epics.get(&next).cloned() else {
                return;
            };
            self.recalculate_one(tables, &epic);
            next = epic.parent_epic_id;
        }
    }

    /// Mirrors the module's `recalculate_one`, via the reused
    /// [`module::derive_epic_status`]/[`module::stamps_completion`].
    pub(super) fn recalculate_one(&self, tables: &mut Tables, epic: &module::Epic) {
        let children: Vec<String> = tables
            .tasks
            .values()
            .filter(|t| t.epic_id == epic.id)
            .map(|t| t.status.clone())
            .chain(
                tables
                    .epics
                    .values()
                    .filter(|e| e.parent_epic_id == epic.id)
                    .map(|e| e.status.clone()),
            )
            .collect();

        if let Some(target) = module::derive_epic_status(&epic.status, &children) {
            let completed_at = if module::stamps_completion(&epic.status, target) {
                self.now()
            } else {
                epic.completed_at.clone()
            };
            let updated = module::Epic {
                status: target.to_string(),
                completed_at,
                updated_at: self.now(),
                ..epic.clone()
            };
            tables.epics.insert(updated.id, updated.clone());
            self.rows.upsert_epic(&updated.into());
        }
    }

    /// Mirrors the module's `collect_epic_subtree_ids`: every epic id in
    /// `root`'s subtree, itself included.
    pub(super) fn collect_epic_subtree_ids(
        tables: &Tables,
        root: i64,
    ) -> std::collections::HashSet<i64> {
        let mut doomed = std::collections::HashSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !doomed.insert(id) {
                continue;
            }
            for child in tables.epics.values().filter(|e| e.parent_epic_id == id) {
                stack.push(child.id);
            }
        }
        doomed
    }

    /// Mirrors the module's `first_undone_task_in`: the first non-`done`
    /// task anywhere in `doomed`, or `None` when every task in the subtree
    /// qualifies (an empty subtree qualifies).
    pub(super) fn first_undone_task_in(
        tables: &Tables,
        doomed: &std::collections::HashSet<i64>,
    ) -> Option<i64> {
        tables
            .tasks
            .values()
            .find(|t| doomed.contains(&t.epic_id) && t.status != DONE)
            .map(|t| t.id)
    }

    /// Mirrors the module's `delete_epic_subtree`: recurse into sub-epics
    /// first, then delete this epic's own tasks, then this epic itself.
    pub(super) fn delete_epic_subtree(&self, tables: &mut Tables, id: i64, depth: usize) {
        if depth > MAX_EPIC_DEPTH {
            return;
        }
        let children: Vec<i64> = tables
            .epics
            .values()
            .filter(|e| e.parent_epic_id == id)
            .map(|e| e.id)
            .collect();
        for child in children {
            self.delete_epic_subtree(tables, child, depth + 1);
        }
        let tasks: Vec<i64> = tables
            .tasks
            .values()
            .filter(|t| t.epic_id == id)
            .map(|t| t.id)
            .collect();
        for task_id in tasks {
            self.delete_task_row(tables, task_id);
            self.delete_task_side_effects(tables, task_id);
        }
        self.delete_epic_row(tables, id);
    }

    pub(super) fn apply_create_task(&self, row: bindings::Task) -> Result<TaskId> {
        let epic_id = row.epic_id;
        let mut tables = self.lock();
        self.write_task(
            &mut tables,
            module::Task {
                id: 0,
                ..module::Task::from(row)
            },
        )
        // Same wording `SdkReducerCaller::create_task`'s `generated_id` uses
        // for a real refusal, so a caller cannot tell the two backends apart
        // from the error text alone.
        .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        // The id `write_task` just assigned — read back off the table rather
        // than threaded out of `write_task`, since every other caller of it
        // (patch/claim/release) already knows its own id and does not need one
        // back.
        let id = tables.next_task_id - 1;
        self.recalculate_epic_chain(&mut tables, epic_id);
        Ok(TaskId(id))
    }

    pub(super) fn apply_patch_task(
        &self,
        id: TaskId,
        patch: bindings::TaskPatch,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let Some(mut row) = tables.tasks.get(&id.0).cloned() else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        let was_in_epic = row.epic_id;
        let prior_status = row.status.clone();
        let patch = module::TaskPatch::from(patch);
        let caller_named_a_completion = patch.completed_at.is_some();
        let moved_to_epic = patch.epic_id;

        module::apply_task_patch(&mut row, patch);

        if !caller_named_a_completion && module::stamps_completion(&prior_status, &row.status) {
            row.completed_at = self.now();
        }
        row.updated_at = self.now();

        let now_in_epic = moved_to_epic.unwrap_or(was_in_epic);
        let status_changed = prior_status != row.status;
        if let Err(why) = self.write_task(&mut tables, row) {
            return Ok(ReducerOutcome::Refused(why));
        }

        if status_changed || now_in_epic != was_in_epic {
            self.recalculate_epic_chain(&mut tables, was_in_epic);
            if now_in_epic != was_in_epic {
                self.recalculate_epic_chain(&mut tables, now_in_epic);
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_task(&self, id: TaskId) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let Some(row) = tables.tasks.get(&id.0).cloned() else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        // Mirrors the module's `delete_task` guard (task #4971): only a
        // `done` task may be permanently deleted. Checked here too, not only
        // by the caller, on the same reasoning the module's own doc comment
        // gives for re-checking server-side.
        if row.status != DONE {
            return Ok(ReducerOutcome::Refused(format!(
                "task {}: cannot delete because it is not done",
                id.0
            )));
        }
        let epic_id = row.epic_id;
        self.retire_task_if_feed_backed(&mut tables, epic_id, &row.external_id);
        self.delete_task_row(&mut tables, id.0);
        self.delete_task_side_effects(&mut tables, id.0);
        self.recalculate_epic_chain(&mut tables, epic_id);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_set_task_epic(
        &self,
        id: TaskId,
        epic_id: Option<EpicId>,
        owner: String,
    ) -> Result<ReducerOutcome> {
        let epic_id = encode::epic_ref(epic_id);
        let mut tables = self.lock();
        let Some(task) = tables.tasks.get(&id.0).cloned() else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        let was_in = task.epic_id;
        if was_in == epic_id {
            return Ok(ReducerOutcome::Applied(vec![]));
        }
        let updated_at = self.now();
        let row = module::Task {
            epic_id,
            owner: if epic_id == 0 { owner } else { String::new() },
            updated_at,
            ..task
        };
        if let Err(why) = self.write_task(&mut tables, row) {
            return Ok(ReducerOutcome::Refused(why));
        }
        self.recalculate_epic_chain(&mut tables, was_in);
        self.recalculate_epic_chain(&mut tables, epic_id);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_claim_backlog_task(
        &self,
        id: TaskId,
        host: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let Some(task) = tables.tasks.get(&id.0).cloned() else {
            return Ok(ReducerOutcome::Refused(format!(
                "task {} no longer exists",
                id.0
            )));
        };
        if task.status != BACKLOG {
            return Ok(ReducerOutcome::Refused(format!(
                "task {} is {} rather than backlog, so it is already claimed",
                id.0, task.status
            )));
        }
        if !module::claimable_by(&task, &host) {
            return Ok(ReducerOutcome::Refused(format!(
                "task {}'s worktree is on {}, so {host} cannot claim it",
                id.0, task.host
            )));
        }
        let now = self.now();
        let row = module::Task {
            status: "running".into(),
            sub_status: "active".into(),
            last_pre_tool_use_at: now.clone(),
            updated_at: now,
            ..task
        };
        match self.write_task(&mut tables, row) {
            Ok(()) => Ok(ReducerOutcome::Applied(vec![])),
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_release_backlog_claim(&self, id: TaskId) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let Some(task) = tables.tasks.get(&id.0).cloned() else {
            return Ok(ReducerOutcome::Refused(format!(
                "task {} no longer exists",
                id.0
            )));
        };
        if task.status != "running" {
            return Ok(ReducerOutcome::Refused(format!(
                "task {} is not claimed",
                id.0
            )));
        }
        if !task.worktree.is_empty() {
            return Ok(ReducerOutcome::Refused(format!(
                "task {} already has a worktree, so releasing it would put a running \
                 agent's task back in the backlog",
                id.0
            )));
        }
        let updated_at = self.now();
        let row = module::Task {
            status: BACKLOG.into(),
            sub_status: "none".into(),
            last_pre_tool_use_at: String::new(),
            updated_at,
            ..task
        };
        match self.write_task(&mut tables, row) {
            Ok(()) => Ok(ReducerOutcome::Applied(vec![])),
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_create_epic(&self, row: bindings::Epic) -> Result<EpicId> {
        let mut tables = self.lock();
        let parent = row.parent_epic_id;
        let id = self.write_epic(
            &mut tables,
            module::Epic {
                id: 0,
                ..module::Epic::from(row)
            },
        );
        self.recalculate_epic_chain(&mut tables, parent);
        Ok(EpicId(id))
    }

    pub(super) fn apply_patch_epic(
        &self,
        id: EpicId,
        patch: bindings::EpicPatch,
    ) -> Result<ReducerOutcome> {
        let id = id.0;
        let mut tables = self.lock();
        let Some(mut row) = tables.epics.get(&id).cloned() else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        let prior_status = row.status.clone();
        let was_child_of = row.parent_epic_id;
        let patch = module::EpicPatch::from(patch);
        let caller_named_a_completion = patch.completed_at.is_some();

        module::apply_epic_patch(&mut row, patch);
        if !caller_named_a_completion && module::stamps_completion(&prior_status, &row.status) {
            row.completed_at = self.now();
        }
        row.updated_at = self.now();
        self.write_epic(&mut tables, row);

        self.recalculate_epic_chain(&mut tables, id);
        if was_child_of != 0 {
            self.recalculate_epic_chain(&mut tables, was_child_of);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_epic(&self, id: EpicId) -> Result<ReducerOutcome> {
        let id = id.0;
        let mut tables = self.lock();
        let Some(row) = tables.epics.get(&id).cloned() else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        // Mirrors the module's `delete_epic` guard (task #4971): every task
        // anywhere in the subtree must be `done`.
        let doomed = Self::collect_epic_subtree_ids(&tables, id);
        if let Some(undone_task_id) = Self::first_undone_task_in(&tables, &doomed) {
            return Ok(ReducerOutcome::Refused(format!(
                "epic {id}: cannot delete while task {undone_task_id} in its subtree is not done"
            )));
        }
        let parent = row.parent_epic_id;
        self.retire_feed_tasks_before_epic_delete(&mut tables, &doomed);
        self.delete_epic_subtree(&mut tables, id, 0);
        self.drop_retired_feed_items_for_epics(&mut tables, &doomed);
        self.recalculate_epic_chain(&mut tables, parent);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_recalculate_epic_status(&self, id: EpicId) -> Result<ReducerOutcome> {
        let id = id.0;
        let mut tables = self.lock();
        self.recalculate_epic_chain(&mut tables, id);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `batch_delete` (task #4971): validate every
    /// selected epic's whole subtree and every selected plain task FIRST,
    /// mutating nothing until all of them pass, then perform every delete
    /// together.
    pub(super) fn apply_batch_delete(
        &self,
        task_ids: Vec<TaskId>,
        epic_ids: Vec<EpicId>,
    ) -> Result<ReducerOutcome> {
        let task_ids: Vec<i64> = task_ids.into_iter().map(|id| id.0).collect();
        let epic_ids: Vec<i64> = epic_ids.into_iter().map(|id| id.0).collect();
        let mut tables = self.lock();

        let mut epic_doomed: Vec<(i64, std::collections::HashSet<i64>)> = Vec::new();
        for &id in &epic_ids {
            if !tables.epics.contains_key(&id) {
                continue;
            }
            let doomed = Self::collect_epic_subtree_ids(&tables, id);
            if let Some(undone_task_id) = Self::first_undone_task_in(&tables, &doomed) {
                return Ok(ReducerOutcome::Refused(format!(
                    "epic {id}: cannot delete while task {undone_task_id} in its subtree is not done"
                )));
            }
            epic_doomed.push((id, doomed));
        }
        for &id in &task_ids {
            if let Some(row) = tables.tasks.get(&id) {
                if row.status != DONE {
                    return Ok(ReducerOutcome::Refused(format!(
                        "task {id}: cannot delete because it is not done"
                    )));
                }
            }
        }

        let mut recalc: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for (id, doomed) in &epic_doomed {
            let Some(row) = tables.epics.get(id).cloned() else {
                // A nested selection: an earlier epic in this same batch
                // already deleted it as part of its own subtree.
                continue;
            };
            recalc.insert(row.parent_epic_id);
            self.retire_feed_tasks_before_epic_delete(&mut tables, doomed);
            self.delete_epic_subtree(&mut tables, *id, 0);
            self.drop_retired_feed_items_for_epics(&mut tables, doomed);
        }
        for &id in &task_ids {
            let Some(row) = tables.tasks.get(&id).cloned() else {
                // Already gone: one of the doomed epics above owned it too.
                continue;
            };
            recalc.insert(row.epic_id);
            self.retire_task_if_feed_backed(&mut tables, row.epic_id, &row.external_id);
            self.delete_task_row(&mut tables, id);
            self.delete_task_side_effects(&mut tables, id);
        }
        for epic_id in recalc {
            self.recalculate_epic_chain(&mut tables, epic_id);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }
}
