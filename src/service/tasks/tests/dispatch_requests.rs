//! Dispatch as request rows, board side (store-split M4, task #53257).
//!
//! Two halves of one flow, pinned against the in-memory store so the row can
//! be driven and inspected directly:
//!
//! - **Asking** — `TaskService::request_dispatch`, the one call every launch
//!   path makes (`store-split.allium: RequestDispatch`, D6). It writes the
//!   request through the module's claim and provisions NOTHING itself.
//! - **Provisioning** — `TaskService::serve_dispatch_request`, this host's
//!   reaction to a `requested` row naming it (`store-split.allium:
//!   HostClaimsDispatch`, `dispatch.allium: ProvisionDispatch`): claim the
//!   row, provision with the row's mode under the row's deadline, and report
//!   dispatched or failed. The module writes the task's worktree, window and
//!   host, and releases the claim on failure; the host releases nothing.
//!
//! The deadline is driven through the injected clock
//! (`dispatch.allium: ProvisioningStopsAtTheDeadline`): a stalled subprocess
//! spends the time it is given on the [`FixedClock`], never the wall clock.

use super::*;

use std::future::Future;
use std::process::Output;
use std::sync::Mutex;
use std::time::Duration;

use chrono::TimeZone;

use crate::clock::FixedClock;
use crate::dispatch::mock_sequence::{DispatchScript, Step};
use crate::models::{
    DispatchMode, DispatchOrigin, DispatchRequest, DispatchRequestId, DispatchRequestState, Task,
};
use crate::process::{MockProcessRunner, ProcessRunner};
use crate::service::ServedDispatch;
use crate::store::RequestDispatchAnswer;
use crate::sync::{MemoryReducerCaller, ReducerCaller, SettledIdentity, SharedRows};

/// This board's host: the one the store handle is built with.
const HOST: &str = "m4-host";

/// The in-memory store with its caller and rows in hand, all on one
/// [`FixedClock`] — the module's clock that stamps `requested_at` and the
/// host's clock that reads the deadline are the same instant source, as they
/// are in production up to clock skew.
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
            .with_ymd_and_hms(2026, 10, 11, 12, 0, 0)
            .unwrap(),
    );
    // Calls are sent as "test-user", who owns this host and the other one a
    // test seeds a request for (`store-split.allium`, D11).
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
    /// A service on this fixture's store and clock, provisioning through
    /// `runner`.
    fn svc(&self, runner: Arc<dyn ProcessRunner>) -> TaskService {
        TaskService::new(self.db.clone(), runner).with_clock(Arc::new(self.clock.clone()))
    }

    /// A backlog task rooted at a temp repo whose `.worktrees/<id>-<slug>`
    /// already exists, i.e. the reuse branch `DispatchScript::dispatch`
    /// scripts. The tempdir is returned so the caller keeps it alive.
    async fn backlog_task(&self, tag: Option<TaskTag>) -> (Task, tempfile::TempDir) {
        let svc = self.svc(MockProcessRunner::unused());
        let id = svc
            .create_task(CreateTaskParams {
                tag,
                ..make_task_params_on_branch("/placeholder", "main")
            })
            .await
            .unwrap();
        let task = svc.get_task(id).await.unwrap();
        let slug = format!("{}-{}", id.0, crate::models::slugify(&task.title));
        let (dir, repo_path, _) = crate::dispatch::tests::make_test_repo_with_worktree(&slug);
        svc.update_task(UpdateTaskParams::for_task(id).repo_path(repo_path))
            .await
            .unwrap();
        (svc.get_task(id).await.unwrap(), dir)
    }

    async fn row(&self, id: DispatchRequestId) -> DispatchRequest {
        self.db
            .dispatch_request(id)
            .await
            .expect("the request row is in this host's subscription")
    }
}

fn requested(answer: RequestDispatchAnswer) -> DispatchRequestId {
    match answer {
        RequestDispatchAnswer::Requested(id) => id,
        RequestDispatchAnswer::Refused(why) => panic!("expected the request to apply: {why}"),
    }
}

/// Poll a future the in-memory caller answers without ever suspending. For a
/// call made from inside a [`ProcessRunner`], which runs on a blocking thread
/// with no runtime to await on.
fn ready<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    match f
        .as_mut()
        .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
    {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => panic!("the in-memory caller answers synchronously"),
    }
}

