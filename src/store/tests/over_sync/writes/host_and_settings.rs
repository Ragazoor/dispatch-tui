use super::*;

// ---------------------------------------------------------------------------
// Phase 6c: feed ingestion and the stragglers
// ---------------------------------------------------------------------------

/// A feed-managed task row in `epic`, as the subscription would deliver it.
fn feed_task_row(id: i64, epic: i64, external_id: &str) -> bindings::Task {
    let mut row = crate::sync::encode::create_task_row(
        &CreateTaskRequest {
            epic_id: Some(EpicId(epic)),
            ..a_request()
        },
        "",
        "user-me",
        AT_STORED,
    );
    row.id = id;
    row.external_id = external_id.to_string();
    row
}

fn a_feed_item(external_id: &str, title: &str) -> crate::models::FeedItem {
    crate::models::FeedItem {
        external_id: external_id.to_string(),
        title: title.to_string(),
        description: String::new(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Bug,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }
}

/// The core of the predict-then-verify design (this task's plan doc, decision
/// 1): a candidate the mock reducer actually removed is reported; a candidate
/// that merely matched the stale predicate but SURVIVED the call (simulating
/// a race) is silently dropped, never reported as removed.
#[tokio::test]
async fn a_confirmed_removal_is_reported_but_a_raced_survivor_is_not() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    rows.upsert_task(&feed_task_row(1, 1, "gone"));
    rows.upsert_task(&feed_task_row(2, 1, "raced"));
    rows.upsert_task(&feed_task_row(3, 1, "kept"));
    // The mock reducer call removes only id 1 — id 2 is a stale candidate by
    // the predicate but the "reducer" (the mock) never actually deletes it,
    // simulating a concurrent write that un-staled it between the pre-read
    // and the call.
    let caller = RecordingCaller::removing_on_feed_write(rows.clone(), &[1]);
    let (writer, sent) = writer_over(rows, caller);

    let outcome = writer
        .ingest_feed(
            EpicId(1),
            &[a_feed_item("kept", "kept")],
            &["/repo".to_string()],
            &["main".to_string()],
            false,
        )
        .await
        .unwrap();

    assert_eq!(
        outcome.removed.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![TaskId(1)],
        "only the CONFIRMED-absent candidate is reported, never a survivor"
    );
    assert_eq!(
        sent.sent(),
        vec![Sent::IngestFeed(1, 1, false, "user-me".to_string())],
        "the wire item count is what was sent, not the candidate count"
    );
}

/// `ingest_feed` is one reducer call carrying the additive flag and the
/// caller's identity; a reconciling call reports the rows the module removed
/// that this board predicted, an additive one reports none.
#[tokio::test]
async fn ingest_feed_is_one_call_and_reports_only_confirmed_removals() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    rows.upsert_task(&feed_task_row(1, 1, "absent-from-emission"));
    let caller = RecordingCaller::removing_on_feed_write(rows.clone(), &[1]);
    let (writer, sent) = writer_over(rows, caller);

    let outcome = writer
        .ingest_feed(EpicId(1), &[], &[], &[], false)
        .await
        .unwrap();

    assert_eq!(outcome.removed.len(), 1);
    assert_eq!(outcome.affected_epics, vec![EpicId(1)]);
    assert_eq!(
        sent.sent(),
        vec![Sent::IngestFeed(1, 0, false, "user-me".to_string())]
    );
}

/// A feed sync must not fail just because this install has never connected to
/// a shared store before — best-effort, unlike every `create_*` call that
/// uses `require_identity`. `created_by` is honestly empty rather than the
/// whole sync being refused. `core.allium: Task.created_by`;
/// `feeds.allium: IngestFeedEmission`.
#[tokio::test]
async fn feed_upsert_with_no_settled_identity_still_applies_with_an_empty_created_by() {
    let (writer, sent) = writer_with_no_identity(RecordingCaller::default());

    writer
        .ingest_feed(
            EpicId(1),
            &[a_feed_item("kept", "kept")],
            &["/repo".to_string()],
            &["main".to_string()],
            false,
        )
        .await
        .unwrap();

    assert_eq!(
        sent.sent(),
        vec![Sent::IngestFeed(1, 1, false, String::new())],
        "no settled identity stamps an empty created_by, not a refusal"
    );
}

