//! Agent session state, task watchers, poll ownership, host registry and the
//! task/epic-delete side effects that reach them.

use anyhow::Result;
use chrono::{DateTime, Utc};

use dispatch_spacetime_module as module;

use crate::models::{NotificationWrite, PollScopeId, SubStatus, TaskId};
use crate::spacetime::bindings;

use super::super::encode;
use super::super::writes::{DrainReadBack, ReducerOutcome};
use super::{MemoryReducerCaller, Tables, ACTIVE, AWAITING_REVIEW, NEEDS_INPUT, REVIEW, RUNNING};

impl MemoryReducerCaller {
    /// Delete every `task_watchers` row naming `task_id` in either direction,
    /// every `task_subagents` row for it, and detach/cascade its learnings.
    /// Mirrors the module's `delete_task_side_effects` — called at every
    /// point that deletes a task row, exactly the set of call sites the
    /// module itself uses (`delete_task`, `batch_delete`'s task pass,
    /// `delete_epic_subtree`, `delete_stale_feed_tasks_in_epic`), on the same
    /// reasoning its own doc comment gives for keeping this in one place
    /// rather than four hand-copied ones.
    pub(super) fn delete_task_side_effects(&self, tables: &mut Tables, task_id: i64) {
        tables.task_subagents.retain(|s| s.task_id != task_id);
        let watches: Vec<i64> = tables
            .task_watchers
            .values()
            .filter(|w| w.watcher_task_id == task_id || w.target_task_id == task_id)
            .map(|w| w.id)
            .collect();
        self.delete_watcher_rows(tables, watches);
        self.detach_learnings_from_task(tables, task_id);
    }

    pub(super) fn delete_watcher_rows(&self, tables: &mut Tables, ids: Vec<i64>) {
        for id in ids {
            if tables.task_watchers.remove(&id).is_some() {
                self.rows.remove_task_watcher(id);
            }
        }
    }

    /// Recompute `live_subagents` from `task_subagents` and write it, if the
    /// task still exists. Mirrors the module's `sync_subagent_count`.
    pub(super) fn sync_subagent_count(
        &self,
        tables: &mut Tables,
        task_id: i64,
    ) -> Result<i64, String> {
        let count = tables
            .task_subagents
            .iter()
            .filter(|s| s.task_id == task_id)
            .count() as i64;
        if let Some(row) = tables.tasks.get(&task_id).cloned() {
            self.write_task(
                tables,
                module::Task {
                    live_subagents: count,
                    ..row
                },
            )?;
        }
        Ok(count)
    }

    /// Evict `task_subagents` rows for `task_id` whose `session_id` differs
    /// from `incoming`. Mirrors the module's `fence_subagent_session`.
    pub(super) fn fence_subagent_session(tables: &mut Tables, task_id: i64, incoming: &str) {
        tables
            .task_subagents
            .retain(|r| r.task_id != task_id || r.session_id == incoming);
    }

    pub(super) fn delete_subagent_entry(tables: &mut Tables, task_id: i64, agent_id: &str) {
        tables
            .task_subagents
            .retain(|r| !(r.task_id == task_id && r.agent_id == agent_id));
    }

    pub(super) fn delete_all_subagents(tables: &mut Tables, task_id: i64) {
        tables.task_subagents.retain(|r| r.task_id != task_id);
    }

