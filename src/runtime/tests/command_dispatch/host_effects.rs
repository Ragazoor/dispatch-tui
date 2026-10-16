use super::*;

// --- SystemCommand -----------------------------------------------------

#[tokio::test]
async fn dispatch_system_send_notification_shells_out_to_notify_send() {
    let mut h = harness(MockProcessRunner::new(vec![MockProcessRunner::ok()])).await;

    h.dispatch(Command::System(SystemCommand::SendNotification {
        title: "Task #1: Fix bug".into(),
        body: "Ready for review".into(),
        urgent: true,
    }))
    .await;

    let call = h.await_call("notify-send").await;
    assert!(call.contains("Task #1: Fix bug"), "got: {call}");
    assert!(call.contains("Ready for review"), "got: {call}");
    assert!(
        call.contains("critical"),
        "an urgent notification must carry the critical urgency: {call}"
    );
}

#[tokio::test]
async fn dispatch_system_open_in_browser_shells_out_to_xdg_open() {
    let mut h = harness(MockProcessRunner::new(vec![MockProcessRunner::ok()])).await;

    h.dispatch(Command::System(SystemCommand::OpenInBrowser {
        url: "https://github.com/org/repo/pull/1".into(),
    }))
    .await;

    let call = h.await_call("xdg-open").await;
    assert!(
        call.contains("https://github.com/org/repo/pull/1"),
        "got: {call}"
    );
}

// --- PrCommand ---------------------------------------------------------

#[tokio::test]
async fn dispatch_pr_check_status_reports_the_state_gh_returned() {
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"OPEN\nAPPROVED\n"),
    ]))
    .await;

    h.dispatch(Command::Pr(PrCommand::CheckStatus {
        id: TaskId(1),
        url: "https://github.com/org/repo/pull/42".into(),
    }))
    .await;

    match h.next_msg().await {
        Message::Pr(crate::tui::messages::PrMessage::ReviewState(id)) => {
            assert_eq!(id, TaskId(1));
        }
        other => panic!("expected a PR review state, got {other:?}"),
    }
}

/// `CheckStatusIfOwned`, the host-less path (`pr-workflow.allium:
/// PollPrStatus`, "Host scoping"): on a single-machine test harness (no
/// shared store attached) `BoardReads::poll_owner` always answers unclaimed
/// and `PollOwnershipStore::claim_poll_owner` is a harmless no-op, so the
/// claim-then-proceed path must still reach the same `gh` check and report
/// the same result `CheckStatus` does — the ownership check must never
/// silently swallow a poll on an install with nothing to contend it against.
#[tokio::test]
async fn dispatch_pr_check_status_if_owned_polls_when_unclaimed() {
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"OPEN\nAPPROVED\n"),
    ]))
    .await;

    h.dispatch(Command::Pr(PrCommand::CheckStatusIfOwned {
        id: TaskId(1),
        url: "https://github.com/org/repo/pull/42".into(),
    }))
    .await;

    match h.next_msg().await {
        Message::Pr(crate::tui::messages::PrMessage::ReviewState(id)) => {
            assert_eq!(id, TaskId(1));
        }
        other => panic!("expected a PR review state, got {other:?}"),
    }
}

// --- RepoSyncCommand ---------------------------------------------------

#[tokio::test]
async fn dispatch_repo_sync_refresh_reports_the_measured_drift() {
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"),
        MockProcessRunner::ok(),                      // fetch
        MockProcessRunner::ok_with_stdout(b"3\t1\n"), // rev-list
    ]))
    .await;

    h.dispatch(Command::RepoSync(RepoSyncCommand::Refresh {
        repo_path: "/repo".into(),
        fetch_first: true,
    }))
    .await;

    match h.next_msg().await {
        Message::RepoSync(crate::tui::messages::RepoSyncMessage::Measured(m)) => {
            assert_eq!(m.repo_path, "/repo");
            assert_eq!(
                m.counts,
                Some(crate::repo_sync::AheadBehind {
                    ahead: 3,
                    behind: 1
                })
            );
        }
        other => panic!("expected a repo-sync measurement, got {other:?}"),
    }
}

/// The two `RepoSync` arms are one wire apart, so the sync arm is pinned by
/// an outcome the refresh arm cannot produce: a `Failed`.
#[tokio::test]
async fn dispatch_repo_sync_sync_reports_a_failure_the_refresh_arm_cannot() {
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"),
        MockProcessRunner::ok_with_stdout(b"feature\n"), // not on the base branch
    ]))
    .await;

    h.dispatch(Command::RepoSync(RepoSyncCommand::Sync {
        repo_path: "/repo".into(),
        base_branch: "main".into(),
    }))
    .await;

    match h.next_msg().await {
        Message::RepoSync(crate::tui::messages::RepoSyncMessage::Failed {
            repo_path,
            detail,
            retryable,
        }) => {
            assert_eq!(repo_path, "/repo");
            assert!(
                detail.contains("feature") && detail.contains("main"),
                "the branch found and the one expected: {detail}"
            );
            assert!(!retryable, "the operator must check out main first");
        }
        other => panic!("expected a sync failure, got {other:?}"),
    }
}

