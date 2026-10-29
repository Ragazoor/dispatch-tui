use super::*;
use crate::models::test_tmux_window;

use crate::dispatch::mock_sequence::DispatchScript;
use crate::mcp::handlers::tasks::WrapUpAction;

async fn wait_for_task_changed(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::board_event::BoardEvent>,
    expected_id: crate::models::TaskId,
) {
    loop {
        match rx.recv().await {
            Some(crate::board_event::BoardEvent::TaskChanged(id)) if id == expected_id => break,
            Some(_) => continue,
            None => panic!("notification channel closed before dispatch completed"),
        }
    }
}

// -- automatic epic chaining on exit_session ---------------------------------
//
// Epic chaining is a server-side side effect of closing a session, not a tool
// an agent calls: `AutoDispatchNextSubtask` in docs/specs/epics.allium is keyed
// on the `SessionClosed(task)` event that `ExitSessionViaMcp` emits last. The
// old `dispatch_next` MCP tool is gone, so every test below drives the real
// close path instead.

/// Wiring shared by the chaining tests: a temp repo, an in-memory DB, an
/// `McpState` with a notification channel, and a runner with a generous queue
/// of successes covering the closing task's detached `tmux kill-window` plus a
/// full worktree provisioning for the chained subtask.
struct ChainFixture {
    _dir: tempfile::TempDir,
    repo_path: String,
    db: Arc<dyn store::TaskStore>,
    state: Arc<McpState>,
    notify_rx: tokio::sync::mpsc::UnboundedReceiver<crate::board_event::BoardEvent>,
    /// The same runner the state holds, kept concrete so a test can inspect
    /// which commands the close path actually issued.
    runner: Arc<MockProcessRunner>,
    /// The host reactor serving this fixture's dispatch requests
    /// ([`spawn_host`]). Aborted with the fixture.
    host: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for ChainFixture {
    fn drop(&mut self) {
        if let Some(host) = self.host.take() {
            host.abort();
        }
    }
}

impl ChainFixture {
    async fn new() -> Self {
        Self::build(None).await
    }

    /// Like [`ChainFixture::new`], but with a caller-supplied runner script, for
    /// tests that assert on the exact command sequence a dispatch issues (or
    /// need one to fail).
    async fn with_runner(runner: Arc<MockProcessRunner>) -> Self {
        Self::build_with(runner, None).await
    }

    /// Like [`ChainFixture::new`], but with a `task_svc` whose `close_task`
    /// always fails, so `update_tasks(status="done")`'s close does not take
    /// effect (`MarkTaskDoneViaMcp`).
    async fn with_failing_close() -> Self {
        let svc = Arc::new(FailingCloseTaskService::default());
        let fx = Self::build(Some(svc.clone())).await;
        let _ = svc.db.set(fx.db.clone());
        fx
    }

    /// Like [`ChainFixture::new`], but with a `task_svc` whose
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
        // The kill-window teardown and the chained dispatch race by design, and
        // MockProcessRunner pops a shared FIFO, so queue uniform successes
        // rather than a command-ordered script.
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
            notify_rx,
            runner,
            host,
        }
    }

    /// Wait for `task`'s dispatch request to finish — dispatched or failed —
    /// and return it. A deadline on a signal, not a wall-clock wait: it wakes
    /// on every row change and returns the instant the row finishes.
    async fn wait_for_dispatch(
        &self,
        task: crate::models::TaskId,
    ) -> crate::models::DispatchRequest {
        let mut changes = self.db.dispatch_request_changes();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(row) = self.db.latest_dispatch_request_for_task(task).await {
                    if !row.is_open() {
                        return row;
                    }
                }
                changes.changed().await.expect("the rows outlive the test");
            }
        })
        .await
        .expect("the request must finish")
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

    async fn epic(&self, auto_dispatch: bool) -> crate::models::EpicId {
        let epic = self
            .db
            .create_epic("Chained Epic", "desc", None)
            .await
            .unwrap();
        self.db
            .patch_epic(
                epic.id,
                &store::EpicPatch::new().auto_dispatch(auto_dispatch),
            )
            .await
            .unwrap();
        epic.id
    }

    /// A backlog subtask that a chain can pick up. `repo_path` defaults to the
    /// fixture's temp repo; pass an override to make provisioning fail.
    async fn backlog_subtask(
        &self,
        epic_id: Option<crate::models::EpicId>,
        title: &str,
        sort_order: Option<i64>,
        repo_path: Option<&str>,
    ) -> crate::models::TaskId {
        let id = self
            .db
            .create_task(CreateTaskRequest {
                plan: Some("docs/plan.md"),
                epic_id,
                sort_order,
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
                "name": "update_tasks",
                "arguments": { "task_ids": [task_id.0], "status": "done" }
            })),
        )
        .await
    }
}

