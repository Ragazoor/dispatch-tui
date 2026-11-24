use super::*;
use crate::models::test_tmux_window;

use crate::dispatch::mock_sequence::DispatchScript;
use crate::mcp::handlers::tasks::WrapUpAction;

// -- closing a session -------------------------------------------------------

/// Wiring shared by the close and dispatch tests: a temp repo, an in-memory DB,
/// an `McpState` with a notification channel, and a runner with a generous
/// queue of successes covering the closing task's detached `tmux kill-window`
/// and a full worktree provisioning.
struct CloseFixture {
    _dir: tempfile::TempDir,
    repo_path: String,
    db: Arc<dyn store::TaskStore>,
    state: Arc<McpState>,
    /// Held so the state's board notifications have a live receiver.
    _notify_rx: tokio::sync::mpsc::UnboundedReceiver<crate::board_event::BoardEvent>,
    /// The same runner the state holds, kept concrete so a test can inspect
    /// which commands the close path actually issued.
    runner: Arc<MockProcessRunner>,
    /// The host reactor serving this fixture's dispatch requests
    /// ([`spawn_host`]). Aborted with the fixture.
    host: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for CloseFixture {
    fn drop(&mut self) {
        if let Some(host) = self.host.take() {
            host.abort();
        }
    }
}

impl CloseFixture {
    async fn new() -> Self {
        Self::build(None).await
    }

    /// Like [`CloseFixture::new`], but with a caller-supplied runner script, for
    /// tests that assert on the exact command sequence a dispatch issues (or
    /// need one to fail).
    async fn with_runner(runner: Arc<MockProcessRunner>) -> Self {
        Self::build_with(runner, None).await
    }

    /// Like [`CloseFixture::new`], but with a `task_svc` whose `close_task`
    /// always fails, so `task_update(status="done")`'s close does not take
    /// effect (`MarkTaskDoneViaMcp`).
    async fn with_failing_close() -> Self {
        let svc = Arc::new(FailingCloseTaskService::default());
        let fx = Self::build(Some(svc.clone())).await;
        let _ = svc.db.set(fx.db.clone());
        fx
    }

    /// Like [`CloseFixture::new`], but with a `task_svc` whose
    /// `request_dispatch` is always refused, standing in for another entry
    /// point having taken the task first. Every other method panics, so a
    /// caller that dispatches without requesting is caught rather than
    /// silently passing.
    async fn with_lost_claim() -> Self {
        let svc = Arc::new(LostClaimTaskService::default());
        let fx = Self::build(Some(svc.clone())).await;
        let _ = svc.db.set(fx.db.clone());
        fx
    }

    async fn build(task_svc_override: Option<Arc<dyn crate::service::TaskServiceApi>>) -> Self {
        // The host's teardown runs beside the test and MockProcessRunner pops a
        // shared FIFO, so queue uniform successes rather than a command-ordered
        // script.
        let runner = Arc::new(MockProcessRunner::new(
            (0..24).map(|_| MockProcessRunner::ok()).collect(),
        ));
        Self::build_with(runner, task_svc_override).await
    }

    async fn build_with(
        runner: Arc<MockProcessRunner>,
        task_svc_override: Option<Arc<dyn crate::service::TaskServiceApi>>,
    ) -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

        let (notify_tx, notify_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::board_event::BoardEvent>();
        let (state, db) = test_state_with_overrides(
            runner.clone() as Arc<dyn ProcessRunner>,
            Some(notify_tx),
            task_svc_override,
        )
        .await;
        let host = Some(spawn_host(&state));
        Self {
            _dir: dir,
            repo_path,
            db,
            state,
            _notify_rx: notify_rx,
            runner,
            host,
        }
    }

