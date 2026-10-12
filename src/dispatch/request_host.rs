//! The host's side of dispatch request rows (store-split M4).
//!
//! Spec: `docs/specs/store-split.allium` (`HostClaimsDispatch`, D4, D5) and
//! `docs/specs/dispatch.allium` (`ProvisionDispatch`,
//! `ProvisioningStopsAtTheDeadline`). Two pure pieces the service and the
//! runtime build on:
//!
//! - [`DispatchRequestObserver`] decides which delivered rows this board must
//!   serve, once each.
//! - [`DeadlineRunner`] bounds every subprocess a provisioning runs by the time
//!   left to the row's deadline, and refuses to start one past it.
//!
//! Store-split M5 adds [`SessionRequestObserver`], the same decision for the
//! wrap_up and exit_session rows (`HostClaimsSessionRequest`,
//! `KillWindowOfFailedExitSession`); [`DeadlineRunner`] bounds their rebase and
//! teardown too.

use std::collections::HashSet;
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::clock::Clock;
use crate::models::{
    DispatchRequest, DispatchRequestId, DispatchRequestState, SessionRequest, SessionRequestId,
    SessionRequestKind, SessionRequestState,
};
use crate::process::{AgentBinaries, ProcessRunner};

/// `store-split.allium: config.dispatch_request_timeout`: how long a request
/// may stay open, from `requested_at`. The module stores the deadline on the
/// row and the host enforces the row's value; this copy is only for what has
/// no row to read — `UnprovisionedIndicator`'s claim-stamp fallback.
pub const DISPATCH_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// `ProvisioningStopsAtTheDeadline`'s reason, as the row records it.
pub const DEADLINE_PASSED: &str = "deadline passed";

/// Which delivered rows this board serves (`HostClaimsDispatch`'s trigger).
///
/// The `requested` rows naming this host, each handed out once however often
/// the row pump redelivers the whole set — a live insert and the initial rows
/// a starting board receives alike (D5). The module refuses a second claim
/// anyway; this keeps the board from asking twice.
///
/// Handing a row out marks it, so two wake-ups racing on the same set never
/// both serve it. A row whose claim was never answered — the claim could not
/// be sent, or the board had no settled identity to check the requester
/// against — is [`released`](Self::release), and the next reload hands it
/// out again. That retry ends when the row stops being `requested`: claimed,
/// or failed by the module's backstop at its deadline.
#[derive(Debug)]
pub struct DispatchRequestObserver {
    host: String,
    served: HashSet<DispatchRequestId>,
}

impl DispatchRequestObserver {
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            served: HashSet::new(),
        }
    }

    /// The rows in `rows` to serve now: requested, this host's, and not
    /// handed out before.
    pub fn observe(&mut self, rows: &[DispatchRequest]) -> Vec<DispatchRequest> {
        rows.iter()
            .filter(|r| r.host == self.host && r.state == DispatchRequestState::Requested)
            .filter(|r| self.served.insert(r.id))
            .cloned()
            .collect()
    }

    /// Forget that `id` was handed out, so the next [`Self::observe`] that
    /// still sees it `requested` hands it out again.
    pub fn release(&mut self, id: DispatchRequestId) {
        self.served.remove(&id);
    }
}

/// A [`ProcessRunner`] that holds a provisioning to its row's deadline
/// (`dispatch.allium: ProvisioningStopsAtTheDeadline`).
///
/// Every call is bounded by the time left to `deadline` on `clock` — a
/// stalled subprocess is killed at the deadline rather than at its own
/// `SUBPROCESS_TIMEOUT` — and a call that would start at or past the deadline
/// is not started: it fails with [`DEADLINE_PASSED`], which stops the attempt
/// at the next step. A plain [`ProcessRunner::run`] is bounded too; nothing a
/// provisioning runs may outlive the deadline.
///
/// The provisioning-failure rollback is exempt: its caller hands it the
/// unwrapped runner (`run_agent_for_mode`'s `teardown`), because what an
/// attempt made past the deadline still has to be torn down.
pub struct DeadlineRunner {
    inner: Arc<dyn ProcessRunner>,
    clock: Arc<dyn Clock>,
    deadline: DateTime<Utc>,
}

impl DeadlineRunner {
    pub fn new(
        inner: Arc<dyn ProcessRunner>,
        clock: Arc<dyn Clock>,
        deadline: DateTime<Utc>,
    ) -> Self {
        Self {
            inner,
            clock,
            deadline,
        }
    }