// -- update_task(status="done") closes but never chains ----------------------
//
// MarkTaskDoneViaMcp (mcp-task-tools.allium) closes in the module since M7
// (`close_task`, store-split.allium: RequestCloseTask), with no token and no
// chain; the window is torn down by the host serving the `close` row. The chain
// is the module's, inside `request_exit_session`'s transaction, and only a
// close with a wrap_up behind it can say the closed work reached base_branch.
// These are the teardown checks plus that one divergence.

/// update_task(status="done") leaves the epic's next backlog subtask in
/// backlog — the divergence from `exit_session_dispatches_first_backlog_subtask`.
///
/// The assertion is sound without waiting for anything: the module's chain
/// claims its subtask in `request_exit_session`'s own transaction, and this
/// path never calls it, so a chain that was going to fire would have moved the
/// row to Running by the time the close responds.
#[tokio::test]
async fn update_task_done_does_not_chain_the_next_subtask() {
    let fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let first = fx
        .backlog_subtask(Some(epic_id), "Task 1", Some(10), None)
        .await;

    let resp = fx.close_via_update_task(closing).await;
    assert!(resp.error.is_none(), "close must succeed: {:?}", resp.error);

    let next = fx.db.get_task(first).await.unwrap();
    assert_eq!(
        next.status,
        TaskStatus::Backlog,
        "update_task(status=\"done\") must not dispatch the next subtask"
    );
    assert!(next.worktree.is_none());
    assert!(next.tmux_window.is_none());

    // The closing task itself still closed.
    assert_eq!(
        fx.db.get_task(closing).await.unwrap().status,
        TaskStatus::Done
    );
}

/// The response never names a chained subtask, because no chain ever runs —
/// contrast `exit_session_response_names_the_chained_subtask`.
#[tokio::test]
async fn update_task_done_response_never_names_a_chained_subtask() {
    let fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    fx.backlog_subtask(Some(epic_id), "Wire up the widget", Some(10), None)
        .await;

    let resp = fx.close_via_update_task(closing).await;
    let text = extract_response_text(&resp);
    assert_eq!(text, format!("Task #{} marked done.", closing.0));
}

