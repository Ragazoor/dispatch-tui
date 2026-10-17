//! Wrap-up and exit-session as request rows, board side (store-split M5,
//! task #53258).
//!
//! Two halves of one flow, pinned against the in-memory store so the row can
//! be driven and inspected directly:
//!
//! - **Asking** — `TaskService::request_wrap_up` and `request_exit_session`
//!   (`store-split.allium: RequestWrapUp`, `RequestExitSession`). Thin reducer
//!   calls; nothing is rebased or killed here, and a refusal is an ordinary
//!   answer.
//! - **Serving** — `TaskService::serve_session_request`, this host's reaction
//!   to a `requested` row naming it (`pr-workflow.allium: WrapUpRebase`,
//!   `ExitSessionTeardown`): claim the row, do the host work under the row's
//!   deadline, report done or failed. The module sets the Conflict
//!   sub_status, applies the close and runs the epic chain; the host writes no
//!   task field.
//!
//! The deadline is driven through the injected clock: a stalled subprocess
//! spends the time it is given on the [`FixedClock`], never the wall clock.

use super::*;

use std::process::Output;
use std::sync::Mutex;
use std::time::Duration;

use chrono::TimeZone;

use crate::clock::FixedClock;
use crate::dispatch::mock_sequence::DispatchScript;
use crate::models::{
    DispatchOrigin, SessionRequest, SessionRequestId, SessionRequestKind, SessionRequestState,
    TaskUrl, UrlType, WrapUpMode,
};
use crate::process::{MockProcessRunner, ProcessRunner};
use crate::service::SessionOutcome;
use crate::store::SessionRequestAnswer;
use crate::sync::{InjectedFault, MemoryReducerCaller, ReducerCaller, SettledIdentity, SharedRows};

/// This board's host: the one the store handle is built with.
const HOST: &str = "m5-host";

/// The task fixtures' repository, worktree and branch: the ones
/// `DispatchScript::finish()` scripts its git calls for.
const REPO: &str = "/repo";
const WORKTREE: &str = "/repo/.worktrees/42-fix-bug";

struct Fx {
    db: Arc<dyn store::TaskStore>,
    caller: Arc<MemoryReducerCaller>,
    rows: Arc<SharedRows>,
    clock: FixedClock,
    _host_dir: tempfile::TempDir,
}

fn fx() -> Fx {
    let rows = Arc::new(SharedRows::new());
    let clock = FixedClock::new(
        chrono::Utc
            .with_ymd_and_hms(2026, 10, 12, 12, 0, 0)
            .unwrap(),
    );
    let caller = Arc::new(
        MemoryReducerCaller::new(rows.clone(), Arc::new(clock.clone()))
            .with_sender("test-user")
            .with_registered_host(HOST, "test-user")
            .with_registered_host("another-host", "test-user"),
    );
    let identity = Arc::new(SettledIdentity::default());
    identity.settle("test-user");
    let host_dir = tempfile::tempdir().unwrap();
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::new(
        rows.clone(),
        caller.clone(),
        identity,
        Arc::new(clock.clone()),
        HOST.to_string(),
        host_dir.path(),
    ));
    Fx {
        db,
        caller,
        rows,
        clock,
        _host_dir: host_dir,
    }
}

impl Fx {
    fn svc(&self, runner: Arc<dyn ProcessRunner>) -> TaskService {
        TaskService::new(self.db.clone(), runner).with_clock(Arc::new(self.clock.clone()))
    }

    /// A task dispatched on this host: running, with the worktree
    /// `DispatchScript::finish()` expects and a window — the state every
    /// wrap-up starts from. Goes through the dispatch request path, so the
    /// task's host is the one the module writes.
    async fn running_task(&self) -> TaskId {
        self.running_task_at(WORKTREE).await
    }

    async fn running_task_at(&self, worktree: &str) -> TaskId {
        let svc = self.svc(MockProcessRunner::unused());
        let id = svc
            .create_task(make_task_params_on_branch(REPO, "main"))
            .await
            .unwrap();
        let request = match svc
            .request_dispatch(
                id,
                DispatchOrigin::User,
                Some(crate::models::DispatchMode::Dispatch),
            )
            .await
            .unwrap()
        {
            crate::store::RequestDispatchAnswer::Requested(r) => r,
            other => panic!("the dispatch request must apply: {other:?}"),
        };
        assert!(self.db.claim_dispatch_request(request).await.unwrap());
        assert!(self
            .db
            .report_dispatch_provisioned(
                request,
                worktree,
                &crate::models::test_tmux_window(&format!("task-{}", id.0)),
                false
            )
            .await
            .unwrap());
        // The token is voided by a dispatch that finished at or after it
        // (`store-split.allium`, W3), so the wrap-ups that follow start later.
        self.clock.advance(chrono::Duration::seconds(1));
        id
    }

    async fn row(&self, id: SessionRequestId) -> SessionRequest {
        self.db
            .session_request(id)
            .await
            .expect("the session row is in this host's subscription")
    }

    /// A done wrap_up row for `task`: its id is the exit token.
    async fn token(&self, task: TaskId, action: WrapUpMode) -> SessionRequestId {
        let svc = self.svc(DispatchScript::finish().no_remote().shared_runner());
        let id = asked(svc.request_wrap_up(task, action).await.unwrap());
        if action == WrapUpMode::Rebase {
            let served = svc.serve_session_request(self.row(id).await).await;
            assert!(matches!(served.outcome, SessionOutcome::Done), "{served:?}");
        }
        id
    }

