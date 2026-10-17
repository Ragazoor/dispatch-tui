use super::*;

// -- record_hook_event ---------------------------------------------------

/// Move a freshly created backlog task into the Running state with a custom
/// sub_status. Used by the hook-event tests to set up scenarios where a
/// hook arrives at a Running task already in NeedsInput / Active.
async fn create_running_task(svc: &TaskService, sub_status: SubStatus) -> TaskId {
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();
    // The hook facts are the current agent's (agents A4).
    svc.db.give_agent(id);
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .sub_status(sub_status),
    )
    .await
    .unwrap();
    id
}

// -- record_peer_message_sent (task #4098: hook-observed native SendMessage) -

#[tokio::test]
async fn record_peer_message_sent_stamps_sender_and_resolved_target() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let sender = svc.create_task(make_task_params("/repo")).await.unwrap();
    let target = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.record_peer_message_sent(sender, &format!("task-{}", target.0))
        .await
        .unwrap();

    let sender_task = svc.get_task(sender).await.unwrap();
    let target_task = svc.get_task(target).await.unwrap();
    assert!(
        sender_task.last_peer_message_sent_at.is_some(),
        "sender's own row should be stamped"
    );
    assert!(
        target_task.last_peer_message_received_at.is_some(),
        "resolved target's row should be stamped"
    );
    assert!(sender_task.last_peer_message_received_at.is_none());
    assert!(target_task.last_peer_message_sent_at.is_none());
}

/// `AgentSessionIsNamedAfterTheAgent`: a session is addressed by its agent's
/// name, so that is what `SendMessage`'s `to` carries.
#[tokio::test]
async fn record_peer_message_sent_resolves_an_agent_name_to_its_task() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let sender = svc.create_task(make_task_params("/repo")).await.unwrap();
    let target = svc.create_task(make_task_params("/repo")).await.unwrap();
    svc.request_dispatch(target, crate::models::DispatchOrigin::Mcp, None)
        .await
        .unwrap();
    let name = db.current_agent(target).await.unwrap().name;

    svc.record_peer_message_sent(sender, &format!("{name} [3fa9c1]"))
        .await
        .unwrap();

    assert!(db
        .get_task(target)
        .await
        .unwrap()
        .last_peer_message_received_at
        .is_some());
    assert!(db
        .get_task(sender)
        .await
        .unwrap()
        .last_peer_message_received_at
        .is_none());
}

#[tokio::test]
async fn record_peer_message_sent_ignores_a_name_outside_dispatchs_own_naming_convention() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let sender = svc.create_task(make_task_params("/repo")).await.unwrap();

    // Not a "task-<id>" name at all — a message to some other local session
    // dispatch didn't launch. Must not error.
    svc.record_peer_message_sent(sender, "my-other-terminal-3f")
        .await
        .unwrap();

    let sender_task = svc.get_task(sender).await.unwrap();
    assert!(
        sender_task.last_peer_message_sent_at.is_some(),
        "sender's own stamp still lands even when the target can't be resolved"
    );
}

#[tokio::test]
async fn record_peer_message_sent_strips_a_disambiguating_ref_suffix() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let sender = svc.create_task(make_task_params("/repo")).await.unwrap();
    let target = svc.create_task(make_task_params("/repo")).await.unwrap();

    // SendMessage's `to` field can carry a disambiguating " [ref]" suffix
    // (per ListAgents' listing format) even though dispatch's task-<id>
    // names are unique by construction and should rarely need one.
    svc.record_peer_message_sent(sender, &format!("task-{} [3fa9c1]", target.0))
        .await
        .unwrap();

    let target_task = svc.get_task(target).await.unwrap();
    assert!(target_task.last_peer_message_received_at.is_some());
}

#[tokio::test]
async fn record_peer_message_sent_ignores_a_task_id_that_no_longer_exists() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let sender = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.record_peer_message_sent(sender, "task-999999")
        .await
        .unwrap();

    let sender_task = svc.get_task(sender).await.unwrap();
    assert!(sender_task.last_peer_message_sent_at.is_some());
}

