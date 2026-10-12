//! `wrap_up` and `exit_session` on session request rows (store-split M5,
//! task #53258).
//!
//! `mcp-task-tools.allium: WrapUpViaMcp` runs the host-local checks, then
//! WRITES a request (`request_wrap_up`) and, for a rebase, WAITS on the row for
//! at most the rebase deadline plus the backstop grace, answering by the row's
//! state with the row's id as the exit token. `ExitSessionViaMcp` calls
//! `request_exit_session` and answers as soon as the module accepts the close,
//! WITHOUT waiting for the teardown (it would kill the caller's own window).
//!
//! The runner here answers only what a tool still runs itself (the commit
//! check). The rebase and the teardown are the host reactor's: a test drives
//! the row the way this board's host would — claim, then report — through the
//! store, which is the in-memory double underneath.

use super::*;

use std::time::Duration;

use crate::models::{
    SessionRequest, SessionRequestId, SessionRequestKind, SessionRequestState, TaskId, TmuxWindow,
};
use crate::store::{SessionRequestRead, SessionRequestStore};
use crate::sync::{InjectedFault, MemoryReducerCaller, SettledIdentity, SharedRows};

/// Bound on awaiting a completion signal — a deadline on a signal, never a
/// wall-clock wait for work to finish.
const SIGNAL_TIMEOUT: Duration = Duration::from_secs(5);

const WORKTREE: &str = "/repo/.worktrees/42-fix-bug";

/// An `McpState` whose `wrap_up` waits at most `cap`, over an in-memory store.
/// `answers` is what the tool itself runs: the commit check, per wrap-up.
fn state_with_wait_cap(
    cap: Duration,
    answers: usize,
) -> (Arc<McpState>, Arc<dyn store::TaskStore>) {
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::open_in_memory().unwrap());
    let state = state_over_db(db.clone(), cap, answers);
    (state, db)
}

fn state_over_db(db: Arc<dyn store::TaskStore>, cap: Duration, answers: usize) -> Arc<McpState> {
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(
        (0..answers)
            .map(|_| MockProcessRunner::ok_with_stdout(b""))
            .collect(),
    ));
    let mut state = McpState::new(
        McpDeps {
            db,
            runner,
            embedding_service: EmbeddingService::new_test(),
            data_dir: std::env::temp_dir(),
        },
        None,
    );
    state.wrap_up_wait_cap = cap;
    Arc::new(state)
}

/// A task dispatched on this board's host: running, with a worktree whose
/// last component names a branch, and a window.
async fn wrappable_task(db: &Arc<dyn store::TaskStore>) -> TaskId {
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("T", "/repo")
        })
        .await
        .unwrap();
    let window = TmuxWindow::for_task(id);
    db.patch_task(
        id,
        &store::TaskPatch::new()
            .worktree(Some(WORKTREE))
            .host(Some(crate::store::MEMORY_HOST_ID))
            .tmux_window(Some(&window)),
    )
    .await
    .unwrap();
    id
}

fn wrap_up_call(
    state: &Arc<McpState>,
    task: TaskId,
    action: &str,
) -> tokio::task::JoinHandle<JsonRpcResponse> {
    let state = state.clone();
    let action = action.to_string();
    tokio::spawn(async move {
        call(
            &state,
            "tools/call",
            Some(json!({
                "name": "wrap_up",
                "arguments": { "task_id": task.0, "action": action }
            })),
        )
        .await
    })
}

async fn exit_call(
    state: &Arc<McpState>,
    task: TaskId,
    token: &str,
    action: &str,
    pr_url: Option<&str>,
) -> JsonRpcResponse {
    let mut arguments = json!({ "task_id": task.0, "token": token, "action": action });
    if let Some(url) = pr_url {
        arguments["pr_url"] = json!(url);
    }
    call(
        state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": arguments })),
    )
    .await
}

async fn answer(handle: tokio::task::JoinHandle<JsonRpcResponse>) -> JsonRpcResponse {
    tokio::time::timeout(SIGNAL_TIMEOUT, handle)
        .await
        .expect("wrap_up must answer once the row finishes")
        .unwrap()
}