    /// The exit_session row of a close of `task`, accepted by the module.
    async fn close(
        &self,
        task: TaskId,
        action: WrapUpMode,
        pr_url: Option<&str>,
    ) -> SessionRequestId {
        let token = self.token(task, action).await;
        let svc = self.svc(MockProcessRunner::unused());
        asked(
            svc.request_exit_session(task, token, action, pr_url.map(str::to_string))
                .await
                .unwrap(),
        )
    }
}

fn asked(answer: SessionRequestAnswer) -> SessionRequestId {
    match answer {
        SessionRequestAnswer::Requested(id) => id,
        SessionRequestAnswer::Refused(why) => panic!("expected the request to apply: {why}"),
    }
}

// ===========================================================================
// Asking
// ===========================================================================

/// The request names THIS host, is a `requested` row with the row's deadline,
/// and runs nothing: the runner panics on any call.
#[tokio::test]
async fn request_wrap_up_writes_a_row_naming_this_host_and_runs_nothing() {
    let fx = fx();
    let task = fx.running_task().await;
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());
    let before = svc.get_task(task).await.unwrap();

    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());

    let row = fx.row(id).await;
    assert_eq!(row.task_id, task);
    assert_eq!(row.kind, SessionRequestKind::WrapUp);
    assert_eq!(row.action, WrapUpMode::Rebase);
    assert_eq!(row.host, HOST, "the rebase runs where the worktree is");
    assert_eq!(row.state, SessionRequestState::Requested);
    assert!(row.deadline.is_some());
    assert!(runner.recorded_calls().is_empty());
    assert_eq!(
        svc.get_task(task).await.unwrap(),
        before,
        "the request writes no task field"
    );
}

/// done and pr need no host work: the row is written already done, and its id
/// is the token.
#[tokio::test]
async fn a_done_or_pr_wrap_up_is_answered_already_done() {
    let fx = fx();
    let svc = fx.svc(MockProcessRunner::unused());
    for action in [WrapUpMode::Done, WrapUpMode::Pr] {
        let task = fx.running_task().await;
        let id = asked(svc.request_wrap_up(task, action).await.unwrap());
        let row = fx.row(id).await;
        assert_eq!(row.state, SessionRequestState::Done, "{action}");
        assert!(!row.has_host_work());
    }
}

/// A refusal is an ORDINARY answer carrying the module's words, and writes
/// nothing.
#[tokio::test]
async fn a_refused_wrap_up_request_is_an_answer_and_writes_nothing() {
    let fx = fx();
    let svc = fx.svc(MockProcessRunner::unused());
    let backlog = svc
        .create_task(make_task_params_on_branch(REPO, "main"))
        .await
        .unwrap();

    let answer = svc
        .request_wrap_up(backlog, WrapUpMode::Rebase)
        .await
        .expect("a refusal is not an error");

    match answer {
        SessionRequestAnswer::Refused(why) => {
            assert!(why.contains("cannot be wrapped up"), "{why}")
        }
        other => panic!("a backlog task must be refused, got {other:?}"),
    }
    assert!(fx.rows.session_requests().is_empty());
}

/// The close is the module's: the task is done, the token consumed and the
/// teardown row written, all by the one call — and nothing is killed here.
#[tokio::test]
async fn request_exit_session_closes_through_the_module_and_kills_nothing() {
    let fx = fx();
    let task = fx.running_task().await;
    let token = fx.token(task, WrapUpMode::Rebase).await;
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());

    let id = asked(
        svc.request_exit_session(task, token, WrapUpMode::Rebase, None)
            .await
            .unwrap(),
    );

    let stored = svc.get_task(task).await.unwrap();
    assert_eq!(stored.status, TaskStatus::Done);
    assert!(stored.tmux_window.is_none());
    let row = fx.row(id).await;
    assert_eq!(row.kind, SessionRequestKind::ExitSession);
    assert_eq!(row.wrap_up_request, Some(token));
    assert!(row.tmux_window.is_some());
    assert!(
        runner.recorded_calls().is_empty(),
        "the teardown is the host's, from the row"
    );
}

/// A refusal carries the module's wording, so the tool can pass it on, and
/// leaves the token valid.
#[tokio::test]
async fn a_refused_exit_session_is_an_answer_with_the_modules_words() {
    let fx = fx();
    let task = fx.running_task().await;
    let svc = fx.svc(MockProcessRunner::unused());

    let none = svc
        .request_exit_session(task, SessionRequestId(1), WrapUpMode::Done, None)
        .await
        .unwrap();
    assert_eq!(
        none,
        SessionRequestAnswer::Refused("no exit token — call wrap_up first".into())
    );

    let token = fx.token(task, WrapUpMode::Pr).await;
    let no_url = svc
        .request_exit_session(task, token, WrapUpMode::Pr, None)
        .await
        .unwrap();
    assert!(
        matches!(&no_url, SessionRequestAnswer::Refused(why) if why.contains("pr_url is required")),
        "{no_url:?}"
    );
    assert_eq!(
        svc.get_task(task).await.unwrap().status,
        TaskStatus::Running
    );
    asked(
        svc.request_exit_session(
            task,
            token,
            WrapUpMode::Pr,
            Some("https://github.com/o/r/pull/9".into()),
        )
        .await
        .unwrap(),
    );
}

// ===========================================================================
// Serving a wrap_up rebase: WrapUpRebase
// ===========================================================================

