use super::*;
use crate::store::{HostStore, RepoConfigRead, RepoConfigStore};

/// `SettingsStore`'s generic accessors refuse the four identity/credential
/// keys outright — a caller mistake must fail loudly rather than silently
/// route a credential into the shared store the moment a writer is attached.
/// See `refuse_identity_key`.
#[tokio::test]
async fn set_setting_string_refuses_the_identity_keys() {
    let db = Database::open_in_memory().await.unwrap();
    for key in [
        "host_id",
        "host_label",
        "user_identity",
        "user_identity_token",
    ] {
        assert!(
            db.set_setting_string(key, "anything").await.is_err(),
            "{key:?} must be refused by the generic settings accessor"
        );
        assert!(
            db.set_setting_bool(key, true).await.is_err(),
            "{key:?} must be refused by the generic settings accessor"
        );
    }
}

#[tokio::test]
async fn get_setting_bool_returns_none_when_absent() {
    let db = Database::open_in_memory().await.unwrap();
    assert_eq!(
        db.get_setting_bool("notifications_enabled").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn set_and_get_setting_bool_roundtrips() {
    let db = Database::open_in_memory().await.unwrap();
    db.set_setting_bool("notifications_enabled", true)
        .await
        .unwrap();
    assert_eq!(
        db.get_setting_bool("notifications_enabled").await.unwrap(),
        Some(true)
    );

    db.set_setting_bool("notifications_enabled", false)
        .await
        .unwrap();
    assert_eq!(
        db.get_setting_bool("notifications_enabled").await.unwrap(),
        Some(false)
    );
}

#[tokio::test]
async fn get_setting_string_returns_none_when_absent() {
    let db = Database::open_in_memory().await.unwrap();
    assert_eq!(db.get_setting_string("repo_filter").await.unwrap(), None);
}

#[tokio::test]
async fn set_and_get_setting_string() {
    let db = Database::open_in_memory().await.unwrap();
    db.set_setting_string("repo_filter", "/repo1\n/repo2")
        .await
        .unwrap();
    assert_eq!(
        db.get_setting_string("repo_filter").await.unwrap(),
        Some("/repo1\n/repo2".to_string())
    );
}

#[tokio::test]
async fn set_setting_string_upserts() {
    let db = Database::open_in_memory().await.unwrap();
    db.set_setting_string("repo_filter", "old").await.unwrap();
    db.set_setting_string("repo_filter", "new").await.unwrap();
    assert_eq!(
        db.get_setting_string("repo_filter").await.unwrap(),
        Some("new".to_string())
    );
}

#[tokio::test]
async fn save_and_list_repo_paths() {
    let db = in_memory_db().await;
    assert!(db.list_repo_paths().await.unwrap().is_empty());
    db.save_repo_path("/home/user/project").await.unwrap();
    db.save_repo_path("/home/user/other").await.unwrap();
    let paths = db.list_repo_paths().await.unwrap();
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&"/home/user/project".to_string()));
    assert!(paths.contains(&"/home/user/other".to_string()));
}

#[tokio::test]
async fn save_repo_path_deduplicates() {
    let db = in_memory_db().await;
    db.save_repo_path("/home/user/project").await.unwrap();
    db.save_repo_path("/home/user/project").await.unwrap();
    assert_eq!(db.list_repo_paths().await.unwrap().len(), 1);
}

#[tokio::test]
async fn list_repo_paths_empty_by_default() {
    let db = in_memory_db().await;
    assert!(db.list_repo_paths().await.unwrap().is_empty());
}

#[tokio::test]
async fn list_repo_paths_returns_all_beyond_nine() {
    let db = in_memory_db().await;
    for i in 0..15 {
        db.save_repo_path(&format!("/home/user/project{i}"))
            .await
            .unwrap();
    }
    let paths = db.list_repo_paths().await.unwrap();
    assert_eq!(
        paths.len(),
        15,
        "all 15 paths should be returned, not just 9"
    );
}

#[tokio::test]
async fn delete_repo_path_removes_entry() {
    let db = in_memory_db().await;
    db.save_repo_path("/home/user/project").await.unwrap();
    db.save_repo_path("/home/user/other").await.unwrap();
    assert_eq!(db.list_repo_paths().await.unwrap().len(), 2);
    db.delete_repo_path("/home/user/project").await.unwrap();
    let paths = db.list_repo_paths().await.unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], "/home/user/other");
}

