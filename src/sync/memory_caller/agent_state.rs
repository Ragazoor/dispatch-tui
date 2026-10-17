//! Agent session state, poll ownership, host registry and the
//! task/epic-delete side effects that reach them.

use anyhow::Result;
use chrono::{DateTime, Utc};

use dispatch_spacetime_module as module;

use crate::models::{NotificationWrite, PollScopeId, SubStatus, TaskId};
use crate::spacetime::bindings;

use super::super::encode;
use super::super::writes::{DrainReadBack, ReducerOutcome, StoreRefusal};
use super::{MemoryReducerCaller, Tables, ACTIVE, AWAITING_REVIEW, NEEDS_INPUT, REVIEW, RUNNING};

impl MemoryReducerCaller {
    /// Delete every `task_subagents` row for `task_id`, and detach/cascade its learnings.
    /// Mirrors the module's `delete_task_side_effects` — called at every
    /// point that deletes a task row, exactly the set of call sites the
    /// module itself uses (`delete_task`, `batch_delete`'s task pass,
    /// `delete_epic_subtree`, `delete_stale_feed_tasks_in_epic`), on the same
    /// reasoning its own doc comment gives for keeping this in one place
    /// rather than four hand-copied ones.
    pub(super) fn delete_task_side_effects(&self, tables: &mut Tables, task_id: i64) {
        tables.task_subagents.retain(|s| s.task_id != task_id);
        // agents.allium: AgentsGoWithTheirTask.
        module::delete_agents_of(&mut self.agent_store(tables), task_id);
        self.detach_learnings_from_task(tables, task_id);
        self.delete_work_package_links_to(tables, module::WORK_PACKAGE_ITEM_TASK, task_id);
        // store-split.allium: DeletingATaskFailsItsOpenRequest (D8).
        self.fail_open_dispatch_requests_of(tables, task_id, module::DISPATCH_REASON_TASK_DELETED);
        // store-split.allium: DeletingATaskFailsItsOpenSessionRequest (W9).
        self.fail_open_session_requests_of(tables, task_id);
        // store-split.allium: DeletingATaskDeletesItsRequestRows.
        self.delete_request_rows_of(tables, task_id);
    }

    /// Recompute `live_subagents` from `task_subagents` and write it on the
    /// task's current agent, if it has one. Mirrors the module's
    /// `sync_subagent_count`.
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
        module::update_current_agent(&mut self.agent_store(tables), task_id, |agent| {
            module::Agent {
                live_subagents: count,
                ..agent
            }
        });
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
        let Some(agent) = module::current_agent(&self.agent_store(tables), task_id) else {
            return Ok(false);
        };
        if row.status == RUNNING && agent.stop_pending && agent.live_subagents == 0 {
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
        let id = row.id;
        self.write_task(
            tables,
            module::Task {
                status: REVIEW.into(),
                sub_status: AWAITING_REVIEW.into(),
                ..row
            },
        )?;
        module::update_current_agent(&mut self.agent_store(tables), id, |agent| module::Agent {
            last_pre_tool_use_at: String::new(),
            last_notification_at: String::new(),
            stop_pending: false,
            ..agent
        });
        self.recalculate_epic_chain(tables, epic_id);
        Ok(())
    }

