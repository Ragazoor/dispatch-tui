//! Session requests (store-split M5): the in-memory twin of
//! `spacetime/module/src/session_request.rs`.
//!
//! As [`super::dispatch_requests`]: every decision is the module's own pure
//! function, called directly; what is reimplemented here is only the row
//! storage and the order the module's reducers apply them in. The module's
//! scheduled backstop has no scheduler here:
//! [`MemoryReducerCaller::expire_session_request`] is its stand-in, an inherent
//! method a test calls where the module's timer would fire.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{SessionRequestId, TaskId};

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

/// The one map from a reducer body's answer to the caller's outcome.
fn outcome(result: Result<Vec<i64>, String>) -> Result<ReducerOutcome> {
    Ok(match result {
        Ok(ids) => ReducerOutcome::Applied(ids),
        Err(why) => ReducerOutcome::Refused(why),
    })
}

/// Mirrors the module's `find_session_request`.
fn find_session(tables: &Tables, id: i64) -> Result<module::SessionRequest, String> {
    tables
        .session_requests
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("session request {id} does not exist"))
}

fn task_has_open_session_request(tables: &Tables, task_id: i64) -> bool {
    tables
        .session_requests
        .values()
        .any(|r| r.task_id == task_id && module::session_request_is_open(&r.state))
}

impl MemoryReducerCaller {
    fn write_session_request(&self, tables: &mut Tables, mut row: module::SessionRequest) -> i64 {
        let id = Self::assign_id(row.id, &mut tables.next_session_request_id);
        row.id = id;
        tables.session_requests.insert(id, row.clone());
        self.rows.upsert_session_request(&row.into());
        id
    }

    /// Mirrors the module's `fail_session_request`, the one failure path: write
    /// the failed row. A failed session request releases no claim and moves no
    /// task, and there is no timer row to cancel here.
    fn fail_session_request(&self, tables: &mut Tables, row: module::SessionRequest, reason: &str) {
        let failed = module::failed_session_request_row(row, reason, &self.now());
        self.write_session_request(tables, failed);
    }

    /// Mirrors the module's `fail_open_session_requests_of`
    /// (`DeletingATaskFailsItsOpenSessionRequest`): called by every task delete.
    pub(super) fn fail_open_session_requests_of(&self, tables: &mut Tables, task_id: i64) {
        let open: Vec<module::SessionRequest> = tables
            .session_requests
            .values()
            .filter(|r| r.task_id == task_id && module::session_request_is_open(&r.state))
            .cloned()
            .collect();
        for row in open {
            self.fail_session_request(tables, row, module::SESSION_REASON_TASK_DELETED);
        }
    }

    pub(super) fn apply_request_wrap_up(
        &self,
        task: TaskId,
        host: String,
        action: String,
    ) -> Result<ReducerOutcome> {
        outcome(self.request_wrap_up_body(task.0, &host, &action))
    }

    fn request_wrap_up_body(
        &self,
        task_id: i64,
        host: &str,
        action: &str,
    ) -> Result<Vec<i64>, String> {
        if host.is_empty() {
            return Err("a wrap-up request must name the requesting host".into());
        }
        let mut tables = self.lock();
        self.acting_for(&tables, host)?;
        module::validate_wrap_up_action(action)?;
        let Some(task) = tables.tasks.get(&task_id).cloned() else {
            return Err(format!("task {task_id} no longer exists"));
        };
        module::validate_wrap_up_request(
            &task,
            host,
            task_has_open_session_request(&tables, task_id),
        )?;
        let id = self.write_session_request(
            &mut tables,
            module::SessionRequest {
                requested_by: self.sender.clone(),
                ..module::new_wrap_up_request(task_id, host, action, &self.now())
            },
        );
        Ok(vec![id])
    }

    pub(super) fn apply_claim_session_request(
        &self,
        id: SessionRequestId,
        host: String,
    ) -> Result<ReducerOutcome> {
        outcome(self.claim_session_request_body(id.0, &host))
    }

    fn claim_session_request_body(&self, id: i64, host: &str) -> Result<Vec<i64>, String> {
        let mut tables = self.lock();
        let row = find_session(&tables, id)?;
        let stamp = self.now();
        module::validate_session_request_claim(&row, host, &stamp)?;
        self.acting_for(&tables, host)?;
        if let Some(task) = tables.tasks.get(&row.task_id).cloned() {
            if let Some(cleared) = module::clear_conflict_on_claim(&row.kind, task, &stamp) {
                self.write_task(&mut tables, cleared)?;
            }
        }
        self.write_session_request(&mut tables, module::claim_session_request_row(row, &stamp));
        Ok(vec![])
    }

    pub(super) fn apply_report_session_request_done(
        &self,
        id: SessionRequestId,
        reporter: String,
    ) -> Result<ReducerOutcome> {
        outcome(self.report_session_request_done_body(id.0, &reporter))
    }