/// An empty batch never reaches the transport at all — it saves a round trip
/// for the tick's common case of nothing having changed.
#[tokio::test]
async fn an_empty_sub_status_batch_never_calls_the_writer() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer.batch_patch_sub_status(&[]).await.unwrap();

    assert!(caller.sent().is_empty());
}

#[tokio::test]
async fn batch_patch_sub_status_sends_the_whole_batch_in_one_call() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .batch_patch_sub_status(&[
            (TaskId(1), SubStatus::Active),
            (TaskId(2), SubStatus::Stale),
        ])
        .await
        .unwrap();

    assert_eq!(caller.sent(), vec![Sent::BatchPatchSubStatus(2)]);
}

/// The successor is stamped with this connection's identity and the labels
/// the caller passed — not `encode::create_task_row`'s empty default.
#[tokio::test]
async fn respawn_phoenix_successor_stamps_the_identity_and_the_labels() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    let id = writer
        .respawn_phoenix_successor(TaskId(1), a_request(), &["a".to_string(), "b".to_string()])
        .await
        .unwrap();

    assert_eq!(id, TaskId(43));
    let sent = caller.sent();
    match sent.as_slice() {
        [Sent::RespawnPhoenixSuccessor(predecessor, row)] => {
            assert_eq!(*predecessor, 1);
            assert_eq!(row.owner, "user-me");
            assert_eq!(row.labels, "[\"a\",\"b\"]");
        }
        other => panic!("expected one RespawnPhoenixSuccessor call, got {other:?}"),
    }
}

#[tokio::test]
async fn a_board_with_no_identity_cannot_respawn_a_phoenix_successor() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    assert!(writer
        .respawn_phoenix_successor(TaskId(1), a_request(), &[])
        .await
        .is_err());
    assert!(caller.sent().is_empty());
}

// -- Host registry (Phase 6c) -------------------------------------------------
//
// `push_host_registration` is NOT a store method — see its doc comment — so it
// is exercised directly rather than through `Store`, against the same `RecordingCaller` every other
// transport-level test here uses.

/// The ordinary case: the call reaches the caller with exactly the row
/// passed in.
#[tokio::test]
async fn push_host_registration_sends_the_row() {
    let caller = RecordingCaller::returning(&[]);

    push_host_registration(
        &caller,
        "host-1".to_string(),
        "My Laptop".to_string(),
        "user-me".to_string(),
    )
    .await;

    assert_eq!(
        caller.sent(),
        vec![Sent::RegisterHost(
            "host-1".to_string(),
            "My Laptop".to_string(),
            "user-me".to_string()
        )]
    );
}

/// Best-effort: a transport failure does not panic or propagate — there is
/// nothing to propagate TO, since this runs fire-and-forget from the
/// connection loop and the rename call site.
#[tokio::test]
async fn push_host_registration_swallows_a_transport_failure() {
    let caller = RecordingCaller::refusing("store unreachable");

    // Would panic on an unhandled `Err` if this propagated instead of
    // logging — the assertion is that this line completes at all.
    push_host_registration(
        &caller,
        "host-1".to_string(),
        String::new(),
        "user-me".to_string(),
    )
    .await;
}

/// Best-effort: a store REFUSAL (a reachable store that said no) is also
/// swallowed rather than surfaced.
#[tokio::test]
async fn push_host_registration_swallows_a_refusal() {
    let caller = RecordingCaller::rejecting();

    push_host_registration(
        &caller,
        "host-1".to_string(),
        String::new(),
        "user-me".to_string(),
    )
    .await;
}

