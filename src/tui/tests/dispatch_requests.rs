// ---------------------------------------------------------------------------
// DispatchingFeedback, UnprovisionedIndicator and the chain's failure report,
// driven by this host's dispatch request rows (store-split M4, task #53257)
// ---------------------------------------------------------------------------
//
// `dispatch.allium: DispatchingFeedback` — a task is a member while a pending
// ask is unanswered OR this board's subscription holds an OPEN request for it;
// a request that reaches failed raises a popup carrying the row's reason
// (FailedRequestReported), which REPLACES the in-memory watchdog
// (DISPATCH_WATCHDOG_TIMEOUT). `UnprovisionedIndicator`'s claim-stamp fallback
// is re-based on the request timeout (60s). `epics.allium:
// ReportChainedDispatchFailure` raises AutoDispatchFailed once, from a failed
// `epic_chain` row, for a task that still exists.
//
// Rows reach the App as `TaskMessage::DispatchRequestsRefreshed`, sent by the
// same board reload that refreshes the cards. Its FIRST delivery is the
// baseline: rows already finished then are history and raise nothing.
use super::*;
use crate::models::{
    DispatchMode, DispatchOrigin, DispatchRequest, DispatchRequestId, DispatchRequestState, EpicId,
    TaskId, TaskStatus,
};
use crate::tui::messages::{SystemMessage, TaskMessage};

const HOST: &str = "test-host";

fn row(id: i64, task: i64, state: DispatchRequestState) -> DispatchRequest {
    let requested_at = chrono::Utc::now() - chrono::Duration::seconds(3);
    let finished = matches!(
        state,
        DispatchRequestState::Dispatched | DispatchRequestState::Failed
    );
    DispatchRequest {
        id: DispatchRequestId(id),
        task_id: TaskId(task),
        host: HOST.to_string(),
        requested_by: "owner".to_string(),
        mode: DispatchMode::Dispatch,
        origin: DispatchOrigin::User,
        state,
        requested_at,
        claimed_at: (state != DispatchRequestState::Requested).then_some(requested_at),
        finished_at: finished.then(chrono::Utc::now),
        reason: None,
        worktree: None,
        tmux_window: None,
        deadline: requested_at + chrono::Duration::seconds(60),
    }
}

fn failed(id: i64, task: i64, reason: &str) -> DispatchRequest {
    DispatchRequest {
        reason: Some(reason.to_string()),
        ..row(id, task, DispatchRequestState::Failed)
    }
}

fn chained(r: DispatchRequest) -> DispatchRequest {
    DispatchRequest {
        origin: DispatchOrigin::EpicChain,
        ..r
    }
}

fn refreshed(rows: Vec<DispatchRequest>) -> Message {
    Message::Task(TaskMessage::DispatchRequestsRefreshed(rows))
}

/// `make_app` (tasks 1 and 2 backlog, 3 running, 4 done) on this host, with
/// the baseline delivered: no rows yet.
fn app() -> App {
    let mut app = make_app();
    app.set_local_host_id(HOST.to_string());
    app.update(refreshed(vec![]));
    app
}

// -- Membership from open rows ---------------------------------------------------

/// OPEN part of the membership: a request this board's subscription holds is
/// shown as dispatching although nothing on this board marked it — the epic
/// chain's and MCP's requests, and requests that survive a board restart.
#[test]
fn an_open_request_row_makes_its_task_a_dispatching_member() {
    let mut app = app();
    assert!(!app.is_dispatching(TaskId(1)));

    app.update(refreshed(vec![row(1, 1, DispatchRequestState::Requested)]));
    assert!(app.is_dispatching(TaskId(1)), "requested is open");

    app.update(refreshed(vec![row(
        1,
        1,
        DispatchRequestState::Provisioning,
    )]));
    assert!(app.is_dispatching(TaskId(1)), "provisioning is open");

    let msg = app.status.message.as_deref().expect("sticky status set");
    assert!(msg.contains("Dispatching"), "got: {msg}");
    assert!(app.status.message_sticky, "StickyUntilEmpty");
}