    /// Wait for `task`'s exit_session or close row to finish — the host's teardown ran
    /// and reported. A deadline on a signal, not a wall-clock wait: it wakes on
    /// every row change and returns the instant the row finishes.
    async fn wait_for_teardown(&self, task: crate::models::TaskId) {
        let mut changes = self.db.session_request_changes();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let finished = self
                    .db
                    .session_requests()
                    .await
                    .into_iter()
                    .any(|r| r.task_id == task && r.kind.is_teardown() && !r.is_open());
                if finished {
                    return;
                }
                changes.changed().await.expect("the rows outlive the test");
            }
        })
        .await
        .expect("the teardown must finish");
    }

    /// Every `tmux kill-window` the close path issued, in order.
    fn kill_window_calls(&self) -> Vec<(String, Vec<String>)> {
        self.runner
            .recorded_calls()
            .into_iter()
            .filter(|(program, args)| {
                program == "tmux" && args.first().is_some_and(|a| a == "kill-window")
            })
            .collect()
    }

    async fn epic(&self) -> crate::models::EpicId {
        self.db.create_epic("Epic", "desc", None).await.unwrap().id
    }

    /// A backlog subtask. `repo_path` defaults to the
    /// fixture's temp repo; pass an override to make provisioning fail.
    async fn backlog_subtask(
        &self,
        epic_id: Option<crate::models::EpicId>,
        title: &str,
        repo_path: Option<&str>,
    ) -> crate::models::TaskId {
        let id = self
            .db
            .create_task(CreateTaskRequest {
                plan: Some("docs/plan.md"),
                epic_id,
                ..CreateTaskRequest::fixture(title, repo_path.unwrap_or(&self.repo_path))
            })
            .await
            .unwrap();
        // Mocked git never creates the worktree directory; pre-create it so
        // provisioning takes the "reuse existing worktree" branch.
        crate::dispatch::tests::create_worktree_dir(
            std::path::Path::new(&self.repo_path)
                .join(".worktrees")
                .join(format!("{}-{}", id.0, crate::models::slugify(title))),
        );
        id
    }

    /// The task whose session is about to close: Running with a worktree and a
    /// tmux window, which is what `wrap_up_block` and `exit_session` demand.
    /// Only the epic membership and the fixture's real temp repo distinguish it
    /// from the shared helper, so it supplies those and delegates.
    async fn closing_subtask(
        &self,
        epic_id: Option<crate::models::EpicId>,
    ) -> crate::models::TaskId {
        create_running_task_with_window_in(&self.state, &self.repo_path, epic_id).await
    }

    /// Close `task_id`'s session with `action`. Thin wrapper over the shared
    /// [`close_session_via_mcp`] so the exit-token shape lives in one place.
    async fn close(
        &self,
        task_id: crate::models::TaskId,
        action: crate::mcp::handlers::tasks::WrapUpAction,
    ) -> JsonRpcResponse {
        close_session_via_mcp(&self.state, task_id, action).await
    }

    /// Close `task_id` via `update_task`'s dedicated `status="done"` path
    /// (MarkTaskDoneViaMcp) instead of wrap_up/exit_session — no exit token,
    /// called as a Session caller so it may close a task that is not its own.
    async fn close_via_update_task(&self, task_id: crate::models::TaskId) -> JsonRpcResponse {
        call(
            &self.state,
            "tools/call",
            Some(json!({
                "name": "task_update",
                "arguments": { "task_ids": [task_id.0], "status": "done" }
            })),
        )
        .await
    }
}

// -- task_update(status="done") ----------------------------------------------
//
// MarkTaskDoneViaMcp (mcp-task-tools.allium) closes in the module
// (`close_task`, store-split.allium: RequestCloseTask, X1), with no token; the
// window is torn down by the host serving the `close` row.

/// Closing a task with a live tmux window via task_update(status="done")
/// tears the window down, the same as exit_session does — parity with
/// `exit_session_successful_close_kills_the_tmux_window`. The module writes a
/// `close` row naming the window and this host's reactor serves it; the test
/// awaits that row finishing on the subscription's change signal rather than
/// sleeping — see the "No `tokio::time::sleep` in tests" section of
/// `docs/conventions.md`.
#[tokio::test]
async fn update_task_done_kills_the_tmux_window() {
    let fx = CloseFixture::new().await;
    let closing = fx.closing_subtask(None).await;

    let resp = fx.close_via_update_task(closing).await;
    assert!(resp.error.is_none(), "close must succeed: {:?}", resp.error);

    fx.wait_for_teardown(closing).await;

    assert_eq!(
        fx.kill_window_calls().len(),
        1,
        "expected exactly one tmux kill-window call: {:?}",
        fx.kill_window_calls()
    );
    assert!(fx.db.get_task(closing).await.unwrap().tmux_window.is_none());
    let close_rows: Vec<_> = fx
        .db
        .session_requests()
        .await
        .into_iter()
        .filter(|r| r.task_id == closing && r.kind == crate::models::SessionRequestKind::Close)
        .collect();
    assert_eq!(close_rows.len(), 1, "one close row: {close_rows:?}");
    assert!(close_rows[0].wrap_up_request.is_none(), "no exit token");
}

