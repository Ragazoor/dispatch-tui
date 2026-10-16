use super::*;

/// The request carries THIS machine's host id, which is what lets the store
/// pass over a task whose worktree is somewhere else — and names this machine
/// as the one that provisions it (`store-split.allium`, D2). No mode is sent
/// as `""`, which asks the module to derive it (D3).
#[tokio::test]
async fn a_request_names_the_machine_making_it() {
    use crate::store::DispatchRequestStore;
    let (writer, caller) = writer_with(RecordingCaller::returning(&[1]));

    writer
        .request_dispatch(TaskId(3), crate::models::DispatchOrigin::Mcp, None)
        .await
        .unwrap();
    writer
        .request_dispatch(
            TaskId(4),
            crate::models::DispatchOrigin::Quick,
            Some(crate::models::DispatchMode::Quick),
        )
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![
            Sent::RequestDispatch(TaskId(3), "host-me".into(), "mcp".into(), String::new()),
            Sent::RequestDispatch(TaskId(4), "host-me".into(), "quick".into(), "quick".into()),
        ]
    );
}

// The chain's candidate loop moved to the service with store-split M4 and into
// the module with M5 (`store-split.allium: RequestExitSession` runs
// `AutoDispatchNextSubtask` in its own transaction): its ordering, phoenix and
// refusal rules are pinned in `src/sync/memory_caller/tests/session_requests.rs`
// and the module's own tests.

/// `store-split.allium: RequestWrapUp`, `RequestExitSession` and the host
/// rules after them. A wrap-up names the machine making it, with the action in
/// the module's spelling; an exit_session carries the token and the url only
/// for the pr action; every host call names this machine. A refusal is an
/// answer, an outage an error, as for dispatch requests.
#[tokio::test]
async fn session_requests_name_the_machine_and_speak_the_modules_vocabulary() {
    use crate::models::{SessionRequestId, WrapUpMode};
    use crate::store::{SessionRequestAnswer, SessionRequestStore};
    let (writer, caller) = writer_with(RecordingCaller::returning(&[7]));

    let wrap = writer
        .request_wrap_up(TaskId(3), WrapUpMode::Rebase)
        .await
        .unwrap();
    assert_eq!(wrap, SessionRequestAnswer::Requested(SessionRequestId(7)));
    writer
        .request_exit_session(
            TaskId(3),
            SessionRequestId(7),
            WrapUpMode::Done,
            Some("ignored"),
        )
        .await
        .unwrap();
    writer
        .request_exit_session(
            TaskId(4),
            SessionRequestId(8),
            WrapUpMode::Pr,
            Some("https://github.com/o/r/pull/9"),
        )
        .await
        .unwrap();
    assert!(writer
        .claim_session_request(SessionRequestId(7))
        .await
        .unwrap());
    assert!(writer
        .report_session_request_done(SessionRequestId(7))
        .await
        .unwrap());
    assert!(writer
        .report_session_request_failed(SessionRequestId(8), "boom", true)
        .await
        .unwrap());
    writer.close_task(TaskId(5)).await.unwrap();
    writer.fail_interrupted_session_requests().await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![
            Sent::RequestWrapUp(TaskId(3), "host-me".into(), "rebase".into()),
            Sent::RequestExitSession(TaskId(3), 7, "done".into(), "ignored".into()),
            Sent::RequestExitSession(
                TaskId(4),
                8,
                "pr".into(),
                "https://github.com/o/r/pull/9".into()
            ),
            Sent::ClaimSessionRequest(7, "host-me".into()),
            Sent::ReportSessionRequestDone(7, "host-me".into()),
            Sent::ReportSessionRequestFailed(8, "host-me".into(), "boom".into(), true),
            Sent::CloseTask(TaskId(5), "host-me".into()),
            Sent::FailInterruptedSessionRequests("host-me".into()),
        ]
    );

    let (loser, _) = writer_with(RecordingCaller::rejecting());
    assert!(matches!(
        loser
            .request_wrap_up(TaskId(1), WrapUpMode::Done)
            .await
            .unwrap(),
        SessionRequestAnswer::Refused(_)
    ));
    assert!(!loser
        .claim_session_request(SessionRequestId(1))
        .await
        .unwrap());
    assert!(
        loser.close_task(TaskId(1)).await.is_err(),
        "a refused close is an error: nothing is known to have landed"
    );
    let (offline, _) = writer_with(RecordingCaller::refusing("store unreachable"));
    assert!(offline
        .request_exit_session(TaskId(1), SessionRequestId(1), WrapUpMode::Done, None)
        .await
        .is_err());
}

/// `epics.allium: ProvisionManagedEpics` (store-split M7, X4) is one reducer
/// call: a disabled feed is `""`, no interval is `0`, and the identity is the
/// module's to stamp, so nothing but the four settings is sent.
#[tokio::test]
async fn ensure_managed_epics_sends_the_four_settings_in_the_modules_spelling() {
    use crate::store::EpicCrud;
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .ensure_managed_epics(Some("reviews.sh"), Some(600), None, None)
        .await
        .unwrap();
    writer
        .ensure_managed_epics(None, None, Some("cve.sh"), Some(900))
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![
            Sent::EnsureManagedEpics("reviews.sh".into(), 600, "".into(), 0),
            Sent::EnsureManagedEpics("".into(), 0, "cve.sh".into(), 900),
        ]
    );
    let (loser, _) = writer_with(RecordingCaller::rejecting());
    assert!(loser
        .ensure_managed_epics(Some("reviews.sh"), None, None, None)
        .await
        .is_err());
}

// -- The refusal names the outage -------------------------------------------

/// `sync.allium: AWriteWithNoConnectionIsRefused` — the refusal carries the
/// connection's reason. "The store is unreachable: connection refused" is
/// something an operator can act on; "could not save" is not.
///
/// Against `SettledIdentity` directly, because the cell is the whole mechanism:
/// the session owns the error and the transport that reports the refusal is
/// deliberately not able to reach the session.
#[test]
fn the_outage_reason_reaches_the_writer_through_the_cell() {
    let cell = crate::sync::SettledIdentity::default();
    assert_eq!(cell.last_error(), None, "nothing has failed yet");

    cell.set_last_error(Some("connection refused".into()));
    assert_eq!(cell.last_error().as_deref(), Some("connection refused"));
}

/// A successful connection clears it, so a refusal never quotes an outage that
/// is over. The clear rides on `settle` rather than being a second call the
/// connection loop has to remember.
#[tokio::test]
async fn connecting_clears_the_last_outage() {
    let cell = crate::sync::SettledIdentity::default();
    cell.set_last_error(Some("connection refused".into()));

    cell.settle("user-me");

    assert_eq!(cell.last_error(), None);
    assert_eq!(cell.user().await.unwrap().as_deref(), Some("user-me"));
}
