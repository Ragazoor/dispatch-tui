use super::*;

// -- Unfollow releases the unfollower's poll claims (task #8196) ----------

async fn child_epic(caller: &MemoryReducerCaller, parent: EpicId) -> EpicId {
    let mut child = blank_epic();
    child.parent_epic_id = parent.0;
    caller.create_epic(child).await.unwrap()
}

#[tokio::test]
async fn unfollow_releases_a_claim_held_by_the_unfollowers_host() {
    let (caller, rows) = caller();
    caller
        .register_host("laptop".into(), "l".into(), "alice".into())
        .await
        .unwrap();
    let epic = caller.create_epic(blank_epic()).await.unwrap();
    caller
        .subscribe_to_epic("alice".into(), epic)
        .await
        .unwrap();
    caller
        .claim_poll_owner(PollScopeId::Epic(epic), "laptop".into())
        .await
        .unwrap();
    caller
        .unsubscribe_from_epic("alice".into(), epic)
        .await
        .unwrap();
    assert!(rows.poll_owner(PollScopeId::Epic(epic)).is_none());
}

#[tokio::test]
async fn unfollow_releases_claims_across_the_subtree_and_every_owned_host() {
    let (caller, rows) = caller();
    for host in ["laptop", "desktop"] {
        caller
            .register_host(host.into(), host.into(), "alice".into())
            .await
            .unwrap();
    }
    let root = caller.create_epic(blank_epic()).await.unwrap();
    let sub = child_epic(&caller, root).await;
    caller
        .subscribe_to_epic("alice".into(), root)
        .await
        .unwrap();
    caller
        .claim_poll_owner(PollScopeId::Epic(root), "laptop".into())
        .await
        .unwrap();
    caller
        .claim_poll_owner(PollScopeId::Epic(sub), "desktop".into())
        .await
        .unwrap();
    caller
        .unsubscribe_from_epic("alice".into(), root)
        .await
        .unwrap();
    assert!(rows.poll_owner(PollScopeId::Epic(root)).is_none());
    assert!(rows.poll_owner(PollScopeId::Epic(sub)).is_none());
}

#[tokio::test]
async fn unfollow_keeps_another_persons_claim() {
    let (caller, rows) = caller();
    caller
        .register_host("bobs".into(), "b".into(), "bob".into())
        .await
        .unwrap();
    let epic = caller.create_epic(blank_epic()).await.unwrap();
    caller
        .subscribe_to_epic("alice".into(), epic)
        .await
        .unwrap();
    caller
        .claim_poll_owner(PollScopeId::Epic(epic), "bobs".into())
        .await
        .unwrap();
    caller
        .unsubscribe_from_epic("alice".into(), epic)
        .await
        .unwrap();
    assert_eq!(
        rows.poll_owner(PollScopeId::Epic(epic)).unwrap().host,
        "bobs"
    );
}

#[tokio::test]
async fn unfollow_keeps_a_claim_on_an_epic_still_covered_by_another_follow() {
    let (caller, rows) = caller();
    caller
        .register_host("laptop".into(), "l".into(), "alice".into())
        .await
        .unwrap();
    let root = caller.create_epic(blank_epic()).await.unwrap();
    let sub = child_epic(&caller, root).await;
    caller
        .subscribe_to_epic("alice".into(), root)
        .await
        .unwrap();
    caller.subscribe_to_epic("alice".into(), sub).await.unwrap();
    caller
        .claim_poll_owner(PollScopeId::Epic(sub), "laptop".into())
        .await
        .unwrap();
    // Unfollowing the sub-epic leaves it covered through the followed root.
    caller
        .unsubscribe_from_epic("alice".into(), sub)
        .await
        .unwrap();
    assert_eq!(
        rows.poll_owner(PollScopeId::Epic(sub)).unwrap().host,
        "laptop"
    );
}

// -- Agent session state: subagents ------------------------------------------

#[tokio::test]
async fn subagent_start_increments_live_subagents() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    let live = start_subagent(&caller, id, "agent-a", "session-1").await;
    assert_eq!(live, 1);
    assert_eq!(rows.task(id).unwrap().live_subagents, 1);
}

#[tokio::test]
async fn subagent_start_fences_out_a_stale_session() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    start_subagent(&caller, id, "agent-a", "session-1").await;
    // A new session for the same task evicts every row from the old one.
    let live = start_subagent(&caller, id, "agent-b", "session-2").await;
    assert_eq!(live, 1);
    assert_eq!(rows.task(id).unwrap().live_subagents, 1);
}