/// The happy path: claim, run `finish_task`, report done. The row is the token
/// from then on, the base branch's repository is reported moved, and the host
/// wrote no task field (the claim's Conflict clear and the conflict write are
/// the module's).
#[tokio::test]
async fn serve_claims_rebases_and_reports_done() {
    let fx = fx();
    let task = fx.running_task().await;
    let runner = DispatchScript::finish().no_remote().shared_runner();
    let svc = fx.svc(runner.clone());
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());
    let before = svc.get_task(task).await.unwrap();

    let served = svc.serve_session_request(fx.row(id).await).await;

    assert!(matches!(served.outcome, SessionOutcome::Done), "{served:?}");
    assert_eq!(
        served.rebased.as_deref(),
        Some(REPO),
        "BranchRebased: the repository whose base branch just moved"
    );
    let row = fx.row(id).await;
    assert_eq!(row.state, SessionRequestState::Done);
    assert!(row.claimed_at.is_some());
    assert!(!runner.recorded_calls().is_empty(), "the rebase ran");
    assert_eq!(svc.get_task(task).await.unwrap(), before);
}

/// A worktree path that names no branch is reported failed without running
/// git: the row cannot be served, which is not a failed rebase. (The tool
/// refuses it before writing a row; this is the host's own guard.)
#[tokio::test]
async fn an_underivable_branch_is_reported_failed_without_running_git() {
    let fx = fx();
    // A root path has no final component, so no branch name.
    let task = fx.running_task_at("/").await;
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());

    let served = svc.serve_session_request(fx.row(id).await).await;

    let SessionOutcome::Failed(reason) = &served.outcome else {
        panic!("expected Failed, got {served:?}");
    };
    assert!(
        reason.contains("Cannot derive branch from worktree"),
        "{reason}"
    );
    assert!(runner.recorded_calls().is_empty());
    assert_eq!(fx.row(id).await.state, SessionRequestState::Failed);
}

/// A task deleted between the request and the claim takes its rows with it,
/// so the claim is refused and no row is left behind.
#[tokio::test]
async fn a_row_whose_task_is_gone_is_removed_and_its_claim_refused() {
    let fx = fx();
    let task = fx.running_task().await;
    let svc = fx.svc(MockProcessRunner::unused());
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());
    let stale = fx.row(id).await;
    // A task is deleted only from Done (`tasks.allium: DeleteTask`).
    fx.db
        .patch_task(task, &store::TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    assert!(fx.caller.delete_task(task).await.unwrap().won());

    let served = svc.serve_session_request(stale).await;

    assert!(
        matches!(served.outcome, SessionOutcome::ClaimRefused(_)),
        "the module removed the row with the task: {served:?}"
    );
    assert!(fx.db.session_request(id).await.is_none());
}

/// A rebase conflict is reported failed with conflict = true and the reason
/// naming the files; THE MODULE then sets the Conflict sub_status, and no
/// token is issued.
#[tokio::test]
async fn serve_reports_a_conflict_and_the_module_sets_the_sub_status() {
    let fx = fx();
    let task = fx.running_task().await;
    let svc = fx.svc(
        DispatchScript::finish()
            .no_remote()
            .rebase_conflicts_in_stderr(&["foo.rs"])
            .shared_runner(),
    );
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());

    let served = svc.serve_session_request(fx.row(id).await).await;

    let SessionOutcome::Failed(reason) = &served.outcome else {
        panic!("expected Failed, got {served:?}");
    };
    assert!(reason.contains("foo.rs"), "{reason}");
    assert!(served.rebased.is_none(), "nothing moved the base branch");
    let row = fx.row(id).await;
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some(reason.as_str()));
    assert_eq!(
        svc.get_task(task).await.unwrap().sub_status,
        SubStatus::Conflict
    );
    assert_eq!(
        svc.get_task(task).await.unwrap().status,
        TaskStatus::Running
    );
}

/// Any other failure — a dirty primary worktree here — is reported with
/// conflict = false, so the task is NOT flagged: there was never a rebase to
/// conflict.
#[tokio::test]
async fn serve_reports_a_non_conflict_failure_without_flagging_the_task() {
    let fx = fx();
    let task = fx.running_task().await;
    let svc = fx.svc(
        DispatchScript::finish()
            .dirty_primary(&["unrelated.rs"])
            .shared_runner(),
    );
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());

    let served = svc.serve_session_request(fx.row(id).await).await;

    let SessionOutcome::Failed(reason) = &served.outcome else {
        panic!("expected Failed, got {served:?}");
    };
    assert!(reason.contains("uncommitted"), "{reason}");
    assert_ne!(
        svc.get_task(task).await.unwrap().sub_status,
        SubStatus::Conflict
    );
}

/// A row already conflicted is cleared by the CLAIM (module-side), so the card
/// stops showing the conflict while the rebase runs.
#[tokio::test]
async fn the_claim_clears_a_previous_conflict_before_the_rebase_runs() {
    let fx = fx();
    let task = fx.running_task().await;
    let svc = fx.svc(DispatchScript::finish().no_remote().shared_runner());
    fx.db
        .patch_task(
            task,
            &store::TaskPatch::new().sub_status(SubStatus::Conflict),
        )
        .await
        .unwrap();
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());

    svc.serve_session_request(fx.row(id).await).await;

    assert_ne!(
        svc.get_task(task).await.unwrap().sub_status,
        SubStatus::Conflict
    );
}

/// `OnlyTheClaimedSessionRequestIsServed`: a row another board (or an earlier
/// delivery) already claimed is refused at the claim, and nothing runs.
#[tokio::test]
async fn serve_runs_nothing_when_its_claim_is_refused() {
    let fx = fx();
    let task = fx.running_task().await;
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());
    let stale_copy = fx.row(id).await;
    assert!(fx.db.claim_session_request(id).await.unwrap());

    let served = svc.serve_session_request(stale_copy).await;

    assert!(
        matches!(served.outcome, SessionOutcome::ClaimRefused(_)),
        "{served:?}"
    );
    assert!(runner.recorded_calls().is_empty());
}

