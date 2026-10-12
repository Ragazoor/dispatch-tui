// ---------------------------------------------------------------------------
// Wrap-up and exit-session as request rows — the board's session host
// (store-split M5, task #53258)
// ---------------------------------------------------------------------------
//
// The board reacts to the session rows its subscription delivers: it serves
// `requested` rows naming this host (`HostClaimsSessionRequest` ->
// `WrapUpRebase` / `ExitSessionTeardown`), tears down the window of an
// exit_session row it sees fail (`KillWindowOfFailedExitSession`), and at
// startup first fails the rows a previous board left `running`
// (`HostRestartFailsInterruptedSessionRequests`, which also kills the window
// of an exit_session row it failed) and then catches up on `requested` ones
// (W8).
use super::*;
use crate::models::{
    DispatchMode, DispatchOrigin, SessionRequestId, SessionRequestKind, SessionRequestState,
    WrapUpMode,
};
use crate::store::{
    DispatchRequestStore, RequestDispatchAnswer, SessionRequestAnswer, SessionRequestRead,
    SessionRequestStore,
};
use crate::sync::{InjectedFault, MemoryReducerCaller, ReducerCaller, SettledIdentity, SharedRows};

/// The worktree `DispatchScript::finish()` scripts its git calls for.
const REPO: &str = "/repo";
const WORKTREE: &str = "/repo/.worktrees/42-fix-bug";

struct SessionRig {
    rt: TuiRuntime,
    msgs: mpsc::UnboundedReceiver<Message>,
    db: Arc<Store>,
    caller: Arc<MemoryReducerCaller>,
    rows: Arc<SharedRows>,
    _host_dir: tempfile::TempDir,
}

async fn session_rig(runner: Arc<dyn ProcessRunner>) -> SessionRig {
    let identity = Arc::new(SettledIdentity::default());
    identity.settle("test-user");
    session_rig_with_identity(runner, identity).await
}

async fn session_rig_with_identity(
    runner: Arc<dyn ProcessRunner>,
    identity: Arc<SettledIdentity>,
) -> SessionRig {
    let rows = Arc::new(SharedRows::new());
    let clock: Arc<dyn crate::clock::Clock> = Arc::new(crate::clock::SystemClock);
    let caller = Arc::new(
        MemoryReducerCaller::new(rows.clone(), clock.clone())
            .with_sender("test-user")
            .with_registered_host(crate::store::MEMORY_HOST_ID, "test-user")
            .with_registered_host("other-host", "test-user"),
    );
    let host_dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Store::new(
        rows.clone(),
        caller.clone(),
        identity,
        clock,
        crate::store::MEMORY_HOST_ID.to_string(),
        host_dir.path(),
    ));
    let (tx, msgs) = mpsc::unbounded_channel();
    let rt = make_runtime(db.clone(), tx, runner).await;
    SessionRig {
        rt,
        msgs,
        db,
        caller,
        rows,
        _host_dir: host_dir,
    }
}

fn asked(answer: SessionRequestAnswer) -> SessionRequestId {
    match answer {
        SessionRequestAnswer::Requested(id) => id,
        SessionRequestAnswer::Refused(why) => panic!("expected the request to apply: {why}"),
    }
}

impl SessionRig {
    /// A task dispatched on this host: running, with the worktree the finish
    /// script expects and a window.
    async fn running_task(&self) -> models::TaskId {
        let task = create_task_returning(
            &*self.db,
            "Wrap me",
            "desc",
            REPO,
            None,
            models::TaskStatus::Backlog,
        )
        .await
        .unwrap();
        let request = match self
            .rt
            .task_svc
            .request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap()
        {
            RequestDispatchAnswer::Requested(r) => r,
            RequestDispatchAnswer::Refused(why) => panic!("{why}"),
        };
        assert!(self.db.claim_dispatch_request(request).await.unwrap());
        assert!(self
            .db
            .report_dispatch_provisioned(
                request,
                WORKTREE,
                &test_tmux_window(&format!("task-{}", task.id.0)),
                false
            )
            .await
            .unwrap());
        task.id
    }