/// ClearedWhenEmpty: the request finishing ends the membership and clears the
/// sticky status.
#[test]
fn a_finished_request_ends_the_membership_and_clears_the_status() {
    let mut app = app();
    app.update(refreshed(vec![row(
        1,
        1,
        DispatchRequestState::Provisioning,
    )]));

    app.update(refreshed(vec![row(1, 1, DispatchRequestState::Dispatched)]));

    assert!(!app.is_dispatching(TaskId(1)));
    assert!(
        app.status.message.is_none(),
        "status clears once no task is a member"
    );
    assert!(
        app.error_popup().is_none(),
        "a dispatched request raises nothing"
    );
}

/// SpansTheClaim: the pending ask hands over to the open row without a gap —
/// the task stays a member from the keypress to the row finishing.
#[test]
fn the_pending_ask_hands_over_to_the_open_row() {
    let mut app = app();
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(1))));
    assert!(app.is_dispatching(TaskId(1)));

    // The reducer answered and the row is in the subscription: the open row
    // supersedes the pending entry, and keeps the task a member.
    app.update(refreshed(vec![row(1, 1, DispatchRequestState::Requested)]));
    assert!(app.is_dispatching(TaskId(1)));

    app.update(refreshed(vec![row(1, 1, DispatchRequestState::Dispatched)]));
    assert!(
        !app.is_dispatching(TaskId(1)),
        "once the row finishes, a stale pending entry must not keep the spinner alive"
    );
}

// -- FailedRequestReported ----------------------------------------------------------

/// A request that reaches failed raises a popup naming the task and carrying
/// the row's reason.
#[test]
fn a_failed_request_raises_a_popup_with_the_rows_reason() {
    let mut app = app();
    app.update(refreshed(vec![row(
        1,
        1,
        DispatchRequestState::Provisioning,
    )]));

    app.update(refreshed(vec![failed(1, 1, "deadline passed")]));

    assert!(!app.is_dispatching(TaskId(1)));
    let popup = app.error_popup().expect("a failed request is reported");
    assert!(
        popup.contains("deadline passed"),
        "the row's reason: {popup}"
    );
    assert!(
        popup.contains("#1") || popup.contains("Task 1"),
        "the popup names the task: {popup}"
    );
}

/// Every reason travels the same path — none is special-cased.
#[test]
fn every_failure_reason_is_reported_the_same_way() {
    for reason in [
        "timed out",
        "host restarted",
        "task deleted",
        "git worktree add failed",
    ] {
        let mut app = app();
        app.update(refreshed(vec![row(
            1,
            1,
            DispatchRequestState::Provisioning,
        )]));
        app.update(refreshed(vec![failed(1, 1, reason)]));
        let popup = app.error_popup().unwrap_or_default().to_string();
        assert!(
            popup.contains(reason),
            "{reason:?} must reach the popup: {popup:?}"
        );
    }
}

/// Several failing together report the count.
#[test]
fn several_failures_together_report_the_count() {
    let mut app = app();
    app.update(refreshed(vec![
        row(1, 1, DispatchRequestState::Provisioning),
        row(2, 2, DispatchRequestState::Requested),
    ]));

    app.update(refreshed(vec![
        failed(1, 1, "timed out"),
        failed(2, 2, "timed out"),
    ]));

    let popup = app.error_popup().expect("the failures are reported");
    assert!(popup.contains('2'), "the count is reported: {popup}");
}

/// A failure is reported once, not on every redelivery of the same rows.
#[test]
fn a_failure_is_reported_once() {
    let mut app = app();
    app.update(refreshed(vec![row(
        1,
        1,
        DispatchRequestState::Provisioning,
    )]));
    app.update(refreshed(vec![failed(1, 1, "timed out")]));
    assert!(app.error_popup().is_some());
    app.status.error_popup = None;

    app.update(refreshed(vec![failed(1, 1, "timed out")]));

    assert!(
        app.error_popup().is_none(),
        "the same failed row raises nothing twice"
    );
}

