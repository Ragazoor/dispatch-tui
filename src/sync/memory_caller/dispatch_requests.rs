//! Dispatch requests (store-split M4): the in-memory twin of
//! `spacetime/module/src/dispatch_request.rs`.
//!
//! Every decision is the module's own pure function, called directly; what is
//! reimplemented here is only the row storage and the order the module's
//! reducers apply them in. The module's scheduled backstop has no scheduler
//! here: [`MemoryReducerCaller::expire_dispatch_request`] is its stand-in, an
//! inherent method a test calls where the module's timer would fire.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{DispatchRequestId, TaskId};

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

impl MemoryReducerCaller {
    fn write_dispatch_request(&self, tables: &mut Tables, mut row: module::DispatchRequest) -> i64 {
        let id = Self::assign_id(row.id, &mut tables.next_dispatch_request_id);
        row.id = id;
        tables.dispatch_requests.insert(id, row.clone());
        self.rows.upsert_dispatch_request(&row.into());
        id
    }

    /// Mirrors the module's `finish_failed_request`: write the failed row and
    /// release the task's claim when it is still the bare claim
    /// (`ReleaseClaimOfFailedRequest`). There is no timer row to cancel here.
    fn finish_failed_request(
        &self,
        tables: &mut Tables,
        failed: module::DispatchRequest,
    ) -> Result<(), String> {
        let stamp = failed.finished_at.clone();
        let task_id = failed.task_id;
        self.write_dispatch_request(tables, failed);
        if let Some(task) = tables.tasks.get(&task_id).cloned() {
            if let Some(released) = module::release_claim_after_failure(task) {
                self.write_task(
                    tables,
                    module::Task {
                        updated_at: stamp,
                        ..released
                    },
                )?;
            }
        }
        Ok(())
    }

    /// Mirrors the module's `fail_dispatch_request`, the one failure path.
    fn fail_dispatch_request(
        &self,
        tables: &mut Tables,
        row: module::DispatchRequest,
        reason: &str,
    ) -> Result<(), String> {
        let failed = module::failed_request_row(row, reason, &self.now());
        self.finish_failed_request(tables, failed)
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
        if requester.is_empty() {
            return Ok(ReducerOutcome::Refused(
                "a dispatch request must name the requesting host".into(),
            ));
        }
        if let Err(why) = module::validate_dispatch_origin(&origin) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let mut tables = self.lock();
        let Some(row) = tables.tasks.get(&task.0).cloned() else {
            return Ok(ReducerOutcome::Refused(format!(
                "task {} no longer exists",
                task.0
            )));
        };
        let has_open = tables
            .dispatch_requests
            .values()
            .any(|r| r.task_id == task.0 && module::dispatch_request_is_open(&r.state));
        if let Err(why) = module::validate_dispatch_request(&row, &requester, has_open) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let mode = match module::choose_dispatch_mode(&row, &mode) {
            Ok(mode) => mode,
            Err(why) => return Ok(ReducerOutcome::Refused(why)),
        };

        let stamp = self.now();
        tables.task_subagents.retain(|s| s.task_id != task.0);
        if let Err(why) = self.write_task(&mut tables, module::apply_dispatch_claim(row, &stamp)) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let id = self.write_dispatch_request(
            &mut tables,
            module::new_dispatch_request(task.0, &requester, mode, &origin, &stamp),
        );
        Ok(ReducerOutcome::Applied(vec![id]))
    }

    pub(super) fn apply_claim_dispatch_request(
        &self,
        id: DispatchRequestId,
        host: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let Some(row) = tables.dispatch_requests.get(&id.0).cloned() else {
            return Ok(ReducerOutcome::Refused(format!(
                "dispatch request {} does not exist",
                id.0
            )));
        };
        if let Err(why) = module::validate_dispatch_request_claim(&row, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let now = self.now();
        self.write_dispatch_request(&mut tables, module::claim_dispatch_request_row(row, &now));
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_report_dispatch_provisioned(
        &self,
        id: DispatchRequestId,
        reporter: String,
        worktree: String,
        tmux_window: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let Some(row) = tables.dispatch_requests.get(&id.0).cloned() else {
            return Ok(ReducerOutcome::Refused(format!(
                "dispatch request {} does not exist",
                id.0
            )));
        };
        if let Err(why) = module::validate_dispatch_report(&row, &reporter) {
            return Ok(ReducerOutcome::Refused(why));
        }
        if worktree.is_empty() {
            return Ok(ReducerOutcome::Refused(format!(
                "dispatch request {}: a dispatched report must name the worktree",
                id.0
            )));
        }
        let Some(task) = tables.tasks.get(&row.task_id).cloned() else {
            return Ok(ReducerOutcome::Refused(format!(
                "task {} no longer exists",
                row.task_id
            )));
        };
        let row = module::dispatched_request_row(row, &worktree, &tmux_window, &self.now());
        if let Err(why) = self.write_task(&mut tables, module::apply_dispatched_to_task(task, &row))
        {
            return Ok(ReducerOutcome::Refused(why));
        }
        self.write_dispatch_request(&mut tables, row);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_report_dispatch_failed(
        &self,
        id: DispatchRequestId,
        reporter: String,
        reason: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let Some(row) = tables.dispatch_requests.get(&id.0).cloned() else {
            return Ok(ReducerOutcome::Refused(format!(
                "dispatch request {} does not exist",
                id.0
            )));
        };
        if let Err(why) = module::validate_dispatch_report(&row, &reporter) {
            return Ok(ReducerOutcome::Refused(why));
        }
        match self.fail_dispatch_request(&mut tables, row, &reason) {
            Ok(()) => Ok(ReducerOutcome::Applied(vec![])),
            Err(why) => Ok(ReducerOutcome::Refused(why)),
        }
    }

    pub(super) fn apply_fail_interrupted_dispatch_requests(
        &self,
        host: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let rows: Vec<module::DispatchRequest> = tables
            .dispatch_requests
            .values()
            .filter(|r| r.host == host)
            .cloned()
            .collect();
        for id in module::interrupted_dispatch_requests(&rows, &host) {
            let Some(row) = tables.dispatch_requests.get(&id).cloned() else {
                continue;
            };
            if let Err(why) =
                self.fail_dispatch_request(&mut tables, row, module::DISPATCH_REASON_HOST_RESTARTED)
            {
                return Ok(ReducerOutcome::Refused(why));
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
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
        match module::expire_dispatch_request_row(row, &self.now()) {
            Some(failed) => match self.finish_failed_request(&mut tables, failed) {
                Ok(()) => ReducerOutcome::Applied(vec![]),
                Err(why) => ReducerOutcome::Refused(why),
            },
            None => {
                ReducerOutcome::Refused(format!("dispatch request {} has already finished", id.0))
            }
        }
    }
}