    async fn wrap_up(&self, task: models::TaskId, action: WrapUpMode) -> SessionRequestId {
        asked(
            self.rt
                .task_svc
                .request_wrap_up(task, action)
                .await
                .unwrap(),
        )
    }

    async fn state(&self, id: SessionRequestId) -> SessionRequestState {
        self.db.session_request(id).await.unwrap().state
    }

    /// The exit_session row of a close of `task`, accepted by the module.
    async fn close(&self, task: models::TaskId) -> SessionRequestId {
        let token = self.wrap_up(task, WrapUpMode::Done).await;
        asked(
            self.rt
                .task_svc
                .request_exit_session(task, token, WrapUpMode::Done, None)
                .await
                .unwrap(),
        )
    }
}

async fn join_all(handles: Vec<tokio::task::JoinHandle<()>>) {
    for handle in handles {
        handle.await.unwrap();
    }
}

fn kill_calls(runner: &MockProcessRunner) -> usize {
    runner
        .flattened_calls()
        .iter()
        .filter(|c| c.contains("kill-window"))
        .count()
}

/// A `requested` session row naming `host`, written straight into the
/// subscription as `requested_by` — a rebase wrap-up for task 1, which need not
/// exist.
fn seed_row(rows: &SharedRows, id: i64, host: &str, requested_by: &str) {
    rows.upsert_session_request(&crate::spacetime::bindings::SessionRequest {
        id,
        task_id: 1,
        kind: "wrap_up".into(),
        action: "rebase".into(),
        host: host.into(),
        requested_by: requested_by.into(),
        state: "requested".into(),
        requested_at: "2026-10-12 12:00:00.000".into(),
        claimed_at: String::new(),
        finished_at: String::new(),
        reason: String::new(),
        pr_url: String::new(),
        wrap_up_request: 0,
        tmux_window: String::new(),
        chained_task: 0,
        deadline: "2099-01-01 00:00:00.000".into(),
    });
}

// -- The reactor serves this host's rows ---------------------------------------

/// A rebase row naming this host is claimed, rebased and reported done by the
/// reactor — once, however often the pump redelivers the set — and another
/// host's row is not this board's to claim.
#[tokio::test]
async fn the_host_serves_its_own_wrap_up_rows_once_and_ignores_others() {
    let runner = DispatchScript::finish().no_remote().shared_runner();
    let r = session_rig(runner.clone()).await;
    let task = r.running_task().await;
    let mine = r.wrap_up(task, WrapUpMode::Rebase).await;
    seed_row(&r.rows, 500, "other-host", "test-user");

    join_all(r.rt.serve_session_requests().await).await;
    let again = r.rt.serve_session_requests().await;

    assert_eq!(r.state(mine).await, SessionRequestState::Done);
    assert!(again.is_empty(), "a row is handed out once");
    assert_eq!(
        r.state(SessionRequestId(500)).await,
        SessionRequestState::Requested,
        "another host's row is not this board's to claim"
    );
}

/// A claim that never reached the module leaves the row `requested` and this
/// board's to serve: the next reload hands it out again.
#[tokio::test]
async fn a_session_claim_that_could_not_be_sent_is_retried_on_the_next_reload() {
    let r = session_rig(DispatchScript::finish().no_remote().shared_runner()).await;
    let task = r.running_task().await;
    let id = r.wrap_up(task, WrapUpMode::Rebase).await;
    r.caller
        .fail_next("claim_session_request", InjectedFault::Unsent);

    join_all(r.rt.serve_session_requests().await).await;
    assert_eq!(r.state(id).await, SessionRequestState::Requested);

    join_all(r.rt.serve_session_requests().await).await;
    assert_eq!(r.state(id).await, SessionRequestState::Done);
}