    /// `[live, task_is_now_in_review]` off the task a drain acted on. Mirrors
    /// `SdkReducerCaller`'s own `subagent_drain_read_back`.
    pub(super) fn drain_read_back(tables: &Tables, task_id: i64) -> DrainReadBack {
        match tables.tasks.get(&task_id) {
            Some(t) => DrainReadBack {
                live: tables
                    .agents
                    .values()
                    .find(|a| a.task_id == task_id && a.state == module::AGENT_CURRENT)
                    .map_or(0, |a| a.live_subagents),
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
        // A hook for a task with no current agent writes no agent fact (A15).
        let Some(owner) = module::current_agent(&self.agent_store(&mut tables), task_id) else {
            return Ok(0);
        };
        Self::fence_subagent_session(&mut tables, task_id, &session_id);
        Self::delete_subagent_entry(&mut tables, task_id, &agent_id);
        tables.task_subagents.push(module::TaskSubagent {
            task_id,
            agent_id,
            session_id,
            started_at,
            agent_row: owner.id,
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
        module::update_current_agent(&mut self.agent_store(&mut tables), task_id, |agent| {
            module::Agent {
                stop_pending: false,
                ..agent
            }
        });
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
        // No current agent holds no subagents: the Stop flips at once (A15).
        let live = module::current_agent(&self.agent_store(&mut tables), id)
            .map_or(0, |a| a.live_subagents);
        if live == 0 {
            self.flip_to_review(&mut tables, row)
                .map_err(|why| anyhow::anyhow!("the shared store refused: {why}"))?;
        } else {
            module::update_current_agent(&mut self.agent_store(&mut tables), id, |agent| {
                module::Agent {
                    stop_pending: true,
                    stop_pending_at,
                    ..agent
                }
            });
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
        module::update_current_agent(&mut self.agent_store(&mut tables), id, |agent| {
            module::Agent {
                last_pre_tool_use_at: at,
                ..agent
            }
        });
        match self.write_task(&mut tables, module::Task { sub_status, ..row }) {
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
        let live = module::current_agent(&self.agent_store(&mut tables), id)
            .map_or(0, |a| a.live_subagents);
        let raised = |row: module::Task| module::Task {
            sub_status: NEEDS_INPUT.into(),
            ..row
        };
        let stamp = |this: &Self, tables: &mut Tables, at: String| {
            module::update_current_agent(&mut this.agent_store(tables), id, |agent| {
                module::Agent {
                    last_notification_at: at,
                    ..agent
                }
            });
        };
        let result = match mode {
            NotificationWrite::Ignore => Ok(()),
            NotificationWrite::Clear => {
                stamp(self, &mut tables, String::new());
                self.write_task(
                    &mut tables,
                    module::Task {
                        sub_status: ACTIVE.into(),
                        ..row
                    },
                )
            }
            NotificationWrite::Raise => {
                stamp(self, &mut tables, at);
                self.write_task(&mut tables, raised(row))
            }
            NotificationWrite::RaiseIfNoOwnWorkLive if live == 0 => {
                stamp(self, &mut tables, at);
                self.write_task(&mut tables, raised(row))
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
        module::update_current_agent(&mut self.agent_store(&mut tables), id, |agent| {
            let void_pending_stop = agent.stop_pending
                && (agent.stop_pending_at.is_empty()
                    || agent.stop_pending_at.as_str() < prompt_at.as_str());
            module::Agent {
                last_pre_tool_use_at: activity_at,
                stop_pending: agent.stop_pending && !void_pending_stop,
                ..agent
            }
        });
        if let Err(why) = self.write_task(
            &mut tables,
            module::Task {
                status: RUNNING.into(),
                sub_status: ACTIVE.into(),
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

    pub(super) fn apply_claim_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> Result<ReducerOutcome> {
        let (scope, scope_id) = target.wire();
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
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
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
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
            // As the module: an entry the status does not allow is skipped.
            let _ = self.write_task(
                &mut tables,
                module::Task {
                    sub_status: update.sub_status,
                    updated_at: stamp.clone(),
                    ..row
                },
            );
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
        if tables.tasks.get(&predecessor).is_some_and(|t| !t.phoenix) {
            return Err(anyhow::anyhow!(
                "predecessor task {predecessor} is not a phoenix task, or already respawned"
            ));
        }
        let epic_id =
            self.route_to_repo_group(&mut tables, successor.epic_id, &successor.repo_path);
        let successor_row = module::Task {
            id: 0,
            epic_id,
            ..module::Task::from(successor)
        };
        self.write_task(&mut tables, successor_row)
            .map_err(StoreRefusal)?;
        let new_id = tables.next_task_id - 1;
        if let Some(predecessor_row) = tables.tasks.get(&predecessor).cloned() {
            let updated = module::Task {
                phoenix: false,
                ..predecessor_row
            };
            tables.tasks.insert(predecessor, updated.clone());
            self.rows.upsert_task(&updated.into());
        }
        self.recalculate_epic_chain(&mut tables, epic_id);
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
        // Mirrors the module's `register_host` guard
        // (`core.allium: LocalHostOwnerIsWrittenOnce`, enforced).
        if let Err(why) =
            module::validate_host_registration(tables.hosts.get(&id), &owner, &self.sender)
        {
            return Ok(ReducerOutcome::Refused(format!("host {id}: {why}")));
        }
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
