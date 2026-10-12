// ---------------------------------------------------------------------------
// Dispatch as request rows — the board's launch paths and its host reactor
// (store-split M4, task #53257)
// ---------------------------------------------------------------------------
//
// Every launch path writes a request and provisions nothing inline
// (`store-split.allium`, D6): the TUI dispatch key and its research and
// retry-fresh siblings (all `TaskCommand::DispatchAgent`, origin `user`), and
// quick dispatch (origin `quick`, mode `quick`). The board then reacts to the
// rows its subscription delivers: it serves `requested` rows naming this
// host (`HostClaimsDispatch` -> `ProvisionDispatch`), and at startup it first
// fails the rows a previous board left `provisioning`
// (`HostRestartFailsInterrupted`) and then catches up on `requested` ones
// (D5). `reload_board` hands this host's rows to the App, which drives
// DispatchingFeedback from them.
use super::*;
use crate::models::{DispatchMode, DispatchOrigin, DispatchRequestId, DispatchRequestState};
use crate::store::{DispatchRequestRead, RequestDispatchAnswer};
use crate::sync::{
    MemoryReducerCaller, ReducerCaller, ReducerOutcome, SettledIdentity, SharedRows,
};
use crate::tui::commands::TaskCommand;

/// The runtime over a store whose rows and caller the test also holds, so it
/// can seed rows the way another entry point (or another host) would, and
/// inspect them.
struct Rig {
    rt: TuiRuntime,
    app: App,
    msgs: mpsc::UnboundedReceiver<Message>,
    db: Arc<Store>,
    caller: Arc<MemoryReducerCaller>,
    rows: Arc<SharedRows>,
    _host_dir: tempfile::TempDir,
}

async fn rig(runner: Arc<dyn ProcessRunner>) -> Rig {
    let identity = Arc::new(SettledIdentity::default());
    identity.settle("test-user");
    rig_with_identity(runner, identity).await
}

/// [`rig`] whose board identity the test holds, so it can leave it unsettled.
async fn rig_with_identity(runner: Arc<dyn ProcessRunner>, identity: Arc<SettledIdentity>) -> Rig {
    let rows = Arc::new(SharedRows::new());
    let clock: Arc<dyn crate::clock::Clock> = Arc::new(crate::clock::SystemClock);
    // Calls are sent as "test-user", who owns this host and the other one a
    // test seeds a request for (`store-split.allium`, D11).
    let caller = Arc::new(
        MemoryReducerCaller::new(rows.clone(), clock.clone())
            .with_sender("test-user")
            .with_registered_host(crate::store::MEMORY_HOST_ID, "test-user")
            .with_registered_host("other-host", "test-user"),
    );
    let host_dir = tempfile::tempdir().unwrap();
    // `make_runtime` names its host "test-host"; the store must agree, or the
    // runtime would be serving some other machine's rows.
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
    let app = App::new(db.list_all().await);
    Rig {
        rt,
        app,
        msgs,
        db,
        caller,
        rows,
        _host_dir: host_dir,
    }
}

impl Rig {
    async fn dispatch(&mut self, cmd: Command) -> Vec<Command> {
        commands::dispatch(cmd, &mut self.app, &self.rt).await
    }

    /// The task's open request, which this host's subscription holds.
    async fn request_for(&self, task: models::TaskId) -> models::DispatchRequest {
        self.db
            .latest_dispatch_request_for_task(task)
            .await
            .unwrap_or_else(|| panic!("no dispatch request was written for {task:?}"))
    }

    /// A backlog task whose `.worktrees/<id>-<slug>` exists, for a test that
    /// lets the host provision.
    async fn provisionable_task(&self) -> (models::Task, tempfile::TempDir) {
        let task = create_task_returning(
            &*self.db,
            "Test Task",
            "desc",
            "/placeholder",
            None,
            models::TaskStatus::Backlog,
        )
        .await
        .unwrap();
        let slug = format!("{}-{}", task.id.0, models::slugify(&task.title));
        let (dir, repo_path, _) = crate::dispatch::tests::make_test_repo_with_worktree(&slug);
        self.db
            .patch_task(
                task.id,
                &crate::store::TaskPatch::new().repo_path(&repo_path),
            )
            .await
            .unwrap();
        (self.db.get_task(task.id).await.unwrap(), dir)
    }

    async fn next_msg(&mut self) -> Message {
        tokio::time::timeout(TEST_TIMEOUT, self.msgs.recv())
            .await
            .expect("the runtime must report well within the timeout")
            .expect("the runtime's message sender should still be alive")
    }
}