    /// The time left, or the refusal once there is none.
    fn left(&self, program: &str) -> Result<Duration> {
        match (self.deadline - self.clock.now()).to_std() {
            Ok(left) if !left.is_zero() => Ok(left),
            _ => anyhow::bail!("{DEADLINE_PASSED}: {program} was not started"),
        }
    }
}

impl ProcessRunner for DeadlineRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        let left = self.left(program)?;
        self.inner.run_with_timeout(program, args, left)
    }

    fn run_with_timeout(&self, program: &str, args: &[&str], timeout: Duration) -> Result<Output> {
        let left = self.left(program)?;
        self.inner
            .run_with_timeout(program, args, timeout.min(left))
    }

    fn agent_binaries(&self) -> AgentBinaries {
        self.inner.agent_binaries()
    }

    fn claude_json_path(&self) -> Option<std::path::PathBuf> {
        self.inner.claude_json_path()
    }

    fn sccache_available(&self) -> bool {
        self.inner.sccache_available()
    }
}

/// `store-split.allium: config.wrap_up_rebase_timeout`: how long a wrap_up
/// rebase request may stay open, from `requested_at`. The module stores the
/// deadline on the row and the host enforces the row's value; this copy is only
/// for what has no row to read — the `wrap_up` tool's wait cap is derived from
/// it.
pub const WRAP_UP_REBASE_TIMEOUT: Duration = Duration::from_secs(120);

/// `store-split.allium: config.exit_session_teardown_timeout`: how long an
/// exit_session request's teardown may stay open. As [`WRAP_UP_REBASE_TIMEOUT`],
/// the host reads the row's own deadline.
pub const EXIT_SESSION_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(30);

/// What a delivery of session request rows asks of this board.
#[derive(Debug, Default)]
pub struct SessionWork {
    /// `requested` rows with host work, to claim and do
    /// (`HostClaimsSessionRequest` -> `WrapUpRebase` / `ExitSessionTeardown`).
    pub serve: Vec<SessionRequest>,
    /// exit_session rows this board SAW fail, whose window is still to be
    /// killed (`pr-workflow.allium: KillWindowOfFailedExitSession`).
    pub tear_down: Vec<SessionRequest>,
}

/// Which delivered session request rows this board must act on
/// (`HostClaimsSessionRequest`'s trigger and `KillWindowOfFailedExitSession`),
/// the session twin of [`DispatchRequestObserver`].
///
/// A row is served once however often the row pump redelivers the whole set,
/// until [`released`](Self::release) because its claim was never answered. A
/// failed exit_session row is torn down only when this board saw it OPEN first:
/// a row that arrives already failed is history, as `store-split.allium` D12
/// says, and a board that was not running when it failed does not go killing
/// windows for it.
#[derive(Debug)]
pub struct SessionRequestObserver {
    host: String,
    served: HashSet<SessionRequestId>,
    /// exit_session rows of this host seen requested or running, and not yet
    /// torn down.
    open_exits: HashSet<SessionRequestId>,
}

impl SessionRequestObserver {
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            served: HashSet::new(),
            open_exits: HashSet::new(),
        }
    }

    fn remember_open_exits(&mut self, rows: &[SessionRequest]) {
        for r in rows {
            if r.host == self.host && r.kind == SessionRequestKind::ExitSession && r.is_open() {
                self.open_exits.insert(r.id);
            }
        }
    }

    /// Record the open exit_session rows in `rows` WITHOUT handing anything
    /// out. Startup calls it before the sweep that fails the rows a previous
    /// board left running, so that failure is then seen as one, and their
    /// windows are killed.
    pub fn note_open(&mut self, rows: &[SessionRequest]) {
        self.remember_open_exits(rows);
    }

    /// What to do about `rows` now.
    pub fn observe(&mut self, rows: &[SessionRequest]) -> SessionWork {
        let tear_down = rows
            .iter()
            .filter(|r| {
                r.host == self.host
                    && r.kind == SessionRequestKind::ExitSession
                    && r.state == SessionRequestState::Failed
                    && r.tmux_window.is_some()
                    && self.open_exits.remove(&r.id)
            })
            .cloned()
            .collect();
        let serve = rows
            .iter()
            .filter(|r| {
                r.host == self.host
                    && r.state == SessionRequestState::Requested
                    && r.has_host_work()
            })
            .filter(|r| self.served.insert(r.id))
            .cloned()
            .collect();
        self.remember_open_exits(rows);
        SessionWork { serve, tear_down }
    }

    /// Forget that `id` was handed out, so the next [`Self::observe`] that
    /// still sees it `requested` hands it out again.
    pub fn release(&mut self, id: SessionRequestId) {
        self.served.remove(&id);
    }
}