/// A claim that got no answer is not a refusal: the reactor serves the row
/// again, and nothing runs meanwhile.
#[tokio::test]
async fn serve_tells_an_unsent_claim_from_a_refused_one() {
    let fx = fx();
    let task = fx.running_task().await;
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());
    fx.caller
        .fail_next("claim_session_request", InjectedFault::Unsent);

    let served = svc.serve_session_request(fx.row(id).await).await;

    assert!(
        matches!(served.outcome, SessionOutcome::ClaimNotSent(_)),
        "{served:?}"
    );
    assert_eq!(fx.row(id).await.state, SessionRequestState::Requested);
    assert!(runner.recorded_calls().is_empty());
}

/// A row naming another host is refused by the module at the claim: only the
/// row's own host claims it.
#[tokio::test]
async fn serve_ignores_a_row_for_another_host() {
    let fx = fx();
    let task = fx.running_task().await;
    // The worktree is on the other machine, so the module accepts a wrap-up
    // requested as that host (its owner is this test's user).
    fx.db
        .patch_task(task, &store::TaskPatch::new().host(Some("another-host")))
        .await
        .unwrap();
    let id = match fx
        .caller
        .request_wrap_up(task, "another-host".into(), "rebase".into())
        .await
        .unwrap()
    {
        crate::sync::ReducerOutcome::Applied(ids) => SessionRequestId(ids[0]),
        other => panic!("seeding the other host's request must apply: {other:?}"),
    };
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());

    let served = svc.serve_session_request(fx.row(id).await).await;

    assert!(
        matches!(served.outcome, SessionOutcome::ClaimRefused(_)),
        "{served:?}"
    );
    assert!(runner.recorded_calls().is_empty());
    assert_eq!(fx.row(id).await.state, SessionRequestState::Requested);
}

// ===========================================================================
// The row's deadline: WrapUpRebase, W7
// ===========================================================================

/// A runner that spends `by` on the injected clock the first time a command
/// containing `needle` is run, then answers from `inner`; `then_fail` makes
/// that call fail the way a killed subprocess does. How a test makes the
/// deadline pass mid-rebase without waiting.
struct SpendsTime {
    inner: MockProcessRunner,
    clock: FixedClock,
    needle: &'static str,
    by: chrono::Duration,
    then_fail: bool,
    seen: Mutex<Vec<String>>,
    spent: Mutex<bool>,
}

impl SpendsTime {
    fn new(
        inner: MockProcessRunner,
        clock: &FixedClock,
        needle: &'static str,
        by: chrono::Duration,
        then_fail: bool,
    ) -> Self {
        Self {
            inner,
            clock: clock.clone(),
            needle,
            by,
            then_fail,
            seen: Mutex::new(Vec::new()),
            spent: Mutex::new(false),
        }
    }

    fn on(&self, program: &str, args: &[&str]) -> bool {
        let line = format!("{program} {}", args.join(" "));
        self.seen.lock().unwrap().push(line.clone());
        let mut spent = self.spent.lock().unwrap();
        if line.contains(self.needle) && !*spent {
            *spent = true;
            self.clock.advance(self.by);
            return true;
        }
        false
    }
}

impl ProcessRunner for SpendsTime {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        if self.on(program, args) && self.then_fail {
            anyhow::bail!("{program} was killed");
        }
        self.inner.run(program, args)
    }

    fn run_with_timeout(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> anyhow::Result<Output> {
        if self.on(program, args) && self.then_fail {
            anyhow::bail!("{program} timed out after {timeout:?}");
        }
        self.inner.run_with_timeout(program, args, timeout)
    }

    fn agent_binaries(&self) -> crate::process::AgentBinaries {
        self.inner.agent_binaries()
    }
}

/// A row whose deadline has passed when this host gets to it is refused at the
/// claim (the module checks `now < deadline`), runs nothing, and is left for
/// the backstop.
#[tokio::test]
async fn a_row_served_after_its_deadline_runs_nothing() {
    let fx = fx();
    let task = fx.running_task().await;
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());
    fx.clock.advance(chrono::Duration::seconds(121));

    let served = svc.serve_session_request(fx.row(id).await).await;

    assert!(
        matches!(served.outcome, SessionOutcome::ClaimRefused(_)),
        "{served:?}"
    );
    assert!(runner.recorded_calls().is_empty());
    assert_eq!(fx.row(id).await.state, SessionRequestState::Requested);
}

/// The behavioural test W7 asks for: a rebase whose subprocess stalls fails
/// at the row's deadline. The stalled call is bounded by the time left to the
/// deadline, the attempt stops there, the fast-forward never runs, and the
/// failure is reported "deadline passed".
#[tokio::test]
async fn a_stalled_rebase_fails_at_the_rows_deadline() {
    let fx = fx();
    let task = fx.running_task().await;
    let runner = Arc::new(SpendsTime::new(
        DispatchScript::finish().no_remote().runner(),
        &fx.clock,
        "rebase",
        chrono::Duration::seconds(30),
        true,
    ));
    let svc = fx.svc(runner.clone());
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());
    // The row sat requested for 100s before this host served it: 20s are left.
    fx.clock.advance(chrono::Duration::seconds(100));

    let served = svc.serve_session_request(fx.row(id).await).await;

    let SessionOutcome::Failed(reason) = &served.outcome else {
        panic!("expected Failed at the deadline, got {served:?}");
    };
    assert_eq!(reason, crate::dispatch::DEADLINE_PASSED);
    let row = fx.row(id).await;
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("deadline passed"));
    let seen = runner.seen.lock().unwrap().clone();
    assert!(
        !seen.iter().any(|c| c.contains("merge --ff-only")),
        "nothing may be fast-forwarded past the deadline: {seen:?}"
    );
    assert_ne!(
        svc.get_task(task).await.unwrap().sub_status,
        SubStatus::Conflict,
        "a deadline is not a conflict"
    );
}

