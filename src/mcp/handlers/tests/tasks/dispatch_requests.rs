//! `task_dispatch` and `task_search` on request rows
//! (`store-split.allium: DispatchRequest`).
//!
//! `mcp-task-tools.allium: DispatchTaskViaMcp` writes a request (origin
//! `mcp`, no mode) and then WAITS on the row for at most
//! `config.dispatch_task_wait_cap` (30s), answering by the row's state:
//! dispatched (the worktree and window), failed (an error carrying the row's
//! reason), or still open when the cap passes (NOT an error: the request id
//! and its state, pointing at `task_search`). `GetTaskViaMcp` shows that request
//! as its `[Dispatch request]` line.
//!
//! No MCP path provisions: the runner here panics on any call. The tests drive
//! the row the way this board's host reactor would — claim, then report —
//! through the store, which is the in-memory double underneath.

use super::*;

use std::time::Duration;

use crate::models::{DispatchMode, DispatchOrigin, DispatchRequestState, TaskId, TmuxWindow};
use crate::store::RequestDispatchAnswer;

/// Bound on awaiting a completion signal — a deadline on a signal, never a
/// wall-clock wait for work to finish.
const SIGNAL_TIMEOUT: Duration = Duration::from_secs(5);

/// An `McpState` whose `task_dispatch` waits at most `cap`, over an in-memory
/// store, with a runner that panics on any subprocess.
fn state_with_wait_cap(cap: Duration) -> (Arc<McpState>, Arc<dyn store::TaskStore>) {
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::open_in_memory().unwrap());
    let mut state = McpState::new(
        McpDeps {
            db: db.clone(),
            runner: Arc::new(MockProcessRunner::new(vec![])),
            embedding_service: EmbeddingService::new_test(),
            data_dir: std::env::temp_dir(),
            key_table: crate::keybindings::KeyTableLoad::defaults(&std::env::temp_dir()),
        },
        None,
    );
    state.dispatch_task_wait_cap = cap;
    (Arc::new(state), db)
}

/// The task's request, once the handler running beside this test has written
/// it. Yields to that handler until the row appears.
async fn request_written_for(
    db: &Arc<dyn store::TaskStore>,
    task: TaskId,
) -> crate::models::DispatchRequest {
    tokio::time::timeout(SIGNAL_TIMEOUT, async {
        loop {
            if let Some(row) = db.latest_dispatch_request_for_task(task).await {
                return row;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("task_dispatch must write a request")
}

fn dispatch_task_call(
    state: &Arc<McpState>,
    task: TaskId,
) -> tokio::task::JoinHandle<JsonRpcResponse> {
    let state = state.clone();
    tokio::spawn(async move {
        call(
            &state,
            "tools/call",
            Some(json!({
                "name": "task_dispatch",
                "arguments": { "task_id": task.0 }
            })),
        )
        .await
    })
}

async fn answer(handle: tokio::task::JoinHandle<JsonRpcResponse>) -> JsonRpcResponse {
    tokio::time::timeout(SIGNAL_TIMEOUT, handle)
        .await
        .expect("task_dispatch must answer once the row finishes")
        .unwrap()
}

/// `config.dispatch_task_wait_cap`.
#[test]
fn dispatch_task_waits_at_most_thirty_seconds() {
    assert_eq!(crate::mcp::DISPATCH_TASK_WAIT_CAP, Duration::from_secs(30));
}

/// The request names this board's host and origin `mcp`, with no mode (the
/// module derives it — here, research for a plan-less research task).
#[tokio::test]
async fn dispatch_task_requests_with_origin_mcp_and_lets_the_module_pick_the_mode() {
    let (state, db) = state_with_wait_cap(Duration::ZERO);
    let task = create_task_fixture(&state).await;
    db.patch_task(
        task,
        &store::TaskPatch::new().tag(Some(crate::models::TaskTag::Research)),
    )
    .await
    .unwrap();

    let _ = call(
        &state,
        "tools/call",
        Some(json!({"name": "task_dispatch", "arguments": {"task_id": task.0}})),
    )
    .await;

    let row = db
        .latest_dispatch_request_for_task(task)
        .await
        .expect("task_dispatch writes a request");
    assert_eq!(row.origin, DispatchOrigin::Mcp);
    assert_eq!(row.mode, DispatchMode::Research);
    assert_eq!(row.host, crate::store::MEMORY_HOST_ID);
}

/// Dispatched: today's success shape — the worktree path and the tmux window.
#[tokio::test]
async fn dispatch_task_answers_with_the_worktree_once_the_row_is_dispatched() {
    let (state, db) = state_with_wait_cap(crate::mcp::DISPATCH_TASK_WAIT_CAP);
    let task = create_task_fixture(&state).await;

    let handle = dispatch_task_call(&state, task);
    let row = request_written_for(&db, task).await;
    assert!(db.claim_dispatch_request(row.id).await.unwrap());
    assert!(db
        .report_dispatch_provisioned(
            row.id,
            "/repo/.worktrees/1-test-task",
            &TmuxWindow::for_task(task),
            false,
        )
        .await
        .unwrap());
    let resp = answer(handle).await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("dispatched {}", task_ref_of(&state, task).await)),
        "{text}"
    );
    assert!(text.contains("/repo/.worktrees/1-test-task"), "{text}");
    assert!(text.contains(&format!("task-{}", task.0)), "{text}");
}

/// Failed: an error carrying the row's reason. The module already released
/// the claim, so the task is back in backlog; the call releases nothing.
#[tokio::test]
async fn dispatch_task_answers_with_the_rows_reason_when_it_fails() {
    let (state, db) = state_with_wait_cap(crate::mcp::DISPATCH_TASK_WAIT_CAP);
    let task = create_task_fixture(&state).await;

    let handle = dispatch_task_call(&state, task);
    let row = request_written_for(&db, task).await;
    assert!(db.claim_dispatch_request(row.id).await.unwrap());
    assert!(db
        .report_dispatch_failed(row.id, "deadline passed")
        .await
        .unwrap());
    let resp = answer(handle).await;

    // A tool failure is an `isError` result, not a protocol error.
    assert_error(&resp, "deadline passed");
    let after = state.task_svc.get_task(task).await.unwrap();
    assert_eq!(after.status, TaskStatus::Backlog);
}

/// Still open when the cap passes: NOT an error. The answer gives the request
/// id and its state and points at task_search; nothing is cancelled.
#[tokio::test]
async fn dispatch_task_answers_with_the_pending_request_when_the_wait_ends_first() {
    let (state, db) = state_with_wait_cap(Duration::ZERO);
    let task = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "task_dispatch", "arguments": {"task_id": task.0}})),
    )
    .await;

    assert!(
        resp.error.is_none(),
        "a pending request is not an error: {:?}",
        resp.error
    );
    let text = extract_response_text(&resp);
    let row = db.latest_dispatch_request_for_task(task).await.unwrap();
    assert_eq!(
        row.state,
        DispatchRequestState::Requested,
        "nothing was cancelled"
    );
    assert!(
        text.contains(&row.id.0.to_string()),
        "the request id: {text}"
    );
    assert!(text.contains("requested"), "its state: {text}");
    assert!(
        text.contains("task_search"),
        "where to learn how it ends: {text}"
    );
}