/// Rows already finished in the first delivery are history: a restart does
/// not re-raise every failure the store still keeps.
#[test]
fn rows_already_failed_in_the_first_delivery_are_history() {
    let mut app = make_app();
    app.set_local_host_id(HOST.to_string());

    app.update(refreshed(vec![failed(1, 1, "timed out")]));

    assert!(app.error_popup().is_none());
}

// -- The watchdog is gone ----------------------------------------------------------

/// The in-memory watchdog is REPLACED by the row (FailedRequestReported): a
/// pending entry however old is never force-failed by a tick. The row's own
/// deadline — enforced by the host and backstopped by the module — is what
/// ends a stuck dispatch now.
#[test]
fn a_tick_never_force_fails_a_dispatch() {
    let mut app = app();
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(1))));

    app.update(Message::System(SystemMessage::Tick));

    assert!(
        app.is_dispatching(TaskId(1)),
        "no watchdog drains the entry"
    );
    assert!(
        app.error_popup().is_none(),
        "no 'timed out' popup from the board"
    );
}

/// The constant itself is gone, so nothing can quietly reintroduce a second
/// timeout beside the row's.
#[test]
fn the_dispatch_watchdog_constant_is_gone() {
    let source = include_str!("../mod.rs");
    assert!(
        !source.contains("const DISPATCH_WATCHDOG_TIMEOUT"),
        "DISPATCH_WATCHDOG_TIMEOUT must be removed (dispatch.allium: FailedRequestReported)"
    );
}

// -- UnprovisionedIndicator ---------------------------------------------------------

fn unprovisioned(id: i64, stamp_age_secs: Option<i64>) -> crate::models::Task {
    let mut task = make_task(id, TaskStatus::Running);
    task.worktree = None;
    task.tmux_window = None;
    task.last_pre_tool_use_at =
        stamp_age_secs.map(|s| chrono::Utc::now() - chrono::Duration::seconds(s));
    task
}

/// The claim-stamp fallback is the request timeout: a claim younger than 60s
/// may still be provisioning; one older is not (the module has failed its
/// request and released the claim by then).
#[test]
fn the_claim_stamp_fallback_uses_the_request_timeout() {
    let now = chrono::Utc::now();
    let young = unprovisioned(10, Some(59));
    let old = unprovisioned(11, Some(61));
    let app = App::new(vec![young.clone(), old.clone()]);

    assert!(app.dispatch_may_be_in_flight(&young, now));
    assert!(
        !app.dispatch_may_be_in_flight(&old, now),
        "past dispatch_request_timeout an unprovisioned claim is abandoned"
    );
}

/// An open request row this board can see counts as in flight, stamp or no
/// stamp.
#[test]
fn an_open_request_row_counts_as_in_flight() {
    let now = chrono::Utc::now();
    let task = unprovisioned(10, None);
    let mut app = App::new(vec![task.clone()]);
    app.set_local_host_id(HOST.to_string());
    app.update(refreshed(vec![]));
    assert!(
        !app.dispatch_may_be_in_flight(&task, now),
        "no stamp, no row"
    );

    app.update(refreshed(vec![row(
        1,
        10,
        DispatchRequestState::Provisioning,
    )]));

    assert!(app.dispatch_may_be_in_flight(&task, now));
}

// -- RetryFresh goes through DispatchTask ---------------------------------------------