/// Closing a task with a live tmux window via update_task(status="done")
/// tears the window down, the same as exit_session does — parity with
/// `exit_session_successful_close_kills_the_tmux_window`. The module writes a
/// `close` row naming the window and this host's reactor serves it; the test
/// awaits that row finishing on the subscription's change signal rather than
/// sleeping — see the "No `tokio::time::sleep` in tests" section of
/// `docs/conventions.md`.
#[tokio::test]
async fn update_task_done_kills_the_tmux_window() {
    let fx = ChainFixture::new().await;
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
/// no status change, no cleared window, no chain — and say so in the response.
/// (The module's close is whole or absent, as `exit_session`'s is:
/// `a_refused_exit_session_changes_nothing_kills_nothing_and_does_not_chain`.)
#[tokio::test]
async fn update_task_done_failed_close_leaves_the_task_unchanged() {
    let fx = ChainFixture::with_failing_close().await;
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

// allow-phantom-symbol: renamed test, cited for provenance
/// Migrated from `dispatch_next_picks_first_backlog_subtask`: closing a subtask
/// dispatches the epic's first backlog subtask and leaves the rest alone.
#[tokio::test]
async fn exit_session_dispatches_first_backlog_subtask() {
    let mut fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let first = fx
        .backlog_subtask(Some(epic_id), "Task 1", Some(10), None)
        .await;
    let second = fx
        .backlog_subtask(Some(epic_id), "Task 2", Some(20), None)
        .await;

    let resp = fx.close(closing, WrapUpAction::Done).await;
    assert!(resp.error.is_none(), "close must succeed: {:?}", resp.error);

    wait_for_task_changed(&mut fx.notify_rx, first).await;
    fx.wait_for_dispatch(first).await;

    let dispatched = fx.db.get_task(first).await.unwrap();
    assert_eq!(dispatched.status, TaskStatus::Running);
    assert!(dispatched.worktree.is_some());
    assert!(dispatched.tmux_window.is_some());
    assert!(
        dispatched.last_pre_tool_use_at.is_some(),
        "last_pre_tool_use_at should be seeded so the tick classifier does not flicker the task to Stale"
    );

    let untouched = fx.db.get_task(second).await.unwrap();
    assert_eq!(
        untouched.status,
        TaskStatus::Backlog,
        "only one subtask may be chained per closed session"
    );
    assert!(untouched.worktree.is_none());
}

/// The chained subtask is reported in the response text so the closing agent
/// can see what it handed off to.
#[tokio::test]
async fn exit_session_response_names_the_chained_subtask() {
    let mut fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let next = fx
        .backlog_subtask(Some(epic_id), "Wire up the widget", Some(10), None)
        .await;

    let resp = fx.close(closing, WrapUpAction::Done).await;
    let text = extract_response_text(&resp);
    assert_eq!(
        text,
        format!(
            "Session closed. Dispatching next epic subtask #{} 'Wire up the widget'.",
            next.0
        ),
    );

    wait_for_task_changed(&mut fx.notify_rx, next).await;
}

// allow-phantom-symbol: renamed test, cited for provenance
/// Migrated from `dispatch_next_respects_sort_order`: selection is by
/// `sort_order` ascending, not creation order.
#[tokio::test]
async fn exit_session_chain_respects_sort_order() {
    let mut fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    // Created first, but sorts last.
    let later = fx
        .backlog_subtask(Some(epic_id), "Task A", Some(20), None)
        .await;
    let earlier = fx
        .backlog_subtask(Some(epic_id), "Task B", Some(10), None)
        .await;

    let resp = fx.close(closing, WrapUpAction::Done).await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("#{}", earlier.0)),
        "expected the lower sort_order subtask to be chained, got: {text}"
    );

    wait_for_task_changed(&mut fx.notify_rx, earlier).await;
    assert_eq!(
        fx.db.get_task(later).await.unwrap().status,
        TaskStatus::Backlog
    );
}

// allow-phantom-symbol: renamed test, cited for provenance
/// Migrated from `dispatch_next_respects_tag_routing`: the chained dispatch
/// still routes through `DispatchMode::for_task`.
#[tokio::test]
async fn exit_session_chain_respects_tag_routing() {
    let mut fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let next = fx
        .backlog_subtask(Some(epic_id), "Research Task", Some(10), None)
        .await;
    // Research + no plan routes to the research agent rather than the standard
    // dispatch agent.
    fx.db
        .patch_task(
            next,
            &store::TaskPatch::new()
                .plan_path(None)
                .tag(Some(crate::models::TaskTag::Research)),
        )
        .await
        .unwrap();
    let task = fx.db.get_task(next).await.unwrap();
    assert_eq!(
        crate::models::DispatchMode::for_task(&task),
        crate::models::DispatchMode::Research,
        "fixture must exercise the research routing branch"
    );

    let resp = fx.close(closing, WrapUpAction::Done).await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    wait_for_task_changed(&mut fx.notify_rx, next).await;
    assert_eq!(
        fx.db.get_task(next).await.unwrap().status,
        TaskStatus::Running
    );
}

// allow-phantom-symbol: renamed test, cited for provenance
/// Migrated from `dispatch_next_no_backlog_returns_success_noop`: closing the
/// epic's last subtask closes cleanly and chains nothing.
#[tokio::test]
async fn exit_session_with_no_backlog_subtask_closes_without_chaining() {
    let fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;

    let resp = fx.close(closing, WrapUpAction::Done).await;
    assert_eq!(extract_response_text(&resp), "Session closed.");
    assert_eq!(
        fx.db.get_task(closing).await.unwrap().status,
        TaskStatus::Done
    );
}

// The missing-epic case of the old board-side chain (`dispatch_next_epic_not_found_returns_error`,
// then `auto_dispatch_next`'s warn-and-skip) has no counterpart any more: the
// chain runs in the module's transaction against the epic it reads there, and a
// dangling epic reference is not a reachable state.