#[tokio::test]
async fn record_peer_message_sent_errors_when_sender_is_missing() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let err = svc
        .record_peer_message_sent(TaskId(999999), "task-1")
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[tokio::test]
async fn record_hook_event_pre_tool_use_stamps_and_clears_needs_input() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::NeedsInput).await;
    let earlier = chrono::Utc::now() - chrono::Duration::seconds(30);
    db.give_agent(id);
    db.patch_task(
        id,
        &crate::store::TaskPatch::new().last_notification_at(Some(earlier)),
    )
    .await
    .unwrap();

    svc.record_hook_event(id, HookEventKind::PreToolUse)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.sub_status, SubStatus::Active);
    assert!(task.last_pre_tool_use_at.is_some());
}

/// Compat/raise path: a Notification with no `notification_type` (older Claude
/// Code, or the field absent) preserves the historical "always needs_input".
#[tokio::test]
async fn record_hook_event_notification_absent_kind_sets_needs_input_and_stamps() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_hook_event(id, HookEventKind::Notification(None))
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.sub_status, SubStatus::NeedsInput);
    assert!(task.last_notification_at.is_some());
}

/// Raise bucket: permission_prompt / idle_prompt / elicitation_dialog each set
/// needs_input and stamp last_notification_at (a genuine block).
#[tokio::test]
async fn record_hook_event_notification_raise_kinds_set_needs_input() {
    for kind in [
        NotificationKind::PermissionPrompt,
        NotificationKind::IdlePrompt,
        NotificationKind::ElicitationDialog,
    ] {
        let db = test_db().await;
        let svc = task_svc(&db);
        let id = create_running_task(&svc, SubStatus::Active).await;

        svc.record_hook_event(id, HookEventKind::Notification(Some(kind)))
            .await
            .unwrap();

        let task = svc.get_task(id).await.unwrap();
        assert_eq!(task.sub_status, SubStatus::NeedsInput, "kind {kind:?}");
        assert!(task.last_notification_at.is_some(), "kind {kind:?}");
    }
}

/// Clear bucket: elicitation_complete / elicitation_response return the task to
/// active and drop last_notification_at the instant the user answers.
#[tokio::test]
async fn record_hook_event_notification_resolve_kinds_clear_needs_input() {
    for kind in [
        NotificationKind::ElicitationComplete,
        NotificationKind::ElicitationResponse,
    ] {
        let db = test_db().await;
        let svc = task_svc(&db);
        let id = create_running_task(&svc, SubStatus::NeedsInput).await;
        db.give_agent(id);
        db.patch_task(
            id,
            &crate::store::TaskPatch::new().last_notification_at(Some(chrono::Utc::now())),
        )
        .await
        .unwrap();

        svc.record_hook_event(id, HookEventKind::Notification(Some(kind)))
            .await
            .unwrap();

        let task = svc.get_task(id).await.unwrap();
        assert_eq!(task.status, TaskStatus::Running, "kind {kind:?}");
        assert_eq!(task.sub_status, SubStatus::Active, "kind {kind:?}");
        assert!(task.last_notification_at.is_none(), "kind {kind:?}");
    }
}

/// Ignore bucket, conditional arm: an agent with a live subagent is waiting
/// on work it dispatched itself, not on the user, so idle_prompt must not
/// raise needs_input there.
#[tokio::test]
async fn record_hook_event_idle_prompt_is_noop_while_a_subagent_is_live() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;
    db.subagent_start(id, "agent-1", "session-1", chrono::Utc::now())
        .await
        .unwrap();

    svc.record_hook_event(
        id,
        HookEventKind::Notification(Some(NotificationKind::IdlePrompt)),
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.sub_status, SubStatus::Active);
    assert!(task.last_notification_at.is_none());
}