    fn report_session_request_done_body(
        &self,
        id: i64,
        reporter: &str,
    ) -> Result<Vec<i64>, String> {
        let mut tables = self.lock();
        let row = find_session(&tables, id)?;
        if module::session_report_is_repeat(&row, reporter) {
            self.acting_for(&tables, reporter)?;
            return Ok(vec![]);
        }
        module::validate_session_report(&row, reporter)?;
        self.acting_for(&tables, reporter)?;
        let stamp = self.now();
        module::validate_session_done_in_time(&row, &stamp)?;
        self.write_session_request(&mut tables, module::done_session_request_row(row, &stamp));
        Ok(vec![])
    }

    pub(super) fn apply_report_session_request_failed(
        &self,
        id: SessionRequestId,
        reporter: String,
        reason: String,
        conflict: bool,
    ) -> Result<ReducerOutcome> {
        outcome(self.report_session_request_failed_body(id.0, &reporter, &reason, conflict))
    }

    fn report_session_request_failed_body(
        &self,
        id: i64,
        reporter: &str,
        reason: &str,
        conflict: bool,
    ) -> Result<Vec<i64>, String> {
        let mut tables = self.lock();
        let row = find_session(&tables, id)?;
        module::validate_session_report(&row, reporter)?;
        module::validate_session_failure_report(&row, conflict)?;
        self.acting_for(&tables, reporter)?;
        let task_id = row.task_id;
        self.fail_session_request(&mut tables, row, reason);
        if conflict {
            if let Some(task) = tables.tasks.get(&task_id).cloned() {
                if let Some(conflicted) = module::apply_rebase_conflict(task, &self.now()) {
                    self.write_task(&mut tables, conflicted)?;
                }
            }
        }
        Ok(vec![])
    }

    pub(super) fn apply_request_exit_session(
        &self,
        task: TaskId,
        token: SessionRequestId,
        action: String,
        pr_url: String,
    ) -> Result<ReducerOutcome> {
        outcome(self.request_exit_session_body(task.0, token.0, &action, &pr_url))
    }

    /// Mirrors the module's `request_exit_session`: the token check, the close
    /// (`ExitSession`), the epic chain and the exit_session row — the module's
    /// ONE transaction, which the held lock stands for here. Every fallible step
    /// that can refuse runs before the first write, as in the module, so a
    /// refusal leaves the task and the token as they were.
    fn request_exit_session_body(
        &self,
        task_id: i64,
        token: i64,
        action: &str,
        pr_url: &str,
    ) -> Result<Vec<i64>, String> {
        let mut tables = self.lock();
        let Some(task) = tables.tasks.get(&task_id).cloned() else {
            return Err(format!("task {task_id} no longer exists"));
        };
        let sessions: Vec<module::SessionRequest> = tables
            .session_requests
            .values()
            .filter(|r| r.task_id == task_id)
            .cloned()
            .collect();
        let dispatches: Vec<module::DispatchRequest> = tables
            .dispatch_requests
            .values()
            .filter(|r| r.task_id == task_id)
            .cloned()
            .collect();
        let token_row = module::check_exit_token(task_id, token, &sessions, &dispatches)?;
        let window = module::session_window(&self.agent_store(&mut tables), task_id);
        module::validate_exit_session(&task, &window, &token_row, action, pr_url)?;
        // D11 (W11): the row names the task's host, so only its owner may ask.
        self.acting_for(&tables, &task.host)?;
        module::validate_no_open_session_request(
            task_id,
            sessions
                .iter()
                .any(|r| module::session_request_is_open(&r.state)),
        )?;

        let stamp = self.now();
        let host = task.host.clone();
        let epic_id = task.epic_id;
        let mut closed = module::apply_session_close(task.clone(), action, pr_url, &stamp);
        if module::should_respawn_phoenix(&task.status, &closed)
            && self.respawn_phoenix_successor_of(&mut tables, &closed, &stamp)
        {
            closed.phoenix = false;
        }
        self.write_task(&mut tables, closed.clone())?;
        module::close_agent_session(&mut self.agent_store(&mut tables), task_id);
        self.recalculate_epic_chain(&mut tables, epic_id);

        let chained = self.chain_next_subtask(&mut tables, &closed, &host);
        let id = self.write_session_request(
            &mut tables,
            module::SessionRequest {
                requested_by: self.sender.clone(),
                ..module::new_exit_session_request(
                    &task,
                    &window,
                    token_row.id,
                    action,
                    pr_url,
                    chained,
                    &stamp,
                )
            },
        );
        Ok(vec![id])
    }

    pub(super) fn apply_close_task(&self, task: TaskId, host: String) -> Result<ReducerOutcome> {
        outcome(self.close_task_body(task.0, &host))
    }