/// A close that does not take effect must leave the task exactly as it was —
/// no status change, no cleared window — and say so in the response.
/// (The module's close is whole or absent, as `exit_session`'s is:
/// `a_refused_exit_session_changes_nothing_and_kills_nothing`.)
#[tokio::test]
async fn update_task_done_failed_close_leaves_the_task_unchanged() {
    let fx = CloseFixture::with_failing_close().await;
    let closing = fx.closing_subtask(None).await;
    let before = fx.db.get_task(closing).await.unwrap();

    let resp = fx.close_via_update_task(closing).await;
    assert_eq!(
        resp.result.as_ref().and_then(|r| r["isError"].as_bool()),
        Some(true),
        "a failed close is a tool error, not a success-shaped result"
    );
    assert!(
        extract_response_text(&resp).contains("could NOT be moved to done"),
        "response must not claim success: {}",
        extract_response_text(&resp)
    );

    let after = fx.db.get_task(closing).await.unwrap();
    assert_eq!(
        after.status, before.status,
        "a close that did not persist must not move the task"
    );
    assert_eq!(
        after.tmux_window, before.tmux_window,
        "the task's record of its window is only cleared when the write lands"
    );
    assert!(
        fx.kill_window_calls().is_empty(),
        "a failed close must issue no tmux kill-window"
    );
}

/// Closing a session of a task in an epic starts nothing else: the epic's
/// backlog subtask stays in backlog with no dispatch request, and the reply is
/// exactly "Session closed." (`orchestrators.allium` O8).
///
/// The module writes any request inside `request_exit_session`'s own
/// transaction, so the absence of a row is final once the close responds.
#[tokio::test]
async fn exit_session_in_an_epic_leaves_its_backlog_subtask_alone() {
    let fx = CloseFixture::new().await;
    let epic_id = fx.epic().await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let next = fx.backlog_subtask(Some(epic_id), "Task 1", None).await;

    let resp = fx.close(closing, WrapUpAction::Done).await;
    assert_eq!(extract_response_text(&resp), "Session closed.");
    assert_eq!(
        fx.db.get_task(closing).await.unwrap().status,
        TaskStatus::Done
    );

    let untouched = fx.db.get_task(next).await.unwrap();
    assert_eq!(untouched.status, TaskStatus::Backlog);
    assert!(untouched.worktree.is_none());
    assert!(untouched.tmux_window.is_none());
    assert!(
        fx.db.latest_dispatch_request_for_task(next).await.is_none(),
        "closing a session must not request a dispatch"
    );
}

/// A task with no epic closes cleanly.
#[tokio::test]
async fn exit_session_without_epic_closes() {
    let fx = CloseFixture::new().await;
    let closing = fx.closing_subtask(None).await;

    let resp = fx.close(closing, WrapUpAction::Done).await;
    assert_eq!(extract_response_text(&resp), "Session closed.");
    assert_eq!(
        fx.db.get_task(closing).await.unwrap().status,
        TaskStatus::Done
    );
}

// -- a close that does not take effect ----------------------------------------
//
// `exit_session`'s close is the module's (`store-split.allium` W4), in ONE
// transaction with the token and the epic rollup
// (`pr-workflow.allium: ExitSession`), and so is `task_update`
// status="done" (`close_task`, X1): either took effect whole or not at all. The
// board does not gate a teardown on a close; the one test below drives a
// close call that gets no effect.

/// A task service whose `close_task` always fails, so `task_update`'s close
/// does not take effect. Every other method inherits the panicking default from
/// `TaskServiceApiStub` — which is itself an assertion.
#[derive(Default)]
struct FailingCloseTaskService {
    /// Bound by the fixture once its database exists; backs the real read the
    /// handler now makes through the service.
    db: std::sync::OnceLock<Arc<dyn crate::store::TaskStore>>,
}