/// A refused request keeps today's "not in backlog" error, naming the status
/// the task holds now.
#[tokio::test]
async fn dispatch_task_reports_a_refused_request_as_not_in_backlog() {
    let (state, db) = state_with_wait_cap(Duration::ZERO);
    let task = create_task_fixture(&state).await;
    assert!(matches!(
        db.request_dispatch(task, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap(),
        RequestDispatchAnswer::Requested(_)
    ));

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "task_dispatch", "arguments": {"task_id": task.0}})),
    )
    .await;

    assert_error(
        &resp,
        &format!(
            "{} is not in backlog (current: running)",
            task_ref_of(&state, task).await
        ),
    );
}

/// A task whose worktree is on another machine is refused with that reason.
#[tokio::test]
async fn dispatch_task_reports_a_foreign_owned_task_with_the_modules_reason() {
    let (state, db) = state_with_wait_cap(Duration::ZERO);
    let task = create_task_fixture(&state).await;
    db.patch_task(
        task,
        &store::TaskPatch::new()
            .worktree(Some("/elsewhere/.worktrees/1-test-task"))
            .host(Some("another-machine")),
    )
    .await
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "task_dispatch", "arguments": {"task_id": task.0}})),
    )
    .await;

    assert_error(&resp, "another-machine");
    assert!(db.latest_dispatch_request_for_task(task).await.is_none());
}

// -- GetTaskViaMcp's [Dispatch request] line ------------------------------------

/// The task's most recent request: its id, its state and, when failed, its
/// reason — between the tmux window and the sort order, before Created.
#[tokio::test]
async fn get_task_shows_the_dispatch_request_line() {
    let (state, db) = state_with_wait_cap(Duration::ZERO);
    let task = create_task_fixture(&state).await;
    let RequestDispatchAnswer::Requested(id) = db
        .request_dispatch(task, DispatchOrigin::Mcp, None)
        .await
        .unwrap()
    else {
        panic!("the request must apply");
    };
    assert!(db.claim_dispatch_request(id).await.unwrap());
    assert!(db.report_dispatch_failed(id, "timed out").await.unwrap());

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "task_search", "arguments": {"task_id": task.0}})),
    )
    .await;
    let text = extract_response_text(&resp);

    let line = text
        .lines()
        .find(|l| l.starts_with("Dispatch request:"))
        .unwrap_or_else(|| panic!("no [Dispatch request] line in:\n{text}"));
    assert!(line.contains(&id.0.to_string()), "{line}");
    assert!(line.contains("failed"), "{line}");
    assert!(line.contains("timed out"), "{line}");
    let at = |prefix: &str| text.find(prefix).unwrap_or(usize::MAX);
    assert!(
        at("Base branch:") < at("Dispatch request:") && at("Dispatch request:") < at("Created:"),
        "ordered after the tmux window / base branch and before Created:\n{text}"
    );
}

/// An open request shows its state and no reason.
#[tokio::test]
async fn get_task_shows_an_open_request_without_a_reason() {
    let (state, db) = state_with_wait_cap(Duration::ZERO);
    let task = create_task_fixture(&state).await;
    db.request_dispatch(task, DispatchOrigin::Mcp, None)
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "task_search", "arguments": {"task_id": task.0}})),
    )
    .await;
    let text = extract_response_text(&resp);

    let line = text
        .lines()
        .find(|l| l.starts_with("Dispatch request:"))
        .unwrap_or_else(|| panic!("no [Dispatch request] line in:\n{text}"));
    assert!(line.contains("requested"), "{line}");
}

/// Omitted when the task has no request.
#[tokio::test]
async fn get_task_omits_the_line_when_there_is_no_request() {
    let (state, _db) = state_with_wait_cap(Duration::ZERO);
    let task = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "task_search", "arguments": {"task_id": task.0}})),
    )
    .await;

    assert!(!extract_response_text(&resp).contains("Dispatch request"));
}
