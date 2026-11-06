use super::*;

// -- Repo configuration -----------------------------------------------------

/// A caller owned by "alice", with three hosts she owns, plus one that is
/// Bob's. Repo-path writes name a host and act only for its owner.
fn repo_path_caller() -> (MemoryReducerCaller, Arc<SharedRows>) {
    let (caller, rows) = caller();
    let caller = caller
        .with_sender("alice")
        .with_registered_host("a", "alice")
        .with_registered_host("b", "alice")
        .with_registered_host("bobs", "bob");
    (caller, rows)
}

async fn save_path(caller: &MemoryReducerCaller, path: &str, host: &str, github: &str) {
    caller
        .save_repo_path(path.into(), at(TEST_STAMP), host.into(), github.into())
        .await
        .unwrap()
        .applied()
        .unwrap();
}

#[tokio::test]
async fn save_repo_path_upserts_by_path_and_host_not_by_call_count() {
    let (caller, rows) = repo_path_caller();
    save_path(&caller, "/repo", "a", "").await;
    save_path(&caller, "/repo", "a", "").await;
    assert_eq!(rows.repo_paths(), vec!["/repo".to_string()]);
}

#[tokio::test]
async fn save_repo_path_stamps_the_saving_host_and_adopts_a_row_with_none() {
    let (caller, rows) = repo_path_caller();
    caller.seed_repo_path("/legacy", "", "", "");
    assert!(rows.repo_paths_saved_by("a").is_empty());

    save_path(&caller, "/legacy", "a", "").await;
    save_path(&caller, "/fresh", "b", "").await;

    assert_eq!(rows.repo_paths_saved_by("a"), vec!["/legacy".to_string()]);
    assert_eq!(rows.repo_paths_saved_by("b"), vec!["/fresh".to_string()]);
    assert!(
        rows.repo_paths_saved_by("").is_empty(),
        "a row with no host is nobody's"
    );
}

#[tokio::test]
async fn save_repo_path_refuses_an_empty_path() {
    let (caller, _rows) = repo_path_caller();
    let outcome = caller
        .save_repo_path("   ".into(), at(TEST_STAMP), "a".into(), String::new())
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Refused(_)));
}

#[tokio::test]
async fn repo_path_writes_refuse_a_host_the_sender_does_not_own() {
    let (caller, rows) = repo_path_caller();
    save_path(&caller, "/repo", "a", "org/repo").await;
    caller.seed_repo_path("/bobs", "bobs", "org/bobs", "make");
    caller.seed_repo_path("/legacy", "", "", "");
    let before = rows.repo_paths_saved_by("bobs");
    for host in ["bobs", "nowhere", ""] {
        let outcomes = [
            caller
                .save_repo_path("/forged".into(), at(TEST_STAMP), host.into(), String::new())
                .await
                .unwrap(),
            caller
                .adopt_repo_path("/legacy".into(), host.into())
                .await
                .unwrap(),
            caller
                .delete_repo_path("/bobs".into(), host.into())
                .await
                .unwrap(),
            caller
                .set_verify_command("/bobs".into(), host.into(), "evil".into())
                .await
                .unwrap(),
        ];
        for outcome in outcomes {
            assert!(
                matches!(outcome, ReducerOutcome::Refused(_)),
                "host {host:?} must be refused, got {outcome:?}"
            );
        }
    }
    assert_eq!(rows.repo_paths_saved_by("bobs"), before, "bob's row stays");
    assert_eq!(
        rows.verify_command("/bobs", "bobs").as_deref(),
        Some("make")
    );
    assert!(rows.repo_paths_saved_by("a").contains(&"/repo".to_string()));
    assert!(!rows.repo_paths().contains(&"/forged".to_string()));
    assert!(rows.repo_paths_unowned().contains(&"/legacy".to_string()));
}

#[tokio::test]
async fn set_verify_command_refuses_a_multi_line_command() {
    let (caller, _rows) = repo_path_caller();
    save_path(&caller, "/repo", "a", "").await;
    let outcome = caller
        .set_verify_command("/repo".into(), "a".into(), "a\nb".into())
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Refused(_)));
}

#[tokio::test]
async fn set_verify_command_refuses_an_unknown_path() {
    let (caller, _rows) = repo_path_caller();
    let outcome = caller
        .set_verify_command("/nowhere".into(), "a".into(), "cargo test".into())
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Refused(_)));
}