/// THE ROW'S DEADLINE BOUNDS THE REBASE, but the abort runs under its own
/// bound: a rebase killed at the deadline is still aborted, so the worktree is
/// not left mid-rebase.
#[tokio::test]
async fn a_rebase_killed_at_the_deadline_is_still_aborted() {
    let fx = fx();
    let task = fx.running_task().await;
    let runner = Arc::new(SpendsTime::new(
        DispatchScript::finish().no_remote().runner(),
        &fx.clock,
        "rebase",
        chrono::Duration::seconds(30),
        true,
    ));
    let svc = fx.svc(runner.clone());
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());
    fx.clock.advance(chrono::Duration::seconds(100));

    let served = svc.serve_session_request(fx.row(id).await).await;

    assert!(
        matches!(&served.outcome, SessionOutcome::Failed(r) if r == crate::dispatch::DEADLINE_PASSED),
        "{served:?}"
    );
    let seen = runner.seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|c| c.contains("rebase --abort")),
        "the killed rebase must be aborted: {seen:?}"
    );
}

/// A rebase that conflicts after the deadline passed is still aborted: the
/// abort is not refused by the deadline runner.
#[tokio::test]
async fn a_conflicted_rebase_past_the_deadline_is_still_aborted() {
    let fx = fx();
    let task = fx.running_task().await;
    let runner = Arc::new(SpendsTime::new(
        DispatchScript::finish()
            .no_remote()
            .rebase_conflicts_in_stderr(&["foo.rs"])
            .runner(),
        &fx.clock,
        "rebase",
        chrono::Duration::seconds(130),
        false,
    ));
    let svc = fx.svc(runner.clone());
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());

    let _ = svc.serve_session_request(fx.row(id).await).await;

    let seen = runner.seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|c| c.contains("rebase --abort")),
        "the abort must run past the deadline: {seen:?}"
    );
}

/// A rebase that FINISHES after the deadline has still moved the base branch
/// (BranchRebased is reported), but the module refuses its late done report;
/// the host then reports failed "deadline passed" so the row does not wait for
/// the backstop, and no token is issued.
#[tokio::test]
async fn a_rebase_finishing_past_the_deadline_is_reported_failed_but_still_rebased() {
    let fx = fx();
    let task = fx.running_task().await;
    let runner = Arc::new(SpendsTime::new(
        DispatchScript::finish().no_remote().runner(),
        &fx.clock,
        "merge --ff-only",
        chrono::Duration::seconds(130),
        false,
    ));
    let svc = fx.svc(runner);
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());

    let served = svc.serve_session_request(fx.row(id).await).await;

    assert!(
        matches!(&served.outcome, SessionOutcome::Failed(r) if r == crate::dispatch::DEADLINE_PASSED),
        "{served:?}"
    );
    assert_eq!(
        served.rebased.as_deref(),
        Some(REPO),
        "the base branch moved on this disk either way"
    );
    let row = fx.row(id).await;
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("deadline passed"));
}

/// A done report that got no answer is sent once more: the module accepts a
/// repeat of a report that already landed (`HostRepeatsSessionRequestDone`).
#[tokio::test]
async fn an_unanswered_done_report_is_sent_again() {
    let fx = fx();
    let task = fx.running_task().await;
    let svc = fx.svc(DispatchScript::finish().no_remote().shared_runner());
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());
    fx.caller
        .fail_next("report_session_request_done", InjectedFault::ReplyLost);

    let served = svc.serve_session_request(fx.row(id).await).await;

    assert!(matches!(served.outcome, SessionOutcome::Done), "{served:?}");
    assert_eq!(fx.row(id).await.state, SessionRequestState::Done);
}

/// A runner that runs `action` once, just before the first call whose command
/// line contains `needle`, then answers from `inner`: how a test makes the row
/// fail mid-rebase (the module's backstop firing) at a known point.
struct OnCall {
    inner: MockProcessRunner,
    needle: &'static str,
    action: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl OnCall {
    fn fire_if_due(&self, program: &str, args: &[&str]) {
        let line = format!("{program} {}", args.join(" "));
        if line.contains(self.needle) {
            if let Some(action) = self.action.lock().unwrap().take() {
                action();
            }
        }
    }
}

impl ProcessRunner for OnCall {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        self.fire_if_due(program, args);
        self.inner.run(program, args)
    }

    fn run_with_timeout(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> anyhow::Result<Output> {
        self.fire_if_due(program, args);
        self.inner.run_with_timeout(program, args, timeout)
    }

    fn agent_binaries(&self) -> crate::process::AgentBinaries {
        self.inner.agent_binaries()
    }
}

/// A row that failed under this host — the module's backstop firing mid-rebase
/// — makes the done report refused; the rebase stays in git, no token is
/// issued, and the row keeps the module's reason.
#[tokio::test]
async fn a_row_that_failed_under_the_host_keeps_the_modules_reason() {
    let fx = fx();
    let task = fx.running_task().await;
    let id_cell = Arc::new(Mutex::new(None));
    let caller = fx.caller.clone();
    let cell = id_cell.clone();
    let runner = Arc::new(OnCall {
        inner: DispatchScript::finish().no_remote().runner(),
        needle: "merge --ff-only",
        action: Mutex::new(Some(Box::new(move || {
            let id = cell.lock().unwrap().expect("the row id is known by then");
            caller.expire_session_request(id);
        }))),
    });
    let svc = fx.svc(runner);
    let id = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());
    *id_cell.lock().unwrap() = Some(id);