/// The task's wrap_up row, once the handler running beside this test has
/// written it. Yields to that handler until the row appears.
async fn row_written_for(db: &Arc<dyn store::TaskStore>, task: TaskId) -> SessionRequest {
    tokio::time::timeout(SIGNAL_TIMEOUT, async {
        loop {
            if let Some(row) = db
                .session_requests()
                .await
                .into_iter()
                .find(|r| r.task_id == task && r.kind == SessionRequestKind::WrapUp)
            {
                return row;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("wrap_up must write a request")
}

/// The token a successful `wrap_up` response carries.
fn token_in(text: &str) -> String {
    let after = text
        .split("Exit token: ")
        .nth(1)
        .unwrap_or_else(|| panic!("no exit token in: {text}"));
    after
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
}

/// `config.wrap_up_rebase_timeout` plus `config.session_request_backstop_grace`.
#[test]
fn wrap_up_waits_for_the_rebase_deadline_plus_the_grace() {
    assert_eq!(crate::mcp::WRAP_UP_WAIT_CAP, Duration::from_secs(130));
}

// -- WrapUpViaMcp: the checks come first ------------------------------------------

/// A refusal by the tool's own checks writes NO row and issues no token: a
/// dirty worktree here.
#[tokio::test]
async fn a_refusal_in_the_tool_writes_no_request_row() {
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b" M src/lib.rs\n"),
    ]));
    let state = {
        let mut s = McpState::new(
            McpDeps {
                db: db.clone(),
                runner,
                embedding_service: EmbeddingService::new_test(),
                data_dir: std::env::temp_dir(),
            },
            None,
        );
        s.wrap_up_wait_cap = Duration::ZERO;
        Arc::new(s)
    };
    let task = wrappable_task(&db).await;

    let resp = answer(wrap_up_call(&state, task, "rebase")).await;

    assert_error(&resp, "uncommitted");
    assert!(db.session_requests().await.is_empty());
}

/// For rebase the branch must be derivable from the worktree: checked in the
/// tool, after the commit check and before any row, so it is a bad request
/// rather than a failed rebase.
#[tokio::test]
async fn an_underivable_branch_is_refused_before_any_row_is_written() {
    let (state, db) = state_with_wait_cap(Duration::ZERO, 1);
    let task = wrappable_task(&db).await;
    db.patch_task(task, &store::TaskPatch::new().worktree(Some("/")))
        .await
        .unwrap();

    let resp = answer(wrap_up_call(&state, task, "rebase")).await;

    assert_error(&resp, "Cannot derive branch from worktree");
    assert!(db.session_requests().await.is_empty());
}

// -- WrapUpViaMcp: done and pr answer at once -----------------------------------------

/// done and pr need no host work: the module writes the row already done, and
/// the tool answers at once with the row's id as the token and today's text.
#[tokio::test]
async fn wrap_up_done_and_pr_answer_at_once_with_the_rows_id_as_the_token() {
    for action in ["done", "pr"] {
        let (state, db) = state_with_wait_cap(Duration::ZERO, 1);
        let task = wrappable_task(&db).await;

        let resp = answer(wrap_up_call(&state, task, action)).await;

        let text = extract_response_text(&resp);
        let rows = db.session_requests().await;
        assert_eq!(rows.len(), 1, "{action}");
        assert_eq!(rows[0].state, SessionRequestState::Done);
        assert_eq!(rows[0].action.as_str(), action);
        assert_eq!(token_in(&text), rows[0].id.0.to_string(), "{text}");
        assert!(
            text.contains(&format!(
                "wrap_up complete (task {}, action: {action})",
                task.0
            )),
            "{text}"
        );
        assert!(text.contains("The session is NOT yet closed."), "{text}");
        assert!(
            text.contains(&format!("exit_session` with action=\"{action}\"")),
            "{text}"
        );
    }
}

// -- WrapUpViaMcp: a rebase waits on the row ---------------------------------------------