/// The suppression is scoped to idle_prompt alone. A permission decision or a
/// question dialog genuinely needs a human while background work churns, so
/// those still raise — mirroring ClassifyAgentActivity, where needs_input
/// outranks live_subagents for the same reason.
#[tokio::test]
async fn record_hook_event_blocking_kinds_still_raise_while_a_subagent_is_live() {
    for kind in [
        NotificationKind::PermissionPrompt,
        NotificationKind::ElicitationDialog,
    ] {
        let db = test_db().await;
        let svc = task_svc(&db);
        let id = create_running_task(&svc, SubStatus::Active).await;
        db.subagent_start(id, "agent-1", "session-1", chrono::Utc::now())
            .await
            .unwrap();

        svc.record_hook_event(id, HookEventKind::Notification(Some(kind)))
            .await
            .unwrap();

        let task = svc.get_task(id).await.unwrap();
        assert_eq!(task.sub_status, SubStatus::NeedsInput, "kind {kind:?}");
        assert!(task.last_notification_at.is_some(), "kind {kind:?}");
    }
}

/// An absent kind (older Claude Code) is not demoted either: it may be a
/// permission prompt, and a false Blocked is cheaper than a missed real one.
#[tokio::test]
async fn record_hook_event_absent_kind_still_raises_while_a_subagent_is_live() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;
    db.subagent_start(id, "agent-1", "session-1", chrono::Utc::now())
        .await
        .unwrap();

    svc.record_hook_event(id, HookEventKind::Notification(None))
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.sub_status, SubStatus::NeedsInput);
    assert!(task.last_notification_at.is_some());
}

/// With no background work of its own, an idle agent is genuinely waiting on
/// the user, so idle_prompt raises exactly as before.
#[tokio::test]
async fn record_hook_event_idle_prompt_still_raises_with_no_live_work() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_hook_event(
        id,
        HookEventKind::Notification(Some(NotificationKind::IdlePrompt)),
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.sub_status, SubStatus::NeedsInput);
    assert!(task.last_notification_at.is_some());
}

/// The suppressed idle_prompt must also be a *pure* no-op: it must not clear a
/// needs_input a real permission prompt raised moments earlier, which would
/// hide a genuine block behind an unrelated idle toast.
#[tokio::test]
async fn record_hook_event_suppressed_idle_prompt_does_not_clobber_needs_input() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::NeedsInput).await;
    db.give_agent(id);
    db.patch_task(
        id,
        &crate::store::TaskPatch::new().last_notification_at(Some(chrono::Utc::now())),
    )
    .await
    .unwrap();
    db.subagent_start(id, "agent-1", "session-1", chrono::Utc::now())
        .await
        .unwrap();

    svc.record_hook_event(
        id,
        HookEventKind::Notification(Some(NotificationKind::IdlePrompt)),
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.sub_status, SubStatus::NeedsInput);
    assert!(task.last_notification_at.is_some());
}

/// Ignore bucket: auth_success is informational — it must not change an active
/// task's sub_status and must not stamp last_notification_at.
#[tokio::test]
async fn record_hook_event_notification_auth_success_is_noop_on_active() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_hook_event(
        id,
        HookEventKind::Notification(Some(NotificationKind::AuthSuccess)),
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.sub_status, SubStatus::Active);
    assert!(task.last_notification_at.is_none());
}

/// Ignore bucket must be a *pure* no-op: an auth_success arriving while the task
/// is already needs_input must not clobber that state back to active.
#[tokio::test]
async fn record_hook_event_notification_auth_success_does_not_clobber_needs_input() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::NeedsInput).await;
    let stamped = chrono::Utc::now();
    db.give_agent(id);
    db.patch_task(
        id,
        &crate::store::TaskPatch::new().last_notification_at(Some(stamped)),
    )
    .await
    .unwrap();

    svc.record_hook_event(
        id,
        HookEventKind::Notification(Some(NotificationKind::AuthSuccess)),
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.sub_status, SubStatus::NeedsInput);
    assert!(task.last_notification_at.is_some());
}