// allow-phantom-symbol: renamed test, cited for provenance
/// Migrated from `dispatch_next_returns_disabled_when_auto_dispatch_off`
/// (previously in tests/tasks/crud.rs): `auto_dispatch = false` closes the
/// session and chains nothing.
#[tokio::test]
async fn exit_session_does_not_chain_when_auto_dispatch_off() {
    let fx = ChainFixture::new().await;
    let epic_id = fx.epic(false).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let next = fx
        .backlog_subtask(Some(epic_id), "Task 1", Some(10), None)
        .await;

    let resp = fx.close(closing, WrapUpAction::Done).await;
    assert_eq!(extract_response_text(&resp), "Session closed.");

    // The claim is synchronous and completes before exit_session returns, so a
    // subtask still in Backlog here was never claimed and can never transition.
    let untouched = fx.db.get_task(next).await.unwrap();
    assert_eq!(untouched.status, TaskStatus::Backlog);
    assert!(untouched.worktree.is_none());
}

/// A task with no epic closes cleanly and chains nothing.
#[tokio::test]
async fn exit_session_without_epic_closes_without_chaining() {
    let fx = ChainFixture::new().await;
    let closing = fx.closing_subtask(None).await;

    let resp = fx.close(closing, WrapUpAction::Done).await;
    assert_eq!(extract_response_text(&resp), "Session closed.");
    assert_eq!(
        fx.db.get_task(closing).await.unwrap().status,
        TaskStatus::Done
    );
}

/// The regression guard for this change: by the time the next subtask is
/// running with a worktree, the closed subtask is already terminal with its
/// tmux window cleared. The old skill-driven ordering violated exactly this.
#[tokio::test]
async fn exit_session_chain_starts_only_after_the_closing_task_is_terminal() {
    let mut fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let next = fx
        .backlog_subtask(Some(epic_id), "Successor", Some(10), None)
        .await;

    fx.close(closing, WrapUpAction::Done).await;
    wait_for_task_changed(&mut fx.notify_rx, next).await;
    fx.wait_for_dispatch(next).await;

    let successor = fx.db.get_task(next).await.unwrap();
    assert_eq!(successor.status, TaskStatus::Running);
    assert!(
        successor.worktree.is_some(),
        "successor should be provisioned once its request is dispatched"
    );

    let predecessor = fx.db.get_task(closing).await.unwrap();
    assert_eq!(
        predecessor.status,
        TaskStatus::Done,
        "the closed subtask must already be terminal before its successor is provisioned"
    );
    assert!(
        predecessor.tmux_window.is_none(),
        "the closed subtask's window must already be cleared"
    );
}

/// All three wrap-up actions chain, matching the behaviour the skill had when
/// it fired `dispatch_next` regardless of action.
#[tokio::test]
async fn exit_session_chains_for_rebase_action() {
    assert_action_chains(WrapUpAction::Rebase).await;
}

#[tokio::test]
async fn exit_session_chains_for_done_action() {
    assert_action_chains(WrapUpAction::Done).await;
}

#[tokio::test]
async fn exit_session_chains_for_pr_action() {
    assert_action_chains(WrapUpAction::Pr).await;
}

async fn assert_action_chains(action: WrapUpAction) {
    let mut fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let next = fx
        .backlog_subtask(Some(epic_id), "Successor", Some(10), None)
        .await;

    let resp = fx.close(closing, action).await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("#{}", next.0)),
        "action {:?} must chain, got: {text}",
        action
    );

    wait_for_task_changed(&mut fx.notify_rx, next).await;
    assert_eq!(
        fx.db.get_task(next).await.unwrap().status,
        TaskStatus::Running
    );
}

/// A failed dispatch reverts the claim, leaving the subtask dispatchable
/// exactly as it was before the chain fired. Since store-split M4 the module
/// reverts it when the request fails (`ReleaseClaimOfFailedRequest`).
#[tokio::test]
async fn exit_session_chain_reverts_claim_when_dispatch_fails() {
    let mut fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    // A repo path that does not exist fails provisioning before any subprocess
    // runs, so the failure is deterministic.
    let next = fx
        .backlog_subtask(
            Some(epic_id),
            "Doomed",
            Some(10),
            Some("/nonexistent/dispatch-chain-repo"),
        )
        .await;

    let resp = fx.close(closing, WrapUpAction::Done).await;
    assert!(
        resp.error.is_none(),
        "a failed chain must not fail the close: {:?}",
        resp.error
    );

    wait_for_task_changed(&mut fx.notify_rx, next).await;
    let request = fx.wait_for_dispatch(next).await;
    assert_eq!(request.state, crate::models::DispatchRequestState::Failed);

    let reverted = fx.db.get_task(next).await.unwrap();
    assert_eq!(
        reverted.status,
        TaskStatus::Backlog,
        "a failed dispatch must return the subtask to backlog"
    );
    assert_eq!(
        reverted.sub_status,
        SubStatus::default_for(TaskStatus::Backlog)
    );
    assert!(reverted.worktree.is_none());
    assert!(
        reverted.last_pre_tool_use_at.is_none(),
        "the revert must also drop the activity timestamp the claim seeded, or the \
         subtask is not left exactly as it was before the chain fired"
    );
}

