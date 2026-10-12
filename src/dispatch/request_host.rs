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

use std::collections::HashSet;
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::clock::Clock;
use crate::models::{DispatchRequest, DispatchRequestId, DispatchRequestState};
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
/// The provisioning-failure rollback is exempt: it reaches the unwrapped
/// runner through [`ProcessRunner::unbounded_for_teardown`], because what an
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

    fn unbounded_for_teardown(&self) -> Option<&dyn ProcessRunner> {
        Some(&*self.inner)
    }
}