#[async_trait::async_trait]
impl crate::service::TaskServiceApiStub for FailingCloseTaskService {
    async fn get_task(
        &self,
        task_id: crate::models::TaskId,
    ) -> Result<crate::models::Task, crate::service::ServiceError> {
        let db = self.db.get().expect("db is bound by the fixture");
        db.get_task(task_id).await.ok_or_else(|| {
            crate::service::ServiceError::NotFound(format!("task {task_id} not found"))
        })
    }

    async fn close_task(
        &self,
        _task_id: crate::models::TaskId,
    ) -> Result<(), crate::service::ServiceError> {
        Err(crate::service::ServiceError::Internal(anyhow::anyhow!(
            "simulated persistence failure"
        )))
    }
}

crate::task_service_api!(service_api_stub_bridge, FailingCloseTaskService);

/// A task service whose dispatch request is always refused, standing in for
/// another dispatch entry point having taken the task a moment earlier.
///
/// Every other method keeps the panicking `TaskServiceApiStub` default on
/// purpose: that is what makes `dispatch_task_lost_claim_provisions_nothing`
/// discriminating. A handler that read the status and provisioned itself,
/// rather than asking the module, would hit an unmocked method.
///
/// That the request itself is exclusive is asserted against the real seam in
/// `service::tasks::tests::dispatch_seam::two_concurrent_requests_write_exactly_one_row`;
/// what is under test here is only the response the handler shapes from it.
#[derive(Default)]
struct LostClaimTaskService {
    /// Bound by the fixture once its database exists; backs the real read the
    /// handler now makes through the service.
    db: std::sync::OnceLock<Arc<dyn crate::store::TaskStore>>,
}

#[async_trait::async_trait]
impl crate::service::TaskServiceApiStub for LostClaimTaskService {
    async fn get_task(
        &self,
        task_id: crate::models::TaskId,
    ) -> Result<crate::models::Task, crate::service::ServiceError> {
        let db = self.db.get().expect("db is bound by the fixture");
        db.get_task(task_id).await.ok_or_else(|| {
            crate::service::ServiceError::NotFound(format!("task {task_id} not found"))
        })
    }

    async fn request_dispatch(
        &self,
        task_id: crate::models::TaskId,
        _origin: crate::models::DispatchOrigin,
        _mode: Option<crate::models::DispatchMode>,
    ) -> Result<crate::store::RequestDispatchAnswer, crate::service::ServiceError> {
        Ok(crate::store::RequestDispatchAnswer::Refused(format!(
            "task {task_id} already holds an open dispatch request"
        )))
    }
}

crate::task_service_api!(service_api_stub_bridge, LostClaimTaskService);

/// A refused close changes NOTHING and leaves the token valid — the module
/// checks the token, applies the close and writes the teardown row in one
/// transaction. The task keeps its status, sub_status and window (so it stays
/// visible in its current column, and its live agent stays attachable), no
/// window is killed and no row is written.
#[tokio::test]
async fn a_refused_exit_session_changes_nothing_and_kills_nothing() {
    let fx = CloseFixture::new().await;
    let closing = fx.closing_subtask(None).await;
    let token = seed_exit_token(&fx.state, closing, WrapUpAction::Done).await;
    let before = fx.db.get_task(closing).await.unwrap();
    let rows_before = fx.db.session_requests().await.len();

    // The action differs from the one the token was issued for.
    let resp = call(
        &fx.state,
        "tools/call",
        Some(json!({
            "name": "session_exit",
            "arguments": { "task_id": closing.0, "token": token, "action": "rebase" }
        })),
    )
    .await;

    assert!(is_error(&resp));
    let after = fx.db.get_task(closing).await.unwrap();
    assert_eq!(after.status, before.status);
    assert_eq!(after.sub_status, before.sub_status);
    assert_eq!(
        after.tmux_window, before.tmux_window,
        "the task's window is only cleared by a close that took effect"
    );
    assert_eq!(fx.db.session_requests().await.len(), rows_before);
    assert!(
        fx.kill_window_calls().is_empty(),
        "a refused close must leave the tmux window alive: {:?}",
        fx.kill_window_calls()
    );

    // The token is still valid: the retry with the right action closes.
    let retry = call(
        &fx.state,
        "tools/call",
        Some(json!({
            "name": "session_exit",
            "arguments": { "task_id": closing.0, "token": token, "action": "done" }
        })),
    )
    .await;
    assert!(
        extract_response_text(&retry).starts_with("Session closed."),
        "{retry:?}"
    );
}