/// A failed chained dispatch must be visible to the operator, not only to
/// `app.log`. Since store-split M4 the chain's request carries origin
/// `epic_chain`, and the board raises AutoDispatchFailed from that row when it
/// fails (`epics.allium: ReportChainedDispatchFailure`, pinned on the board in
/// `src/tui/tests/dispatch_requests.rs`). What the chain owes is a row that
/// says so: its origin, its failure and why.
#[tokio::test]
async fn exit_session_chain_leaves_a_failed_epic_chain_row_for_the_board() {
    let fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let next = fx
        .backlog_subtask(
            Some(epic_id),
            "Doomed",
            Some(10),
            Some("/nonexistent/dispatch-chain-repo"),
        )
        .await;

    fx.close(closing, WrapUpAction::Done).await;
    let request = fx.wait_for_dispatch(next).await;

    assert_eq!(request.origin, crate::models::DispatchOrigin::EpicChain);
    assert_eq!(request.state, crate::models::DispatchRequestState::Failed);
    assert!(
        request.reason.as_deref().is_some_and(|r| !r.is_empty()),
        "the row must carry why the dispatch failed"
    );
}

/// A chained dispatch that SUCCEEDS leaves a dispatched row, which raises
/// nothing on the board. Guards against every chained subtask reading as
/// stalled.
#[tokio::test]
async fn exit_session_chain_reports_no_failure_when_dispatch_succeeds() {
    let fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let next = fx
        .backlog_subtask(Some(epic_id), "Successor", Some(10), None)
        .await;

    fx.close(closing, WrapUpAction::Done).await;
    let request = fx.wait_for_dispatch(next).await;

    assert_eq!(request.origin, crate::models::DispatchOrigin::EpicChain);
    assert_eq!(
        request.state,
        crate::models::DispatchRequestState::Dispatched,
        "a successful chained dispatch must not fail its row: {:?}",
        request.reason
    );
}

// -- a close that does not take effect ----------------------------------------
//
// Since store-split M5 `exit_session`'s close is the module's, in ONE
// transaction with the token, the epic rollup and the chain
// (`pr-workflow.allium: ExitSession`), and since M7 so is `update_tasks`
// status="done" (`close_task`): either took effect whole or not at all. The
// board no longer gates a teardown on a close; the one test below drives a
// close call that gets no effect.

