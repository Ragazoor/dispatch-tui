//! `dispatch_task`, `get_task` and the epic chain on request rows
//! (store-split M4, task #53257).
//!
//! `mcp-task-tools.allium: DispatchTaskViaMcp` writes a request (origin
//! `mcp`, no mode) and then WAITS on the row for at most
//! `config.dispatch_task_wait_cap` (30s), answering by the row's state:
//! dispatched (the worktree and window), failed (an error carrying the row's
//! reason), or still open when the cap passes (NOT an error: the request id
//! and its state, pointing at `get_task`). `GetTaskViaMcp` shows that request
//! as its `[Dispatch request]` line. `epics.allium: AutoDispatchNextSubtask`
//! requests its next subtask with origin `epic_chain`, no mode, and moves to
//! the next candidate on a refusal.
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

/// An `McpState` whose `dispatch_task` waits at most `cap`, over an in-memory
/// store, with a runner that panics on any subprocess.
fn state_with_wait_cap(cap: Duration) -> (Arc<McpState>, Arc<dyn store::TaskStore>) {
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::open_in_memory().unwrap());
    let mut state = McpState::new(
        McpDeps {
            db: db.clone(),
            runner: Arc::new(MockProcessRunner::new(vec![])),
            embedding_service: EmbeddingService::new_test(),
            data_dir: std::env::temp_dir(),
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
    .expect("dispatch_task must write a request")
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
                "name": "dispatch_task",
                "arguments": { "task_id": task.0 }
            })),
        )
        .await
    })
}

async fn answer(handle: tokio::task::JoinHandle<JsonRpcResponse>) -> JsonRpcResponse {
    tokio::time::timeout(SIGNAL_TIMEOUT, handle)
        .await
        .expect("dispatch_task must answer once the row finishes")
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
        Some(json!({"name": "dispatch_task", "arguments": {"task_id": task.0}})),
    )
    .await;

    let row = db
        .latest_dispatch_request_for_task(task)
        .await
        .expect("dispatch_task writes a request");
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
        )
        .await
        .unwrap());
    let resp = answer(handle).await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("dispatched task #{}", task.0)),
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

    let err = resp.error.expect("a failed request is an error");
    assert!(err.message.contains("deadline passed"), "{}", err.message);
    let after = state.task_svc.get_task(task).await.unwrap();
    assert_eq!(after.status, TaskStatus::Backlog);
}

/// Still open when the cap passes: NOT an error. The answer gives the request
/// id and its state and points at get_task; nothing is cancelled.
#[tokio::test]
async fn dispatch_task_answers_with_the_pending_request_when_the_wait_ends_first() {
    let (state, db) = state_with_wait_cap(Duration::ZERO);
    let task = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "dispatch_task", "arguments": {"task_id": task.0}})),
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
        text.contains("get_task"),
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
        Some(json!({"name": "dispatch_task", "arguments": {"task_id": task.0}})),
    )
    .await;

    let err = resp.error.expect("a refused request is an error");
    assert!(
        err.message.contains("not in backlog (current: running)"),
        "{}",
        err.message
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
        Some(json!({"name": "dispatch_task", "arguments": {"task_id": task.0}})),
    )
    .await;

    let err = resp.error.expect("a foreign-owned task is refused");
    assert!(err.message.contains("another-machine"), "{}", err.message);
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
        Some(json!({"name": "get_task", "arguments": {"task_id": task.0}})),
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
        Some(json!({"name": "get_task", "arguments": {"task_id": task.0}})),
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
        Some(json!({"name": "get_task", "arguments": {"task_id": task.0}})),
    )
    .await;

    assert!(!extract_response_text(&resp).contains("Dispatch request"));
}

// -- AutoDispatchNextSubtask ------------------------------------------------------

async fn auto_epic(db: &Arc<dyn store::TaskStore>) -> crate::models::EpicId {
    let epic = db.create_epic("Chained", "desc", None).await.unwrap();
    db.patch_epic(epic.id, &store::EpicPatch::new().auto_dispatch(true))
        .await
        .unwrap();
    epic.id
}

async fn subtask(
    db: &Arc<dyn store::TaskStore>,
    epic: crate::models::EpicId,
    title: &str,
    sort_order: i64,
    tag: Option<crate::models::TaskTag>,
) -> TaskId {
    db.create_task(CreateTaskRequest {
        epic_id: Some(epic),
        sort_order: Some(sort_order),
        tag,
        ..CreateTaskRequest::fixture(title, "/repo")
    })
    .await
    .unwrap()
}

/// The chain writes a request like every launch path: origin `epic_chain`, no
/// mode (a plan-less research subtask gets research from the module), naming
/// this board's host — and provisions nothing itself.
#[tokio::test]
async fn the_chain_requests_its_next_subtask_with_origin_epic_chain() {
    let (state, db) = state_with_wait_cap(Duration::ZERO);
    let epic = auto_epic(&db).await;
    let next = subtask(
        &db,
        epic,
        "Look into it",
        10,
        Some(crate::models::TaskTag::Research),
    )
    .await;

    let chosen = crate::mcp::handlers::tasks::dispatch::auto_dispatch_next(&state, epic).await;

    assert_eq!(chosen.map(|(id, _)| id), Some(next));
    let row = db
        .latest_dispatch_request_for_task(next)
        .await
        .expect("the chain writes a request");
    assert_eq!(row.origin, DispatchOrigin::EpicChain);
    assert_eq!(row.mode, DispatchMode::Research);
    assert_eq!(row.host, crate::store::MEMORY_HOST_ID);
    assert_eq!(row.state, DispatchRequestState::Requested);
}

/// A refused request is ORDINARY: the chain moves to the next candidate.
/// Here the first candidate's worktree is on another machine, so the module
/// refuses it although the board sees it in backlog.
#[tokio::test]
async fn the_chain_moves_to_the_next_candidate_on_a_refused_request() {
    let (state, db) = state_with_wait_cap(Duration::ZERO);
    let epic = auto_epic(&db).await;
    let foreign = subtask(&db, epic, "Owned elsewhere", 10, None).await;
    db.patch_task(
        foreign,
        &store::TaskPatch::new()
            .worktree(Some("/elsewhere/.worktrees/x"))
            .host(Some("another-machine")),
    )
    .await
    .unwrap();
    let second = subtask(&db, epic, "Mine", 20, None).await;

    let chosen = crate::mcp::handlers::tasks::dispatch::auto_dispatch_next(&state, epic).await;

    assert_eq!(chosen.map(|(id, _)| id), Some(second));
    assert!(db.latest_dispatch_request_for_task(foreign).await.is_none());
    assert_eq!(
        db.get_task(foreign).await.unwrap().status,
        TaskStatus::Backlog,
        "a refusal writes nothing"
    );
    assert_eq!(
        db.latest_dispatch_request_for_task(second)
            .await
            .unwrap()
            .origin,
        DispatchOrigin::EpicChain
    );
}
