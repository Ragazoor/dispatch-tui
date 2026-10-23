use super::*;

// -- Repo configuration -----------------------------------------------------

#[tokio::test]
async fn save_repo_path_upserts_by_path_not_by_call_count() {
    let (caller, rows) = caller();
    caller
        .save_repo_path("/repo".into(), at("2026-01-01 00:00:00.000"), "h".into())
        .await
        .unwrap();
    caller
        .save_repo_path("/repo".into(), at("2026-01-02 00:00:00.000"), "h".into())
        .await
        .unwrap();
    let paths = rows.repo_paths();
    assert_eq!(paths, vec!["/repo".to_string()]);
}

#[tokio::test]
async fn save_repo_path_stamps_the_saving_host_and_adopts_a_row_with_none() {
    let (caller, rows) = caller();
    caller
        .save_repo_path(
            "/legacy".into(),
            at("2026-01-01 00:00:00.000"),
            String::new(),
        )
        .await
        .unwrap();
    assert!(rows.repo_paths_saved_by("laptop").is_empty());

    caller
        .save_repo_path(
            "/legacy".into(),
            at("2026-01-02 00:00:00.000"),
            "laptop".into(),
        )
        .await
        .unwrap();
    caller
        .save_repo_path(
            "/fresh".into(),
            at("2026-01-02 00:00:00.000"),
            "desk".into(),
        )
        .await
        .unwrap();

    assert_eq!(
        rows.repo_paths_saved_by("laptop"),
        vec!["/legacy".to_string()]
    );
    assert_eq!(rows.repo_paths_saved_by("desk"), vec!["/fresh".to_string()]);
    assert!(
        rows.repo_paths_saved_by("").is_empty(),
        "a row with no host is nobody's"
    );
}

#[tokio::test]
async fn save_repo_path_refuses_an_empty_path() {
    let (caller, _rows) = caller();
    let outcome = caller
        .save_repo_path("   ".into(), at(TEST_STAMP), "h".into())
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Refused(_)));
}

#[tokio::test]
async fn set_verify_command_refuses_a_multi_line_command() {
    let (caller, _rows) = caller();
    caller
        .save_repo_path("/repo".into(), at(TEST_STAMP), "h".into())
        .await
        .unwrap();
    let outcome = caller
        .set_verify_command("/repo".into(), "a\nb".into())
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Refused(_)));
}

#[tokio::test]
async fn set_verify_command_refuses_an_unknown_path() {
    let (caller, _rows) = caller();
    let outcome = caller
        .set_verify_command("/nowhere".into(), "cargo test".into())
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Refused(_)));
}

#[tokio::test]
async fn record_base_branch_upserts_by_repo_and_branch() {
    let (caller, rows) = caller();
    caller
        .record_base_branch("/repo".into(), "main".into(), at("2026-01-01 00:00:00.000"))
        .await
        .unwrap();
    caller
        .record_base_branch("/repo".into(), "main".into(), at("2026-01-02 00:00:00.000"))
        .await
        .unwrap();
    assert_eq!(
        rows.base_branches(),
        vec![("/repo".to_string(), "main".to_string())]
    );
}

#[tokio::test]
async fn delete_repo_path_removes_every_row_for_that_path() {
    let (caller, rows) = caller();
    caller
        .save_repo_path("/repo".into(), at(TEST_STAMP), "h".into())
        .await
        .unwrap();
    caller.delete_repo_path("/repo".into()).await.unwrap();
    assert!(rows.repo_paths().is_empty());
}

// -- Subscriptions ------------------------------------------------------------

#[tokio::test]
async fn subscribe_to_epic_requires_an_identity_and_an_existing_epic() {
    let (caller, _rows) = caller();
    let no_identity = caller
        .subscribe_to_epic(String::new(), EpicId(1))
        .await
        .unwrap();
    assert!(matches!(no_identity, ReducerOutcome::Refused(_)));

    let no_epic = caller
        .subscribe_to_epic("alice".into(), EpicId(999))
        .await
        .unwrap();
    assert!(matches!(no_epic, ReducerOutcome::Refused(_)));
}

#[tokio::test]
async fn subscribe_to_epic_is_idempotent() {
    let (caller, rows) = caller();
    let epic_id = caller.create_epic(blank_epic()).await.unwrap();
    caller
        .subscribe_to_epic("alice".into(), epic_id)
        .await
        .unwrap();
    caller
        .subscribe_to_epic("alice".into(), epic_id)
        .await
        .unwrap();
    assert_eq!(rows.subscribed_epics("alice"), vec![epic_id]);
}

