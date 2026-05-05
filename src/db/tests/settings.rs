use super::*;

#[test]
fn get_setting_bool_returns_none_when_absent() {
    let db = Database::open_in_memory().unwrap();
    assert_eq!(db.get_setting_bool("notifications_enabled").unwrap(), None);
}

#[test]
fn set_and_get_setting_bool_roundtrips() {
    let db = Database::open_in_memory().unwrap();
    db.set_setting_bool("notifications_enabled", true).unwrap();
    assert_eq!(
        db.get_setting_bool("notifications_enabled").unwrap(),
        Some(true)
    );

    db.set_setting_bool("notifications_enabled", false).unwrap();
    assert_eq!(
        db.get_setting_bool("notifications_enabled").unwrap(),
        Some(false)
    );
}

#[test]
fn get_setting_string_returns_none_when_absent() {
    let db = Database::open_in_memory().unwrap();
    assert_eq!(db.get_setting_string("repo_filter").unwrap(), None);
}

#[test]
fn set_and_get_setting_string() {
    let db = Database::open_in_memory().unwrap();
    db.set_setting_string("repo_filter", "/repo1\n/repo2")
        .unwrap();
    assert_eq!(
        db.get_setting_string("repo_filter").unwrap(),
        Some("/repo1\n/repo2".to_string())
    );
}

#[test]
fn set_setting_string_upserts() {
    let db = Database::open_in_memory().unwrap();
    db.set_setting_string("repo_filter", "old").unwrap();
    db.set_setting_string("repo_filter", "new").unwrap();
    assert_eq!(
        db.get_setting_string("repo_filter").unwrap(),
        Some("new".to_string())
    );
}

#[test]
fn save_and_list_repo_paths() {
    let db = in_memory_db();
    assert!(db.list_repo_paths().unwrap().is_empty());
    db.save_repo_path("/home/user/project").unwrap();
    db.save_repo_path("/home/user/other").unwrap();
    let paths = db.list_repo_paths().unwrap();
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&"/home/user/project".to_string()));
    assert!(paths.contains(&"/home/user/other".to_string()));
}

#[test]
fn save_repo_path_deduplicates() {
    let db = in_memory_db();
    db.save_repo_path("/home/user/project").unwrap();
    db.save_repo_path("/home/user/project").unwrap();
    assert_eq!(db.list_repo_paths().unwrap().len(), 1);
}

#[test]
fn list_repo_paths_empty_by_default() {
    let db = in_memory_db();
    assert!(db.list_repo_paths().unwrap().is_empty());
}

#[test]
fn list_repo_paths_returns_all_beyond_nine() {
    let db = in_memory_db();
    for i in 0..15 {
        db.save_repo_path(&format!("/home/user/project{i}"))
            .unwrap();
    }
    let paths = db.list_repo_paths().unwrap();
    assert_eq!(
        paths.len(),
        15,
        "all 15 paths should be returned, not just 9"
    );
}

#[test]
fn filter_presets_save_and_list() {
    let db = Database::open_in_memory().unwrap();
    db.save_filter_preset(
        "frontend",
        &["/repo-a".to_string(), "/repo-b".to_string()],
        "include",
    )
    .unwrap();
    db.save_filter_preset("backend", &["/repo-c".to_string()], "exclude")
        .unwrap();

    let presets = db.list_filter_presets().unwrap();
    assert_eq!(presets.len(), 2);
    assert_eq!(presets[0].0, "backend"); // sorted by name
    assert_eq!(presets[0].2, "exclude");
    assert_eq!(presets[1].0, "frontend");
    assert_eq!(
        presets[1].1,
        vec!["/repo-a".to_string(), "/repo-b".to_string()]
    );
    assert_eq!(presets[1].2, "include");
}

#[test]
fn filter_presets_overwrite_and_delete() {
    let db = Database::open_in_memory().unwrap();
    db.save_filter_preset("frontend", &["/repo-a".to_string()], "include")
        .unwrap();
    db.save_filter_preset(
        "frontend",
        &["/repo-x".to_string(), "/repo-y".to_string()],
        "exclude",
    )
    .unwrap();

    let presets = db.list_filter_presets().unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(
        presets[0].1,
        vec!["/repo-x".to_string(), "/repo-y".to_string()]
    );
    assert_eq!(presets[0].2, "exclude");

    db.delete_filter_preset("frontend").unwrap();
    let presets = db.list_filter_presets().unwrap();
    assert!(presets.is_empty());
}

