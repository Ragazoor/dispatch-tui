//! Dispatch requests (store-split M4): the in-memory twin of
//! `spacetime/module/src/dispatch_request.rs`.
//!
//! Every decision is the module's own pure function, called directly; what is
//! reimplemented here is only the row storage and the order the module's
//! reducers apply them in. Each reducer body is written as the module's is —
//! a `Result<_, String>` with `?` — and mapped to a [`ReducerOutcome`] once, at
//! the boundary. The module's scheduled backstop has no scheduler here:
//! [`MemoryReducerCaller::expire_dispatch_request`] is its stand-in, an
//! inherent method a test calls where the module's timer would fire.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{DispatchRequestId, TaskId};

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

/// The one map from a reducer body's answer to the caller's outcome.
fn outcome(result: Result<Vec<i64>, String>) -> Result<ReducerOutcome> {
    Ok(match result {
        Ok(ids) => ReducerOutcome::Applied(ids),
        Err(why) => ReducerOutcome::Refused(why),
    })
}

/// Mirrors the module's `find_request`.
fn find_request(tables: &Tables, id: i64) -> Result<module::DispatchRequest, String> {
    tables
        .dispatch_requests
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("dispatch request {id} does not exist"))
}

impl MemoryReducerCaller {
    /// Mirrors the module's `require_acting_for` (D11): only `host`'s
    /// registered owner — this caller's sender — may act for it.
    pub(super) fn acting_for(&self, tables: &Tables, host: &str) -> Result<(), String> {
        module::validate_host_acting_for(tables.hosts.get(host), host, &self.sender)
    }

    fn write_dispatch_request(&self, tables: &mut Tables, mut row: module::DispatchRequest) -> i64 {
        let id = Self::assign_id(row.id, &mut tables.next_dispatch_request_id);
        row.id = id;
        tables.dispatch_requests.insert(id, row.clone());
        self.rows.upsert_dispatch_request(&row.into());
        id
    }

    /// Mirrors the module's `fail_dispatch_request`, the one failure path:
    /// write the failed row, release the task's claim when it is still the
    /// bare claim and roll up its epic chain (`ReleaseClaimOfFailedRequest`).
    /// There is no timer row to cancel here.
    fn fail_dispatch_request(
        &self,
        tables: &mut Tables,
        row: module::DispatchRequest,
        reason: &str,
    ) -> Result<(), String> {
        let failed = module::failed_request_row(row, reason, &self.now());
        let stamp = failed.finished_at.clone();
        let task_id = failed.task_id;
        self.write_dispatch_request(tables, failed);
        if let Some(task) = tables.tasks.get(&task_id).cloned() {
            let epic_id = task.epic_id;
            if let Some(released) = module::release_claim_after_failure(task) {
                self.write_task(
                    tables,
                    module::Task {
                        updated_at: stamp,
                        ..released
                    },
                )?;
                // agents.allium: EndAgentWhenLaunchFails.
                module::end_current_agent(
                    &mut self.agent_store(tables),
                    task_id,
                    module::AGENT_END_LAUNCH_FAILED,
                );
            }
            self.recalculate_epic_chain(tables, epic_id);
        }
        Ok(())
    }

    /// Mirrors the module's `fail_open_dispatch_requests_of`
    /// (`DeletingATaskFailsItsOpenRequest`): called by every task delete,
    /// after the task row is gone.
    pub(super) fn fail_open_dispatch_requests_of(&self, tables: &mut Tables, task_id: i64) {
        let open: Vec<module::DispatchRequest> = tables
            .dispatch_requests
            .values()
            .filter(|r| r.task_id == task_id && module::dispatch_request_is_open(&r.state))
            .cloned()
            .collect();
        for row in open {
            let _ = self.fail_dispatch_request(tables, row, module::DISPATCH_REASON_TASK_DELETED);
        }
    }