    let served = svc.serve_session_request(fx.row(id).await).await;

    assert!(
        matches!(&served.outcome, SessionOutcome::Failed(r) if r == "timed out"),
        "the row keeps the module's reason: {served:?}"
    );
    assert_eq!(
        served.rebased.as_deref(),
        Some(REPO),
        "the rebase happened in git"
    );
    let row = fx.row(id).await;
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("timed out"));
}

// ===========================================================================
// Serving an exit_session row: ExitSessionTeardown
// ===========================================================================

/// Claim, kill the window the ROW names (the task no longer carries it), report
/// done.
#[tokio::test]
async fn serve_kills_the_window_the_row_names_and_reports_done() {
    let fx = fx();
    let task = fx.running_task().await;
    let close = fx.close(task, WrapUpMode::Rebase, None).await;
    let runner = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
    let svc = fx.svc(runner.clone());
    let row = fx.row(close).await;
    let window = row.tmux_window.clone().unwrap();

    let served = svc.serve_session_request(row).await;

    assert!(matches!(served.outcome, SessionOutcome::Done), "{served:?}");
    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains("kill-window"), "{calls:?}");
    let _ = window;
    assert_eq!(fx.row(close).await.state, SessionRequestState::Done);
    assert_eq!(
        svc.get_task(task).await.unwrap().status,
        TaskStatus::Done,
        "the teardown writes no task field"
    );
}

/// An already-gone window counts as torn down: that is the state the kill
/// asks for.
#[tokio::test]
async fn an_already_gone_window_counts_as_done() {
    let fx = fx();
    let task = fx.running_task().await;
    let close = fx.close(task, WrapUpMode::Done, None).await;
    let runner = Arc::new(MockProcessRunner::new(vec![]).with_windows(&[]));
    let svc = fx.svc(runner);

    let served = svc.serve_session_request(fx.row(close).await).await;

    assert!(matches!(served.outcome, SessionOutcome::Done), "{served:?}");
    assert_eq!(fx.row(close).await.state, SessionRequestState::Done);
}

/// A failed teardown is reported failed with the reason and does NOT revert
/// the close: the task stays Done.
#[tokio::test]
async fn a_failed_teardown_is_recorded_and_never_reverts_the_close() {
    let fx = fx();
    let task = fx.running_task().await;
    let close = fx.close(task, WrapUpMode::Rebase, None).await;
    let runner = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "kill-window: server exited unexpectedly",
    )]));
    let svc = fx.svc(runner);

    let served = svc.serve_session_request(fx.row(close).await).await;

    let SessionOutcome::Failed(reason) = &served.outcome else {
        panic!("expected Failed, got {served:?}");
    };
    assert!(reason.contains("kill-window"), "{reason}");
    let row = fx.row(close).await;
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some(reason.as_str()));
    assert_eq!(svc.get_task(task).await.unwrap().status, TaskStatus::Done);
}

/// ClosePrOnDone: a close into Done of a task that carries a PR url runs
/// `gh pr close` on this host, alongside the kill. The close of a `pr` action
/// goes to Review, so it closes nothing.
#[tokio::test]
async fn a_close_into_done_of_a_task_with_a_pr_runs_gh_pr_close() {
    let fx = fx();
    let task = fx.running_task().await;
    let url = TaskUrl::new("https://github.com/o/r/pull/9".to_string(), UrlType::Pr);
    fx.db
        .patch_task(task, &store::TaskPatch::new().url(Some(&url)))
        .await
        .unwrap();
    let close = fx.close(task, WrapUpMode::Done, None).await;
    let runner = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(),
        MockProcessRunner::ok(),
    ]));
    let svc = fx.svc(runner.clone());

    let served = svc.serve_session_request(fx.row(close).await).await;
    svc.wait_for_background_pr_closes().await;

    assert!(matches!(served.outcome, SessionOutcome::Done), "{served:?}");
    let calls = runner.flattened_calls();
    assert!(
        calls
            .iter()
            .any(|c| c.contains("gh pr close https://github.com/o/r/pull/9")),
        "{calls:?}"
    );

    // A pr close lands in Review and closes nothing.
    let other = fx.running_task().await;
    let pr_close = fx
        .close(
            other,
            WrapUpMode::Pr,
            Some("https://github.com/o/r/pull/10"),
        )
        .await;
    let runner = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
    let svc = fx.svc(runner.clone());
    svc.serve_session_request(fx.row(pr_close).await).await;
    svc.wait_for_background_pr_closes().await;
    assert!(
        !runner
            .flattened_calls()
            .iter()
            .any(|c| c.contains("gh pr close")),
        "{:?}",
        runner.flattened_calls()
    );
}