fn requested(answer: RequestDispatchAnswer) -> DispatchRequestId {
    match answer {
        RequestDispatchAnswer::Requested(id) => id,
        RequestDispatchAnswer::Refused(why) => panic!("expected the request to apply: {why}"),
    }
}

// -- The launch paths ask, they do not provision ----------------------------

/// `dispatch.allium: DispatchTask` — the TUI dispatch key (and, through the
/// same command, retry-fresh) writes a `user` request naming this host with
/// mode `dispatch`, and provisions nothing inline: the runner panics on any
/// call, and the row is still `requested` when the arm returns.
#[tokio::test]
async fn the_dispatch_key_requests_and_provisions_nothing_inline() {
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let mut r = rig(runner.clone()).await;
    let task = create_task_returning(
        &*r.db,
        "Keyed",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();

    r.dispatch(Command::Task(TaskCommand::DispatchAgent {
        id: task.id,
        mode: Some(DispatchMode::Dispatch),
    }))
    .await;

    let row = r.request_for(task.id).await;
    assert_eq!(row.host, crate::store::MEMORY_HOST_ID);
    assert_eq!(row.origin, DispatchOrigin::User);
    assert_eq!(row.mode, DispatchMode::Dispatch);
    assert_eq!(
        row.state,
        DispatchRequestState::Requested,
        "the arm only asks; the host reactor claims"
    );
    assert!(
        runner.recorded_calls().is_empty(),
        "no launch path provisions inline any more: {:?}",
        runner.recorded_calls()
    );
}

/// `dispatch.allium: DispatchResearchTask` — the research key names its mode
/// on the row; origin is still `user`.
#[tokio::test]
async fn the_research_key_requests_research_mode() {
    let mut r = rig(Arc::new(MockProcessRunner::new(vec![]))).await;
    let task = create_task_returning(
        &*r.db,
        "Investigate",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();

    r.dispatch(Command::Task(TaskCommand::DispatchAgent {
        id: task.id,
        mode: Some(DispatchMode::Research),
    }))
    .await;

    let row = r.request_for(task.id).await;
    assert_eq!(row.mode, DispatchMode::Research);
    assert_eq!(row.origin, DispatchOrigin::User);
}

/// `dispatch.allium: RetryFresh` re-dispatches with no mode, and the module
/// derives it (`store-split.allium` D3): a research-tagged task without a plan
/// goes back to research (`DispatchResearchTask`), not to a standard agent.
#[tokio::test]
async fn a_dispatch_without_a_mode_lets_the_module_derive_research() {
    let mut r = rig(Arc::new(MockProcessRunner::new(vec![]))).await;
    let task = create_task_returning(
        &*r.db,
        "Retry me",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    r.db.patch_task(
        task.id,
        &crate::store::TaskPatch::new().tag(Some(models::TaskTag::Research)),
    )
    .await
    .unwrap();

    r.dispatch(Command::Task(TaskCommand::DispatchAgent {
        id: task.id,
        mode: None,
    }))
    .await;

    let row = r.request_for(task.id).await;
    assert_eq!(row.mode, DispatchMode::Research);
    assert_eq!(row.origin, DispatchOrigin::User);
}

/// `LostClaimReported`: a refused request drains the pending entry with a
/// failure message, and writes no second row.
#[tokio::test]
async fn a_refused_request_is_reported_as_abandoned() {
    let mut r = rig(Arc::new(MockProcessRunner::new(vec![]))).await;
    let task = create_task_returning(
        &*r.db,
        "Contended",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    // Another entry point got there first.
    requested(
        r.rt.task_svc
            .request_dispatch(task.id, DispatchOrigin::Mcp, None)
            .await
            .unwrap(),
    );

    r.dispatch(Command::Task(TaskCommand::DispatchAgent {
        id: task.id,
        mode: Some(DispatchMode::Dispatch),
    }))
    .await;

    let msg = r.next_msg().await;
    assert!(
        matches!(msg, Message::Task(crate::tui::messages::TaskMessage::DispatchAbandoned(id)) if id == task.id),
        "a refused request drains its pending entry, got: {msg:?}"
    );
    assert_eq!(
        r.rows.dispatch_requests().len(),
        1,
        "a refusal writes nothing"
    );
    assert_eq!(
        r.request_for(task.id).await.origin,
        DispatchOrigin::Mcp,
        "the winner's request is untouched"
    );
}

/// `tasks.allium: CreateQuickTask` — quick dispatch creates the task, then
/// requests it with origin `quick` and mode `quick`. The only subprocess it
/// may run itself is the default-branch probe task creation makes.
#[tokio::test]
async fn quick_dispatch_creates_then_requests_a_quick_row() {
    let runner = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"),
    ]));
    let mut r = rig(runner.clone()).await;
    let repo = tempfile::tempdir().unwrap();
    let claude_dir = tempfile::tempdir().unwrap();
    r.rt.claude_json_path = claude_dir.path().join(".claude.json");

    r.dispatch(Command::Task(TaskCommand::TrustAndQuickDispatch {
        draft: tui::TaskDraft {
            title: "Quick one".into(),
            description: String::new(),
            repo_path: repo.path().to_str().unwrap().into(),
            ..Default::default()
        },
        epic_id: None,
    }))
    .await;

    let tasks = r.db.list_all().await;
    assert_eq!(tasks.len(), 1, "quick dispatch creates its task");
    let row = r.request_for(tasks[0].id).await;
    assert_eq!(row.origin, DispatchOrigin::Quick);
    assert_eq!(row.mode, DispatchMode::Quick);
    assert_eq!(row.state, DispatchRequestState::Requested);
    let calls = runner.flattened_calls();
    assert!(
        calls.iter().all(|c| c.contains("symbolic-ref")),
        "quick dispatch provisions nothing inline: {calls:?}"
    );
}

// -- The host reactor -----------------------------------------------------------

/// `HostClaimsDispatch` -> `ProvisionDispatch`: a `requested` row naming this
/// host is claimed, provisioned and reported dispatched; one naming another
/// host is left alone.
#[tokio::test]
async fn the_host_serves_its_own_requested_rows_and_ignores_others() {
    let r = rig(DispatchScript::dispatch().shared_runner()).await;
    let (mine, _repo) = r.provisionable_task().await;
    let theirs = create_task_returning(
        &*r.db,
        "Theirs",
        "desc",
        "/elsewhere",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    let my_req = requested(
        r.rt.task_svc
            .request_dispatch(mine.id, DispatchOrigin::Mcp, None)
            .await
            .unwrap(),
    );
    let ReducerOutcome::Applied(ids) = r
        .caller
        .request_dispatch(
            theirs.id,
            "other-host".into(),
            "user".into(),
            "dispatch".into(),
        )
        .await
        .unwrap()
    else {
        panic!("seeding another host's request must apply");
    };
    let their_req = DispatchRequestId(ids[0]);

    for handle in r.rt.serve_dispatch_requests().await {
        handle.await.unwrap();
    }

    let mine_row = r.db.dispatch_request(my_req).await.unwrap();
    assert_eq!(mine_row.state, DispatchRequestState::Dispatched);
    let stored = r.db.get_task(mine.id).await.unwrap();
    assert!(
        stored.worktree.is_some(),
        "the module recorded the worktree"
    );
    assert_eq!(
        r.db.dispatch_request(their_req).await.unwrap().state,
        DispatchRequestState::Requested,
        "another host's row is not this board's to claim"
    );
}

/// A `requested` row naming this host, written straight into the
/// subscription as `requested_by` — a row for task 1, which need not exist.
fn seed_requested_row(rows: &SharedRows, id: i64, requested_by: &str) {
    rows.upsert_dispatch_request(&crate::spacetime::bindings::DispatchRequest {
        id,
        task_id: 1,
        kind: "dispatch".into(),
        host: crate::store::MEMORY_HOST_ID.into(),
        mode: "dispatch".into(),
        origin: "user".into(),
        state: "requested".into(),
        requested_at: "2026-10-11 12:00:00.000".into(),
        claimed_at: String::new(),
        finished_at: String::new(),
        reason: String::new(),
        worktree: String::new(),
        tmux_window: String::new(),
        deadline: "2099-01-01 00:00:00.000".into(),
        requested_by: requested_by.into(),
    });
}

/// A claim that never reached the module leaves the row `requested` and this
/// board's to serve: the next reload hands it out again, and that claim
/// provisions it (`store-split.allium: HostClaimsDispatch`). Only an
/// answered claim — accepted or refused — marks a row served.
#[tokio::test]
async fn a_claim_that_could_not_be_sent_is_retried_on_the_next_reload() {
    let r = rig(DispatchScript::dispatch().shared_runner()).await;
    let (task, _repo) = r.provisionable_task().await;
    let id = requested(
        r.rt.task_svc
            .request_dispatch(task.id, DispatchOrigin::User, None)
            .await
            .unwrap(),
    );
    r.caller
        .fail_next("claim_dispatch_request", crate::sync::InjectedFault::Unsent);

    for handle in r.rt.serve_dispatch_requests().await {
        handle.await.unwrap();
    }
    assert_eq!(
        r.db.dispatch_request(id).await.unwrap().state,
        DispatchRequestState::Requested,
        "the claim never reached the module"
    );

    for handle in r.rt.serve_dispatch_requests().await {
        handle.await.unwrap();
    }
    assert_eq!(
        r.db.dispatch_request(id).await.unwrap().state,
        DispatchRequestState::Dispatched,
        "the next reload serves the row again"
    );
}

/// A board whose identity has not settled serves nothing yet — and forgets
/// nothing: once it settles, the row it held back is served.
#[tokio::test]
async fn a_row_held_back_for_an_unsettled_identity_is_served_once_it_settles() {
    let identity = Arc::new(SettledIdentity::default());
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let r = rig_with_identity(runner, identity.clone()).await;
    seed_requested_row(&r.rows, 77, "test-user");

    assert!(
        r.rt.serve_dispatch_requests().await.is_empty(),
        "no settled identity, nothing served"
    );

    identity.settle("test-user");
    let served = r.rt.serve_dispatch_requests().await;
    assert_eq!(
        served.len(),
        1,
        "the held-back row is served once it settles"
    );
    for handle in served {
        handle.await.unwrap();
    }
}

/// D11's second line (`dispatch.allium: ProvisionDispatch`): a row naming
/// this host that its owner did not request is ignored — not claimed, not
/// provisioned — whatever let it in.
#[tokio::test]
async fn the_host_ignores_a_row_its_owner_did_not_request() {
    let runner = Arc::new(MockProcessRunner::new(vec![]));
    let r = rig(runner.clone()).await;
    seed_requested_row(&r.rows, 99, "beef");

    let served = r.rt.serve_dispatch_requests().await;

    assert!(
        served.is_empty(),
        "a foreign requester's row must not be served"
    );
    assert!(runner.recorded_calls().is_empty());
    assert_eq!(
        r.db.dispatch_request(DispatchRequestId(99))
            .await
            .unwrap()
            .state,
        DispatchRequestState::Requested
    );
}

/// `HostRestartFailsInterrupted` then D5 catch-up: at startup the board first
/// fails the rows a previous board left `provisioning` ("host restarted",
/// releasing their claims), then serves the rows still `requested`.
#[tokio::test]
async fn startup_fails_interrupted_rows_then_serves_requested_ones() {
    let r = rig(DispatchScript::dispatch().shared_runner()).await;
    let interrupted = create_task_returning(
        &*r.db,
        "Interrupted",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    let interrupted_req = requested(
        r.rt.task_svc
            .request_dispatch(
                interrupted.id,
                DispatchOrigin::User,
                Some(DispatchMode::Dispatch),
            )
            .await
            .unwrap(),
    );
    // The previous board claimed it, then died mid-provision.
    assert!(r
        .caller
        .claim_dispatch_request(interrupted_req, crate::store::MEMORY_HOST_ID.into())
        .await
        .unwrap()
        .won());
    let (waiting, _repo) = r.provisionable_task().await;
    let waiting_req = requested(
        r.rt.task_svc
            .request_dispatch(waiting.id, DispatchOrigin::EpicChain, None)
            .await
            .unwrap(),
    );

    for handle in r.rt.recover_dispatch_requests().await {
        handle.await.unwrap();
    }

    let failed = r.db.dispatch_request(interrupted_req).await.unwrap();
    assert_eq!(failed.state, DispatchRequestState::Failed);
    assert_eq!(failed.reason.as_deref(), Some("host restarted"));
    assert_eq!(
        r.db.get_task(interrupted.id).await.unwrap().status,
        models::TaskStatus::Backlog,
        "the interrupted claim is released at once, not left for the timer"
    );
    assert_eq!(
        r.db.dispatch_request(waiting_req).await.unwrap().state,
        DispatchRequestState::Dispatched,
        "a requested row from the initial rows is picked up"
    );
}

/// The App reads this host's rows from the same reload as the cards, so
/// DispatchingFeedback follows the row (`dispatch.allium: DispatchingFeedback`).
#[tokio::test]
async fn a_board_reload_hands_the_app_this_hosts_dispatch_requests() {
    let mut r = rig(Arc::new(MockProcessRunner::new(vec![]))).await;
    let task = create_task_returning(
        &*r.db,
        "Shown",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    let id = requested(
        r.rt.task_svc
            .request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
    );

    r.rt.spawn_refresh_from_db().await.unwrap();

    let mut delivered = None;
    while let Ok(msg) = r.msgs.try_recv() {
        if let Message::Task(crate::tui::messages::TaskMessage::DispatchRequestsRefreshed(rows)) =
            msg
        {
            delivered = Some(rows);
        }
    }
    let rows = delivered.expect("reload_board must send this host's dispatch requests");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
}

// -- Bootstrap ----------------------------------------------------------------

/// `store-split.allium: HostRestartFailsInterrupted` is wired into startup:
/// a board starting over a row its host left `provisioning` fails it before
/// it draws, on the first connection.
mod bootstrap {
    use super::*;
    use crate::store_connection::StoreParts;
    use crate::sync::tests::{accepted, ScriptedConnector};

    fn stamped_task(title: &str) -> crate::spacetime::bindings::Task {
        crate::spacetime::bindings::Task {
            title: title.into(),
            repo_path: "/repo".into(),
            created_at: "2026-10-11 12:00:00.000".into(),
            updated_at: "2026-10-11 12:00:00.000".into(),
            owner: "c0ffee".into(),
            created_by: "c0ffee".into(),
            ..dispatch_spacetime_module::blank_task().into()
        }
    }

    /// Poll a future the in-memory caller answers without suspending — the
    /// store builder is a plain `fn`, so it cannot await.
    fn ready<F: std::future::Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        match f
            .as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
        {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("the in-memory caller answers synchronously"),
        }
    }

    /// A store that already holds one row THIS host was provisioning when its
    /// previous board stopped (request id 1, task id 1).
    fn store_with_an_interrupted_dispatch(data_dir: &Path, host: &str) -> StoreParts {
        use crate::sync as sy;
        let rows = Arc::new(sy::SharedRows::new());
        let clock: Arc<dyn crate::clock::Clock> = Arc::new(crate::clock::SystemClock);
        // The calls are sent as the identity the connector settles, which
        // owns this host (`store-split.allium`, D11).
        let caller = Arc::new(
            sy::memory_caller::MemoryReducerCaller::new(rows.clone(), clock.clone())
                .with_sender("c0ffee")
                .with_registered_host(host, "c0ffee"),
        );
        let task = ready(caller.create_task(stamped_task("Interrupted"))).unwrap();
        ready(caller.request_dispatch(task, host.into(), "epic_chain".into(), String::new()))
            .unwrap()
            .applied()
            .unwrap();
        assert!(
            ready(caller.claim_dispatch_request(DispatchRequestId(1), host.into()))
                .unwrap()
                .won()
        );
        let settled_identity = Arc::new(sy::SettledIdentity::default());
        settled_identity.settle("c0ffee");
        let reducer_caller: Arc<dyn sy::ReducerCaller> = caller;
        let database = crate::store::Store::new(
            rows.clone(),
            reducer_caller.clone(),
            settled_identity.clone(),
            clock,
            host.to_string(),
            data_dir,
        );
        StoreParts {
            database: Arc::new(database),
            rows,
            connector: ScriptedConnector::new(vec![accepted("c0ffee", "token")]),
            settled_identity,
            reducer_caller,
            store_identity: |_| None,
        }
    }

    #[tokio::test]
    async fn bootstrap_fails_this_hosts_interrupted_dispatches() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let paths = StartupPaths {
            claude_dir: dir.path().join("claude"),
            claude_json_path: dir.path().join(".claude.json"),
        };
        crate::host_file::resolve_for_launch(dir.path()).unwrap();
        crate::host_file::rename_host(dir.path(), "m4-bootstrap-host").unwrap();

        let bootstrap = TuiRuntime::bootstrap_with(
            &data_dir,
            0,
            &paths,
            "http://store.test".into(),
            store_with_an_interrupted_dispatch,
            false,
        )
        .await
        .expect("bootstrap must succeed");

        let row = bootstrap
            .runtime
            .database
            .dispatch_request(DispatchRequestId(1))
            .await
            .expect("the row stays as history");
        assert_eq!(row.state, DispatchRequestState::Failed);
        assert_eq!(row.reason.as_deref(), Some("host restarted"));
        assert_eq!(
            bootstrap
                .runtime
                .database
                .get_task(row.task_id)
                .await
                .unwrap()
                .status,
            models::TaskStatus::Backlog,
            "the restart releases the claim instead of leaving it for the timer"
        );
    }
}