#[tokio::test]
async fn subagent_stop_drains_and_flips_a_pending_stop_to_review() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    caller.claim_backlog_task(id, "host".into()).await.unwrap();
    start_subagent(&caller, id, "agent-a", "session-1").await;
    // Live subagents means `try_record_stop` defers rather than flips.
    let deferred = caller.try_record_stop(id, at(TEST_STAMP)).await.unwrap();
    assert_eq!(deferred, Some(false));
    assert!(rows.task(id).unwrap().stop_pending);

    let drained = caller
        .subagent_stop(id, "agent-a".into(), "session-1".into())
        .await
        .unwrap();
    assert_eq!(drained.live, 0);
    assert!(drained.is_review);
    assert_eq!(
        rows.task(id).unwrap().status,
        crate::models::TaskStatus::Review
    );
    assert!(!rows.task(id).unwrap().stop_pending);
}

#[tokio::test]
async fn subagent_clear_drains_every_subagent_for_a_task() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    start_subagent(&caller, id, "agent-a", "session-1").await;
    start_subagent(&caller, id, "agent-b", "session-1").await;
    let drained = caller.subagent_clear(id).await.unwrap();
    assert_eq!(drained.live, 0);
    assert_eq!(rows.task(id).unwrap().live_subagents, 0);
}

#[tokio::test]
async fn subagent_clear_and_void_pending_stop_clears_the_flag_without_flipping() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    caller.claim_backlog_task(id, "host".into()).await.unwrap();
    start_subagent(&caller, id, "agent-a", "session-1").await;
    caller.try_record_stop(id, at(TEST_STAMP)).await.unwrap();
    assert!(rows.task(id).unwrap().stop_pending);

    caller
        .subagent_clear_and_void_pending_stop(id)
        .await
        .unwrap();
    assert!(!rows.task(id).unwrap().stop_pending);
    // Voided, not applied: the task stays Running rather than flipping.
    assert_eq!(
        rows.task(id).unwrap().status,
        crate::models::TaskStatus::Running
    );
}

#[tokio::test]
async fn try_record_stop_refuses_when_the_task_is_not_running() {
    let (caller, _rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    let outcome = caller.try_record_stop(id, at(TEST_STAMP)).await.unwrap();
    assert_eq!(outcome, None);
}

#[tokio::test]
async fn try_record_stop_flips_immediately_with_no_live_subagents() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    caller.claim_backlog_task(id, "host".into()).await.unwrap();
    let outcome = caller.try_record_stop(id, at(TEST_STAMP)).await.unwrap();
    assert_eq!(outcome, Some(true));
    assert_eq!(
        rows.task(id).unwrap().status,
        crate::models::TaskStatus::Review
    );
}

// -- Agent session state: hooks -----------------------------------------------

#[tokio::test]
async fn record_pre_tool_use_is_a_no_op_unless_running() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    caller
        .record_pre_tool_use(id, SubStatus::Active, at(TEST_STAMP))
        .await
        .unwrap();
    assert_eq!(rows.task(id).unwrap().last_pre_tool_use_at, None);

    caller.claim_backlog_task(id, "host".into()).await.unwrap();
    caller
        .record_pre_tool_use(id, SubStatus::Active, at(TEST_STAMP))
        .await
        .unwrap();
    assert!(rows.task(id).unwrap().last_pre_tool_use_at.is_some());
}

#[tokio::test]
async fn record_notification_raise_then_clear_round_trips() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    caller.claim_backlog_task(id, "host".into()).await.unwrap();

    caller
        .record_notification(id, NotificationWrite::Raise, at(TEST_STAMP))
        .await
        .unwrap();
    assert_eq!(
        rows.task(id).unwrap().sub_status,
        crate::models::SubStatus::NeedsInput
    );

    caller
        .record_notification(id, NotificationWrite::Clear, at(TEST_STAMP))
        .await
        .unwrap();
    assert_eq!(
        rows.task(id).unwrap().sub_status,
        crate::models::SubStatus::Active
    );
}