fn emb() -> Arc<crate::embeddings::EmbeddingService> {
    crate::embeddings::EmbeddingService::new_test()
}

// ===========================================================================
// Asking: RequestDispatch through the service
// ===========================================================================

/// The request names THIS host (D2), carries the caller's origin and mode,
/// claims the task in the same transaction — and provisions nothing: the
/// runner panics on any call.
#[tokio::test]
async fn request_dispatch_writes_a_row_naming_this_host_and_provisions_nothing() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());

    let id = requested(
        svc.request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
    );

    let row = fx.row(id).await;
    assert_eq!(row.task_id, task.id);
    assert_eq!(
        row.host, HOST,
        "a request is always provisioned by its requester"
    );
    assert_eq!(row.origin, DispatchOrigin::User);
    assert_eq!(row.mode, DispatchMode::Dispatch);
    assert_eq!(row.state, DispatchRequestState::Requested);

    let claimed = svc.get_task(task.id).await.unwrap();
    assert_eq!(
        claimed.status,
        TaskStatus::Running,
        "the claim is the request"
    );
    assert!(claimed.worktree.is_none());
    assert!(
        runner.recorded_calls().is_empty(),
        "asking provisions nothing inline: {:?}",
        runner.recorded_calls()
    );
}

/// No mode: the module derives it (MCP and the epic chain ask this way).
#[tokio::test]
async fn request_dispatch_without_a_mode_lets_the_module_derive_it() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(Some(TaskTag::Research)).await;
    let svc = fx.svc(MockProcessRunner::unused());

    let id = requested(
        svc.request_dispatch(task.id, DispatchOrigin::Mcp, None)
            .await
            .unwrap(),
    );

    let row = fx.row(id).await;
    assert_eq!(row.mode, DispatchMode::Research);
    assert_eq!(row.origin, DispatchOrigin::Mcp);
}

/// A refusal is an ORDINARY answer (`RequestDispatch` guidance), not an
/// error: it is how a caller learns another entry point got there first.
#[tokio::test]
async fn a_refused_request_is_an_answer_and_writes_nothing() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let svc = fx.svc(MockProcessRunner::unused());
    requested(
        svc.request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
    );

    let second = svc
        .request_dispatch(task.id, DispatchOrigin::Mcp, None)
        .await
        .expect("a refusal is not an error");

    match second {
        RequestDispatchAnswer::Refused(why) => assert!(why.contains("backlog"), "{why}"),
        other => panic!("a claimed task must be refused, got {other:?}"),
    }
    assert_eq!(fx.rows.dispatch_requests().len(), 1);
}

// ===========================================================================
// Provisioning: HostClaimsDispatch + ProvisionDispatch
// ===========================================================================

/// The happy path: claim, provision, report. The task's worktree, window and
/// host are written by the module from the report, and the row ends
/// dispatched with the same values.
#[tokio::test]
async fn serve_claims_provisions_and_reports_dispatched() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let runner = DispatchScript::dispatch().shared_runner();
    let svc = fx.svc(runner);
    let id = requested(
        svc.request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
    );

    let outcome = svc.serve_dispatch_request(fx.row(id).await, emb()).await;

    let ServedDispatch::Dispatched(result) = outcome else {
        panic!("expected Dispatched, got {outcome:?}");
    };
    let row = fx.row(id).await;
    assert_eq!(row.state, DispatchRequestState::Dispatched);
    assert_eq!(row.worktree.as_deref(), Some(result.worktree_path.as_str()));
    assert!(
        row.claimed_at.is_some(),
        "the host claimed the row before provisioning"
    );

    let stored = svc.get_task(task.id).await.unwrap();
    assert_eq!(stored.status, TaskStatus::Running);
    assert_eq!(
        stored.worktree.as_deref(),
        Some(result.worktree_path.as_str())
    );
    assert_eq!(
        stored.tmux_window.as_ref().map(|w| w.as_str()),
        Some(result.tmux_window.as_str())
    );
    assert_eq!(stored.host.as_deref(), Some(HOST), "HostTracksWorktree");
}