    pub(super) fn apply_request_dispatch(
        &self,
        task: TaskId,
        requester: String,
        origin: String,
        mode: String,
    ) -> Result<ReducerOutcome> {
        outcome(self.request_dispatch_body(task.0, &requester, &origin, &mode))
    }

    fn request_dispatch_body(
        &self,
        task_id: i64,
        requester: &str,
        origin: &str,
        mode: &str,
    ) -> Result<Vec<i64>, String> {
        if requester.is_empty() {
            return Err("a dispatch request must name the requesting host".into());
        }
        let mut tables = self.lock();
        self.acting_for(&tables, requester)?;
        module::validate_dispatch_origin(origin)?;
        let Some(row) = tables.tasks.get(&task_id).cloned() else {
            return Err(format!("task {task_id} no longer exists"));
        };
        let has_open = tables
            .dispatch_requests
            .values()
            .any(|r| r.task_id == task_id && module::dispatch_request_is_open(&r.state));
        module::validate_dispatch_request(&row, requester, origin, has_open)?;
        let mode = module::choose_dispatch_mode(&row, mode)?;
        let id = self.create_dispatch_request_body(&mut tables, row, requester, mode, origin)?;
        Ok(vec![id])
    }

    /// Mirrors the module's `create_dispatch_request`, shared by
    /// `request_dispatch` and the epic chain `request_exit_session` runs: claim
    /// the task, roll up its epic chain, write the `requested` row. The one
    /// fallible step, the claim's write, runs first, so an `Err` has written
    /// nothing and the chain can move on to its next candidate.
    pub(super) fn create_dispatch_request_body(
        &self,
        tables: &mut Tables,
        task: module::Task,
        requester: &str,
        mode: &str,
        origin: &str,
    ) -> Result<i64, String> {
        let stamp = self.now();
        let task_id = task.id;
        let epic_id = task.epic_id;
        let claimed = module::apply_dispatch_claim(task, &stamp);
        self.write_task(tables, claimed.clone())?;
        // agents.allium: StartAgentOnFreshLaunch.
        module::start_agent_on_fresh_launch(
            &mut self.agent_store(tables),
            &claimed,
            requester,
            mode,
        );
        Self::delete_all_subagents(tables, task_id);
        self.recalculate_epic_chain(tables, epic_id);
        Ok(self.write_dispatch_request(
            tables,
            module::DispatchRequest {
                requested_by: self.sender.clone(),
                ..module::new_dispatch_request(task_id, requester, mode, origin, &stamp)
            },
        ))
    }

    pub(super) fn apply_claim_dispatch_request(
        &self,
        id: DispatchRequestId,
        host: String,
    ) -> Result<ReducerOutcome> {
        outcome(self.claim_dispatch_request_body(id.0, &host))
    }

    fn claim_dispatch_request_body(&self, id: i64, host: &str) -> Result<Vec<i64>, String> {
        let mut tables = self.lock();
        let row = find_request(&tables, id)?;
        module::validate_dispatch_request_claim(&row, host)?;
        self.acting_for(&tables, host)?;
        let now = self.now();
        self.write_dispatch_request(&mut tables, module::claim_dispatch_request_row(row, &now));
        Ok(vec![])
    }

    pub(super) fn apply_report_dispatch_provisioned(
        &self,
        id: DispatchRequestId,
        reporter: String,
        worktree: String,
        tmux_window: String,
        reused_worktree: bool,
    ) -> Result<ReducerOutcome> {
        outcome(self.report_dispatch_provisioned_body(
            id.0,
            &reporter,
            &worktree,
            &tmux_window,
            reused_worktree,
        ))
    }