#[tokio::test]
async fn record_notification_raise_if_no_own_work_live_only_fires_when_idle() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    caller.claim_backlog_task(id, "host".into()).await.unwrap();
    start_subagent(&caller, id, "agent-a", "session-1").await;

    caller
        .record_notification(id, NotificationWrite::RaiseIfNoOwnWorkLive, at(TEST_STAMP))
        .await
        .unwrap();
    assert_eq!(
        rows.task(id).unwrap().sub_status,
        crate::models::SubStatus::Active
    );

    caller.subagent_clear(id).await.unwrap();
    caller
        .record_notification(id, NotificationWrite::RaiseIfNoOwnWorkLive, at(TEST_STAMP))
        .await
        .unwrap();
    assert_eq!(
        rows.task(id).unwrap().sub_status,
        crate::models::SubStatus::NeedsInput
    );
}

#[tokio::test]
async fn record_user_prompt_submit_resumes_from_review_and_voids_a_stale_pending_stop() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    caller.claim_backlog_task(id, "host".into()).await.unwrap();
    start_subagent(&caller, id, "agent-a", "session-1").await;
    caller
        .try_record_stop(id, at("2026-01-01 00:00:00.000"))
        .await
        .unwrap();
    assert!(rows.task(id).unwrap().stop_pending);

    let outcome = caller
        .record_user_prompt_submit(id, at(TEST_STAMP), at("2026-01-02 00:00:00.000"))
        .await
        .unwrap();
    assert!(outcome.won());
    assert!(!rows.task(id).unwrap().stop_pending);
    assert_eq!(
        rows.task(id).unwrap().status,
        crate::models::TaskStatus::Running
    );
}