#[test]
fn seed_github_query_defaults_sets_my_prs() {
    let db = in_memory_db();
    db.seed_github_query_defaults().unwrap();

    let my_prs = db
        .get_setting_string("github_queries_my_prs")
        .unwrap()
        .expect("my_prs queries should be set");
    assert!(my_prs.contains("author:@me"));

    for key in [
        "github_queries_review",
        "github_queries_security",
        "github_queries_bot",
        "dependabot_config",
    ] {
        assert!(
            db.get_setting_string(key).unwrap().is_none(),
            "{key} must not be seeded after the fetch-* CLI removal",
        );
    }
}

#[test]
fn seed_github_query_defaults_does_not_overwrite_user_edits() {
    let db = in_memory_db();
    db.seed_github_query_defaults().unwrap();

    // User edits the surviving setting.
    db.set_setting_string("github_queries_my_prs", "my custom query")
        .unwrap();

    // Re-seed should not overwrite.
    db.seed_github_query_defaults().unwrap();

    let my_prs = db
        .get_setting_string("github_queries_my_prs")
        .unwrap()
        .unwrap();
    assert_eq!(my_prs, "my custom query");
}

#[test]
fn delete_repo_path_removes_entry() {
    let db = in_memory_db();
    db.save_repo_path("/home/user/project").unwrap();
    db.save_repo_path("/home/user/other").unwrap();
    assert_eq!(db.list_repo_paths().unwrap().len(), 2);
    db.delete_repo_path("/home/user/project").unwrap();
    let paths = db.list_repo_paths().unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], "/home/user/other");
}

#[test]
fn delete_repo_path_nonexistent_is_ok() {
    let db = in_memory_db();
    db.delete_repo_path("/does/not/exist").unwrap();
}

#[test]
fn delete_repo_path_cleans_presets() {
    let db = in_memory_db();
    db.save_repo_path("/home/user/a").unwrap();
    db.save_repo_path("/home/user/b").unwrap();
    db.save_filter_preset(
        "my_preset",
        &["/home/user/a".to_string(), "/home/user/b".to_string()],
        "include",
    )
    .unwrap();
    db.delete_repo_path("/home/user/a").unwrap();
    let presets = db.list_filter_presets().unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0].0, "my_preset");
    assert_eq!(presets[0].1, vec!["/home/user/b".to_string()]);
}

#[test]
fn delete_repo_path_removes_empty_preset() {
    let db = in_memory_db();
    db.save_repo_path("/home/user/solo").unwrap();
    db.save_filter_preset("solo_preset", &["/home/user/solo".to_string()], "include")
        .unwrap();
    db.delete_repo_path("/home/user/solo").unwrap();
    let presets = db.list_filter_presets().unwrap();
    assert!(presets.is_empty());
}

#[test]
fn tips_state_defaults_on_fresh_db() {
    let db = in_memory_db();
    let (seen_up_to, show_mode) = db.get_tips_state().unwrap();
    assert_eq!(seen_up_to, 0);
    assert_eq!(show_mode, crate::models::TipsShowMode::Always);
}

#[test]
fn tips_state_round_trip() {
    let db = in_memory_db();
    db.save_tips_state(7, crate::models::TipsShowMode::NewOnly)
        .unwrap();
    let (seen_up_to, show_mode) = db.get_tips_state().unwrap();
    assert_eq!(seen_up_to, 7);
    assert_eq!(show_mode, crate::models::TipsShowMode::NewOnly);
}

#[test]
fn tips_state_overwrite() {
    let db = in_memory_db();
    db.save_tips_state(3, crate::models::TipsShowMode::NewOnly)
        .unwrap();
    db.save_tips_state(5, crate::models::TipsShowMode::Never)
        .unwrap();
    let (seen_up_to, show_mode) = db.get_tips_state().unwrap();
    assert_eq!(seen_up_to, 5);
    assert_eq!(show_mode, crate::models::TipsShowMode::Never);
}
