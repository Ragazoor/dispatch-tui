//! Tasks and epics: row storage, the epic status chain, and the reducers that
//! create, patch, move and delete them.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{EpicId, TaskId};
use crate::spacetime::bindings;

use super::super::encode;
use super::super::writes::{ReducerOutcome, StoreRefusal};
use super::{MemoryReducerCaller, Tables, DONE, MAX_EPIC_DEPTH};

impl MemoryReducerCaller {
    /// Mirrors the module's `write_task`: validates ownership and status,
    /// stamps `completed_at` on a row born `done`, assigns an id if none was
    /// given, and pushes the result to `SharedRows`. Held under `tables`'s lock
    /// by every caller.
    pub(super) fn write_task(
        &self,
        tables: &mut Tables,
        mut row: module::Task,
    ) -> Result<(), String> {
        Self::validate_task_write(tables, &row, &self.now())?;
        // Mirrors the module's `store_task`: a created row is armed from its
        // own schedule (schedules.allium: SetSchedule).
        if row.id == 0 {
            row = module::arm_created_task(row, &self.now())?;
        }
        if row.status == DONE && row.completed_at.is_empty() {
            row.completed_at = self.now();
        }
        // The six live-session facts are the current agent's since A4.
        let mut row = module::without_session_facts(row);
        let id = Self::assign_id(row.id, &mut tables.next_task_id);
        row.id = id;
        let prior = tables.tasks.insert(id, row.clone());
        // Mirrors the module's `write_task`: EndAgentWhenWorktreeGoes
        // (agents.allium).
        module::follow_task_write(&mut self.agent_store(tables), prior.as_ref(), &row);
        self.rows.upsert_task(&row.into());
        Ok(())
    }

    /// Every refusal [`Self::write_task`] can make, and nothing else.
    ///
    /// The module's reducer rolls its whole transaction back on an `Err`; this
    /// double has no rollback (its writes reach `SharedRows` as they happen),
    /// so a reducer body asks here BEFORE its first side effect whenever a
    /// later `write_task` could refuse. A create (`row.id == 0`) is also
    /// armed from its schedule, which refuses a cron that never matches.
    pub(super) fn validate_task_write(
        tables: &Tables,
        row: &module::Task,
        now: &str,
    ) -> Result<(), String> {
        module::validate_task_ownership(row.epic_id, &row.owner)
            .map_err(|why| format!("task {}: {why}", row.id))?;
        module::validate_status("task", row.id, &row.status)?;
        module::check_sub_status_write(tables.tasks.get(&row.id), row)?;
        if row.id == 0 {
            module::arm_created_task(row.clone(), now)?;
        }
        Ok(())
    }

    pub(super) fn delete_task_row(&self, tables: &mut Tables, id: i64) {
        if tables.tasks.remove(&id).is_some() {
            self.rows.remove_task(TaskId(id));
        }
    }