/// A task service whose `close_task` always fails, so `update_tasks`' close
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
/// window is killed, no row is written and the chain does not fire. This
/// replaces the old board-side "terminal write did not land but the token is
/// spent" cases, which can no longer happen.
#[tokio::test]
async fn a_refused_exit_session_changes_nothing_kills_nothing_and_does_not_chain() {
    let fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let next = fx
        .backlog_subtask(Some(epic_id), "Successor", Some(10), None)
        .await;
    let token = seed_exit_token(&fx.state, closing, WrapUpAction::Done).await;
    let before = fx.db.get_task(closing).await.unwrap();
    let rows_before = fx.db.session_requests().await.len();

    // The action differs from the one the token was issued for.
    let resp = call(
        &fx.state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
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
    assert_eq!(
        fx.db.get_task(next).await.unwrap().status,
        TaskStatus::Backlog,
        "a refused close must not chain"
    );

    // The token is still valid: the retry with the right action closes.
    let retry = call(
        &fx.state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
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
    let fx = ChainFixture::new().await;
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

/// The full agent-facing sequence — `wrap_up(action="done")` then
/// `exit_session` with the issued token — chains without the agent asking for
/// it. There is no `dispatch_next` tool to call any more.
#[tokio::test]
async fn wrap_up_then_exit_session_chains_without_an_agent_tool_call() {
    let mut fx = ChainFixture::new().await;
    let epic_id = fx.epic(true).await;
    let closing = fx.closing_subtask(Some(epic_id)).await;
    let next = fx
        .backlog_subtask(Some(epic_id), "Successor", Some(10), None)
        .await;

    let wrap = call(
        &fx.state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": closing.0, "action": "done" }
        })),
    )
    .await;
    assert!(wrap.error.is_none(), "wrap_up failed: {:?}", wrap.error);
    let token = token_in_response(&extract_response_text(&wrap));

    let resp = call(
        &fx.state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": closing.0, "token": token, "action": "done" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("#{}", next.0)),
        "the close itself must chain, got: {text}"
    );

    wait_for_task_changed(&mut fx.notify_rx, next).await;
    assert_eq!(
        fx.db.get_task(next).await.unwrap().status,
        TaskStatus::Running
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
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("wrap_up complete"));
    assert!(
        text.contains("exit_session"),
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
            "name": "wrap_up",
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
            "name": "wrap_up",
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
// dispatch_task tests
// ---------------------------------------------------------------------------

/// The exact command sequence one successful dispatch issues. Scripted rather
/// than uniformly-ok because the order is itself the assertion, and because the
/// split-window call must return a pane id.
fn dispatch_runner_script() -> Arc<MockProcessRunner> {
    crate::dispatch::mock_sequence::DispatchScript::dispatch().shared_runner()
}

/// The `dispatch_task` tools/call for `task_id`.
async fn call_dispatch_task(
    state: &Arc<McpState>,
    task_id: crate::models::TaskId,
) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({
            "name": "dispatch_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await
}

#[tokio::test]
async fn dispatch_task_dispatches_backlog_task() {
    let fx = ChainFixture::with_runner(dispatch_runner_script()).await;
    let task_id = fx
        .backlog_subtask(None, "My Backlog Task", None, None)
        .await;

    let resp = call_dispatch_task(&fx.state, task_id).await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("dispatched"),
        "Expected 'dispatched' in response, got: {text}"
    );

    // dispatch_task is synchronous — no sleep needed
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
/// `ChainFixture::backlog_subtask` pre-creates the worktree directory, which is
/// what puts provisioning on the reuse branch.
#[tokio::test]
async fn dispatch_task_success_text_names_a_reused_worktree() {
    let fx = ChainFixture::with_runner(dispatch_runner_script()).await;
    let task_id = fx.backlog_subtask(None, "Crashed Task", None, None).await;

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
        crate::models::TaskId(7),
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
    let fx = ChainFixture::with_runner(dispatch_runner_script()).await;
    let task_id = fx.backlog_subtask(None, "Contended Task", None, None).await;
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
    let fx = ChainFixture::with_lost_claim().await;
    let task_id = fx.backlog_subtask(None, "Contended Task", None, None).await;

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
    let fx = ChainFixture::new().await;
    // A repo path that does not exist fails provisioning before any subprocess
    // runs, so the failure is deterministic — the same shape
    // `exit_session_chain_reverts_claim_when_dispatch_fails` uses.
    let task_id = fx
        .backlog_subtask(None, "Doomed", None, Some("/nonexistent/dispatch-mcp-repo"))
        .await;

    let resp = call_dispatch_task(&fx.state, task_id).await;

    assert_error(&resp, "dispatch failed");
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
            "name": "dispatch_task",
            "arguments": { "task_id": 9999 }
        })),
    )
    .await;

    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_task_respects_tag_routing() {
    let fx = ChainFixture::with_runner(dispatch_runner_script()).await;
    // Feature-tagged task with no plan → still routes to the standard dispatch
    // agent (only Research + no plan routes elsewhere).
    let task_id = fx.backlog_subtask(None, "Feature Task", None, None).await;
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
    let fx = ChainFixture::with_runner(dispatch_runner_script()).await;
    let title = "Bump foo from 1.0.0 to 1.0.1";
    let task_id = fx.backlog_subtask(None, title, None, None).await;
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
    let fx = ChainFixture::with_runner(
        crate::dispatch::mock_sequence::DispatchScript::dispatch()
            .fails_at(crate::dispatch::mock_sequence::Step::NewWindow)
            .shared_runner(),
    )
    .await;
    let task_id = fx.backlog_subtask(None, "Backlog Task", None, None).await;

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