/// The teardown is the HOST's, from the exit_session row: a close that took
/// effect leaves exactly one `tmux kill-window` for the closing task's window,
/// issued only after the terminal Done write landed (the row is written in the
/// same transaction). Without it a closed task's tmux window would survive
/// forever.
///
/// Awaits the row finishing on the subscription's change signal rather than
/// sleeping — see the "No `tokio::time::sleep` in tests" section of
/// `docs/conventions.md`.
#[tokio::test]
async fn exit_session_successful_close_kills_the_tmux_window() {
    let fx = CloseFixture::new().await;
    let closing = fx.closing_subtask(None).await;
    let window = fx
        .db
        .get_task(closing)
        .await
        .unwrap()
        .tmux_window
        .expect("closing_subtask must have a tmux window");

    let resp = fx.close(closing, WrapUpAction::Done).await;
    assert!(resp.error.is_none(), "close must succeed: {:?}", resp.error);
    assert_eq!(
        fx.db.get_task(closing).await.unwrap().status,
        TaskStatus::Done,
        "the Done write lands with the row the teardown is served from"
    );
    fx.wait_for_teardown(closing).await;

    // `kill_window` resolves the window name to tmux's pane-id target
    // (`window_target` in `src/tmux.rs`) before issuing `kill-window`, so the
    // command carries `%N`, not the literal window name — resolve the same way
    // `MockProcessRunner`'s permissive `AnyName` lookup would have.
    let pane_id = fx.runner.pane_id_of(window.as_str());
    let kills = fx.kill_window_calls();
    assert_eq!(
        kills.len(),
        1,
        "expected exactly one tmux kill-window call, got: {kills:?}"
    );
    assert!(
        kills[0].1.iter().any(|arg| arg == pane_id.as_str()),
        "expected kill-window to target {pane_id:?} (window {window:?}), got: {kills:?}"
    );
}

/// `dispatch_next` is gone: an agent calling it gets an unknown-tool error.
#[tokio::test]
async fn dispatch_next_tool_no_longer_exists() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": 1 }
        })),
    )
    .await;
    assert_error(&resp, "Unknown tool");
}

#[tokio::test]
async fn wrap_up_rebase_preserves_tmux_window() {
    let (state, db) = test_state_with_host(
        with_clean_status(DispatchScript::finish().no_remote()),
        None,
        None,
    )
    .await;

    let task_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture("Rebase Preserve Window", "/repo")
        })
        .await
        .unwrap();
    db.give_agent(task_id);
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-rebase-preserve"))
            .tmux_window(Some(&test_tmux_window("task-99"))),
    )
    .await
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "session_wrap-up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("wrap_up complete"));
    assert!(
        text.contains("session_exit"),
        "response should instruct agent to call exit_session; got: {text}"
    );

    let task = db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "wrap_up must not change status — exit_session owns the Done transition"
    );
    assert!(
        task.tmux_window.is_some(),
        "tmux_window must NOT be cleared — exit_session owns the window kill"
    );
}

#[tokio::test]
async fn wrap_up_rebase_conflict_sets_conflict_substatus() {
    let (state, db) = test_state_with_host(
        with_clean_status(
            DispatchScript::finish()
                .no_remote()
                .rebase_conflicts_in_stderr(&["foo.rs"]),
        ),
        None,
        None,
    )
    .await;

    let task_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture("Conflict Sub", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-conflict-sub")),
    )
    .await
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "session_wrap-up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    assert_error(&resp, "conflict");
    let task = db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on rebase conflict"
    );
    assert_eq!(
        task.sub_status,
        SubStatus::Conflict,
        "sub_status should be Conflict after rebase conflict"
    );
}

