use super::*;

/// A SECOND instant, distinct from `AT` — the writer's own `FixedClock`, used
/// for the methods (`save_repo_path` and friends) that read `self.now()`.
/// The agent-session-state methods take `now` as an argument instead
/// (decision 4 in this task's plan doc: it must be the event time the hook
/// itself observed, not this connection's clock), so a test that passed `AT`
/// for both could not tell "used the argument" apart from "resampled
/// `self.clock` and got the same answer by coincidence" — exactly the bug an
/// adversarial review of this plan's implementation caught twice (once in
/// the now-removed shell-tracking feature's own reducer twin, and once in
/// `try_record_stop`) before this file existed to catch it again.
const CALL_AT: &str = "2026-01-02T03:04:05.678Z";

const CALL_AT_STORED: &str = "2026-01-02 03:04:05.678";

fn call_at() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(CALL_AT)
        .unwrap()
        .with_timezone(&chrono::Utc)
}

/// `subagent_start` never refuses and answers with the live count the
/// module read back, not a placeholder.
#[tokio::test]
async fn subagent_start_reads_the_live_count_back() {
    let (writer, caller) = writer_with(RecordingCaller::returning_count(3));

    let live = writer
        .subagent_start(TaskId(1), "agent-1", "session-1", at())
        .await
        .unwrap();

    assert_eq!(live, 3);
    assert_eq!(
        caller.sent(),
        vec![Sent::SubagentStart(
            1,
            "agent-1".into(),
            "session-1".into(),
            at().to_rfc3339(),
        )]
    );
}

/// `started_at` is RFC 3339 for a subagent — this column is never compared.
#[tokio::test]
async fn subagent_start_stamps_rfc3339_of_the_passed_now() {
    let (writer, caller) = writer_with(RecordingCaller::returning_count(1));

    writer
        .subagent_start(TaskId(1), "agent-1", "session-1", call_at())
        .await
        .unwrap();

    let Some(Sent::SubagentStart(_, _, _, started_at)) = caller.sent().into_iter().next() else {
        panic!("expected a subagent start");
    };
    assert_eq!(started_at, "2026-01-02T03:04:05.678+00:00");
}

/// A drain that did NOT reach zero (or reached zero without a pending Stop to
/// apply) reports no flip, whatever the module's `live` answer says by
/// itself — `applied_pending_stop` is a SEPARATE bit from `live`.
#[tokio::test]
async fn a_subagent_stop_that_does_not_drain_reports_no_flip() {
    let (writer, _) = writer_seeded_with(
        1,
        TaskStatus::Running,
        RecordingCaller::returning_drain(2, false),
    );

    let drain = writer
        .subagent_stop(TaskId(1), "agent-1", "session-1")
        .await
        .unwrap();

    assert_eq!(
        drain,
        SubagentDrain {
            live: 2,
            applied_pending_stop: false,
        }
    );
}

/// The task was seen `Running` just before the call, and the module reports
/// the row is now in `review` — the flip is real, and reported as one.
#[tokio::test]
async fn a_subagent_stop_that_drains_a_running_task_reports_the_flip() {
    let (writer, _) = writer_seeded_with(
        1,
        TaskStatus::Running,
        RecordingCaller::returning_drain(0, true),
    );

    let drain = writer
        .subagent_stop(TaskId(1), "agent-1", "session-1")
        .await
        .unwrap();

    assert_eq!(
        drain,
        SubagentDrain {
            live: 0,
            applied_pending_stop: true,
        }
    );
}

/// The module's `review` answer alone is not enough — a task already sitting
/// in `review` before this call reports `review` too, and it drained
/// nothing. The pre-read is what tells the two apart: without one seeing
/// `Running`, this reports no flip even though the module says `review`.
/// (Decision 3 in this task's plan doc: advisory, and safe to under-report
/// here because the module's own recalculation, not this bit, is what kept
/// the epic status correct.)
#[tokio::test]
async fn a_review_answer_with_no_running_pre_read_reports_no_flip() {
    let (writer, _) = writer_with(RecordingCaller::returning_drain(0, true));

    let drain = writer
        .subagent_stop(TaskId(1), "agent-1", "session-1")
        .await
        .unwrap();

    assert!(!drain.applied_pending_stop);
}

#[tokio::test]
async fn a_subagent_clear_that_drains_a_running_task_reports_the_flip() {
    let (writer, _) = writer_seeded_with(
        1,
        TaskStatus::Running,
        RecordingCaller::returning_drain(0, true),
    );

    let drain = writer.subagent_clear(TaskId(1)).await.unwrap();

    assert!(drain.applied_pending_stop);
}

/// `try_record_stop`: once accepted, `Flipped` and `Deferred` are read back
/// from the module's answer directly, with no pre-read needed — see this
/// task's plan doc, decision 3, for why this pair (unlike resume/refresh
/// below) needs none.
#[tokio::test]
async fn try_record_stop_reads_flipped_back() {
    let (writer, _) = writer_with(RecordingCaller::returning_stop_flag(true));

    assert_eq!(
        writer.try_record_stop(TaskId(1), at()).await.unwrap(),
        StopOutcome::Flipped
    );
}

#[tokio::test]
async fn try_record_stop_reads_deferred_back() {
    let (writer, _) = writer_with(RecordingCaller::returning_stop_flag(false));

    assert_eq!(
        writer.try_record_stop(TaskId(1), at()).await.unwrap(),
        StopOutcome::Deferred
    );
}