    fn report_dispatch_provisioned_body(
        &self,
        id: i64,
        reporter: &str,
        worktree: &str,
        tmux_window: &str,
        reused_worktree: bool,
    ) -> Result<Vec<i64>, String> {
        let mut tables = self.lock();
        let row = find_request(&tables, id)?;
        if module::dispatch_report_is_repeat(&row, reporter, worktree, tmux_window) {
            self.acting_for(&tables, reporter)?;
            return Ok(vec![]);
        }
        module::validate_dispatch_report(&row, reporter)?;
        self.acting_for(&tables, reporter)?;
        let stamp = self.now();
        module::validate_dispatched_report_in_time(&row, &stamp)?;
        if worktree.is_empty() {
            return Err(format!(
                "dispatch request {id}: a dispatched report must name the worktree"
            ));
        }
        let Some(task) = tables.tasks.get(&row.task_id).cloned() else {
            return Err(format!("task {} no longer exists", row.task_id));
        };
        let row =
            module::dispatched_request_row(row, worktree, tmux_window, reused_worktree, &stamp);
        let epic_id = task.epic_id;
        self.write_task(&mut tables, module::apply_dispatched_to_task(task, &row))?;
        // The window belongs to the agent the claim started (agents.allium, A4).
        module::update_current_agent(&mut self.agent_store(&mut tables), row.task_id, |agent| {
            module::Agent {
                tmux_window: row.tmux_window.clone(),
                ..agent
            }
        });
        self.recalculate_epic_chain(&mut tables, epic_id);
        self.write_dispatch_request(&mut tables, row);
        Ok(vec![])
    }

    pub(super) fn apply_report_dispatch_failed(
        &self,
        id: DispatchRequestId,
        reporter: String,
        reason: String,
    ) -> Result<ReducerOutcome> {
        outcome(self.report_dispatch_failed_body(id.0, &reporter, &reason))
    }

    fn report_dispatch_failed_body(
        &self,
        id: i64,
        reporter: &str,
        reason: &str,
    ) -> Result<Vec<i64>, String> {
        let mut tables = self.lock();
        let row = find_request(&tables, id)?;
        module::validate_dispatch_report(&row, reporter)?;
        self.acting_for(&tables, reporter)?;
        self.fail_dispatch_request(&mut tables, row, reason)?;
        Ok(vec![])
    }

    pub(super) fn apply_fail_interrupted_dispatch_requests(
        &self,
        host: String,
    ) -> Result<ReducerOutcome> {
        outcome(self.fail_interrupted_dispatch_requests_body(&host))
    }

    fn fail_interrupted_dispatch_requests_body(&self, host: &str) -> Result<Vec<i64>, String> {
        let mut tables = self.lock();
        self.acting_for(&tables, host)?;
        let rows: Vec<module::DispatchRequest> = tables
            .dispatch_requests
            .values()
            .filter(|r| r.host == host)
            .cloned()
            .collect();
        for row in module::interrupted_dispatch_requests(rows) {
            self.fail_dispatch_request(&mut tables, row, module::DISPATCH_REASON_HOST_RESTARTED)?;
        }
        Ok(vec![])
    }

    /// The stand-in for the module's scheduled `expire_dispatch_request`
    /// (`DispatchRequestTimesOut`): fail the row "timed out" if it is still
    /// open, else leave it. Inherent, not on
    /// [`ReducerCaller`](super::super::writes::ReducerCaller), because no
    /// client may call the module's version — a test calls this where the
    /// module's timer would fire.
    pub fn expire_dispatch_request(&self, id: DispatchRequestId) -> ReducerOutcome {
        let mut tables = self.lock();
        let Some(row) = tables.dispatch_requests.get(&id.0).cloned() else {
            return ReducerOutcome::Applied(vec![]);
        };
        // Its own report won: Ok, as the module's version answers.
        if !module::dispatch_request_is_open(&row.state) {
            return ReducerOutcome::Applied(vec![]);
        }
        match self.fail_dispatch_request(&mut tables, row, module::DISPATCH_REASON_TIMED_OUT) {
            Ok(()) => ReducerOutcome::Applied(vec![]),
            Err(why) => ReducerOutcome::Refused(why),
        }
    }
}