/// A teardown that cannot finish by its deadline (30s from `requested_at`) is
/// abandoned and reported "deadline passed".
#[tokio::test]
async fn a_teardown_past_its_deadline_is_not_started() {
    let fx = fx();
    let task = fx.running_task().await;
    let close = fx.close(task, WrapUpMode::Done, None).await;
    let row = fx.row(close).await;
    fx.clock.advance(chrono::Duration::seconds(31));
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());

    let served = svc.serve_session_request(row).await;

    // The module refuses the claim past the deadline, so nothing ran and the
    // backstop will fail the row; the window is the one thing it leaves, which
    // the failed-row rule below picks up.
    assert!(
        matches!(served.outcome, SessionOutcome::ClaimRefused(_)),
        "{served:?}"
    );
    assert!(runner.recorded_calls().is_empty());
}

// ===========================================================================
// KillWindowOfFailedExitSession
// ===========================================================================

/// A row this host sees fail still owes its window a kill: the close already
/// cleared the window from the task, so nothing else would ever kill it. Best
/// effort — nothing is reported, the row is already finished.
#[tokio::test]
async fn a_failed_exit_session_row_has_its_window_killed() {
    let fx = fx();
    let task = fx.running_task().await;
    let close = fx.close(task, WrapUpMode::Done, None).await;
    fx.caller.expire_session_request(close);
    let row = fx.row(close).await;
    assert_eq!(row.state, SessionRequestState::Failed);
    let runner = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
    let svc = fx.svc(runner.clone());

    svc.tear_down_failed_exit_window(&row).await;

    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains("kill-window"), "{calls:?}");
    assert_eq!(fx.row(close).await, row, "nothing is reported");
}

/// Only an exit_session row with a window has anything to kill; a failed
/// wrap_up row does not, and a kill that fails is swallowed.
#[tokio::test]
async fn only_a_failed_exit_session_row_with_a_window_is_torn_down() {
    let fx = fx();
    let task = fx.running_task().await;
    let wrap = asked(
        fx.svc(MockProcessRunner::unused())
            .request_wrap_up(task, WrapUpMode::Rebase)
            .await
            .unwrap(),
    );
    fx.caller.expire_session_request(wrap);
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());

    svc.tear_down_failed_exit_window(&fx.row(wrap).await).await;
    assert!(runner.recorded_calls().is_empty());

    let other = fx.running_task().await;
    let close = fx.close(other, WrapUpMode::Done, None).await;
    fx.caller.expire_session_request(close);
    let failing = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "boom",
    )]));
    fx.svc(failing.clone())
        .tear_down_failed_exit_window(&fx.row(close).await)
        .await;
    assert_eq!(
        failing.recorded_calls().len(),
        1,
        "attempted once, swallowed"
    );
}

// ===========================================================================
// HostRestartFailsInterruptedSessionRequests
// ===========================================================================

/// A starting board fails the rows its previous board left running; rows still
/// requested are left for the catch-up.
#[tokio::test]
async fn fail_interrupted_fails_running_rows_and_leaves_requested_ones() {
    let fx = fx();
    let running_task = fx.running_task().await;
    let waiting_task = fx.running_task().await;
    let svc = fx.svc(MockProcessRunner::unused());
    let running = asked(
        svc.request_wrap_up(running_task, WrapUpMode::Rebase)
            .await
            .unwrap(),
    );
    assert!(fx.db.claim_session_request(running).await.unwrap());
    let waiting = asked(
        svc.request_wrap_up(waiting_task, WrapUpMode::Rebase)
            .await
            .unwrap(),
    );

    svc.fail_interrupted_session_requests().await.unwrap();

    let row = fx.row(running).await;
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("host restarted"));
    assert_eq!(fx.row(waiting).await.state, SessionRequestState::Requested);
}

/// A kill still running at the row's deadline (30s from `requested_at`) is
/// abandoned and reported failed "deadline passed"; the close stands.
#[tokio::test]
async fn a_teardown_that_stalls_past_its_deadline_is_reported_deadline_passed() {
    let fx = fx();
    let task = fx.running_task().await;
    let close = fx.close(task, WrapUpMode::Done, None).await;
    let runner = Arc::new(SpendsTime::new(
        MockProcessRunner::new(vec![MockProcessRunner::ok()]),
        &fx.clock,
        "kill-window",
        chrono::Duration::seconds(10),
        true,
    ));
    let svc = fx.svc(runner);
    // 25s of the 30s are gone when this host serves the row.
    fx.clock.advance(chrono::Duration::seconds(25));

    let served = svc.serve_session_request(fx.row(close).await).await;

    assert!(
        matches!(&served.outcome, SessionOutcome::Failed(r) if r == crate::dispatch::DEADLINE_PASSED),
        "{served:?}"
    );
    assert_eq!(
        fx.row(close).await.reason.as_deref(),
        Some("deadline passed")
    );
    assert_eq!(svc.get_task(task).await.unwrap().status, TaskStatus::Done);
}

// ===========================================================================
// update_tasks' done-path close: RequestCloseTask (store-split M7, X1)
// ===========================================================================

impl Fx {
    /// The one `close` row of `task`.
    async fn close_row(&self, task: TaskId) -> SessionRequest {
        let mut rows: Vec<_> = self
            .db
            .session_requests()
            .await
            .into_iter()
            .filter(|r| r.task_id == task && r.kind == SessionRequestKind::Close)
            .collect();
        assert_eq!(rows.len(), 1, "exactly one close row: {rows:?}");
        rows.remove(0)
    }
}