/// The tool writes a rebase row and WAITS: it answers when this board's host
/// reports done, with the row's id as the token and today's rebase text.
#[tokio::test]
async fn wrap_up_rebase_waits_for_the_host_and_answers_with_the_token() {
    let (state, db) = state_with_wait_cap(crate::mcp::WRAP_UP_WAIT_CAP, 1);
    let task = wrappable_task(&db).await;

    let handle = wrap_up_call(&state, task, "rebase");
    let row = row_written_for(&db, task).await;
    assert_eq!(row.state, SessionRequestState::Requested);
    assert_eq!(row.host, crate::store::MEMORY_HOST_ID);
    assert!(db.claim_session_request(row.id).await.unwrap());
    assert!(
        !handle.is_finished(),
        "the call waits for the host's report"
    );
    assert!(db.report_session_request_done(row.id).await.unwrap());

    let text = extract_response_text(&answer(handle).await);

    assert_eq!(token_in(&text), row.id.0.to_string(), "{text}");
    assert!(
        text.contains(&format!(
            "wrap_up complete (task {}, action: rebase)",
            task.0
        )),
        "{text}"
    );
    assert!(
        text.contains("without it, the tmux window stays alive"),
        "{text}"
    );
}

/// A failed row is an INTERNAL_ERROR `wrap_up failed: <reason>`: a conflict's
/// reason names the files, and the module has set the Conflict sub_status.
#[tokio::test]
async fn wrap_up_rebase_reports_a_failed_row_with_its_reason() {
    let (state, db) = state_with_wait_cap(crate::mcp::WRAP_UP_WAIT_CAP, 1);
    let task = wrappable_task(&db).await;

    let handle = wrap_up_call(&state, task, "rebase");
    let row = row_written_for(&db, task).await;
    assert!(db.claim_session_request(row.id).await.unwrap());
    assert!(db
        .report_session_request_failed(row.id, "Rebase conflict on b in foo.rs", true)
        .await
        .unwrap());

    let resp = answer(handle).await;

    assert_error(&resp, "wrap_up failed: Rebase conflict on b in foo.rs");
    assert_eq!(
        db.get_task(task).await.unwrap().sub_status,
        SubStatus::Conflict,
        "the module sets the flag, not the tool"
    );
    assert!(!error_message(&resp).contains("Exit token"));
}

/// "timed out", "deadline passed" and "host restarted" read the same way: the
/// agent calls wrap_up again once the cause is fixed.
#[tokio::test]
async fn wrap_up_rebase_reports_the_backstops_reason() {
    let (state, db) = state_with_wait_cap(crate::mcp::WRAP_UP_WAIT_CAP, 1);
    let task = wrappable_task(&db).await;
    let caller_rows = db.clone();

    let handle = wrap_up_call(&state, task, "rebase");
    let row = row_written_for(&caller_rows, task).await;
    assert!(db.claim_session_request(row.id).await.unwrap());
    assert!(db
        .report_session_request_failed(row.id, "deadline passed", false)
        .await
        .unwrap());

    assert_error(&answer(handle).await, "wrap_up failed: deadline passed");
}

/// Still open when the wait ends — the store unreachable, or the host gone
/// without the backstop yet firing: an INTERNAL_ERROR naming the request and
/// saying no token was issued and to call wrap_up again.
#[tokio::test]
async fn wrap_up_rebase_still_open_after_the_wait_is_an_error_naming_the_request() {
    let (state, db) = state_with_wait_cap(Duration::ZERO, 1);
    let task = wrappable_task(&db).await;

    let resp = answer(wrap_up_call(&state, task, "rebase")).await;

    let row = row_written_for(&db, task).await;
    let message = error_message(&resp);
    assert!(message.contains(&format!("#{}", row.id.0)), "{message}");
    assert!(message.contains("no exit token was issued"), "{message}");
    assert!(message.contains("call wrap_up again"), "{message}");
}

/// A refusal by the module is an INVALID_PARAMS error carrying the module's
/// reason: here a second wrap-up while the first rebase is still open.
#[tokio::test]
async fn a_second_wrap_up_while_one_is_open_is_refused_with_the_modules_reason() {
    let (state, db) = state_with_wait_cap(Duration::ZERO, 2);
    let task = wrappable_task(&db).await;
    answer(wrap_up_call(&state, task, "rebase")).await;

    let resp = answer(wrap_up_call(&state, task, "rebase")).await;

    assert_error(&resp, "open session request");
    assert_eq!(db.session_requests().await.len(), 1);
}