#[tokio::test]
async fn delete_repo_path_nonexistent_is_ok() {
    let db = in_memory_db().await;
    db.delete_repo_path("/does/not/exist").await.unwrap();
}

#[tokio::test]
async fn verify_command_default_is_none() {
    let db = in_memory_db().await;
    db.save_repo_path("/home/me/repo").await.unwrap();
    assert_eq!(db.get_verify_command("/home/me/repo").await.unwrap(), None);
}

#[tokio::test]
async fn verify_command_round_trip() {
    let db = in_memory_db().await;
    db.save_repo_path("/home/me/repo").await.unwrap();
    db.set_verify_command("/home/me/repo", Some("cargo test"))
        .await
        .unwrap();
    assert_eq!(
        db.get_verify_command("/home/me/repo").await.unwrap(),
        Some("cargo test".to_string())
    );
}

#[tokio::test]
async fn verify_command_get_unknown_path_is_none() {
    let db = in_memory_db().await;
    assert_eq!(
        db.get_verify_command("/does/not/exist").await.unwrap(),
        None
    );
}

// --- managed-feed config keys (WP5) ---

#[tokio::test]
async fn reviews_feed_command_round_trips_and_clears() {
    let db = in_memory_db().await;
    assert_eq!(db.get_reviews_feed_command().await.unwrap(), None);
    db.set_reviews_feed_command(Some("/scripts/fetch-reviews.sh"))
        .await
        .unwrap();
    assert_eq!(
        db.get_reviews_feed_command().await.unwrap(),
        Some("/scripts/fetch-reviews.sh".to_string())
    );
    db.set_reviews_feed_command(None).await.unwrap();
    assert_eq!(db.get_reviews_feed_command().await.unwrap(), None);
}

#[tokio::test]
async fn reviews_feed_interval_secs_round_trips_and_clears() {
    let db = in_memory_db().await;
    assert_eq!(db.get_reviews_feed_interval_secs().await.unwrap(), None);
    db.set_reviews_feed_interval_secs(Some(300)).await.unwrap();
    assert_eq!(
        db.get_reviews_feed_interval_secs().await.unwrap(),
        Some(300)
    );
    db.set_reviews_feed_interval_secs(None).await.unwrap();
    assert_eq!(db.get_reviews_feed_interval_secs().await.unwrap(), None);
}

#[tokio::test]
async fn cve_feed_command_round_trips_and_clears() {
    let db = in_memory_db().await;
    assert_eq!(db.get_cve_feed_command().await.unwrap(), None);
    db.set_cve_feed_command(Some("/scripts/fetch-cve.sh"))
        .await
        .unwrap();
    assert_eq!(
        db.get_cve_feed_command().await.unwrap(),
        Some("/scripts/fetch-cve.sh".to_string())
    );
    db.set_cve_feed_command(None).await.unwrap();
    assert_eq!(db.get_cve_feed_command().await.unwrap(), None);
}

#[tokio::test]
async fn cve_feed_interval_secs_round_trips_and_clears() {
    let db = in_memory_db().await;
    assert_eq!(db.get_cve_feed_interval_secs().await.unwrap(), None);
    db.set_cve_feed_interval_secs(Some(900)).await.unwrap();
    assert_eq!(db.get_cve_feed_interval_secs().await.unwrap(), Some(900));
    db.set_cve_feed_interval_secs(None).await.unwrap();
    assert_eq!(db.get_cve_feed_interval_secs().await.unwrap(), None);
}

// ---------------------------------------------------------------------------
// repo_base_branches — per-repo base_branch history. See docs/specs/dispatch.allium
// (rule RecordBaseBranch, surface BaseBranchPicker, config.max_base_branches_per_repo,
// invariant BranchHistoryCapped) and docs/specs/core.allium (entity SavedRepoBranch).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_all_base_branches_empty_by_default() {
    let db = in_memory_db().await;
    assert!(db.list_all_base_branches().await.unwrap().is_empty());
}

#[tokio::test]
async fn record_base_branch_inserts_new_row() {
    let db = in_memory_db().await;
    db.record_base_branch("/repo/a", "main").await.unwrap();
    let all = db.list_all_base_branches().await.unwrap();
    assert_eq!(all, vec![("/repo/a".to_string(), "main".to_string())]);
}

// -- host identity (task #4812 distributed-dispatch foundations) ------------
// See docs/specs/host.allium: MintHostIdentity, RenameHost. Iteration 2:
// MintHostIdentity mints the id only now — the label starts null and is set
// only by RenameHost, first from the startup gate
// (startup.allium: PromptForHostLabelWhenUnnamed / NameHostFromStartupPrompt)
// and later by an operator-initiated rename.