    /// Mirrors the module's `close_task` (`RequestCloseTask`): fail the open
    /// session request, apply the Done close, respawn a phoenix task, roll the
    /// epic up and write the close row when there is host work and its host is
    /// the caller's own. One transaction, which the held lock stands for.
    fn close_task_body(&self, task_id: i64, host: &str) -> Result<Vec<i64>, String> {
        let mut tables = self.lock();
        let Some(task) = tables.tasks.get(&task_id).cloned() else {
            return Err(format!("task {task_id} no longer exists"));
        };
        let stamp = self.now();
        let window = module::session_window(&self.agent_store(&mut tables), task_id);
        let open: Vec<module::SessionRequest> = tables
            .session_requests
            .values()
            .filter(|r| r.task_id == task_id && module::session_request_is_open(&r.state))
            .cloned()
            .collect();
        for row in open {
            self.fail_session_request(&mut tables, row, module::SESSION_REASON_TASK_CLOSED);
        }

        let epic_id = task.epic_id;
        let mut closed = module::apply_done_close(task.clone(), &stamp);
        if module::should_respawn_phoenix(&task.status, &closed)
            && self.respawn_phoenix_successor_of(&mut tables, &closed, &stamp)
        {
            closed.phoenix = false;
        }
        self.write_task(&mut tables, closed)?;
        module::close_agent_session(&mut self.agent_store(&mut tables), task_id);
        self.recalculate_epic_chain(&mut tables, epic_id);

        let row = module::new_close_request(&task, &window, host, &stamp);
        if module::close_has_host_work(&task, &window)
            && self.acting_for(&tables, &row.host).is_ok()
        {
            self.write_session_request(
                &mut tables,
                module::SessionRequest {
                    requested_by: self.sender.clone(),
                    ..row
                },
            );
        }
        Ok(vec![])
    }

    /// Mirrors the module's `respawn_phoenix_successor_of`: the successor row,
    /// and whether it landed. A failure is swallowed: Done outranks the respawn.
    pub(super) fn respawn_phoenix_successor_of(
        &self,
        tables: &mut Tables,
        closed: &module::Task,
        stamp: &str,
    ) -> bool {
        let epic_id = self.route_to_repo_group(tables, closed.epic_id, &closed.repo_path);
        let successor = module::phoenix_successor(closed, epic_id, stamp, &self.sender);
        self.write_task(tables, successor).is_ok()
    }

    /// Mirrors the module's `chain_next_subtask` (`AutoDispatchNextSubtask`):
    /// the first backlog subtask of the closed task's epic whose `epic_chain`
    /// request is accepted, requested on the closed task's host through the
    /// shared dispatch path. `0` when the chain stops; nothing here fails the
    /// close.
    fn chain_next_subtask(&self, tables: &mut Tables, closed: &module::Task, host: &str) -> i64 {
        if closed.epic_id == 0 {
            return 0;
        }
        match tables.epics.get(&closed.epic_id) {
            Some(epic) if epic.auto_dispatch => {}
            _ => return 0,
        }
        let subtasks: Vec<module::Task> = tables
            .tasks
            .values()
            .filter(|t| t.epic_id == closed.epic_id)
            .cloned()
            .collect();
        for candidate in module::chain_candidates(subtasks) {
            let has_open = tables
                .dispatch_requests
                .values()
                .any(|r| r.task_id == candidate.id && module::dispatch_request_is_open(&r.state));
            if module::validate_dispatch_request(
                &candidate,
                host,
                module::DISPATCH_ORIGIN_EPIC_CHAIN,
                has_open,
            )
            .is_err()
            {
                continue;
            }
            let Ok(mode) = module::choose_dispatch_mode(&candidate, "") else {
                continue;
            };
            let id = candidate.id;
            if self
                .create_dispatch_request_body(
                    tables,
                    candidate,
                    host,
                    mode,
                    module::DISPATCH_ORIGIN_EPIC_CHAIN,
                )
                .is_ok()
            {
                return id;
            }
        }
        0
    }

    pub(super) fn apply_fail_interrupted_session_requests(
        &self,
        host: String,
    ) -> Result<ReducerOutcome> {
        outcome(self.fail_interrupted_session_requests_body(&host))
    }

    fn fail_interrupted_session_requests_body(&self, host: &str) -> Result<Vec<i64>, String> {
        let mut tables = self.lock();
        self.acting_for(&tables, host)?;
        let rows: Vec<module::SessionRequest> = tables
            .session_requests
            .values()
            .filter(|r| r.host == host)
            .cloned()
            .collect();
        for row in module::interrupted_session_requests(rows) {
            self.fail_session_request(&mut tables, row, module::SESSION_REASON_HOST_RESTARTED);
        }
        Ok(vec![])
    }

    /// The stand-in for the module's scheduled `expire_session_request`
    /// (`SessionRequestTimesOut`): fail the row "timed out" if it is still open,
    /// else leave it. Inherent, not on
    /// [`ReducerCaller`](super::super::writes::ReducerCaller), because no client
    /// may call the module's version — a test calls this where the module's
    /// timer would fire.
    pub fn expire_session_request(&self, id: SessionRequestId) -> ReducerOutcome {
        let mut tables = self.lock();
        let Some(row) = tables.session_requests.get(&id.0).cloned() else {
            return ReducerOutcome::Applied(vec![]);
        };
        // Its own report won: Ok, as the module's version answers.
        if !module::session_request_is_open(&row.state) {
            return ReducerOutcome::Applied(vec![]);
        }
        self.fail_session_request(&mut tables, row, module::SESSION_REASON_TIMED_OUT);
        ReducerOutcome::Applied(vec![])
    }
}