// -- ExitSessionViaMcp ------------------------------------------------------------------------

/// A done wrap-up's token closes the session in one call, and the answer does
/// NOT wait for the teardown: no host reactor runs here, the row is still
/// `requested`, and the response is already "Session closed.".
#[tokio::test]
async fn exit_session_answers_when_the_module_accepts_without_waiting_for_the_teardown() {
    let (state, db) = state_with_wait_cap(Duration::ZERO, 1);
    let task = wrappable_task(&db).await;
    let text = extract_response_text(&answer(wrap_up_call(&state, task, "done")).await);
    let token = token_in(&text);

    let resp = exit_call(&state, task, &token, "done", None).await;

    assert_eq!(extract_response_text(&resp), "Session closed.");
    let stored = db.get_task(task).await.unwrap();
    assert_eq!(stored.status, TaskStatus::Done);
    assert!(stored.tmux_window.is_none());
    let exit_row = db
        .session_requests()
        .await
        .into_iter()
        .find(|r| r.kind == SessionRequestKind::ExitSession)
        .expect("the close wrote its teardown row");
    assert_eq!(exit_row.state, SessionRequestState::Requested);
    assert_eq!(
        exit_row.wrap_up_request,
        Some(SessionRequestId(token.parse().unwrap()))
    );
}

/// The pr action closes to Review with the url.
#[tokio::test]
async fn exit_session_pr_takes_the_task_to_review_with_the_url() {
    let (state, db) = state_with_wait_cap(Duration::ZERO, 1);
    let task = wrappable_task(&db).await;
    let text = extract_response_text(&answer(wrap_up_call(&state, task, "pr")).await);

    let resp = exit_call(
        &state,
        task,
        &token_in(&text),
        "pr",
        Some("https://github.com/o/r/pull/9"),
    )
    .await;

    assert_eq!(extract_response_text(&resp), "Session closed.");
    let stored = db.get_task(task).await.unwrap();
    assert_eq!(stored.status, TaskStatus::Review);
    assert_eq!(
        stored.url.map(|u| u.url),
        Some("https://github.com/o/r/pull/9".to_string())
    );
}

/// The chained subtask is named from the exit_session row's `chained_task`:
/// the module ran the chain in the same transaction.
#[tokio::test]
async fn exit_session_response_names_the_chained_subtask() {
    let (state, db) = state_with_wait_cap(Duration::ZERO, 1);
    let epic = db.create_epic("E", "d", None).await.unwrap();
    db.patch_epic(epic.id, &store::EpicPatch::new().auto_dispatch(true))
        .await
        .unwrap();
    let task = wrappable_task(&db).await;
    db.set_task_epic_id(task, Some(epic.id)).await.unwrap();
    let next = db
        .create_task(CreateTaskRequest {
            epic_id: Some(epic.id),
            sort_order: Some(10),
            ..CreateTaskRequest::fixture("Wire up the widget", "/repo")
        })
        .await
        .unwrap();
    let text = extract_response_text(&answer(wrap_up_call(&state, task, "done")).await);

    let resp = exit_call(&state, task, &token_in(&text), "done", None).await;

    assert_eq!(
        extract_response_text(&resp),
        format!(
            "Session closed. Dispatching next epic subtask #{} 'Wire up the widget'.",
            next.0
        )
    );
    assert_eq!(
        db.get_task(next).await.unwrap().status,
        TaskStatus::Running,
        "the module's request is the claim"
    );
}

/// The tool itself refuses a missing token or action, with today's text,
/// before asking the module anything.
#[tokio::test]
async fn exit_session_without_a_token_or_an_action_is_refused_in_the_tool() {
    let (state, db) = state_with_wait_cap(Duration::ZERO, 1);
    let task = wrappable_task(&db).await;

    let no_token = call(
        &state,
        "tools/call",
        Some(json!({"name": "exit_session", "arguments": {"task_id": task.0, "action": "done"}})),
    )
    .await;
    assert_error(&no_token, "no exit token — call wrap_up first");

    let no_action = call(
        &state,
        "tools/call",
        Some(json!({"name": "exit_session", "arguments": {"task_id": task.0, "token": "1"}})),
    )
    .await;
    assert_error(
        &no_action,
        "action is required — pass the same action used in wrap_up",
    );
    assert_eq!(db.get_task(task).await.unwrap().status, TaskStatus::Running);
}