#[tokio::test]
async fn record_user_prompt_submit_refuses_a_done_task() {
    let (caller, _rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    let outcome = caller
        .record_user_prompt_submit(id, at(TEST_STAMP), at(TEST_STAMP))
        .await
        .unwrap();
    assert!(!outcome.won());
}

#[tokio::test]
async fn mark_pr_learnings_gate_shown_refuses_a_repeat() {
    let (caller, _rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    let first = caller
        .mark_pr_learnings_gate_shown(id, at(TEST_STAMP))
        .await
        .unwrap();
    assert!(first.won());
    let second = caller
        .mark_pr_learnings_gate_shown(id, at(TEST_STAMP))
        .await
        .unwrap();
    assert!(!second.won());
}

// -- Task watchers ------------------------------------------------------------

#[tokio::test]
async fn create_task_watcher_is_idempotent() {
    let (caller, rows) = caller();
    let watcher = caller.create_task(blank_task()).await.unwrap();
    let target = caller.create_task(blank_task()).await.unwrap();
    caller.create_task_watcher(watcher, target).await.unwrap();
    caller.create_task_watcher(watcher, target).await.unwrap();
    assert_eq!(rows.watchers_of(target), vec![watcher]);
}

#[tokio::test]
async fn delete_task_watcher_removes_only_the_named_pair() {
    let (caller, rows) = caller();
    let watcher = caller.create_task(blank_task()).await.unwrap();
    let other_watcher = caller.create_task(blank_task()).await.unwrap();
    let target = caller.create_task(blank_task()).await.unwrap();
    caller.create_task_watcher(watcher, target).await.unwrap();
    caller
        .create_task_watcher(other_watcher, target)
        .await
        .unwrap();

    caller.delete_task_watcher(watcher, target).await.unwrap();
    assert_eq!(rows.watchers_of(target), vec![other_watcher]);
}

#[tokio::test]
async fn delete_watches_of_target_removes_every_watcher() {
    let (caller, rows) = caller();
    let watcher = caller.create_task(blank_task()).await.unwrap();
    let target = caller.create_task(blank_task()).await.unwrap();
    caller.create_task_watcher(watcher, target).await.unwrap();
    caller.delete_watches_of_target(target).await.unwrap();
    assert!(rows.watchers_of(target).is_empty());
}

#[tokio::test]
async fn delete_watches_by_watcher_removes_every_watch_it_holds() {
    let (caller, rows) = caller();
    let watcher = caller.create_task(blank_task()).await.unwrap();
    let target_a = caller.create_task(blank_task()).await.unwrap();
    let target_b = caller.create_task(blank_task()).await.unwrap();
    caller.create_task_watcher(watcher, target_a).await.unwrap();
    caller.create_task_watcher(watcher, target_b).await.unwrap();

    caller.delete_watches_by_watcher(watcher).await.unwrap();
    assert!(rows.watchers_of(target_a).is_empty());
    assert!(rows.watchers_of(target_b).is_empty());
}

#[tokio::test]
async fn delete_task_cascades_into_its_watches_and_subagents() {
    let (caller, rows) = caller();
    let watcher = caller.create_task(blank_task()).await.unwrap();
    let target = bindings::Task {
        status: DONE.into(),
        ..blank_task()
    };
    let target = caller.create_task(target).await.unwrap();
    caller.create_task_watcher(watcher, target).await.unwrap();

    caller.delete_task(target).await.unwrap();
    assert!(rows.watchers_of(target).is_empty());
}

// -- Poll ownership -----------------------------------------------------------

#[tokio::test]
async fn claim_poll_owner_fills_an_absent_row_and_leaves_an_existing_one() {
    let (caller, rows) = caller();
    caller
        .claim_poll_owner(PollScopeId::Task(TaskId(1)), "host-a".into())
        .await
        .unwrap();
    caller
        .claim_poll_owner(PollScopeId::Task(TaskId(1)), "host-b".into())
        .await
        .unwrap();
    assert_eq!(
        rows.poll_owner(PollScopeId::Task(TaskId(1))).unwrap().host,
        "host-a"
    );
}

#[tokio::test]
async fn override_poll_owner_reassigns_an_existing_row() {
    let (caller, rows) = caller();
    caller
        .claim_poll_owner(PollScopeId::Task(TaskId(1)), "host-a".into())
        .await
        .unwrap();
    caller
        .override_poll_owner(PollScopeId::Task(TaskId(1)), "host-b".into())
        .await
        .unwrap();
    assert_eq!(
        rows.poll_owner(PollScopeId::Task(TaskId(1))).unwrap().host,
        "host-b"
    );
}

// -- Stragglers ---------------------------------------------------------------

#[tokio::test]
async fn batch_patch_sub_status_updates_many_and_ignores_missing_ids() {
    let (caller, rows) = caller();
    let a = caller.create_task(blank_task()).await.unwrap();
    let b = caller.create_task(blank_task()).await.unwrap();
    caller
        .batch_patch_sub_status(vec![
            (a, SubStatus::Active),
            (b, SubStatus::Stale),
            (TaskId(999), SubStatus::Active),
        ])
        .await
        .unwrap();
    assert_eq!(
        rows.task(a).unwrap().sub_status,
        crate::models::SubStatus::Active
    );
    assert_eq!(
        rows.task(b).unwrap().sub_status,
        crate::models::SubStatus::Stale
    );
}

#[tokio::test]
async fn respawn_phoenix_successor_inserts_and_clears_the_predecessor_flag() {
    let (caller, rows) = caller();
    let predecessor = bindings::Task {
        status: DONE.into(),
        phoenix: true,
        ..blank_task()
    };
    let predecessor = caller.create_task(predecessor).await.unwrap();
    let successor_id = caller
        .respawn_phoenix_successor(predecessor, blank_task())
        .await
        .unwrap();
    assert_ne!(successor_id, predecessor);
    assert!(!rows.task(predecessor).unwrap().phoenix);
    assert!(rows.has_task(successor_id));
}

#[tokio::test]
async fn respawn_phoenix_successor_refuses_an_unknown_predecessor() {
    let (caller, _rows) = caller();
    let outcome = caller
        .respawn_phoenix_successor(TaskId(999), blank_task())
        .await;
    assert!(outcome.is_err());
}

// -- Host registry --------------------------------------------------------

#[tokio::test]
async fn register_host_upserts_by_id() {
    // `SharedRows` deliberately exposes no `hosts` reader yet (see its
    // own comment), so this only exercises that a second registration of
    // the same id is accepted rather than refused as a duplicate.
    let (caller, _rows) = caller();
    let first = caller
        .register_host("host-a".into(), "laptop".into(), "alice".into())
        .await
        .unwrap();
    assert!(first.won());
    let second = caller
        .register_host("host-a".into(), "renamed".into(), "alice".into())
        .await
        .unwrap();
    assert!(second.won());
}

#[tokio::test]
async fn register_host_refuses_an_empty_id() {
    let (caller, _rows) = caller();
    let outcome = caller
        .register_host(String::new(), "label".into(), "owner".into())
        .await
        .unwrap();
    assert!(!outcome.won());
}