#[tokio::test]
async fn wrap_up_rebase_clears_conflict_substatus_on_non_conflict_error() {
    // When a task has Conflict sub_status from a previous rebase attempt,
    // and a new rebase fails with a non-conflict error (e.g. Other), the
    // stale Conflict sub_status should be cleared — matching TUI behavior.
    // This queue was stale before the script existed: it led with a
    // `detect_default_branch` response `finish_task` never asks for, and omitted
    // the dirty-worktree porcelain read, so every response after the first
    // answered the wrong call and it still passed.
    let (state, db) = test_state_with_host(
        with_clean_status(DispatchScript::finish().no_remote().rebase_fails()),
        None,
        None,
    )
    .await;

    let task_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture("Stale Conflict", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-stale-conflict"))
            .sub_status(SubStatus::Conflict),
    )
    .await
    .unwrap();

    // Verify conflict is set before wrap_up
    let task = db.get_task(task_id).await.unwrap();
    assert_eq!(task.sub_status, SubStatus::Conflict);

    let _resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "session_wrap-up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let task = db.get_task(task_id).await.unwrap();
    assert_ne!(
        task.sub_status,
        SubStatus::Conflict,
        "Stale Conflict sub_status should be cleared even on non-conflict rebase error"
    );
}

// ---------------------------------------------------------------------------
// task_dispatch tests
// ---------------------------------------------------------------------------

/// The exact command sequence one successful dispatch issues. Scripted rather
/// than uniformly-ok because the order is itself the assertion, and because the
/// split-window call must return a pane id.
fn dispatch_runner_script() -> Arc<MockProcessRunner> {
    crate::dispatch::mock_sequence::DispatchScript::dispatch().shared_runner()
}

/// The `task_dispatch` tools/call for `task_id`.
async fn call_dispatch_task(
    state: &Arc<McpState>,
    task_id: crate::models::TaskId,
) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({
            "name": "task_dispatch",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await
}

#[tokio::test]
async fn dispatch_task_dispatches_backlog_task() {
    let fx = CloseFixture::with_runner(dispatch_runner_script()).await;
    let task_id = fx.backlog_subtask(None, "My Backlog Task", None).await;

    let resp = call_dispatch_task(&fx.state, task_id).await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("dispatched"),
        "Expected 'dispatched' in response, got: {text}"
    );

    // task_dispatch is synchronous — no sleep needed
    let task = fx.db.get_task(task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert!(
        task.worktree.is_some(),
        "worktree should be set after dispatch"
    );
    assert!(
        task.tmux_window.is_some(),
        "tmux_window should be set after dispatch"
    );
    assert!(
        task.last_pre_tool_use_at.is_some(),
        "last_pre_tool_use_at should be seeded so the tick classifier does not flicker the task to Stale"
    );
}

/// rule-guidance.DispatchTaskViaMcp ("The success text names which provisioning
/// path ran"): the reuse path must SAY it reused.
///
/// `CloseFixture::backlog_subtask` pre-creates the worktree directory, which is
/// what puts provisioning on the reuse branch.
#[tokio::test]
async fn dispatch_task_success_text_names_a_reused_worktree() {
    let fx = CloseFixture::with_runner(dispatch_runner_script()).await;
    let task_id = fx.backlog_subtask(None, "Crashed Task", None).await;

    let resp = call_dispatch_task(&fx.state, task_id).await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("reused existing worktree"),
        "a reused worktree must be named as reused, got: {text}"
    );
}

/// The other half of the same guidance. A full fresh-path dispatch cannot be
/// driven end-to-end here — mocked git never creates the worktree directory, so
/// the `.claude-prompt` write fails and the dispatch rolls back — so the wording
/// is asserted on the formatter that owns it, with the fresh path's own
/// `reused_worktree = false` supplied directly. That the flag itself is set
/// correctly on both paths is covered by
/// `provision_worktree_reports_reused_worktree_false_when_dir_missing` and its
/// true-side twin in `src/dispatch/tests.rs`.
#[test]
fn dispatch_task_success_text_names_a_created_worktree() {
    let text = crate::mcp::handlers::tasks::dispatch::dispatched_text(
        "Task 7 (ada, \"Thing\")",
        &crate::models::DispatchResult {
            worktree_path: "/repo/.worktrees/7-thing".to_string(),
            tmux_window: test_tmux_window("task-7"),
            reused_worktree: false,
        },
    );
    assert!(
        text.contains("created worktree"),
        "a freshly provisioned worktree must be named as created, got: {text}"
    );
    assert!(
        !text.contains("reused"),
        "the fresh path must not claim a reuse, got: {text}"
    );
}

#[tokio::test]
async fn dispatch_task_returns_error_for_non_backlog_task() {
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            description: "already running",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("Running Task", "/repo")
        })
        .await
        .unwrap();

    let resp = call_dispatch_task(&state, task_id).await;

    assert_error(&resp, "not in backlog");
}