/// D3: the mode is read from the row, never re-derived. A plain task whose row
/// names `research` gets the research agent.
#[tokio::test]
async fn serve_launches_the_agent_the_row_names() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let svc = fx.svc(DispatchScript::dispatch().shared_runner());
    let id = requested(
        svc.request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Research))
            .await
            .unwrap(),
    );

    let outcome = svc.serve_dispatch_request(fx.row(id).await, emb()).await;

    let ServedDispatch::Dispatched(result) = outcome else {
        panic!("expected Dispatched, got {outcome:?}");
    };
    let prompt =
        std::fs::read_to_string(format!("{}/.claude-prompt", result.worktree_path)).unwrap();
    assert!(
        prompt.contains(crate::dispatch::RESEARCH_AGENT_INTRO),
        "the row's research mode must reach the research agent: {prompt}"
    );
}

/// `tasks.allium: CreateQuickTask` — a `quick` row launches the quick-dispatch
/// agent with its own prompt variant.
#[tokio::test]
async fn serve_launches_the_quick_agent_for_a_quick_row() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let svc = fx.svc(DispatchScript::dispatch().shared_runner());
    let id = requested(
        svc.request_dispatch(task.id, DispatchOrigin::Quick, Some(DispatchMode::Quick))
            .await
            .unwrap(),
    );

    let outcome = svc.serve_dispatch_request(fx.row(id).await, emb()).await;

    let ServedDispatch::Dispatched(result) = outcome else {
        panic!("expected Dispatched, got {outcome:?}");
    };
    let prompt =
        std::fs::read_to_string(format!("{}/.claude-prompt", result.worktree_path)).unwrap();
    assert!(
        prompt.contains("quick-dispatched task"),
        "a quick row must reach build_quick_dispatch_prompt: {prompt}"
    );
}

/// `requires: request.host = local_host()` — another machine's row is not
/// this board's to claim or provision.
#[tokio::test]
async fn serve_ignores_a_row_for_another_host() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let applied = fx
        .caller
        .request_dispatch(
            task.id,
            "another-host".into(),
            "user".into(),
            "dispatch".into(),
        )
        .await
        .unwrap();
    let crate::sync::ReducerOutcome::Applied(ids) = applied else {
        panic!("seeding the other host's request must apply: {applied:?}");
    };
    let id = DispatchRequestId(ids[0]);
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());

    let outcome = svc.serve_dispatch_request(fx.row(id).await, emb()).await;

    assert!(
        matches!(outcome, ServedDispatch::NotThisHost),
        "expected NotThisHost, got {outcome:?}"
    );
    assert_eq!(fx.row(id).await.state, DispatchRequestState::Requested);
    assert!(runner.recorded_calls().is_empty());
}

/// `OnlyTheClaimedRequestProvisions`: a row another board (or an earlier
/// delivery) already claimed is refused at the claim, and nothing runs.
#[tokio::test]
async fn serve_provisions_nothing_when_its_claim_is_refused() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());
    let id = requested(
        svc.request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
    );
    let stale_copy = fx.row(id).await;
    assert!(fx
        .caller
        .claim_dispatch_request(id, HOST.into())
        .await
        .unwrap()
        .won());

    let outcome = svc.serve_dispatch_request(stale_copy, emb()).await;

    assert!(
        matches!(outcome, ServedDispatch::ClaimRefused(_)),
        "expected ClaimRefused, got {outcome:?}"
    );
    assert!(runner.recorded_calls().is_empty());
}

/// A provisioning failure is rolled back here and REPORTED; the module fails
/// the row with the host's reason and releases the claim.
#[tokio::test]
async fn serve_reports_a_provisioning_failure_and_the_module_releases_the_claim() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let svc = fx.svc(
        DispatchScript::dispatch()
            .fails_at(Step::NewWindow)
            .shared_runner(),
    );
    let id = requested(
        svc.request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
    );

    let outcome = svc.serve_dispatch_request(fx.row(id).await, emb()).await;

    let ServedDispatch::Failed(reason) = outcome else {
        panic!("expected Failed, got {outcome:?}");
    };
    let row = fx.row(id).await;
    assert_eq!(row.state, DispatchRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some(reason.as_str()));
    let stored = svc.get_task(task.id).await.unwrap();
    assert_eq!(stored.status, TaskStatus::Backlog);
    assert!(stored.worktree.is_none());
    assert!(stored.tmux_window.is_none());
}

// ===========================================================================
// ProvisioningStopsAtTheDeadline
// ===========================================================================

fn is_fetch(args: &[&str]) -> bool {
    args.contains(&"fetch")
}