/// The module's refusals reach the agent in the module's words, change
/// nothing, and leave the token valid for a retry.
#[tokio::test]
async fn a_refused_exit_session_carries_the_modules_words_and_leaves_the_token_valid() {
    let (state, db) = state_with_wait_cap(Duration::ZERO, 1);
    let task = wrappable_task(&db).await;
    let text = extract_response_text(&answer(wrap_up_call(&state, task, "pr")).await);
    let token = token_in(&text);

    assert_error(
        &exit_call(&state, task, "123456", "pr", Some("https://x/pull/1")).await,
        "invalid exit token",
    );
    assert_error(
        &exit_call(&state, task, "not-a-number", "pr", Some("https://x/pull/1")).await,
        "invalid exit token",
    );
    let mismatch = exit_call(&state, task, &token, "done", None).await;
    assert_error(
        &mismatch,
        "exit token was issued for wrap_up(action=\"pr\")",
    );
    assert_error(
        &exit_call(&state, task, &token, "pr", None).await,
        "pr_url is required for action 'pr'",
    );
    assert_eq!(db.get_task(task).await.unwrap().status, TaskStatus::Running);

    // Still valid.
    let ok = exit_call(&state, task, &token, "pr", Some("https://x/pull/1")).await;
    assert_eq!(extract_response_text(&ok), "Session closed.");
    // Spent now.
    assert_error(
        &exit_call(&state, task, &token, "pr", Some("https://x/pull/1")).await,
        "no exit token — call wrap_up first",
    );
}

/// A task with no live window is "has no active session", which the /wrap-up
/// skill treats as success.
#[tokio::test]
async fn exit_session_on_a_task_with_no_window_says_it_has_no_active_session() {
    let (state, db) = state_with_wait_cap(Duration::ZERO, 1);
    let task = wrappable_task(&db).await;
    let text = extract_response_text(&answer(wrap_up_call(&state, task, "done")).await);
    db.patch_task(task, &store::TaskPatch::new().tmux_window(None))
        .await
        .unwrap();

    let resp = exit_call(&state, task, &token_in(&text), "done", None).await;

    assert_error(&resp, &format!("task #{} has no active session", task.0));
}

/// W3: unlike the in-memory token this replaces, the token survives a board
/// restart — a new `McpState` over the same store honours it.
#[tokio::test]
async fn an_exit_token_survives_a_board_restart() {
    let (state, db) = state_with_wait_cap(Duration::ZERO, 1);
    let task = wrappable_task(&db).await;
    let text = extract_response_text(&answer(wrap_up_call(&state, task, "done")).await);
    let restarted = state_over_db(db.clone(), Duration::ZERO, 0);

    let resp = exit_call(&restarted, task, &token_in(&text), "done", None).await;

    assert_eq!(extract_response_text(&resp), "Session closed.");
}

// -- No answer from the store --------------------------------------------------------------------

/// A store whose caller the test can fault, for the no-answer settling.
fn faultable_state() -> (
    Arc<McpState>,
    Arc<dyn store::TaskStore>,
    Arc<MemoryReducerCaller>,
) {
    let rows = Arc::new(SharedRows::new());
    let clock: Arc<dyn crate::clock::Clock> = Arc::new(crate::clock::SystemClock);
    let caller = Arc::new(
        MemoryReducerCaller::new(rows.clone(), clock.clone())
            .with_sender("test-user")
            .with_registered_host(crate::store::MEMORY_HOST_ID, "test-user"),
    );
    let identity = Arc::new(SettledIdentity::default());
    identity.settle("test-user");
    let dir = tempfile::tempdir().unwrap();
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::new(
        rows,
        caller.clone(),
        identity,
        clock,
        crate::store::MEMORY_HOST_ID.to_string(),
        dir.keep().as_path(),
    ));
    let state = state_over_db(db.clone(), Duration::ZERO, 1);
    (state, db, caller)
}