// --- FeedCommand -------------------------------------------------------

#[tokio::test]
async fn dispatch_feed_trigger_epic_runs_the_epics_own_feed_command() {
    let mut h = quiet_harness().await;
    let epic =
        h.db.create_epic("Security Vulnerabilities", "", None)
            .await
            .unwrap();
    set_feed_command(
            &h.db,
            epic.id,
            r#"echo '[{"external_id":"vuln:1","title":"CVE-1","description":"d","status":"backlog","tag":"fix"}]'"#,
        )
        .await;

    h.dispatch(Command::Feed(FeedCommand::TriggerEpic {
        epic_id: epic.id,
        epic_title: "Security Vulnerabilities".into(),
    }))
    .await;

    let msg = h.next_msg().await;
    assert_feed_failed_because_not_applicable(&msg);
    assert!(
        matches!(
            msg,
            Message::Feed(crate::tui::messages::FeedMessage::Refreshed { count: 1, .. })
        ),
        "expected one synced item, got: {msg:?}"
    );
    assert_eq!(
        h.db.list_tasks_for_epic(epic.id).await.len(),
        1,
        "the cycle must have upserted the emitted item"
    );
}

/// A feed cycle that failed for a *configuration* reason never reached the
/// command, so it proves nothing about the wire. Kept as its own assertion
/// rather than folded into the match so the failure names the cause.
fn assert_feed_failed_because_not_applicable(msg: &Message) {
    if let Message::Feed(crate::tui::messages::FeedMessage::Failed { error, .. }) = msg {
        panic!("the feed cycle failed before it ran the command: {error}");
    }
}

#[tokio::test]
async fn dispatch_learning_archive_stale_sweeps_through_the_learning_service() {
    let mut h = quiet_harness().await;
    let svc = Arc::new(CountingSweep {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    h.rt.learning_svc = svc.clone();

    h.dispatch(Command::Learning(
        crate::tui::commands::LearningCommand::ArchiveStale,
    ))
    .await;

    assert_eq!(
        svc.calls.load(Ordering::Relaxed),
        1,
        "the arm must reach the injected learning seam exactly once"
    );
}

// --- UsageCommand ------------------------------------------------------

/// Await the fire-and-forget usage write by re-reading until it lands.
///
/// `Usage::Record` spawns its write and keeps no handle, sends no message
/// and has no test hook, so the row appearing is the only observable there
/// is. Each iteration awaits a real DB round-trip — which yields to the
/// runtime rather than spinning — and the whole loop is bounded by
/// `TEST_TIMEOUT` structurally, never by the wall clock (docs/conventions.md,
/// "No `tokio::time::sleep` in tests").
async fn await_usage_row(rt: &TuiRuntime, action: &str) -> crate::models::UsageSummary {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            let rows = rt
                .database
                .query_usage(&store::UsageQuery::default())
                .await
                .unwrap();
            if let Some(row) = rows.into_iter().find(|r| r.action == action) {
                return row;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the spawned usage write must land well within the timeout")
}

#[tokio::test]
async fn dispatch_usage_record_writes_the_event_in_the_background() {
    let mut h = quiet_harness().await;

    h.dispatch(Command::Usage(crate::tui::commands::UsageCommand::Record(
        crate::models::UsageEvent {
            category: crate::models::UsageCategory::Keybinding,
            action: "move_task_right".into(),
            detail: Some("l".into()),
            actor: crate::models::UsageActor::Human,
        },
    )))
    .await;

    let row = await_usage_row(&h.rt, "move_task_right").await;
    assert_eq!(row.detail.as_deref(), Some("l"));
    assert_eq!(row.count, 1);
}

// --- BudgetCommand -----------------------------------------------------

#[tokio::test]
async fn dispatch_budget_refresh_reads_the_snapshot_file_off_the_event_loop() {
    let mut h = quiet_harness().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rate-limits.json");
    std::fs::write(
        &path,
        r#"{"five_hour":{"used_percentage":42.5,"resets_at":1000},"captured_at":900}"#,
    )
    .unwrap();
    h.rt.budget_snapshot_path = path;

    h.dispatch(Command::Budget(
        crate::tui::commands::BudgetCommand::Refresh,
    ))
    .await;

    match h.next_msg().await {
        Message::Budget(crate::tui::messages::BudgetMessage::Updated(Some(snapshot))) => {
            assert_eq!(snapshot.captured_at, 900);
            assert_eq!(
                snapshot.five_hour.map(|w| w.used_percentage),
                Some(42.5),
                "the parsed window must be the one on disk"
            );
        }
        other => panic!("expected a parsed budget snapshot, got {other:?}"),
    }
}