#[tokio::test]
async fn record_hook_event_stop_transitions_to_review_and_clears_stamps() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;
    let now = chrono::Utc::now();
    db.give_agent(id);
    db.patch_task(
        id,
        &crate::store::TaskPatch::new()
            .last_pre_tool_use_at(Some(now))
            .last_notification_at(Some(now)),
    )
    .await
    .unwrap();

    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::AwaitingReview);
    assert!(task.last_pre_tool_use_at.is_none());
    assert!(task.last_notification_at.is_none());
}

#[tokio::test]
async fn record_hook_event_noop_for_non_running_task() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc
        .create_task(CreateTaskParams::fixture("T", "/repo"))
        .await
        .unwrap();

    svc.record_hook_event(id, HookEventKind::PreToolUse)
        .await
        .unwrap();
    svc.record_hook_event(id, HookEventKind::Notification(None))
        .await
        .unwrap();
    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert!(task.last_pre_tool_use_at.is_none());
    assert!(task.last_notification_at.is_none());
}

#[tokio::test]
async fn record_hook_event_user_prompt_submit_resumes_review_task_to_running() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;
    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();
    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Review);

    svc.record_hook_event(id, HookEventKind::UserPromptSubmit)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.sub_status, SubStatus::Active);
    assert!(task.last_pre_tool_use_at.is_some());
}

#[tokio::test]
async fn record_hook_event_user_prompt_submit_refreshes_running_task() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::NeedsInput).await;

    svc.record_hook_event(id, HookEventKind::UserPromptSubmit)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.sub_status, SubStatus::Active);
    assert!(task.last_pre_tool_use_at.is_some());
}

#[tokio::test]
async fn record_hook_event_user_prompt_submit_noop_for_backlog_task() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.record_hook_event(id, HookEventKind::UserPromptSubmit)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert!(task.last_pre_tool_use_at.is_none());
}

#[tokio::test]
async fn record_hook_event_unknown_task_returns_not_found() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let err = svc
        .record_hook_event(TaskId(99_999), HookEventKind::PreToolUse)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)), "got {err:?}");
}

// -- record_subagent_event -------------------------------------------------

#[tokio::test]
async fn stop_with_live_subagents_defers_the_review_flip() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_subagent_event(
        id,
        SubagentEvent::Start {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();

    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "a Stop with live subagents must not move the task to Review"
    );
    assert!(task.stop_pending, "the deferred flip must be recorded");
}

#[tokio::test]
async fn last_subagent_stop_performs_the_deferred_flip() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_subagent_event(
        id,
        SubagentEvent::Start {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();
    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();

    svc.record_subagent_event(
        id,
        SubagentEvent::Stop {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "draining the last subagent flips to Review"
    );
    assert!(!task.stop_pending, "the pending bit must be consumed");
    assert!(task.last_pre_tool_use_at.is_none());
    assert!(task.last_notification_at.is_none());
}

#[tokio::test]
async fn subagent_stop_with_others_still_live_does_not_flip() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    for a in ["a1", "a2"] {
        svc.record_subagent_event(
            id,
            SubagentEvent::Start {
                agent_id: a.into(),
                session_id: "s1".into(),
            },
        )
        .await
        .unwrap();
    }
    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();

    svc.record_subagent_event(
        id,
        SubagentEvent::Stop {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "one of two draining must not flip"
    );
    assert!(task.stop_pending);
}

#[tokio::test]
async fn stop_with_no_subagents_flips_immediately_as_before() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "unchanged behaviour when nothing is in flight"
    );
    assert!(!task.stop_pending);
}