/// A board whose identity has not settled serves nothing yet — and forgets
/// nothing: once it settles, the row it held back is served.
#[tokio::test]
async fn a_session_row_held_back_for_an_unsettled_identity_is_served_once_it_settles() {
    let identity = Arc::new(SettledIdentity::default());
    let r =
        session_rig_with_identity(Arc::new(MockProcessRunner::new(vec![])), identity.clone()).await;
    seed_row(&r.rows, 77, crate::store::MEMORY_HOST_ID, "test-user");

    assert!(
        r.rt.serve_session_requests().await.is_empty(),
        "no settled identity, nothing served"
    );

    identity.settle("test-user");
    assert_eq!(
        r.rt.serve_session_requests().await.len(),
        1,
        "the held-back row is served once it settles"
    );
}

/// D11's second line: a row naming this host that its owner did not request is
/// ignored — not claimed, not run — whatever let it in.
#[tokio::test]
async fn the_host_ignores_a_session_row_its_owner_did_not_request() {
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let r = session_rig(runner.clone()).await;
    seed_row(&r.rows, 99, crate::store::MEMORY_HOST_ID, "beef");

    let served = r.rt.serve_session_requests().await;

    assert!(
        served.is_empty(),
        "a foreign requester's row must not be served"
    );
    assert!(runner.recorded_calls().is_empty());
    assert_eq!(
        r.state(SessionRequestId(99)).await,
        SessionRequestState::Requested
    );
}

/// An exit_session row is served too: the window it names is killed and the
/// row reported done, and a wrap_up row written already done is never work.
#[tokio::test]
async fn the_host_serves_an_exit_session_row_and_skips_rows_with_no_host_work() {
    let runner = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
    let r = session_rig(runner.clone()).await;
    let task = r.running_task().await;
    let close = r.close(task).await;

    let served = r.rt.serve_session_requests().await;
    assert_eq!(served.len(), 1, "the done wrap_up row is not work");
    join_all(served).await;

    assert_eq!(r.state(close).await, SessionRequestState::Done);
    assert_eq!(kill_calls(&runner), 1);
}

/// After a rebase the repository's drift is measured again, without a fetch:
/// `repo-sync.allium: RefreshRepoSyncStateAfterRebase`, which used to be sent
/// from the MCP handler as `BranchRebased`.
#[tokio::test]
async fn a_served_rebase_refreshes_the_repository_drift() {
    let mut responses = DispatchScript::finish().no_remote().responses();
    responses.extend((0..12).map(|_| (None, MockProcessRunner::ok())));
    let runner = Arc::new(MockProcessRunner::new_with_delays(responses));
    let mut r = session_rig(runner.clone()).await;
    let task = r.running_task().await;
    r.wrap_up(task, WrapUpMode::Rebase).await;

    join_all(r.rt.serve_session_requests().await).await;

    let measured = loop {
        match r.msgs.recv().await.expect("the runtime's sender is alive") {
            Message::RepoSync(crate::tui::messages::RepoSyncMessage::Measured(m)) => break m,
            _ => continue,
        }
    };
    assert_eq!(measured.repo_path, REPO);
    assert!(
        !runner
            .recorded_calls()
            .iter()
            .any(|(_, a)| a.contains(&"fetch".to_string())),
        "the rebase already refreshed the refs"
    );
}

// -- KillWindowOfFailedExitSession -----------------------------------------------

/// A teardown that itself failed leaves the window alive on a task whose close
/// already cleared it; the reactor SEES the row fail and tries once more.
/// Rows it first sees already failed are history and are left alone.
#[tokio::test]
async fn an_exit_session_row_seen_to_fail_has_its_window_killed_once_more() {
    let runner = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("kill-window: boom"),
        MockProcessRunner::ok(),
    ]));
    let r = session_rig(runner.clone()).await;
    let task = r.running_task().await;
    let close = r.close(task).await;

    join_all(r.rt.serve_session_requests().await).await;
    assert_eq!(r.state(close).await, SessionRequestState::Failed);
    assert_eq!(kill_calls(&runner), 1, "the teardown itself");

    join_all(r.rt.serve_session_requests().await).await;
    assert_eq!(kill_calls(&runner), 2, "one more attempt");

    join_all(r.rt.serve_session_requests().await).await;
    assert_eq!(kill_calls(&runner), 2, "and only one");
}