/// `sync.allium: RegisterHostOnConnect` registers only once the identity has
/// settled, and always as that identity: an empty owner is never sent. The
/// store refuses a NEW ownerless host anyway (`store-split.allium`, D11), and
/// waiting for the next settle is what leaves the id free for its owner.
#[tokio::test]
async fn push_host_registration_never_sends_an_ownerless_row() {
    let caller = RecordingCaller::returning(&[]);

    push_host_registration(
        &caller,
        "host-1".to_string(),
        "label".to_string(),
        String::new(),
    )
    .await;

    assert_eq!(
        caller.sent(),
        vec![],
        "an unsettled identity registers nothing"
    );
}

// -- Settings (Phase 9) -------------------------------------------------------
//
// `host` is never a caller-supplied argument on `SettingsStore` — see
// `Store.host`'s doc comment — so the load-bearing assertion here is not "the call
// reaches the store" (every other routed method already proves that shape);
// it is that the host id the store SEES is this writer's own, with no way for
// a caller to name a different one.

#[tokio::test]
async fn save_setting_is_scoped_to_this_writers_own_host() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.set_setting_string("theme", "dark").await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::SaveSetting(
            "host-me".to_string(),
            "theme".to_string(),
            "dark".to_string()
        )]
    );
}

#[tokio::test]
async fn clear_setting_is_scoped_to_this_writers_own_host() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.set_reviews_feed_command(None).await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::ClearSetting(
            "host-me".to_string(),
            "reviews_feed_command".to_string()
        )]
    );
}

/// Neither of these requires a settled identity — unlike `create_task`/
/// `create_epic`, host is known before any connection settles
/// one (`host.allium: MintHostIdentity`).
#[tokio::test]
async fn settings_route_with_no_identity_settled() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    writer.set_setting_string("theme", "dark").await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::SaveSetting(
            "host-me".to_string(),
            "theme".to_string(),
            "dark".to_string()
        )]
    );
}

// -- Coverage gaps closed by task #16686 --------------------------------------

/// A subscribe sends the subscriber and the epic as given; a refusal is an
/// error, because following an epic that did not take effect is not a success.
#[tokio::test]
async fn subscribing_to_an_epic_sends_it_and_a_refusal_is_an_error() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer
        .subscribe_to_epic("user-me", EpicId(7))
        .await
        .unwrap();

    assert_eq!(caller.sent(), [Sent::Subscribe("user-me".to_string(), 7)]);

    let (refusing, _) = writer_with(RecordingCaller::rejecting());
    assert!(refusing
        .subscribe_to_epic("user-me", EpicId(7))
        .await
        .is_err());
}

/// The production identity starts unsettled and answers with the name the
/// store gave once the connection settles.
#[tokio::test]
async fn the_settled_identity_is_none_until_the_store_names_us() {
    let identity = crate::sync::SettledIdentity::default();

    assert_eq!(identity.user().await.unwrap(), None);

    identity.settle("user-me");

    assert_eq!(identity.user().await.unwrap(), Some("user-me".to_string()));
}