#[tokio::test]
async fn subagent_stop_without_a_pending_stop_leaves_status_alone() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_subagent_event(
        id,
        SubagentEvent::Start {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();
    svc.record_subagent_event(
        id,
        SubagentEvent::Stop {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "draining without a pending Stop is not a reason to move to Review"
    );
}

/// The draining `Clear` variant — reached only from detach, which owns no
/// status of its own. `SessionStart` uses `clear_subagents_no_drain` instead.
#[tokio::test]
async fn detach_clear_drains_and_performs_a_pending_flip() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_subagent_event(
        id,
        SubagentEvent::Start {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();
    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();

    svc.record_subagent_event(id, SubagentEvent::Clear)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "a detach removes the agent that would have drained the count, so the \
         deferred Stop must resolve rather than strand"
    );
    assert_eq!(task.live_subagents, 0);
    assert!(!task.stop_pending);
}

/// The non-draining twin, used by `SessionStart` and by the crash /
/// dispatch-claim paths: entries and `stop_pending` go, status stays.
#[tokio::test]
async fn clear_no_drain_voids_a_pending_stop_without_flipping_to_review() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_subagent_event(
        id,
        SubagentEvent::Start {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();
    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();

    svc.clear_subagents_no_drain(id).await.unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "a stale Stop from the previous turn must be voided, not applied"
    );
    assert_eq!(task.live_subagents, 0);
    assert!(!task.stop_pending);
}

/// The interleaving that used to strand a task: the last `SubagentStop`
/// commits *before* the `Stop`, so the drain sees no pending bit and the Stop
/// then lands on a row whose count has already reached zero. Under the old
/// read-decide-write it wrote `stop_pending = true` there, leaving Running +
/// `stop_pending` + zero live subagents with nothing left to resolve it. The
/// Stop's write is now conditional on the committed count, so it flips instead.
#[tokio::test]
async fn a_stop_landing_after_the_last_drain_flips_instead_of_stranding() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_subagent_event(
        id,
        SubagentEvent::Start {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();
    svc.record_subagent_event(
        id,
        SubagentEvent::Stop {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();
    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "the Stop saw a committed count of zero and applied the flip"
    );
    assert!(!task.stop_pending);
    assert_eq!(task.live_subagents, 0);
}

/// Whichever order the two hook processes commit in, the task must end in
/// Review and must never come to rest in the stranded triple. This is the
/// assertion that replaces the retired tick reconciler and its test.
#[tokio::test]
async fn neither_hook_order_can_strand_a_task() {
    for stop_first in [true, false] {
        let db = test_db().await;
        let svc = task_svc(&db);
        let id = create_running_task(&svc, SubStatus::Active).await;

        svc.record_subagent_event(
            id,
            SubagentEvent::Start {
                agent_id: "a1".into(),
                session_id: "s1".into(),
            },
        )
        .await
        .unwrap();

        let drain = SubagentEvent::Stop {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        };
        if stop_first {
            svc.record_hook_event(id, HookEventKind::Stop)
                .await
                .unwrap();
            svc.record_subagent_event(id, drain).await.unwrap();
        } else {
            svc.record_subagent_event(id, drain).await.unwrap();
            svc.record_hook_event(id, HookEventKind::Stop)
                .await
                .unwrap();
        }

        let task = svc.get_task(id).await.unwrap();
        assert_eq!(
            task.status,
            TaskStatus::Review,
            "stop_first = {stop_first} must still end in Review"
        );
        assert!(
            !(task.status == TaskStatus::Running && task.stop_pending && task.live_subagents == 0),
            "stop_first = {stop_first} left the task stranded"
        );
    }
}

/// Park a task in the shape a live fan-out plus a Stop hook leaves behind:
/// Running, one live subagent, and the deferred flip recorded.
async fn running_task_with_a_deferred_stop(svc: &TaskService) -> TaskId {
    let id = create_running_task(svc, SubStatus::Active).await;
    svc.record_subagent_event(
        id,
        SubagentEvent::Start {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();
    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();
    assert!(svc.get_task(id).await.unwrap().stop_pending);
    id
}

#[tokio::test]
async fn update_task_moving_out_of_running_clears_stop_pending() {
    let db = test_db().await;
    let svc = task_svc(&db);

    for target in [TaskStatus::Review, TaskStatus::Backlog] {
        let id = running_task_with_a_deferred_stop(&svc).await;
        svc.update_task(UpdateTaskParams::for_task(id).status(target))
            .await
            .unwrap();
        let task = svc.get_task(id).await.unwrap();
        assert_eq!(task.status, target);
        assert!(
            !task.stop_pending,
            "leaving Running must void the deferred Stop (target {target:?})"
        );
    }
}

#[tokio::test]
async fn update_task_staying_in_running_keeps_stop_pending() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = running_task_with_a_deferred_stop(&svc).await;

    // A redundant Running write, and a write that carries no status at all:
    // neither ends the turn the Stop was deferred under.
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .await
        .unwrap();
    assert!(svc.get_task(id).await.unwrap().stop_pending);

    svc.update_task(UpdateTaskParams::for_task(id).title("renamed".to_string()))
        .await
        .unwrap();
    assert!(svc.get_task(id).await.unwrap().stop_pending);
}

#[tokio::test]
async fn update_task_to_done_clears_stop_pending() {
    let db = test_db().await;
    let svc = task_svc(&db);

    // Done is not in update_task_moving_out_of_running_clears_stop_pending's
    // loop (that one covers Review/Backlog), and it is the transition the
    // finishing-status write path takes.
    let id = running_task_with_a_deferred_stop(&svc).await;
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();
    assert!(!svc.get_task(id).await.unwrap().stop_pending);
}

#[tokio::test]
async fn close_task_clears_stop_pending() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let id = running_task_with_a_deferred_stop(&svc).await;
    svc.close_task(id).await.unwrap();
    assert!(!svc.get_task(id).await.unwrap().stop_pending);
}

/// The reported sequence, end to end (#3847): a Stop deferred during a live
/// fan-out, the card dragged to Review, the subagents finishing, then the card
/// dragged back to Running to keep working. Before the leaving-Running clear
/// the bit survived step two, and the deferred Stop was applied against the
/// new turn — flipping the card straight back to Review, where it stuck,
/// undraggable without a re-dispatch.
#[tokio::test]
async fn moving_back_into_running_after_a_deferred_stop_does_not_re_flip() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = running_task_with_a_deferred_stop(&svc).await;

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Review))
        .await
        .unwrap();

    // The subagent finishes after the move. The drain path is skipped (status
    // is no longer Running), so nothing here clears the bit either.
    svc.record_subagent_event(
        id,
        SubagentEvent::Stop {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .await
        .unwrap();

    // A further drain must find nothing to apply: the bit was voided on the
    // way out of Running, so the new turn does not inherit the old Stop.
    svc.record_subagent_event(
        id,
        SubagentEvent::Stop {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "the card must stay where the human put it"
    );
    assert!(!task.stop_pending);
}

#[tokio::test]
async fn clear_no_drain_on_a_missing_task_is_not_found() {
    let db = test_db().await;
    let svc = task_svc(&db);
    assert!(matches!(
        svc.clear_subagents_no_drain(TaskId(9999)).await,
        Err(crate::service::ServiceError::NotFound(_))
    ));
}

/// The clock is injected and advanced rather than left on the wall clock: the
/// void is conditional on the prompt having fired *after* the Stop it
/// supersedes, so back-to-back calls sharing an instant would tie — and a tie
/// preserves. A human resuming is never that fast; a test is.
#[tokio::test]
async fn user_prompt_submit_voids_a_pending_stop() {
    let db = test_db().await;
    let (svc, clock) = task_svc_with_fixed_clock(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    svc.record_subagent_event(
        id,
        SubagentEvent::Start {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();
    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();
    clock.advance(chrono::Duration::seconds(1));
    svc.record_hook_event(id, HookEventKind::UserPromptSubmit)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert!(
        !task.stop_pending,
        "a human resuming voids the deferred flip"
    );
    assert_eq!(task.status, TaskStatus::Running);
}

/// The late-write race the conditional clear exists for, end to end. Every
/// Claude Code hook is its own process, so a `UserPromptSubmit` write can land
/// after the `Stop` of the very turn that prompt started. Voiding there would
/// delete a Stop no drain can re-create; the task would sit in Running with no
/// agent left to move it. The clock runs backwards between the two calls to
/// stage exactly that ordering without depending on scheduling.
#[tokio::test]
async fn user_prompt_submit_that_lands_after_the_stop_it_races_keeps_it_pending() {
    let db = test_db().await;
    let (svc, clock) = task_svc_with_fixed_clock(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;
    svc.record_subagent_event(
        id,
        SubagentEvent::Start {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();

    // The Stop fires at t2, after the prompt fired at t0 …
    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();
    assert!(svc.get_task(id).await.unwrap().stop_pending);

    // … and only now does the prompt's own write land.
    clock.advance(-chrono::Duration::seconds(1));
    svc.record_hook_event(id, HookEventKind::UserPromptSubmit)
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert!(
        task.stop_pending,
        "the Stop belongs to the turn this prompt started — voiding it would \
         strand the task in Running"
    );
    assert_eq!(task.status, TaskStatus::Running);

    // And the drain still owes the flip.
    svc.record_subagent_event(
        id,
        SubagentEvent::Stop {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();
    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert!(!task.stop_pending);
}

/// Regression test: `live_subagents` is hook-owned, like
/// `last_pre_tool_use_at`. A stale in-memory snapshot riding a generic
/// `update_task` call must not zero it.
#[tokio::test]
async fn generic_persist_does_not_clobber_the_subagent_count() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;

    let snapshot = db.get_task(id).await.unwrap();
    assert_eq!(snapshot.live_subagents, 0);

    db.subagent_start(id, "a1", "s1", chrono::Utc::now())
        .await
        .unwrap();

    // Persist the *stale* snapshot, which still says zero.
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(snapshot.status)
            .sub_status(snapshot.sub_status),
    )
    .await
    .unwrap();

    let reread = db.get_task(id).await.unwrap();
    assert_eq!(
        reread.live_subagents, 1,
        "a generic persist must not overwrite the hook-owned count"
    );
}

/// A `Start` can never drain: it inserts its row before the recount, so the
/// resulting count is never zero. That is why the `Start` arm needs neither the
/// task's status nor its `stop_pending`, and why it must not reach the drain
/// path — asserted here against the hardest input, a task already stranded in
/// Running + `stop_pending` + zero live subagents.
#[tokio::test]
async fn subagent_start_never_drains_a_pending_stop() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = create_running_task(&svc, SubStatus::Active).await;
    // The stranded state directly, rather than reproducing the interleaving via
    // hook events — this test is about the subagent arm, not how it got there.
    db.give_agent(id);
    db.patch_task(id, &crate::store::TaskPatch::new().stop_pending(true))
        .await
        .unwrap();

    svc.record_subagent_event(
        id,
        SubagentEvent::Start {
            agent_id: "a2".into(),
            session_id: "s1".into(),
        },
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "a Start must never apply a deferred Stop"
    );
    assert!(task.stop_pending, "nor consume the pending bit");
    assert_eq!(task.live_subagents, 1);
}

#[tokio::test]
async fn record_subagent_event_unknown_task_returns_not_found() {
    let db = test_db().await;
    let svc = task_svc(&db);
    // All three variants share the contract but no longer share one existence
    // check: Start uses `task_exists`, Stop and Clear read the row they need for
    // the drain decision. A fourth variant must remember to check too.
    for event in [
        SubagentEvent::Start {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
        SubagentEvent::Stop {
            agent_id: "a1".into(),
            session_id: "s1".into(),
        },
        SubagentEvent::Clear,
    ] {
        let err = svc
            .record_subagent_event(TaskId(99_999), event.clone())
            .await
            .unwrap_err();
        assert!(
            matches!(err, ServiceError::NotFound(_)),
            "got {err:?} for {event:?}"
        );
    }
}