/// `dispatch.allium: RetryFresh` re-dispatches through the standard dispatch
/// command, a `user` request, naming NO mode: the module derives it
/// (`store-split.allium` D3), so a research-tagged task without a plan goes
/// back to research (`DispatchResearchTask`) and every other one to dispatch.
#[test]
fn retry_fresh_redispatches_through_the_standard_dispatch() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.board.tasks[0].sub_status = crate::models::SubStatus::Stale;
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::Task(TaskMessage::RetryFresh(TaskId(4))));

    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::DispatchAgent { mode: None, .. })
        )),
        "retry-fresh lets the module derive the mode, got: {cmds:?}"
    );
}

// -- ReportChainedDispatchFailure -----------------------------------------------------

fn app_with_epic_subtask() -> App {
    let mut subtask = make_task(5, TaskStatus::Running);
    subtask.worktree = None;
    subtask.tmux_window = None;
    subtask.epic_id = Some(EpicId(7));
    let mut app = App::new(vec![subtask, make_task(6, TaskStatus::Backlog)]);
    app.set_local_host_id(HOST.to_string());
    app.update(refreshed(vec![]));
    app
}

/// A chained request that fails marks its subtask, exactly as the old
/// in-process AutoDispatchFailed did (SurfaceAutoDispatchFailure), and does so
/// once.
#[test]
fn a_failed_epic_chain_row_raises_auto_dispatch_failed_once() {
    let mut app = app_with_epic_subtask();
    app.update(refreshed(vec![chained(row(
        1,
        5,
        DispatchRequestState::Provisioning,
    ))]));

    app.update(refreshed(vec![chained(failed(1, 5, "deadline passed"))]));

    assert!(app.auto_dispatch_failed(TaskId(5)));
    let status = app.status_message().unwrap_or_default().to_string();
    assert!(
        status.contains("#5") || status.contains("Task 5"),
        "StatusMessageNamesTheSubtask: {status:?}"
    );

    // Cleared by the operator's own re-dispatch, then the same failed row is
    // redelivered: it must not mark the subtask again.
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(5))));
    assert!(!app.auto_dispatch_failed(TaskId(5)));
    app.update(refreshed(vec![chained(failed(1, 5, "deadline passed"))]));
    assert!(
        !app.auto_dispatch_failed(TaskId(5)),
        "AutoDispatchFailed is raised once per row"
    );
}

/// Only `epic_chain` rows are the chain's.
#[test]
fn a_failed_user_row_is_not_a_chain_failure() {
    let mut app = app_with_epic_subtask();
    app.update(refreshed(vec![row(
        1,
        5,
        DispatchRequestState::Provisioning,
    )]));

    app.update(refreshed(vec![failed(1, 5, "deadline passed")]));

    assert!(!app.auto_dispatch_failed(TaskId(5)));
}

/// `requires: exists request.task` — a subtask deleted under its request has
/// no card to mark.
#[test]
fn a_chain_failure_for_a_deleted_task_marks_nothing() {
    let mut app = app_with_epic_subtask();
    app.update(refreshed(vec![chained(row(
        1,
        99,
        DispatchRequestState::Provisioning,
    ))]));

    app.update(refreshed(vec![chained(failed(1, 99, "task deleted"))]));

    assert!(!app.auto_dispatch_failed(TaskId(99)));
}

/// The restart case the row exists for: the board that started the chain is
/// gone, and the restarting board's own startup fails the row ("host
/// restarted") AFTER its first delivery — so the chain's failure is still
/// reported.
#[test]
fn a_chained_row_failed_by_this_boards_restart_is_reported() {
    let mut subtask = make_task(5, TaskStatus::Running);
    subtask.worktree = None;
    subtask.tmux_window = None;
    subtask.epic_id = Some(EpicId(7));
    let mut app = App::new(vec![subtask]);
    app.set_local_host_id(HOST.to_string());
    // First delivery: the row the dead board left provisioning.
    app.update(refreshed(vec![chained(row(
        1,
        5,
        DispatchRequestState::Provisioning,
    ))]));

    app.update(refreshed(vec![chained(failed(1, 5, "host restarted"))]));

    assert!(app.auto_dispatch_failed(TaskId(5)));
}