#[tokio::test]
async fn set_verify_command_acts_through_the_calling_hosts_row_not_the_first_by_path() {
    let (caller, rows) = repo_path_caller();
    // Two machines hold DIFFERENT repositories at one path; b's row comes
    // second, so a first-by-path lookup would pick a's.
    save_path(&caller, "/code/x", "a", "org/one").await;
    save_path(&caller, "/code/x", "b", "org/two").await;
    caller
        .set_verify_command("/code/x".into(), "b".into(), "make two".into())
        .await
        .unwrap()
        .applied()
        .unwrap();
    assert_eq!(
        rows.verify_command("/code/x", "b").as_deref(),
        Some("make two")
    );
    assert_eq!(
        rows.verify_command("/code/x", "a"),
        None,
        "another repository at the same path keeps its own command"
    );
}

#[tokio::test]
async fn set_verify_command_refuses_a_path_only_another_host_has() {
    let (caller, rows) = repo_path_caller();
    save_path(&caller, "/repo", "a", "").await;
    let outcome = caller
        .set_verify_command("/repo".into(), "b".into(), "x".into())
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Refused(_)), "{outcome:?}");
    assert_eq!(rows.verify_command("/repo", "a"), None);
}

#[tokio::test]
async fn delete_repo_path_removes_every_row_for_that_path() {
    let (caller, rows) = repo_path_caller();
    save_path(&caller, "/repo", "a", "").await;
    caller
        .delete_repo_path("/repo".into(), "a".into())
        .await
        .unwrap();
    assert!(rows.repo_paths().is_empty());
}

#[tokio::test]
async fn delete_repo_path_refuses_a_path_only_another_host_has_and_removes_an_unowned_one() {
    let (caller, rows) = repo_path_caller();
    save_path(&caller, "/repo", "a", "").await;
    caller.seed_repo_path("/legacy", "", "", "");
    let refused = caller
        .delete_repo_path("/repo".into(), "b".into())
        .await
        .unwrap();
    assert!(
        matches!(refused, ReducerOutcome::Refused(ref m) if m.contains('a')),
        "{refused:?}"
    );
    caller
        .delete_repo_path("/legacy".into(), "b".into())
        .await
        .unwrap();
    assert_eq!(rows.repo_paths(), vec!["/repo".to_string()]);
}

#[tokio::test]
async fn two_hosts_each_own_a_row_of_one_path_and_the_picker_lists_it_once() {
    let (caller, rows) = repo_path_caller();
    for host in ["a", "b"] {
        save_path(&caller, "/repo", host, "org/repo").await;
    }
    assert_eq!(rows.repo_paths_saved_by("a"), vec!["/repo".to_string()]);
    assert_eq!(rows.repo_paths_saved_by("b"), vec!["/repo".to_string()]);
    assert_eq!(rows.repo_paths(), vec!["/repo".to_string()], "listed once");
}

#[tokio::test]
async fn the_verify_command_and_base_branches_follow_the_github_path() {
    let (caller, rows) = repo_path_caller();
    save_path(&caller, "/home/a/api", "a", "org/api").await;
    save_path(&caller, "/home/b/api", "b", "org/api").await;
    caller
        .set_verify_command("/home/a/api".into(), "a".into(), "make ci".into())
        .await
        .unwrap();
    caller
        .record_base_branch("/home/a/api".into(), "dev".into(), at(TEST_STAMP))
        .await
        .unwrap();
    assert_eq!(
        rows.verify_command("/home/b/api", "b").as_deref(),
        Some("make ci")
    );
    assert!(
        rows.base_branches()
            .contains(&("/home/b/api".to_string(), "dev".to_string())),
        "the other host's path sees the branch: {:?}",
        rows.base_branches()
    );
}

#[tokio::test]
async fn adopt_repo_path_stamps_only_an_unowned_row() {
    let (caller, rows) = repo_path_caller();
    caller.seed_repo_path("/legacy", "", "", "");
    save_path(&caller, "/owned", "a", "").await;
    caller
        .adopt_repo_path("/legacy".into(), "b".into())
        .await
        .unwrap();
    caller
        .adopt_repo_path("/owned".into(), "b".into())
        .await
        .unwrap();
    caller
        .adopt_repo_path("/missing".into(), "b".into())
        .await
        .unwrap();
    assert_eq!(rows.repo_paths_saved_by("b"), vec!["/legacy".to_string()]);
    assert_eq!(rows.repo_paths_saved_by("a"), vec!["/owned".to_string()]);
    assert_eq!(rows.repo_paths().len(), 2);
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
        // Whatever a client names here, the store stamps the caller.
        recorded_by: "someone-else".into(),
    }
}

async fn usage_counts(caller: &MemoryReducerCaller) -> Vec<i64> {
    caller
        .own_usage_summary(None, None, None, 50)
        .await
        .unwrap()
        .iter()
        .map(|s| s.count)
        .collect()
}