/// The close is the module's: Done, the window cleared and a `close` row naming
/// it, all from the one call. Nothing is killed or closed here.
#[tokio::test]
async fn close_task_closes_through_the_module_and_kills_nothing() {
    let fx = fx();
    let task = fx.running_task().await;
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());

    svc.close_task(task).await.unwrap();

    let stored = svc.get_task(task).await.unwrap();
    assert_eq!(stored.status, TaskStatus::Done);
    assert!(stored.tmux_window.is_none());
    assert!(stored.completed_at.is_some());
    let row = fx.close_row(task).await;
    assert_eq!(row.host, HOST);
    assert_eq!(row.state, SessionRequestState::Requested);
    assert!(row.tmux_window.is_some());
    assert!(row.wrap_up_request.is_none(), "no token");
    assert!(
        runner.recorded_calls().is_empty(),
        "the teardown is the host's, from the row"
    );
}

/// A task with neither a window nor a PR has no host work: no row.
#[tokio::test]
async fn close_task_without_host_work_writes_no_row() {
    let fx = fx();
    let svc = fx.svc(MockProcessRunner::unused());
    let task = svc
        .create_task(make_task_params_on_branch(REPO, "main"))
        .await
        .unwrap();

    svc.close_task(task).await.unwrap();

    assert_eq!(svc.get_task(task).await.unwrap().status, TaskStatus::Done);
    assert!(fx.rows.session_requests().is_empty());
}

/// The module refuses a close of a task that is gone, and the call says so.
#[tokio::test]
async fn close_task_of_a_missing_task_is_an_error() {
    let fx = fx();
    let svc = fx.svc(MockProcessRunner::unused());
    assert!(svc.close_task(TaskId(999_999)).await.is_err());
}

/// A close row is served as an exit_session teardown: claim, kill the window
/// the row names, run `gh pr close` for the task's PR, report done.
#[tokio::test]
async fn serve_tears_down_a_close_row_and_closes_the_pr() {
    let fx = fx();
    let task = fx.running_task().await;
    let url = TaskUrl::new("https://github.com/o/r/pull/9".to_string(), UrlType::Pr);
    fx.db
        .patch_task(task, &store::TaskPatch::new().url(Some(&url)))
        .await
        .unwrap();
    fx.svc(MockProcessRunner::unused())
        .close_task(task)
        .await
        .unwrap();
    let runner = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(),
        MockProcessRunner::ok(),
    ]));
    let svc = fx.svc(runner.clone());
    let row = fx.close_row(task).await;

    let served = svc.serve_session_request(row.clone()).await;
    svc.wait_for_background_pr_closes().await;

    assert!(matches!(served.outcome, SessionOutcome::Done), "{served:?}");
    let calls = runner.flattened_calls();
    assert!(calls.iter().any(|c| c.contains("kill-window")), "{calls:?}");
    assert!(
        calls
            .iter()
            .any(|c| c.contains("gh pr close https://github.com/o/r/pull/9")),
        "{calls:?}"
    );
    assert_eq!(fx.row(row.id).await.state, SessionRequestState::Done);
    assert_eq!(svc.get_task(task).await.unwrap().status, TaskStatus::Done);
}

/// A close row with no window (a task that carries only a PR) has nothing to
/// kill: no tmux call, `gh pr close` runs, the row is done. An exit_session row
/// never lacks a window; a close row may.
#[tokio::test]
async fn serve_a_close_row_without_a_window_only_closes_the_pr() {
    let fx = fx();
    let svc = fx.svc(MockProcessRunner::unused());
    let task = svc
        .create_task(make_task_params_on_branch(REPO, "main"))
        .await
        .unwrap();
    let url = TaskUrl::new("https://github.com/o/r/pull/9".to_string(), UrlType::Pr);
    fx.db
        .patch_task(task, &store::TaskPatch::new().url(Some(&url)))
        .await
        .unwrap();
    // No host of its own: the row names the caller's host, which is this one.
    svc.close_task(task).await.unwrap();
    let row = fx.close_row(task).await;
    assert!(row.tmux_window.is_none());
    let runner = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
    let svc = fx.svc(runner.clone());

    let served = svc.serve_session_request(row.clone()).await;
    svc.wait_for_background_pr_closes().await;

    assert!(matches!(served.outcome, SessionOutcome::Done), "{served:?}");
    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains("gh pr close"), "{calls:?}");
    assert_eq!(fx.row(row.id).await.state, SessionRequestState::Done);
}

/// KillWindowOfFailedExitSession covers a close row too: the close cleared the
/// window from the task, so a row that failed before its teardown ran still
/// owes the kill. The PR is left open (best effort ends at the window).
#[tokio::test]
async fn a_failed_close_row_has_its_window_killed() {
    let fx = fx();
    let task = fx.running_task().await;
    fx.svc(MockProcessRunner::unused())
        .close_task(task)
        .await
        .unwrap();
    let row = fx.close_row(task).await;
    fx.caller.expire_session_request(row.id);
    let row = fx.row(row.id).await;
    assert_eq!(row.state, SessionRequestState::Failed);
    let runner = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
    let svc = fx.svc(runner.clone());

    svc.tear_down_failed_exit_window(&row).await;

    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains("kill-window"), "{calls:?}");
}

/// A stuck wrap_up rebase does not block the close: the module fails it "task
/// closed", and the host's later report is refused.
#[tokio::test]
async fn close_task_fails_a_stuck_wrap_up_instead_of_refusing() {
    let fx = fx();
    let task = fx.running_task().await;
    let svc = fx.svc(MockProcessRunner::unused());
    let stuck = asked(svc.request_wrap_up(task, WrapUpMode::Rebase).await.unwrap());

    svc.close_task(task).await.unwrap();

    let stuck = fx.row(stuck).await;
    assert_eq!(stuck.state, SessionRequestState::Failed);
    assert_eq!(stuck.reason.as_deref(), Some("task closed"));
}