    /// Apply a deferred `Stop` if this write is the one that drained the last
    /// subagent. Mirrors the module's `apply_pending_stop_if_drained`.
    pub(super) fn apply_pending_stop_if_drained(
        &self,
        tables: &mut Tables,
        task_id: i64,
    ) -> Result<bool, String> {
        let Some(row) = tables.tasks.get(&task_id).cloned() else {
            return Ok(false);
        };
        if row.status == RUNNING && row.stop_pending && row.live_subagents == 0 {
            self.flip_to_review(tables, row)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Flip `row` to `Review`, clearing the hook-activity timestamps and the
    /// deferred-Stop bit, and recalculate the epic that leaves as a
    /// derivation input. Mirrors the module's `flip_to_review`.
    pub(super) fn flip_to_review(
        &self,
        tables: &mut Tables,
        row: module::Task,
    ) -> Result<(), String> {
        let epic_id = row.epic_id;
        self.write_task(
            tables,
            module::Task {
                status: REVIEW.into(),
                sub_status: AWAITING_REVIEW.into(),
                last_pre_tool_use_at: String::new(),
                last_notification_at: String::new(),
                stop_pending: false,
                ..row
            },
        )?;
        self.recalculate_epic_chain(tables, epic_id);
        Ok(())
    }

    /// `[live, task_is_now_in_review]` off the task a drain acted on. Mirrors
    /// `SdkReducerCaller`'s own `subagent_drain_read_back`.
    pub(super) fn drain_read_back(tables: &Tables, task_id: i64) -> DrainReadBack {
        match tables.tasks.get(&task_id) {
            Some(t) => DrainReadBack {
                live: t.live_subagents,
                is_review: t.status == REVIEW,
            },
            None => DrainReadBack::default(),
        }
    }

    /// Find-or-create-or-reassign a [`module::PollOwner`] row for
    /// `(scope, scope_id)`. Mirrors the module's `write_poll_owner_row`.
    pub(super) fn write_poll_owner_row(
        &self,
        tables: &mut Tables,
        scope: &str,
        scope_id: i64,
        host: String,
        force: bool,
    ) {
        let existing = tables
            .poll_owners
            .values()
            .find(|p| p.scope_id == scope_id && p.scope == scope)
            .cloned();
        match existing {
            Some(row) if force => {
                let updated = module::PollOwner {
                    host,
                    claimed_at: self.now(),
                    ..row
                };
                tables.poll_owners.insert(updated.id, updated.clone());
                self.rows.upsert_poll_owner(&updated.into());
            }
            Some(_) => {}
            None => {
                let id = Self::assign_id(0, &mut tables.next_poll_owner_id);
                let row = module::PollOwner {
                    id,
                    scope: scope.to_string(),
                    scope_id,
                    host,
                    claimed_at: self.now(),
                };
                tables.poll_owners.insert(id, row.clone());
                self.rows.upsert_poll_owner(&row.into());
            }
        }
    }

    pub(super) fn apply_subagent_start(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
        started_at: DateTime<Utc>,
    ) -> Result<i64> {
        let task_id = task_id.0;
        let started_at = encode::subagent_started_at(started_at);
        let mut tables = self.lock();
        Self::fence_subagent_session(&mut tables, task_id, &session_id);
        Self::delete_subagent_entry(&mut tables, task_id, &agent_id);
        tables.task_subagents.push(module::TaskSubagent {
            task_id,
            agent_id,
            session_id,
            started_at,
        });
        self.sync_subagent_count(&mut tables, task_id)
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))
    }

    pub(super) fn apply_subagent_stop(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
    ) -> Result<DrainReadBack> {
        let task_id = task_id.0;
        let mut tables = self.lock();
        Self::fence_subagent_session(&mut tables, task_id, &session_id);
        Self::delete_subagent_entry(&mut tables, task_id, &agent_id);
        self.sync_subagent_count(&mut tables, task_id)
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        self.apply_pending_stop_if_drained(&mut tables, task_id)
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        Ok(Self::drain_read_back(&tables, task_id))
    }

    pub(super) fn apply_subagent_clear(&self, task_id: TaskId) -> Result<DrainReadBack> {
        let task_id = task_id.0;
        let mut tables = self.lock();
        Self::delete_all_subagents(&mut tables, task_id);
        self.sync_subagent_count(&mut tables, task_id)
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        self.apply_pending_stop_if_drained(&mut tables, task_id)
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        Ok(Self::drain_read_back(&tables, task_id))
    }