#[tokio::test]
async fn ensure_host_identity_mints_a_non_empty_id_and_no_label() {
    let db = in_memory_db().await;

    let (id, label) = db.ensure_host_identity().await.unwrap();

    assert!(!id.is_empty(), "the id must be generated, not left blank");
    assert_eq!(
        label, None,
        "the label must NOT be seeded from the hostname at mint — a seeded \
         label would make the startup prompt unable to fire (host.allium: \
         MintHostIdentity's 'THE LABEL IS NOT MINTED AT ALL' guidance)"
    );
}

#[tokio::test]
async fn ensure_host_identity_is_idempotent() {
    let db = in_memory_db().await;

    let (first_id, first_label) = db.ensure_host_identity().await.unwrap();
    let (second_id, second_label) = db.ensure_host_identity().await.unwrap();

    assert_eq!(
        first_id, second_id,
        "mint is not re-run: a second call must return the SAME id, never a fresh one"
    );
    assert_eq!(first_label, second_label);
    assert_eq!(first_label, None);
}

#[tokio::test]
async fn ensure_host_identity_does_not_remint_after_a_rename() {
    let db = in_memory_db().await;
    let (id, label) = db.ensure_host_identity().await.unwrap();
    assert_eq!(label, None, "unnamed until RenameHost writes a label");
    db.rename_host("renamed-machine").await.unwrap();

    let (id_after_rename, label_after_rename) = db.ensure_host_identity().await.unwrap();

    assert_eq!(
        id, id_after_rename,
        "the id is untouched by a rename — see host.allium: RenameHost's guidance"
    );
    assert_eq!(label_after_rename, Some("renamed-machine".to_string()));
}

#[tokio::test]
async fn rename_host_updates_the_label_only() {
    let db = in_memory_db().await;
    let (original_id, original_label) = db.ensure_host_identity().await.unwrap();
    assert_eq!(original_label, None);

    db.rename_host("my-laptop").await.unwrap();

    let (id, label) = db.ensure_host_identity().await.unwrap();
    assert_eq!(id, original_id);
    assert_eq!(label, Some("my-laptop".to_string()));
}

#[tokio::test]
async fn rename_host_rejects_an_empty_label() {
    let db = in_memory_db().await;
    db.ensure_host_identity().await.unwrap();

    assert!(
        db.rename_host("").await.is_err(),
        "an empty label is refused — a blank-looking name is worse than staying unnamed"
    );
}

#[tokio::test]
async fn rename_host_rejects_a_whitespace_only_label() {
    let db = in_memory_db().await;
    db.ensure_host_identity().await.unwrap();

    assert!(db.rename_host("   ").await.is_err());
}

#[tokio::test]
async fn rename_host_stores_the_trimmed_label() {
    let db = in_memory_db().await;
    db.ensure_host_identity().await.unwrap();

    db.rename_host("  padded-name  ").await.unwrap();

    let (_id, label) = db.ensure_host_identity().await.unwrap();
    assert_eq!(
        label,
        Some("padded-name".to_string()),
        "leading/trailing whitespace from a pasted or padded value must not \
         become part of the stored label"
    );
}

#[tokio::test]
async fn rename_host_before_any_mint_still_mints_a_stable_id() {
    // RenameHost never runs before MintHostIdentity in the real system (see
    // host.allium's `FirstRun` ordering), but `rename_host` itself only ever
    // touches the label column, so calling it first must not corrupt a
    // subsequent mint.
    let db = in_memory_db().await;

    db.rename_host("early-rename").await.unwrap();
    let (id, label) = db.ensure_host_identity().await.unwrap();

    assert!(!id.is_empty());
    assert_eq!(label, Some("early-rename".to_string()));
}

#[tokio::test]
async fn rename_host_can_rename_an_already_named_host() {
    // The same rule covers the first naming (from the startup gate) and every
    // later rename — host.allium: RenameHost's "THIS IS THE ONLY RULE THAT
    // WRITES THE LABEL" guidance.
    let db = in_memory_db().await;
    db.ensure_host_identity().await.unwrap();
    db.rename_host("first-name").await.unwrap();

    db.rename_host("second-name").await.unwrap();

    let (_id, label) = db.ensure_host_identity().await.unwrap();
    assert_eq!(label, Some("second-name".to_string()));
}