/// A runner whose `git fetch` stalls: it never answers on its own, so the only
/// way past it is the timeout the caller hands it. The stall spends exactly
/// that timeout — on the injected clock, not the wall clock — then fails the
/// way a killed subprocess does.
struct StallingFetch {
    inner: MockProcessRunner,
    clock: FixedClock,
    fetch_timeouts: Mutex<Vec<Duration>>,
}

impl ProcessRunner for StallingFetch {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        assert!(
            !is_fetch(args),
            "a stalled fetch run with no timeout never returns — provisioning must bound it"
        );
        self.inner.run(program, args)
    }

    fn run_with_timeout(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> anyhow::Result<Output> {
        if is_fetch(args) {
            // Recorded by the inner mock so the call log stays complete.
            let _ = self.inner.run_with_timeout(program, args, timeout);
            self.fetch_timeouts.lock().unwrap().push(timeout);
            self.clock
                .advance(chrono::Duration::from_std(timeout).unwrap());
            anyhow::bail!("{program} timed out after {timeout:?}");
        }
        self.inner.run_with_timeout(program, args, timeout)
    }

    fn agent_binaries(&self) -> crate::process::AgentBinaries {
        self.inner.agent_binaries()
    }
}

/// The behavioural test D4 asks for: a provisioning whose subprocess stalls
/// fails at the row's deadline. The stalled call is bounded by the time left
/// to the deadline (not by a whole `SUBPROCESS_TIMEOUT`), the attempt stops
/// there, nothing is launched, and the failure is reported "deadline passed",
/// which releases the claim.
#[tokio::test]
async fn a_stalled_subprocess_fails_the_dispatch_at_the_rows_deadline() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let stalling = Arc::new(StallingFetch {
        inner: DispatchScript::dispatch().runner(),
        clock: fx.clock.clone(),
        fetch_timeouts: Mutex::new(Vec::new()),
    });
    let svc = fx.svc(stalling.clone());
    let id = requested(
        svc.request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
    );
    // The row sat requested for 55s before this host served it: 5s are left
    // (one deadline from requested_at, whatever the state — D4).
    fx.clock.advance(chrono::Duration::seconds(55));

    let outcome = svc.serve_dispatch_request(fx.row(id).await, emb()).await;

    let timeouts = stalling.fetch_timeouts.lock().unwrap().clone();
    assert!(
        !timeouts.is_empty(),
        "the scripted fetch must have been reached"
    );
    assert!(
        timeouts.iter().all(|t| *t <= Duration::from_secs(5)),
        "a call made 5s before the deadline may run for at most 5s: {timeouts:?}"
    );
    let ServedDispatch::Failed(reason) = outcome else {
        panic!("expected Failed at the deadline, got {outcome:?}");
    };
    assert_eq!(reason, crate::dispatch::DEADLINE_PASSED);
    let row = fx.row(id).await;
    assert_eq!(row.state, DispatchRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("deadline passed"));
    let calls = stalling.inner.flattened_calls();
    assert!(
        !calls.iter().any(|c| c.contains("new-window")),
        "nothing may be launched past the deadline: {calls:?}"
    );
    let stored = svc.get_task(task.id).await.unwrap();
    assert_eq!(stored.status, TaskStatus::Backlog, "the claim is released");
}

/// A row whose deadline has already passed when this host gets to it runs no
/// subprocess at all and fails "deadline passed".
#[tokio::test]
async fn a_row_served_after_its_deadline_runs_nothing() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let svc = fx.svc(runner.clone());
    let id = requested(
        svc.request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
    );
    fx.clock.advance(chrono::Duration::seconds(61));

    let outcome = svc.serve_dispatch_request(fx.row(id).await, emb()).await;

    assert!(
        matches!(&outcome, ServedDispatch::Failed(r) if r == crate::dispatch::DEADLINE_PASSED),
        "expected Failed(deadline passed), got {outcome:?}"
    );
    assert!(
        runner.recorded_calls().is_empty(),
        "{:?}",
        runner.recorded_calls()
    );
    let row = fx.row(id).await;
    assert_eq!(row.state, DispatchRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("deadline passed"));
}

// ===========================================================================
// A row that fails under this host
// ===========================================================================

/// A runner that runs `action` once, just before the first call whose command
/// line contains `needle`, then answers from `inner`. How a test makes the
/// row fail mid-provisioning — the module's backstop firing, or the task being
/// deleted — at a known point in the call sequence.
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