    pub(super) fn write_epic(&self, tables: &mut Tables, mut row: module::Epic) -> i64 {
        // The module's `inherit_workspace`, for every epic a module path
        // inserts under a parent (feed grouping, role routing, repo groups).
        if row.id == 0 && row.parent_epic_id != 0 {
            if let Some(parent) = tables.epics.get(&row.parent_epic_id) {
                row.workspace_id = parent.workspace_id;
            }
        }
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

    /// Mirrors the module's `validate_epic_deletable` depth check: a subtree
    /// nested deeper than [`MAX_EPIC_DEPTH`] is refused, not half-deleted.
    pub(super) fn subtree_too_deep(tables: &Tables, id: i64) -> Option<String> {
        module::subtree_exceeds_depth(
            id,
            |epic| {
                tables
                    .epics
                    .values()
                    .filter(|e| e.parent_epic_id == epic)
                    .map(|e| e.id)
                    .collect()
            },
            MAX_EPIC_DEPTH,
        )
        .then(|| {
            format!(
                "epic {id}: cannot delete a subtree nested more than {MAX_EPIC_DEPTH} levels deep"
            )
        })
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
        self.delete_work_package_links_to(tables, module::WORK_PACKAGE_ITEM_EPIC, id);
    }

    /// Mirrors the module's `delete_epic_subtree_with_feed_cleanup`.
    pub(super) fn delete_epic_subtree_with_feed_cleanup(
        &self,
        tables: &mut Tables,
        id: i64,
        doomed: &std::collections::HashSet<i64>,
    ) {
        self.retire_feed_tasks_before_epic_delete(tables, doomed);
        self.delete_epic_subtree(tables, id, 0);
        self.drop_retired_feed_items_for_epics(tables, doomed);
    }

    pub(super) fn apply_create_task(&self, row: bindings::Task) -> Result<TaskId> {
        let scope = self.scope();
        let mut tables = self.lock();
        let row = module::Task {
            id: 0,
            ..module::Task::from(row)
        };
        tables
            .check_task_may_enter_epic(&scope, "task", row.epic_id)
            .map_err(StoreRefusal)?;
        // Before routing, which can create a repo sub-epic: the module rolls
        // that back with a refused write, and this double has no rollback.
        Self::validate_task_write(&tables, &row, &self.now()).map_err(StoreRefusal)?;
        let epic_id = self.route_to_repo_group(&mut tables, row.epic_id, &row.repo_path);
        self.write_task(&mut tables, module::Task { epic_id, ..row })
            // Same wording `SdkReducerCaller::create_task`'s `generated_id` uses
            // for a real refusal, so a caller cannot tell the two backends apart
            // from the error text alone.
            .map_err(StoreRefusal)?;
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
        self.patch_task_row(id, patch, false)
    }

    /// `apply_patch_task`'s body; `store_owns_completion` is the module's
    /// `patch_task_row` flag, set only by `update_task`.
    fn patch_task_row(
        &self,
        id: TaskId,
        patch: bindings::TaskPatch,
        store_owns_completion: bool,
    ) -> Result<ReducerOutcome> {
        let scope = self.scope();
        let mut tables = self.lock();
        let Some(mut row) = tables.visible_task(&scope, id.0) else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        let patch = module::TaskPatch::from(patch);
        if let Err(why) = module::check_task_patch(&row, &patch) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let has_agent = module::current_agent(&self.agent_store(&mut tables), id.0).is_some();
        if let Err(why) = module::check_session_patch(has_agent, &patch) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let was_in_epic = row.epic_id;
        let prior_status = row.status.clone();
        let caller_named_a_completion = patch.completed_at.is_some();
        let caller_named_stop_pending = patch.stop_pending.is_some();
        let session_patch = patch.clone();
        let schedule = patch.schedule.clone();
        let repo_changed_in_place = patch
            .repo_path
            .as_deref()
            .is_some_and(|repo| repo != row.repo_path);

        module::apply_task_patch(&mut row, patch);
        // Mirrors the module: after the rest of the patch, before any write.
        if let Some(cron) = schedule {
            row = match module::apply_schedule(row, &cron, &self.now()) {
                Ok(row) => row,
                Err(why) => return Ok(ReducerOutcome::Refused(why)),
            };
        }
        row = module::schedule_after_status_write(&prior_status, row, &self.now());
        // Before the first side effect below: a refused patch leaves nothing
        // behind, as the module's rolled-back transaction does.
        if let Err(why) = Self::validate_task_write(&tables, &row, &self.now()) {
            return Ok(ReducerOutcome::Refused(why));
        }
        // The session facts go to the current agent (agents.allium, A4).
        module::update_current_agent(&mut self.agent_store(&mut tables), id.0, |agent| {
            module::apply_session_patch(agent, &session_patch)
        });

        if (store_owns_completion || !caller_named_a_completion)
            && module::stamps_completion(&prior_status, &row.status)
        {
            row.completed_at = self.now();
        }
        row.updated_at = self.now();
        // store-split.allium X2: the module's two transition rules on a patch.
        if module::patch_voids_pending_stop(&prior_status, &row.status, caller_named_stop_pending) {
            module::update_current_agent(&mut self.agent_store(&mut tables), id.0, |agent| {
                module::Agent {
                    stop_pending: false,
                    ..agent
                }
            });
        }
        let stamp = self.now();
        if module::should_respawn_phoenix(&prior_status, &row)
            && self.respawn_phoenix_successor_of(&mut tables, &row, &stamp)
        {
            row.phoenix = false;
        }

        let status_changed = prior_status != row.status;
        let finished = status_changed && row.status == DONE;
        if let Err(why) = self.write_task(&mut tables, row) {
            return Ok(ReducerOutcome::Refused(why));
        }
        if finished {
            self.clear_unseen_for_task(&mut tables, id.0);
        }

        if status_changed {
            self.recalculate_epic_chain(&mut tables, was_in_epic);
        }
        if repo_changed_in_place {
            if let Err(why) = self.reroute_on_repo_change(&mut tables, id.0) {
                return Ok(ReducerOutcome::Refused(why));
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `update_task`: the relink and the patch of one
    /// task, whole or not at all.
    ///
    /// The module leans on its rolled-back transaction; this double has none
    /// (see [`Self::validate_task_write`]), so every refusal either half can
    /// make is raised here BEFORE the first write, by running the patch's
    /// validation against a copy of the row. The two writes that follow, in
    /// the module's order, then cannot refuse.
    pub(super) fn apply_update_task(
        &self,
        id: TaskId,
        epic_id: Option<i64>,
        owner: String,
        patch: bindings::TaskPatch,
    ) -> Result<ReducerOutcome> {
        {
            let scope = self.scope();
            let mut tables = self.lock();
            let Some(row) = tables.visible_task(&scope, id.0) else {
                return Ok(ReducerOutcome::Refused(format!("task {} not found", id.0)));
            };
            if let Some(epic_id) = epic_id {
                if epic_id != 0 && !tables.epics.contains_key(&epic_id) {
                    return Ok(ReducerOutcome::Refused(format!(
                        "task {}: epic {epic_id} does not exist",
                        id.0
                    )));
                }
                if let Err(why) =
                    tables.check_task_may_enter_epic(&scope, &format!("task {}", id.0), epic_id)
                {
                    return Ok(ReducerOutcome::Refused(why));
                }
            }
            let patch = module::TaskPatch::from(patch.clone());
            if let Err(why) = self.check_patch_would_apply(&mut tables, id, row, patch) {
                return Ok(ReducerOutcome::Refused(why));
            }
        }
        let relink_first = patch.repo_path.is_some();
        let Some(epic_id) = epic_id else {
            return self.patch_task_row(id, patch, true);
        };
        let epic = (epic_id != 0).then_some(EpicId(epic_id));
        if relink_first {
            let moved = self.apply_set_task_epic(id, epic, owner)?;
            if matches!(moved, ReducerOutcome::Refused(_)) {
                return Ok(moved);
            }
            self.patch_task_row(id, patch, true)
        } else {
            let patched = self.patch_task_row(id, patch, true)?;
            if matches!(patched, ReducerOutcome::Refused(_)) {
                return Ok(patched);
            }
            self.apply_set_task_epic(id, epic, owner)
        }
    }

    /// The refusals `apply_patch_task` can make, read off a copy of `row`:
    /// the same checks in the same order, with no write.
    fn check_patch_would_apply(
        &self,
        tables: &mut Tables,
        id: TaskId,
        mut row: module::Task,
        patch: module::TaskPatch,
    ) -> Result<(), String> {
        module::check_task_patch(&row, &patch)?;
        let has_agent = module::current_agent(&self.agent_store(tables), id.0).is_some();
        module::check_session_patch(has_agent, &patch)?;
        let prior_status = row.status.clone();
        let schedule = patch.schedule.clone();
        module::apply_task_patch(&mut row, patch);
        if let Some(cron) = schedule {
            row = module::apply_schedule(row, &cron, &self.now())?;
        }
        row = module::schedule_after_status_write(&prior_status, row, &self.now());
        Self::validate_task_write(tables, &row, &self.now())
    }

    pub(super) fn apply_delete_task(&self, id: TaskId) -> Result<ReducerOutcome> {
        let scope = self.scope();
        let mut tables = self.lock();
        let Some(row) = tables.visible_task(&scope, id.0) else {
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
        let scope = self.scope();
        let mut tables = self.lock();
        let Some(task) = tables.visible_task(&scope, id.0) else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        if epic_id != 0 && !tables.epics.contains_key(&epic_id) {
            return Ok(ReducerOutcome::Refused(format!(
                "task {}: epic {epic_id} does not exist",
                id.0
            )));
        }
        if let Err(why) =
            tables.check_task_may_enter_epic(&scope, &format!("task {}", id.0), epic_id)
        {
            return Ok(ReducerOutcome::Refused(why));
        }
        // Routed first, then compared, as the module does.
        let epic_id = self.route_to_repo_group(&mut tables, epic_id, &task.repo_path);
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
        self.clear_unseen_for_task(&mut tables, id.0);
        self.recalculate_epic_chain(&mut tables, was_in);
        self.recalculate_epic_chain(&mut tables, epic_id);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_create_epic(&self, row: bindings::Epic) -> Result<EpicId> {
        let scope = self.scope();
        let mut tables = self.lock();
        // The module validates on its two epic reducers, not on every epic
        // insert; the double mirrors that placement.
        module::validate_status("epic", row.id, &row.status).map_err(anyhow::Error::msg)?;
        let row = tables
            .settle_new_epic(&scope, module::Epic::from(row))
            .map_err(StoreRefusal)?;
        let parent = row.parent_epic_id;
        let id = self.write_epic(&mut tables, module::Epic { id: 0, ..row });
        self.recalculate_epic_chain(&mut tables, parent);
        Ok(EpicId(id))
    }

    pub(super) fn apply_patch_epic(
        &self,
        id: EpicId,
        patch: bindings::EpicPatch,
    ) -> Result<ReducerOutcome> {
        let id = id.0;
        let scope = self.scope();
        let mut tables = self.lock();
        let Some(mut row) = tables.visible_epic(&scope, id) else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        let patch = module::EpicPatch::from(patch);
        if let Some(parent) = patch.parent_epic_id {
            if let Err(why) = tables.check_epic_move(&scope, &row, parent) {
                return Ok(ReducerOutcome::Refused(why));
            }
        }
        if let Err(why) = module::check_epic_patch(&row, &patch, |epic| {
            tables.epics.get(&epic).map(|e| e.parent_epic_id)
        }) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let prior_status = row.status.clone();
        let was_child_of = row.parent_epic_id;
        let caller_named_a_completion = patch.completed_at.is_some();

        module::apply_epic_patch(&mut row, patch);
        if !caller_named_a_completion && module::stamps_completion(&prior_status, &row.status) {
            row.completed_at = self.now();
        }
        row.updated_at = self.now();
        if let Err(why) = module::validate_status("epic", row.id, &row.status) {
            return Ok(ReducerOutcome::Refused(why));
        }
        self.write_epic(&mut tables, row);

        self.recalculate_epic_chain(&mut tables, id);
        if was_child_of != 0 {
            self.recalculate_epic_chain(&mut tables, was_child_of);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_epic(&self, id: EpicId) -> Result<ReducerOutcome> {
        let id = id.0;
        let scope = self.scope();
        let mut tables = self.lock();
        let Some(row) = tables.visible_epic(&scope, id) else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        // Sight first, so a refusal below names nothing a private sub-epic
        // holds. Then the module's `delete_epic` guard (task #4971): every
        // task anywhere in the subtree must be `done`.
        let doomed = Self::collect_epic_subtree_ids(&tables, id);
        if let Err(why) = tables.check_subtree_all_seen(&scope, id, &doomed) {
            return Ok(ReducerOutcome::Refused(why));
        }
        if let Some(why) = Self::subtree_too_deep(&tables, id) {
            return Ok(ReducerOutcome::Refused(why));
        }
        if let Some(undone_task_id) = Self::first_undone_task_in(&tables, &doomed) {
            return Ok(ReducerOutcome::Refused(format!(
                "epic {id}: cannot delete while task {undone_task_id} in its subtree is not done"
            )));
        }
        let parent = row.parent_epic_id;
        self.delete_epic_subtree_with_feed_cleanup(&mut tables, id, &doomed);
        self.recalculate_epic_chain(&mut tables, parent);
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
        let scope = self.scope();
        let mut tables = self.lock();

        let mut epic_doomed: Vec<(i64, std::collections::HashSet<i64>)> = Vec::new();
        for &id in &epic_ids {
            if tables.visible_epic(&scope, id).is_none() {
                continue;
            }
            let doomed = Self::collect_epic_subtree_ids(&tables, id);
            if let Err(why) = tables.check_subtree_all_seen(&scope, id, &doomed) {
                return Ok(ReducerOutcome::Refused(why));
            }
            if let Some(why) = Self::subtree_too_deep(&tables, id) {
                return Ok(ReducerOutcome::Refused(why));
            }
            if let Some(undone_task_id) = Self::first_undone_task_in(&tables, &doomed) {
                return Ok(ReducerOutcome::Refused(format!(
                    "epic {id}: cannot delete while task {undone_task_id} in its subtree is not done"
                )));
            }
            epic_doomed.push((id, doomed));
        }
        for &id in &task_ids {
            if let Some(row) = tables.visible_task(&scope, id) {
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
            self.delete_epic_subtree_with_feed_cleanup(&mut tables, *id, doomed);
        }
        for &id in &task_ids {
            let Some(row) = tables.visible_task(&scope, id) else {
                // Already gone, or never the caller's to see.
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