#[tokio::test]
async fn unsubscribe_from_epic_refuses_when_not_subscribed() {
    let (caller, rows) = caller();
    let epic_id = caller.create_epic(blank_epic()).await.unwrap();
    let outcome = caller
        .unsubscribe_from_epic("alice".into(), epic_id)
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Refused(_)));

    caller
        .subscribe_to_epic("alice".into(), epic_id)
        .await
        .unwrap();
    let outcome = caller
        .unsubscribe_from_epic("alice".into(), epic_id)
        .await
        .unwrap();
    assert!(outcome.won());
    assert!(rows.subscribed_epics("alice").is_empty());
}

// -- Usage events -----------------------------------------------------------

fn usage_event() -> bindings::UsageEvent {
    bindings::UsageEvent {
        id: 0,
        recorded_at: TEST_STAMP.into(),
        category: "tool".into(),
        action: "used".into(),
        detail: None,
        actor: "tester".into(),
    }
}

#[tokio::test]
async fn record_usage_event_prunes_beyond_the_cap() {
    let (caller, rows) = caller();
    for _ in 0..5 {
        caller.record_usage_event(usage_event(), 2).await.unwrap();
    }
    let total: i64 = rows
        .usage_summary(&crate::models::UsageQuery::default())
        .iter()
        .map(|s| s.count)
        .sum();
    assert_eq!(total, 2);
}

#[tokio::test]
async fn record_usage_event_refuses_a_non_positive_cap() {
    let (caller, _rows) = caller();
    let outcome = caller.record_usage_event(usage_event(), 0).await.unwrap();
    assert!(!outcome.won());
}

// -- Settings -----------------------------------------------------------------

/// A caller that owns `host-a`, so it may write that host's settings (D11).
fn owning_caller() -> (MemoryReducerCaller, Arc<SharedRows>) {
    let (caller, rows) = caller();
    let caller = caller
        .with_sender("alice")
        .with_registered_host("host-a", "alice");
    (caller, rows)
}

#[tokio::test]
async fn setting_writes_refuse_a_host_the_sender_does_not_own() {
    let (caller, rows) = owning_caller();
    let caller = caller
        .with_registered_host("bobs", "bob")
        .with_registered_host("unowned", "");
    caller
        .save_setting("host-a".into(), "theme".into(), "dark".into())
        .await
        .unwrap();
    for host in ["bobs", "unowned", "nowhere"] {
        let save = caller
            .save_setting(host.into(), "theme".into(), "x".into())
            .await
            .unwrap();
        let clear = caller
            .clear_setting(host.into(), "theme".into())
            .await
            .unwrap();
        assert!(!save.won() && !clear.won(), "{host} must be refused");
    }
    assert_eq!(rows.setting("theme").as_deref(), Some("dark"));
}

#[tokio::test]
async fn save_setting_upserts_by_host_and_key() {
    let (caller, rows) = owning_caller();
    caller
        .save_setting("host-a".into(), "theme".into(), "dark".into())
        .await
        .unwrap();
    assert_eq!(rows.setting("theme").as_deref(), Some("dark"));

    caller
        .save_setting("host-a".into(), "theme".into(), "light".into())
        .await
        .unwrap();
    assert_eq!(rows.setting("theme").as_deref(), Some("light"));
}

#[tokio::test]
async fn save_setting_refuses_an_empty_host() {
    let (caller, rows) = caller();
    let outcome = caller
        .save_setting(String::new(), "theme".into(), "dark".into())
        .await
        .unwrap();
    assert!(!outcome.won());
    assert!(rows.setting("theme").is_none());
}

#[tokio::test]
async fn clear_setting_removes_a_saved_key() {
    let (caller, rows) = owning_caller();
    caller
        .save_setting("host-a".into(), "theme".into(), "dark".into())
        .await
        .unwrap();
    let outcome = caller
        .clear_setting("host-a".into(), "theme".into())
        .await
        .unwrap();
    assert!(outcome.won());
    assert!(rows.setting("theme").is_none());
}

#[tokio::test]
async fn clear_setting_on_a_never_set_key_is_a_no_op() {
    let (caller, _rows) = owning_caller();
    let outcome = caller
        .clear_setting("host-a".into(), "never-set".into())
        .await
        .unwrap();
    assert!(outcome.won());
}