/// A refused call — the module's own guard against a task that is not
/// `Running` — reads as `NoOp`, not an error. The refusal IS the answer here.
#[tokio::test]
async fn try_record_stop_reads_a_refusal_as_no_op() {
    let (writer, _) = writer_with(RecordingCaller::rejecting());

    assert_eq!(
        writer.try_record_stop(TaskId(1), at()).await.unwrap(),
        StopOutcome::NoOp
    );
}

/// `stop_pending_at` is the millisecond-precision fixed-width format, the
/// same `encode::stamp` the create path uses — it is compared against the
/// arriving prompt's own timestamp, so its format is load-bearing.
#[tokio::test]
async fn try_record_stop_stamps_the_passed_now_not_this_connections_clock() {
    let (writer, caller) = writer_with(RecordingCaller::returning_stop_flag(true));

    writer.try_record_stop(TaskId(1), call_at()).await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::TryRecordStop(1, CALL_AT_STORED.to_string())]
    );
}

/// `Resumed` vs. `Refreshed` cannot be read back from the module's answer —
/// both write the identical row (decision 3). The task was seen `Review`
/// just before the call, so this reports `Resumed`.
#[tokio::test]
async fn record_user_prompt_submit_reads_a_review_pre_read_as_resumed() {
    let (writer, _) = writer_seeded_with(1, TaskStatus::Review, RecordingCaller::returning(&[]));

    assert_eq!(
        writer
            .record_user_prompt_submit(TaskId(1), at())
            .await
            .unwrap(),
        UserPromptOutcome::Resumed
    );
}

/// The other arm: seen `Running` already, so this is a plain refresh.
#[tokio::test]
async fn record_user_prompt_submit_reads_a_running_pre_read_as_refreshed() {
    let (writer, _) = writer_seeded_with(1, TaskStatus::Running, RecordingCaller::returning(&[]));

    assert_eq!(
        writer
            .record_user_prompt_submit(TaskId(1), at())
            .await
            .unwrap(),
        UserPromptOutcome::Refreshed
    );
}

#[tokio::test]
async fn record_user_prompt_submit_reads_a_refusal_as_no_op() {
    let (writer, _) = writer_with(RecordingCaller::rejecting());

    assert_eq!(
        writer
            .record_user_prompt_submit(TaskId(1), at())
            .await
            .unwrap(),
        UserPromptOutcome::NoOp
    );
}

/// Both timestamps come off the SAME `now` — a single-`now`,
/// two-precision split (seconds for `last_pre_tool_use_at`, millis for the
/// void-comparison) — see this task's plan doc, decision 4.
#[tokio::test]
async fn record_user_prompt_submit_stamps_both_timestamps_from_the_passed_now() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .record_user_prompt_submit(TaskId(1), call_at())
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::RecordUserPromptSubmit(
            1,
            CALL_AT_STORED.to_string(),
            CALL_AT_STORED.to_string(),
        )]
    );
}

/// `Applied`/`Refused` map straight onto the PR gate's `true`/`false` via
/// `ReducerOutcome::won()`.
#[tokio::test]
async fn mark_pr_learnings_gate_shown_wins_when_applied_and_loses_when_refused() {
    let (applied_writer, _) = writer_with(RecordingCaller::returning(&[]));
    assert!(applied_writer
        .mark_pr_learnings_gate_shown(TaskId(1))
        .await
        .unwrap());

    let (refused_writer, _) = writer_with(RecordingCaller::rejecting());
    assert!(!refused_writer
        .mark_pr_learnings_gate_shown(TaskId(1))
        .await
        .unwrap());
}

/// `Ignore` never reaches the store at all — the client-side short-circuit
/// matching the SQL path's early `return Ok(())`, checked here by asserting
/// nothing was sent rather than merely that the call succeeded.
#[tokio::test]
async fn an_ignored_notification_never_reaches_the_store() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer
        .record_notification(TaskId(1), NotificationWrite::Ignore, at())
        .await
        .unwrap();

    assert!(caller.sent().is_empty());
}

/// The other three `NotificationWrite` modes DO reach the store, each named
/// by the module's own spelling.
#[tokio::test]
async fn a_notification_names_the_modules_mode() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .record_notification(TaskId(1), NotificationWrite::Raise, at())
        .await
        .unwrap();
    writer
        .record_notification(TaskId(1), NotificationWrite::Clear, at())
        .await
        .unwrap();
    writer
        .record_notification(TaskId(1), NotificationWrite::RaiseIfNoOwnWorkLive, at())
        .await
        .unwrap();

    let modes: Vec<String> = caller
        .sent()
        .into_iter()
        .map(|s| match s {
            Sent::RecordNotification(_, mode, _) => mode,
            other => panic!("expected a notification, got {other:?}"),
        })
        .collect();
    assert_eq!(modes, vec!["raise", "clear", "raise_if_no_own_work_live"]);
}

/// `record_pre_tool_use` passes the already-classified `sub_status` straight
/// through, stamped with the event time.
#[tokio::test]
async fn record_pre_tool_use_names_the_sub_status_and_the_passed_now() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .record_pre_tool_use(TaskId(1), SubStatus::Active, call_at())
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::RecordPreToolUse(
            1,
            "active".into(),
            CALL_AT_STORED.to_string()
        )]
    );
}

#[tokio::test]
async fn subagent_clear_and_void_pending_stop_is_a_plain_applied_call() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .subagent_clear_and_void_pending_stop(TaskId(1))
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::SubagentClearAndVoidPendingStop(1)]
    );
}