/// The call may or may not have landed. A reply lost AFTER the module applied
/// the close is settled from the rows — an exit_session row consumed the
/// token — and answered as accepted.
#[tokio::test]
async fn a_lost_exit_session_reply_is_settled_from_the_rows_as_accepted() {
    let (state, db, caller) = faultable_state();
    let task = wrappable_task(&db).await;
    let text = extract_response_text(&answer(wrap_up_call(&state, task, "done")).await);
    caller.fail_next("request_exit_session", InjectedFault::ReplyLost);

    let resp = exit_call(&state, task, &token_in(&text), "done", None).await;

    assert_eq!(extract_response_text(&resp), "Session closed.");
    assert_eq!(db.get_task(task).await.unwrap().status, TaskStatus::Done);
}

/// A call that never reached the module did not take effect: no row consumed
/// the token, so the answer says the close did not take effect, the session is
/// still alive and the token is still valid.
#[tokio::test]
async fn an_unsent_exit_session_is_an_error_saying_the_token_is_still_valid() {
    let (state, db, caller) = faultable_state();
    let task = wrappable_task(&db).await;
    let text = extract_response_text(&answer(wrap_up_call(&state, task, "done")).await);
    let token = token_in(&text);
    caller.fail_next("request_exit_session", InjectedFault::Unsent);

    let resp = exit_call(&state, task, &token, "done", None).await;

    let message = error_message(&resp);
    assert!(message.contains("did not take effect"), "{message}");
    assert!(message.contains("still alive"), "{message}");
    assert!(message.contains("call exit_session again"), "{message}");
    assert_eq!(db.get_task(task).await.unwrap().status, TaskStatus::Running);

    let retry = exit_call(&state, task, &token, "done", None).await;
    assert_eq!(extract_response_text(&retry), "Session closed.");
}

// -- What stays on the board -----------------------------------------------------------------------

/// `update_tasks` status="done" keeps its board-side close and teardown until
/// M7 (`MarkTaskDoneViaMcp`): no token, no session row, no chain.
#[tokio::test]
async fn update_tasks_done_keeps_its_board_side_close_and_writes_no_session_row() {
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::open_in_memory().unwrap());
    let runner = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
    let (state, db) = {
        let state = {
            let s = McpState::new(
                McpDeps {
                    db: db.clone(),
                    runner: runner.clone(),
                    embedding_service: EmbeddingService::new_test(),
                    data_dir: std::env::temp_dir(),
                },
                None,
            );
            Arc::new(s)
        };
        (state, db)
    };
    let task = wrappable_task(&db).await;

    let resp = call(
        &state,
        "tools/call",
        Some(
            json!({"name": "update_tasks", "arguments": {"task_ids": [task.0], "status": "done"}}),
        ),
    )
    .await;

    assert_eq!(
        extract_response_text(&resp),
        format!("Task #{} marked done.", task.0)
    );
    assert!(db.session_requests().await.is_empty());
    assert_eq!(db.get_task(task).await.unwrap().status, TaskStatus::Done);
}

/// The tool descriptions stay consistent with the spec: wrap_up says it
/// returns an exit token, exit_session that it closes the session with it, and
/// neither carries instructions about the other
/// (`TheNextCallIsNamedJustInTime`).
#[tokio::test]
async fn the_tool_descriptions_say_what_each_tool_does() {
    let state = test_state().await;
    let resp = call(&state, "tools/list", None).await;
    let tools = resp.result.as_ref().unwrap()["tools"].as_array().unwrap();
    let describe = |name: &str| -> String {
        tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("no tool {name}"))["description"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let wrap = describe("wrap_up");
    let exit = describe("exit_session");
    assert!(wrap.contains("exit token"), "{wrap}");
    assert!(exit.contains("token"), "{exit}");
    assert!(!wrap.contains("MUST"), "{wrap}");
    assert!(!exit.contains("MUST"), "{exit}");
}