/// EVERY STORE MUTATION REACHES THE TRANSPORT, checked one call at a time.
///
/// A store trait method whose body forgets its reducer call compiles and
/// passes every test that only reads it back from rows nothing wrote. This
/// calls each mutation once, through the store traits, and names the reducer
/// call each one produced — in order, so a method that sends nothing or sends
/// the wrong call shows up as a mismatch at its position.
///
/// The epic chain's candidate loop is absent: it is the service's, not a
/// store method, and `a_chain_refused_on_every_candidate_requests_nothing` and
/// its neighbours in `src/store/tests/tasks_patch.rs` cover it.
#[tokio::test]
async fn every_store_mutation_reaches_the_transport() {
    let (db, caller) = writer_with(RecordingCaller::default());
    let now = at();

    db.create_task(a_request()).await.unwrap();
    db.patch_task(TaskId(1), &TaskPatch::new().title("t"))
        .await
        .unwrap();
    db.set_task_epic_id(TaskId(1), Some(EpicId(2)))
        .await
        .unwrap();
    db.delete_task(TaskId(1)).await.unwrap();
    // The default caller reads no id back, which the store reports as an
    // error AFTER the call reached the transport — the only part asserted here.
    let _ = crate::store::DispatchRequestStore::request_dispatch(
        &db,
        TaskId(1),
        crate::models::DispatchOrigin::User,
        None,
    )
    .await;
    {
        use crate::store::DispatchRequestStore;
        let request = crate::models::DispatchRequestId(1);
        db.claim_dispatch_request(request).await.unwrap();
        db.report_dispatch_provisioned(
            request,
            "/wt",
            &crate::models::TmuxWindow::for_task(TaskId(1)),
            false,
        )
        .await
        .unwrap();
        db.report_dispatch_failed(request, "boom").await.unwrap();
        db.fail_interrupted_dispatch_requests().await.unwrap();
    }
    {
        use crate::models::{SessionRequestId, WrapUpMode};
        use crate::store::SessionRequestStore;
        let request = SessionRequestId(1);
        // The default caller reads no id back: the call reached the transport.
        let _ = db.request_wrap_up(TaskId(1), WrapUpMode::Rebase).await;
        db.claim_session_request(request).await.unwrap();
        db.report_session_request_done(request).await.unwrap();
        db.report_session_request_failed(request, "boom", false)
            .await
            .unwrap();
        let _ = db
            .request_exit_session(TaskId(1), request, WrapUpMode::Done, None)
            .await;
        db.close_task(TaskId(1)).await.unwrap();
        db.fail_interrupted_session_requests().await.unwrap();
    }

    db.create_epic("E", "", None).await.unwrap();
    db.patch_epic(EpicId(1), &crate::models::EpicPatch::new().title("e"))
        .await
        .unwrap();
    db.regroup_epic(EpicId(1)).await.unwrap();
    db.flatten_epic(EpicId(1)).await.unwrap();
    db.delete_epic(EpicId(1)).await.unwrap();
    db.batch_delete(&[TaskId(1)], &[EpicId(1)]).await.unwrap();

    db.save_repo_path("/repo").await.unwrap();
    db.set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();
    db.record_base_branch("/repo", "main").await.unwrap();
    db.delete_repo_path("/repo").await.unwrap();

    db.subscribe_to_epic("user-me", EpicId(1)).await.unwrap();
    db.unsubscribe_from_epic("user-me", EpicId(1))
        .await
        .unwrap();

    db.set_setting_bool("notifications", true).await.unwrap();
    db.set_setting_string("theme", "dark").await.unwrap();
    db.set_reviews_feed_command(None).await.unwrap();

    db.subagent_start(TaskId(1), "agent-1", "session-1", now)
        .await
        .unwrap();
    db.subagent_stop(TaskId(1), "agent-1", "session-1")
        .await
        .unwrap();
    db.subagent_clear(TaskId(1)).await.unwrap();
    db.subagent_clear_and_void_pending_stop(TaskId(1))
        .await
        .unwrap();
    db.try_record_stop(TaskId(1), now).await.unwrap();
    db.record_pre_tool_use(TaskId(1), SubStatus::Active, now)
        .await
        .unwrap();
    db.record_notification(TaskId(1), NotificationWrite::Raise, now)
        .await
        .unwrap();
    db.record_user_prompt_submit(TaskId(1), now).await.unwrap();
    db.mark_pr_learnings_gate_shown(TaskId(1)).await.unwrap();

    db.ingest_feed(EpicId(1), &[], &[], &[], false)
        .await
        .unwrap();
    db.ensure_managed_epics(Some("reviews.sh"), None, None, None)
        .await
        .unwrap();

    db.batch_patch_sub_status(&[(TaskId(1), SubStatus::Active)])
        .await
        .unwrap();
    db.respawn_phoenix_successor(TaskId(1), a_request(), &[])
        .await
        .unwrap();

    db.claim_poll_owner(crate::models::PollScopeId::Epic(EpicId(1)))
        .await
        .unwrap();
    db.override_poll_owner(crate::models::PollScopeId::Epic(EpicId(1)))
        .await
        .unwrap();

    db.create_learning(crate::models::CreateLearningRow {
        kind: crate::models::LearningKind::Landscape,
        summary: "a learning",
        detail: None,
        scope: crate::models::LearningScope::Repo,
        scope_ref: Some("/repo"),
        tags: &[],
        source_task_id: None,
        embedding: None,
    })
    .await
    .unwrap();
    db.patch_learning(
        LearningId(1),
        &crate::models::LearningPatch::new().summary("s"),
    )
    .await
    .unwrap();
    db.delete_learning(LearningId(1)).await.unwrap();
    db.rescope_epic_learnings(EpicId(1), EpicId(2))
        .await
        .unwrap();
    db.archive_stale_learnings(now).await.unwrap();
    db.record_retrieval(TaskId(1), LearningId(1), RetrievalSource::QueryLearnings)
        .await
        .unwrap();
    db.apply_verdicts_tx(&[(LearningId(1), crate::models::LearningVerdict::Helped)])
        .await
        .unwrap();

    db.record_usage_event(&crate::models::UsageEvent {
        category: crate::models::UsageCategory::Keybinding,
        action: "dispatch_task".to_string(),
        detail: None,
        actor: crate::models::UsageActor::Human,
    })
    .await
    .unwrap();

    db.create_work_package("W", "").await.unwrap();
    db.patch_work_package(WorkPackageId(1), Some("W2"), None)
        .await
        .unwrap();
    db.link_work_package_item(WorkPackageId(1), WorkPackageItem::Task(TaskId(1)))
        .await
        .unwrap();
    db.unlink_work_package_item(WorkPackageId(1), WorkPackageItem::Epic(EpicId(1)))
        .await
        .unwrap();
    db.delete_work_package(WorkPackageId(1)).await.unwrap();

    let names: Vec<String> = caller
        .sent()
        .iter()
        .map(|s| {
            let debug = format!("{s:?}");
            debug
                .split(|c: char| !c.is_alphanumeric())
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert_eq!(
        names,
        vec![
            "CreateTask",
            "PatchTask",
            "SetTaskEpic",
            "DeleteTask",
            "RequestDispatch",
            "ClaimDispatchRequest",
            "ReportDispatchProvisioned",
            "ReportDispatchFailed",
            "FailInterruptedDispatchRequests",
            "RequestWrapUp",
            "ClaimSessionRequest",
            "ReportSessionRequestDone",
            "ReportSessionRequestFailed",
            "RequestExitSession",
            "CloseTask",
            "FailInterruptedSessionRequests",
            "CreateEpic",
            "PatchEpic",
            "Regroup",
            "Flatten",
            "DeleteEpic",
            "BatchDelete",
            "SaveRepoPath",
            "SetVerifyCommand",
            "RecordBaseBranch",
            "DeleteRepoPath",
            "Subscribe",
            "Unsubscribe",
            "SaveSetting",
            "SaveSetting",
            "ClearSetting",
            "SubagentStart",
            "SubagentStop",
            "SubagentClear",
            "SubagentClearAndVoidPendingStop",
            "TryRecordStop",
            "RecordPreToolUse",
            "RecordNotification",
            "RecordUserPromptSubmit",
            "MarkPrLearningsGateShown",
            "IngestFeed",
            "EnsureManagedEpics",
            "BatchPatchSubStatus",
            "RespawnPhoenixSuccessor",
            "ClaimPollOwner",
            "OverridePollOwner",
            "CreateLearning",
            "PatchLearning",
            "DeleteLearning",
            "RescopeEpicLearnings",
            "ArchiveStaleLearnings",
            "RecordLearningRetrieval",
            "ApplyLearningVerdicts",
            "RecordUsageEvent",
            "CreateWorkPackage",
            "PatchWorkPackage",
            "LinkWorkPackageItem",
            "UnlinkWorkPackageItem",
            "DeleteWorkPackage",
        ]
    );
}