#[tokio::test]
async fn dispatch_task_reports_the_status_a_lost_claim_left_behind() {
    // The claim, not a pre-read, is what rejects this call now. A task another
    // entry point already took is simply no longer Backlog, so the conditional
    // transition finds nothing to move and the handler re-reads to name the
    // status the task actually holds (DispatchTaskViaMcp in
    // docs/specs/mcp-task-tools.allium).
    let fx = CloseFixture::with_runner(dispatch_runner_script()).await;
    let task_id = fx.backlog_subtask(None, "Contended Task", None).await;
    assert!(
        matches!(
            fx.state
                .task_svc
                .request_dispatch(task_id, crate::models::DispatchOrigin::User, None)
                .await
                .unwrap(),
            crate::store::RequestDispatchAnswer::Requested(_)
        ),
        "another entry point requests it first"
    );

    let resp = call_dispatch_task(&fx.state, task_id).await;

    assert_error(&resp, "not in backlog");
    assert_error(&resp, "running");
    assert_eq!(
        fx.db.dispatch_requests().await.len(),
        1,
        "the losing call must write no second request, so nothing provisions twice"
    );
}

/// A lost claim must gate provisioning, not merely be noticed afterwards.
///
/// Discriminating by construction: `LostClaimTaskService` mocks only
/// `request_dispatch` (and the read), and every other `TaskServiceApi` method
/// panics. A handler that read the status instead of requesting would sail past
/// the Backlog row and reach an unmocked method. Reaching the assertions at all
/// proves the request was consulted; the empty runner log proves nothing was
/// provisioned. The refusal's own reason reaches the caller, because the task
/// still reads Backlog.
#[tokio::test]
async fn dispatch_task_lost_claim_provisions_nothing() {
    let fx = CloseFixture::with_lost_claim().await;
    let task_id = fx.backlog_subtask(None, "Contended Task", None).await;

    let resp = call_dispatch_task(&fx.state, task_id).await;

    assert_error(&resp, "already holds an open dispatch request");
    assert!(
        fx.runner.recorded_calls().is_empty(),
        "a lost claim must run no provisioning commands at all, got: {:?}",
        fx.runner.recorded_calls()
    );
    let task = fx.db.get_task(task_id).await.unwrap();
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
}

/// A dispatch that fails after the claim must release it, leaving the task
/// exactly as dispatchable as before the call. The module releases it when
/// the request fails (`ReleaseClaimOfFailedRequest`); the call itself releases
/// nothing, and answers with the row's reason.
#[tokio::test]
async fn dispatch_task_releases_the_claim_when_provisioning_fails() {
    let fx = CloseFixture::new().await;
    // A repo path that does not exist fails provisioning before any subprocess
    // runs, so the failure is deterministic.
    let task_id = fx
        .backlog_subtask(None, "Doomed", Some("/nonexistent/dispatch-mcp-repo"))
        .await;

    let resp = call_dispatch_task(&fx.state, task_id).await;

    assert_error(&resp, "dispatch failed");
    // Every task_dispatch text names the task as a task ref.
    assert_error(&resp, &task_ref_of(&fx.state, task_id).await);
    let task = fx.db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Backlog,
        "a failed dispatch must release the claim, not leave the task Running"
    );
    assert_eq!(
        task.sub_status,
        crate::models::SubStatus::default_for(TaskStatus::Backlog)
    );
    assert!(
        task.last_pre_tool_use_at.is_none(),
        "the release clears the stamp the claim seeded"
    );
    assert!(task.worktree.is_none());
    assert!(
        matches!(
            fx.state
                .task_svc
                .request_dispatch(task_id, crate::models::DispatchOrigin::User, None)
                .await
                .unwrap(),
            crate::store::RequestDispatchAnswer::Requested(_)
        ),
        "the released task is dispatchable again"
    );
}