#[tokio::test]
async fn a_usage_event_is_stamped_with_its_recorder_and_served_only_to_them() {
    let (caller, _rows) = caller();
    let alice = caller.with_sender("alice");
    alice.record_usage_event(usage_event(), 10).await.unwrap();
    assert_eq!(usage_counts(&alice).await, [1]);
    assert!(usage_counts(&alice.with_sender("bob")).await.is_empty());
}

#[tokio::test]
async fn record_usage_event_prunes_beyond_the_cap() {
    let (caller, _rows) = caller();
    for _ in 0..5 {
        caller.record_usage_event(usage_event(), 2).await.unwrap();
    }
    assert_eq!(usage_counts(&caller).await.iter().sum::<i64>(), 2);
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

/// A caller owning every host the verify-command tests below save from.
fn many_hosts_caller() -> (MemoryReducerCaller, Arc<SharedRows>) {
    let (caller, rows) = caller();
    let mut caller = caller.with_sender("alice");
    for host in ["a", "b", "c", "h1", "h2", "h3", "h5", "h6"] {
        caller = caller.with_registered_host(host, "alice");
    }
    (caller, rows)
}

/// `core.allium: VerifyCommandIsPerRepository`: a row that first learns its
/// GitHub path joins that repository's verify command, rather than keeping
/// its own and splitting the repository in two.
#[tokio::test]
async fn a_row_that_learns_its_github_path_takes_the_repositorys_verify_command() {
    let (caller, rows) = many_hosts_caller();
    caller
        .save_repo_path(
            "/home/a/api".into(),
            at(TEST_STAMP),
            "a".into(),
            "org/api".into(),
        )
        .await
        .unwrap();
    caller
        .set_verify_command("/home/a/api".into(), "a".into(), "make ci".into())
        .await
        .unwrap();
    // b's clone is saved before its origin is known, with a command of its own.
    caller
        .save_repo_path(
            "/home/b/api".into(),
            at(TEST_STAMP),
            "b".into(),
            String::new(),
        )
        .await
        .unwrap();
    caller
        .set_verify_command("/home/b/api".into(), "b".into(), "cargo test".into())
        .await
        .unwrap();
    caller
        .save_repo_path(
            "/home/b/api".into(),
            at(TEST_STAMP),
            "b".into(),
            "org/api".into(),
        )
        .await
        .unwrap();
    assert_eq!(
        rows.verify_command("/home/b/api", "b").as_deref(),
        Some("make ci")
    );
    assert_eq!(
        rows.verify_command("/home/a/api", "a").as_deref(),
        Some("make ci")
    );
}

/// The other direction: a row that brings a command into a repository whose
/// rows have none gives it to them, so the rows still agree.
#[tokio::test]
async fn a_row_that_brings_a_command_to_a_repository_without_one_shares_it() {
    let (caller, rows) = many_hosts_caller();
    caller
        .save_repo_path(
            "/home/a/api".into(),
            at(TEST_STAMP),
            "a".into(),
            "org/api".into(),
        )
        .await
        .unwrap();
    caller
        .save_repo_path(
            "/home/b/api".into(),
            at(TEST_STAMP),
            "b".into(),
            String::new(),
        )
        .await
        .unwrap();
    caller
        .set_verify_command("/home/b/api".into(), "b".into(), "cargo test".into())
        .await
        .unwrap();
    caller
        .save_repo_path(
            "/home/b/api".into(),
            at(TEST_STAMP),
            "b".into(),
            "org/api".into(),
        )
        .await
        .unwrap();
    assert_eq!(
        rows.verify_command("/home/a/api", "a").as_deref(),
        Some("cargo test")
    );
    assert_eq!(
        rows.verify_command("/home/b/api", "b").as_deref(),
        Some("cargo test")
    );
}

/// Setting a command through a path reaches every row of the repository,
/// even when the first row of that path has no GitHub path of its own.
#[tokio::test]
async fn a_verify_command_set_through_a_path_reaches_the_whole_repository() {
    let (caller, rows) = many_hosts_caller();
    for (host, github) in [("c", ""), ("a", "org/api")] {
        caller
            .save_repo_path(
                "/home/a/api".into(),
                at(TEST_STAMP),
                host.into(),
                github.into(),
            )
            .await
            .unwrap();
    }
    caller
        .save_repo_path(
            "/home/b/api".into(),
            at(TEST_STAMP),
            "b".into(),
            "org/api".into(),
        )
        .await
        .unwrap();
    caller
        .set_verify_command("/home/a/api".into(), "a".into(), "make ci".into())
        .await
        .unwrap();
    assert_eq!(
        rows.verify_command("/home/b/api", "b").as_deref(),
        Some("make ci")
    );
}

/// A row that already belongs to its repository keeps its command on a plain
/// re-save: only learning a GitHub path joins a repository.
#[tokio::test]
async fn a_plain_resave_keeps_the_rows_own_command() {
    let (caller, rows) = many_hosts_caller();
    caller
        .save_repo_path(
            "/home/a/api".into(),
            at(TEST_STAMP),
            "a".into(),
            "org/api".into(),
        )
        .await
        .unwrap();
    caller
        .set_verify_command("/home/a/api".into(), "a".into(), "make ci".into())
        .await
        .unwrap();
    caller
        .save_repo_path(
            "/home/a/api".into(),
            at(TEST_STAMP),
            "a".into(),
            "org/api".into(),
        )
        .await
        .unwrap();
    assert_eq!(
        rows.verify_command("/home/a/api", "a").as_deref(),
        Some("make ci")
    );
}

/// A new row of a known repository takes that repository's command, not the
/// command of an unrelated-looking row that merely shares its local path.
#[tokio::test]
async fn a_new_row_prefers_its_github_siblings_command() {
    let (caller, rows) = many_hosts_caller();
    // A: path P on host 1, origin unknown, command x.
    caller
        .save_repo_path("/p".into(), at(TEST_STAMP), "h1".into(), String::new())
        .await
        .unwrap();
    caller
        .set_verify_command("/p".into(), "h1".into(), "x".into())
        .await
        .unwrap();
    // B: path Q on host 3, repository G, command y.
    caller
        .save_repo_path("/q".into(), at(TEST_STAMP), "h3".into(), "org/g".into())
        .await
        .unwrap();
    caller
        .set_verify_command("/q".into(), "h3".into(), "y".into())
        .await
        .unwrap();
    // C: path P on host 2, repository G.
    caller
        .save_repo_path("/p".into(), at(TEST_STAMP), "h2".into(), "org/g".into())
        .await
        .unwrap();
    let t = caller.lock();
    let of_g: Vec<String> = t
        .repo_paths
        .values()
        .filter(|r| r.github_path == "org/g")
        .map(|r| r.verify_command.clone())
        .collect();
    assert_eq!(of_g, ["y", "y"], "both rows of org/g agree");
    drop(t);
    assert_eq!(rows.verify_command("/q", "h3").as_deref(), Some("y"));
}

/// A new row whose repository has no command yet gives its own nothing to
/// lose, and a row of that repository that later gets one shares it.
#[tokio::test]
async fn a_new_row_with_a_path_command_shares_it_with_its_repository() {
    let (caller, _rows) = many_hosts_caller();
    caller
        .save_repo_path("/p".into(), at(TEST_STAMP), "h1".into(), String::new())
        .await
        .unwrap();
    caller
        .set_verify_command("/p".into(), "h1".into(), "x".into())
        .await
        .unwrap();
    caller
        .save_repo_path("/q".into(), at(TEST_STAMP), "h3".into(), "org/g".into())
        .await
        .unwrap();
    // C joins org/g carrying x from its path sibling; B has none and takes it.
    caller
        .save_repo_path("/p".into(), at(TEST_STAMP), "h2".into(), "org/g".into())
        .await
        .unwrap();
    let t = caller.lock();
    let of_g: Vec<String> = t
        .repo_paths
        .values()
        .filter(|r| r.github_path == "org/g")
        .map(|r| r.verify_command.clone())
        .collect();
    assert_eq!(of_g, ["x", "x"]);
}

/// A command shared from a row with no GitHub path reaches the whole
/// repository of every row at its path, not only the rows at that path.
#[tokio::test]
async fn a_command_shared_from_a_path_row_reaches_whole_repositories() {
    let (caller, _rows) = many_hosts_caller();
    for (path, host, github) in [
        ("/x", "h1", "org/a"),
        ("/y", "h2", "org/a"),
        ("/w", "h6", "org/b"),
    ] {
        caller
            .save_repo_path(path.into(), at(TEST_STAMP), host.into(), github.into())
            .await
            .unwrap();
    }
    caller
        .set_verify_command("/w".into(), "h6".into(), "c".into())
        .await
        .unwrap();
    for (host, github) in [("h5", "org/b"), ("h3", "")] {
        caller
            .save_repo_path("/x".into(), at(TEST_STAMP), host.into(), github.into())
            .await
            .unwrap();
    }
    let t = caller.lock();
    let of_a: Vec<String> = t
        .repo_paths
        .values()
        .filter(|r| r.github_path == "org/a")
        .map(|r| r.verify_command.clone())
        .collect();
    assert_eq!(of_a.len(), 2);
    assert!(of_a[0] == of_a[1], "org/a rows disagree: {of_a:?}");
}