/// A row first seen already failed is history: this board was not running
/// when it failed, and does not go killing windows for it.
#[tokio::test]
async fn an_exit_session_row_first_seen_failed_is_history() {
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let r = session_rig(runner.clone()).await;
    r.rows
        .upsert_session_request(&crate::spacetime::bindings::SessionRequest {
            id: 12,
            task_id: 1,
            kind: "exit_session".into(),
            action: "done".into(),
            host: crate::store::MEMORY_HOST_ID.into(),
            requested_by: "test-user".into(),
            state: "failed".into(),
            requested_at: "2026-10-12 12:00:00.000".into(),
            claimed_at: String::new(),
            finished_at: "2026-10-12 12:00:40.000".into(),
            reason: "timed out".into(),
            pr_url: String::new(),
            wrap_up_request: 3,
            tmux_window: "task-1".into(),
            chained_task: 0,
            deadline: "2026-10-12 12:00:30.000".into(),
        });

    let served = r.rt.serve_session_requests().await;

    assert!(served.is_empty());
    assert!(runner.recorded_calls().is_empty());
}

// -- Startup ------------------------------------------------------------------------

/// `HostRestartFailsInterruptedSessionRequests` then W8's catch-up: at startup
/// the board first fails the rows a previous board left `running` ("host
/// restarted"), kills the window of an exit_session row it failed, and then
/// serves the rows still `requested`.
#[tokio::test]
async fn startup_fails_interrupted_rows_kills_their_windows_then_serves_requested_ones() {
    let mut responses = vec![(None, MockProcessRunner::ok())]; // the interrupted close's kill
    responses.extend(DispatchScript::finish().no_remote().responses());
    let runner = Arc::new(MockProcessRunner::new_with_delays(responses));
    let r = session_rig(runner.clone()).await;

    let interrupted_task = r.running_task().await;
    let interrupted_close = r.close(interrupted_task).await;
    // The previous board claimed the teardown, then died before it ran.
    assert!(r.db.claim_session_request(interrupted_close).await.unwrap());
    let interrupted_row = r.db.session_request(interrupted_close).await.unwrap();
    assert_eq!(interrupted_row.kind, SessionRequestKind::ExitSession);

    let waiting_task = r.running_task().await;
    let waiting = r.wrap_up(waiting_task, WrapUpMode::Rebase).await;

    join_all(r.rt.recover_session_requests().await).await;

    let failed = r.db.session_request(interrupted_close).await.unwrap();
    assert_eq!(failed.state, SessionRequestState::Failed);
    assert_eq!(failed.reason.as_deref(), Some("host restarted"));
    assert_eq!(
        kill_calls(&runner),
        1,
        "the window the failed row names is killed, best effort"
    );
    assert_eq!(
        r.state(waiting).await,
        SessionRequestState::Done,
        "a requested row from the initial rows is picked up, after the sweep"
    );
}

/// A running wrap_up row is failed and not resumed: the process running it is
/// gone and the worktree may be mid-rebase.
#[tokio::test]
async fn startup_does_not_resume_an_interrupted_rebase() {
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let r = session_rig(runner.clone()).await;
    let task = r.running_task().await;
    let id = r.wrap_up(task, WrapUpMode::Rebase).await;
    assert!(r.db.claim_session_request(id).await.unwrap());

    join_all(r.rt.recover_session_requests().await).await;

    let row = r.db.session_request(id).await.unwrap();
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("host restarted"));
    assert!(runner.recorded_calls().is_empty());
}