/// The dispatch script, plus spare successes for whatever teardown the host
/// runs once it learns its row failed.
fn script_with_spare_teardown() -> MockProcessRunner {
    let mut responses = DispatchScript::dispatch().responses();
    responses.extend((0..8).map(|_| (None, MockProcessRunner::ok())));
    MockProcessRunner::new_with_delays(responses)
}

/// `dispatch.allium: ProvisionDispatch`, "A row that fails under this host":
/// the module's backstop fails the row while the window is being created. The
/// dispatched report is refused, and the host tears down what THIS attempt
/// made — the window — but never the reused worktree. The task is left as
/// the module's release put it: backlog, with nothing recorded.
#[tokio::test]
async fn a_row_timed_out_under_the_host_is_torn_down_not_recorded() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let svc0 = fx.svc(MockProcessRunner::unused());
    let id = requested(
        svc0.request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
    );
    let caller = fx.caller.clone();
    let runner = Arc::new(OnCall {
        inner: script_with_spare_teardown(),
        needle: "new-window",
        action: Mutex::new(Some(Box::new(move || {
            assert!(caller.expire_dispatch_request(id).won());
        }))),
    });
    let svc = fx.svc(runner.clone());

    let outcome = svc.serve_dispatch_request(fx.row(id).await, emb()).await;

    assert!(
        matches!(outcome, ServedDispatch::Failed(_)),
        "a row that failed under the host is a failure, got {outcome:?}"
    );
    let row = fx.row(id).await;
    assert_eq!(row.state, DispatchRequestState::Failed);
    assert_eq!(
        row.reason.as_deref(),
        Some("timed out"),
        "the module's reason stands"
    );
    let stored = svc.get_task(task.id).await.unwrap();
    assert_eq!(stored.status, TaskStatus::Backlog);
    assert!(
        stored.worktree.is_none(),
        "a refused report is never recorded"
    );

    let calls = runner.inner.flattened_calls();
    // Kills target the pane id the window's name resolves to
    // (`TmuxWindowTargetedExactly`), never the bare name.
    let window = format!("task-{}", task.id.0);
    let pane = runner.inner.pane_id_of(&window);
    assert!(
        calls
            .iter()
            .any(|c| c.contains("kill-window") && c.ends_with(&format!("-t {pane}"))),
        "the window this attempt opened ({window}, {pane}) must be killed: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("worktree remove")),
        "a REUSED worktree is never removed by a rollback: {calls:?}"
    );
    let worktree_dir = std::path::Path::new(&task.repo_path)
        .join(".worktrees")
        .join(format!(
            "{}-{}",
            task.id.0,
            crate::models::slugify(&task.title)
        ));
    assert!(worktree_dir.exists(), "the reused worktree is left on disk");
}

/// D8: deleting the task fails its open request "task deleted"; the host sees
/// it and tears down what it made, so no agent runs for a task that is gone.
#[tokio::test]
async fn a_task_deleted_under_the_host_is_torn_down() {
    let fx = fx();
    let (task, _repo) = fx.backlog_task(None).await;
    let svc0 = fx.svc(MockProcessRunner::unused());
    let id = requested(
        svc0.request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
    );
    let caller = fx.caller.clone();
    let db = fx.db.clone();
    let task_id = task.id;
    let runner = Arc::new(OnCall {
        inner: script_with_spare_teardown(),
        needle: "new-window",
        action: Mutex::new(Some(Box::new(move || {
            // Only a done task may be deleted (`tasks.allium: DeleteTask`):
            // the operator moves it to done, then deletes it, while its
            // request is still open.
            ready(
                db.patch_task(
                    task_id,
                    &store::TaskPatch::new()
                        .status(TaskStatus::Done)
                        .sub_status(crate::models::SubStatus::default_for(TaskStatus::Done)),
                ),
            )
            .unwrap();
            ready(caller.delete_task(task_id))
                .unwrap()
                .applied()
                .unwrap();
        }))),
    });
    let svc = fx.svc(runner.clone());

    let outcome = svc.serve_dispatch_request(fx.row(id).await, emb()).await;

    assert!(
        matches!(outcome, ServedDispatch::Failed(_)),
        "expected Failed, got {outcome:?}"
    );
    let row = fx.row(id).await;
    assert_eq!(row.reason.as_deref(), Some("task deleted"));
    let calls = runner.inner.flattened_calls();
    assert!(
        calls.iter().any(|c| c.contains("kill-window")),
        "the deleted task's window must be killed: {calls:?}"
    );
}