#[tokio::test]
async fn dispatch_task_unknown_task_id_returns_error() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "task_dispatch",
            "arguments": { "task_id": 9999 }
        })),
    )
    .await;

    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_task_respects_tag_routing() {
    let fx = CloseFixture::with_runner(dispatch_runner_script()).await;
    // Feature-tagged task with no plan → still routes to the standard dispatch
    // agent (only Research + no plan routes elsewhere).
    let task_id = fx.backlog_subtask(None, "Feature Task", None).await;
    fx.db
        .patch_task(
            task_id,
            &store::TaskPatch::new()
                .plan_path(None)
                .tag(Some(crate::models::TaskTag::Feature)),
        )
        .await
        .unwrap();

    let resp = call_dispatch_task(&fx.state, task_id).await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("dispatched"),
        "Expected dispatch confirmation, got: {text}"
    );
    let task = fx.db.get_task(task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

#[tokio::test]
async fn dispatch_task_dependabot_tag_routes_through_dispatch_agent() {
    // Dependabot tag is a label now — it routes through the unified dispatch
    // agent like any other task without a dedicated dispatch mode.
    let fx = CloseFixture::with_runner(dispatch_runner_script()).await;
    let title = "Bump foo from 1.0.0 to 1.0.1";
    let task_id = fx.backlog_subtask(None, title, None).await;
    fx.db
        .patch_task(
            task_id,
            &store::TaskPatch::new().tag(Some(crate::models::TaskTag::Dependabot)),
        )
        .await
        .unwrap();
    let worktree_dir = std::path::Path::new(&fx.repo_path)
        .join(".worktrees")
        .join(format!("{}-{}", task_id.0, crate::models::slugify(title)));

    let resp = call_dispatch_task(&fx.state, task_id).await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("dispatched"),
        "Expected dispatch confirmation, got: {text}"
    );

    // Should have written the unified prompt with a Dependabot-specific
    // section gated on the tag — not the deleted Dependabot triage agent.
    let prompt = std::fs::read_to_string(worktree_dir.join(".claude-prompt"))
        .expect("dispatch agent should have written a prompt file");
    assert!(
        prompt.contains("Your task is:"),
        "expected the unified dispatch prompt, got:\n{prompt}"
    );
    assert!(
        !prompt.contains("Dependabot triage agent"),
        "Dependabot tag must no longer route to a specialised agent"
    );
    assert!(
        prompt.contains("Dependabot PR review"),
        "Dependabot tag must inject the dependabot review section, got:\n{prompt}"
    );
    assert!(
        prompt.contains("gh pr view") && prompt.contains("gh pr merge"),
        "Dependabot section must include gh PR commands, got:\n{prompt}"
    );
    // The runbook no longer forbids /wrap-up: `wrap_up` refuses a review-tagged
    // task itself (`ReviewTasksAreNotWrappedUp` in mcp-task-tools.allium), so
    // what the opening line states is the role, not a prohibition. The count is
    // pinned by `review_runbooks_no_longer_forbid_wrap_up`.
    assert!(
        prompt.contains("not a code-edit task: do not edit files or write a plan"),
        "Dependabot section must state the agent's role, got:\n{prompt}"
    );
    // The bump is classified before the prompt is rendered, so the agent is
    // told which one this is rather than asked to parse it.
    assert!(
        prompt.contains("Bump: patch — foo 1.0.0 → 1.0.1"),
        "Dependabot section must carry the classified bump, got:\n{prompt}"
    );
}

#[tokio::test]
async fn dispatch_task_returns_error_when_dispatch_fails() {
    // `tmux new-window` fails → dispatch errors out.
    //
    // This used to queue a single failure and call it "the new-window call", but
    // `backlog_subtask` pre-creates the worktree and sets base_branch, so the
    // first call is `git fetch`: the lone response was consumed there and
    // `new-window` then found an empty queue. `do_dispatch` runs under
    // `spawn_blocking`, so the mock's panic surfaced as a `JoinError` — the very
    // error this test asserts on. It passed on the mock breaking, not on
    // dispatch failing.
    let fx = CloseFixture::with_runner(
        crate::dispatch::mock_sequence::DispatchScript::dispatch()
            .fails_at(crate::dispatch::mock_sequence::Step::NewWindow)
            .shared_runner(),
    )
    .await;
    let task_id = fx.backlog_subtask(None, "Backlog Task", None).await;

    let resp = call_dispatch_task(&fx.state, task_id).await;

    assert!(is_error(&resp), "expected error when dispatch fails");

    // Task status must remain Backlog — dispatch failure must not leave it as Running
    let task = fx.db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Backlog,
        "task should remain Backlog after dispatch failure"
    );
}