    pub(super) fn apply_subagent_clear_and_void_pending_stop(
        &self,
        task_id: TaskId,
    ) -> Result<ReducerOutcome> {
        let task_id = task_id.0;
        let mut tables = self.lock();
        Self::delete_all_subagents(&mut tables, task_id);
        if let Err(why) = self.sync_subagent_count(&mut tables, task_id) {
            return Ok(ReducerOutcome::Refused(why));
        }
        if let Some(row) = tables.tasks.get(&task_id).cloned() {
            if let Err(why) = self.write_task(
                &mut tables,
                module::Task {
                    stop_pending: false,
                    ..row
                },
            ) {
                return Ok(ReducerOutcome::Refused(why));
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_try_record_stop(
        &self,
        id: TaskId,
        stop_pending_at: DateTime<Utc>,
    ) -> Result<Option<bool>> {
        let id = id.0;
        let stop_pending_at = encode::stamp(stop_pending_at);
        let mut tables = self.lock();
        let Some(row) = tables.tasks.get(&id).cloned() else {
            return Ok(None);
        };
        if row.status != RUNNING {
            return Ok(None);
        }
        if row.live_subagents == 0 {
            self.flip_to_review(&mut tables, row)
                .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        } else {
            self.write_task(
                &mut tables,
                module::Task {
                    stop_pending: true,
                    stop_pending_at,
                    ..row
                },
            )
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        }
        Ok(Some(
            tables.tasks.get(&id).is_some_and(|t| t.status == REVIEW),
        ))
    }

    pub(super) fn apply_record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        at: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        let id = id.0;
        let sub_status = sub_status.as_str().to_string();
        let at = encode::stamp(at);
        let mut tables = self.lock();
        let Some(row) = tables.tasks.get(&id).cloned() else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        if row.status != RUNNING {
            return Ok(ReducerOutcome::Applied(vec![]));
        }
        match self.write_task(
            &mut tables,
            module::Task {
                sub_status,
                last_pre_tool_use_at: at,
                ..row
            },
        ) {
            Ok(()) => Ok(ReducerOutcome::Applied(vec![])),
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_record_notification(
        &self,
        id: TaskId,
        mode: NotificationWrite,
        at: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        let id = id.0;
        let at = encode::stamp(at);
        let mut tables = self.lock();
        let Some(row) = tables.tasks.get(&id).cloned() else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        if row.status != RUNNING {
            return Ok(ReducerOutcome::Applied(vec![]));
        }
        let raised = |row: module::Task, at: String| module::Task {
            sub_status: NEEDS_INPUT.into(),
            last_notification_at: at,
            ..row
        };
        let result = match mode {
            NotificationWrite::Ignore => Ok(()),
            NotificationWrite::Clear => self.write_task(
                &mut tables,
                module::Task {
                    sub_status: ACTIVE.into(),
                    last_notification_at: String::new(),
                    ..row
                },
            ),
            NotificationWrite::Raise => self.write_task(&mut tables, raised(row, at)),
            NotificationWrite::RaiseIfNoOwnWorkLive if row.live_subagents == 0 => {
                self.write_task(&mut tables, raised(row, at))
            }
            NotificationWrite::RaiseIfNoOwnWorkLive => Ok(()),
        };
        match result {
            Ok(()) => Ok(ReducerOutcome::Applied(vec![])),
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_record_user_prompt_submit(
        &self,
        id: TaskId,
        activity_at: DateTime<Utc>,
        prompt_at: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        let id = id.0;
        let activity_at = encode::stamp(activity_at);
        let prompt_at = encode::stamp(prompt_at);
        let mut tables = self.lock();
        let Some(row) = tables.tasks.get(&id).cloned() else {
            return Ok(ReducerOutcome::Refused(format!("task {id} not found")));
        };
        if row.status != RUNNING && row.status != REVIEW {
            return Ok(ReducerOutcome::Refused(format!(
                "task {id} is neither running nor in review"
            )));
        }
        let resumed = row.status == REVIEW;
        let epic_id = row.epic_id;
        let void_pending_stop = row.stop_pending
            && (row.stop_pending_at.is_empty()
                || row.stop_pending_at.as_str() < prompt_at.as_str());
        let stop_pending = if void_pending_stop {
            false
        } else {
            row.stop_pending
        };
        if let Err(why) = self.write_task(
            &mut tables,
            module::Task {
                status: RUNNING.into(),
                sub_status: ACTIVE.into(),
                last_pre_tool_use_at: activity_at,
                stop_pending,
                ..row
            },
        ) {
            return Ok(ReducerOutcome::Refused(why));
        }
        if resumed {
            self.recalculate_epic_chain(&mut tables, epic_id);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_mark_pr_learnings_gate_shown(
        &self,
        id: TaskId,
        at: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        let id = id.0;
        let at = encode::stamp(at);
        let mut tables = self.lock();
        let Some(row) = tables.tasks.get(&id).cloned() else {
            return Ok(ReducerOutcome::Refused(format!("task {id} not found")));
        };
        if !row.pr_learnings_gate_shown_at.is_empty() {
            return Ok(ReducerOutcome::Refused(format!(
                "task {id} has already shown the PR learnings gate"
            )));
        }
        match self.write_task(
            &mut tables,
            module::Task {
                pr_learnings_gate_shown_at: at,
                ..row
            },
        ) {
            Ok(()) => Ok(ReducerOutcome::Applied(vec![])),
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<ReducerOutcome> {
        let watcher_task_id = watcher_task_id.0;
        let target_task_id = target_task_id.0;
        let mut tables = self.lock();
        let exists = tables
            .task_watchers
            .values()
            .any(|w| w.watcher_task_id == watcher_task_id && w.target_task_id == target_task_id);
        if !exists {
            let id = Self::assign_id(0, &mut tables.next_task_watcher_id);
            let row = module::TaskWatcher {
                id,
                watcher_task_id,
                target_task_id,
                created_at: self.now(),
            };
            tables.task_watchers.insert(id, row.clone());
            self.rows.upsert_task_watcher(&row.into());
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<ReducerOutcome> {
        let watcher_task_id = watcher_task_id.0;
        let target_task_id = target_task_id.0;
        let mut tables = self.lock();
        let ids: Vec<i64> = tables
            .task_watchers
            .values()
            .filter(|w| w.watcher_task_id == watcher_task_id && w.target_task_id == target_task_id)
            .map(|w| w.id)
            .collect();
        self.delete_watcher_rows(&mut tables, ids);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_watches_of_target(
        &self,
        target_task_id: TaskId,
    ) -> Result<ReducerOutcome> {
        let target_task_id = target_task_id.0;
        let mut tables = self.lock();
        let ids: Vec<i64> = tables
            .task_watchers
            .values()
            .filter(|w| w.target_task_id == target_task_id)
            .map(|w| w.id)
            .collect();
        self.delete_watcher_rows(&mut tables, ids);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_watches_by_watcher(
        &self,
        watcher_task_id: TaskId,
    ) -> Result<ReducerOutcome> {
        let watcher_task_id = watcher_task_id.0;
        let mut tables = self.lock();
        let ids: Vec<i64> = tables
            .task_watchers
            .values()
            .filter(|w| w.watcher_task_id == watcher_task_id)
            .map(|w| w.id)
            .collect();
        self.delete_watcher_rows(&mut tables, ids);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_claim_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> Result<ReducerOutcome> {
        let (scope, scope_id) = target.wire();
        let mut tables = self.lock();
        self.write_poll_owner_row(&mut tables, scope, scope_id, host, false);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_override_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> Result<ReducerOutcome> {
        let (scope, scope_id) = target.wire();
        let mut tables = self.lock();
        self.write_poll_owner_row(&mut tables, scope, scope_id, host, true);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// No recalculation and no `write_task` stamping: `sub_status` is derived
    /// board state, not a status/epic-linkage change — mirrors the module's
    /// own `batch_patch_sub_status` exactly, including bypassing `write_task`.
    pub(super) fn apply_batch_patch_sub_status(
        &self,
        updates: Vec<(TaskId, SubStatus)>,
    ) -> Result<ReducerOutcome> {
        let updates = encode::sub_status_updates(&updates);
        let mut tables = self.lock();
        let stamp = self.now();
        for update in updates {
            let update = module::SubStatusUpdate::from(update);
            let Some(row) = tables.tasks.get(&update.task_id).cloned() else {
                continue;
            };
            let updated = module::Task {
                sub_status: update.sub_status,
                updated_at: stamp.clone(),
                ..row
            };
            tables.tasks.insert(updated.id, updated.clone());
            self.rows.upsert_task(&updated.into());
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Atomic: inserts the successor and clears the predecessor's `phoenix`
    /// flag in one lock hold. No internal recalculation — mirrors the
    /// module's own `respawn_phoenix_successor` exactly.
    pub(super) fn apply_respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        successor: bindings::Task,
    ) -> Result<TaskId> {
        let predecessor = predecessor.0;
        let mut tables = self.lock();
        if !tables.tasks.contains_key(&predecessor) {
            return Err(anyhow::anyhow!("predecessor task {predecessor} not found"));
        }
        let successor_row = module::Task {
            id: 0,
            ..module::Task::from(successor)
        };
        self.write_task(&mut tables, successor_row)
            .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        let new_id = tables.next_task_id - 1;
        if let Some(predecessor_row) = tables.tasks.get(&predecessor).cloned() {
            let updated = module::Task {
                phoenix: false,
                ..predecessor_row
            };
            tables.tasks.insert(predecessor, updated.clone());
            self.rows.upsert_task(&updated.into());
        }
        Ok(TaskId(new_id))
    }

    pub(super) fn apply_register_host(
        &self,
        id: String,
        label: String,
        owner: String,
    ) -> Result<ReducerOutcome> {
        if id.trim().is_empty() {
            return Ok(ReducerOutcome::Refused(
                "a host id must not be empty".into(),
            ));
        }
        let mut tables = self.lock();
        let row = module::Host {
            id: id.clone(),
            label,
            owner,
        };
        tables.hosts.insert(id, row.clone());
        self.rows.upsert_host(&row.into());
        Ok(ReducerOutcome::Applied(vec![]))
    }
}
