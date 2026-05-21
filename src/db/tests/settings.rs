#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

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
async fn filter_presets_save_and_list() {
    let db = Database::open_in_memory().await.unwrap();
    db.save_filter_preset(
        "frontend",
        &["/repo-a".to_string(), "/repo-b".to_string()],
        "include",
    )
    .await
    .unwrap();
    db.save_filter_preset("backend", &["/repo-c".to_string()], "exclude")
        .await
        .unwrap();

    let presets = db.list_filter_presets().await.unwrap();
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

#[tokio::test]
async fn filter_presets_overwrite_and_delete() {
    let db = Database::open_in_memory().await.unwrap();
    db.save_filter_preset("frontend", &["/repo-a".to_string()], "include")
        .await
        .unwrap();
    db.save_filter_preset(
        "frontend",
        &["/repo-x".to_string(), "/repo-y".to_string()],
        "exclude",
    )
    .await
    .unwrap();

    let presets = db.list_filter_presets().await.unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(
        presets[0].1,
        vec!["/repo-x".to_string(), "/repo-y".to_string()]
    );
    assert_eq!(presets[0].2, "exclude");

    db.delete_filter_preset("frontend").await.unwrap();
    let presets = db.list_filter_presets().await.unwrap();
    assert!(presets.is_empty());
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
async fn delete_repo_path_cleans_presets() {
    let db = in_memory_db().await;
    db.save_repo_path("/home/user/a").await.unwrap();
    db.save_repo_path("/home/user/b").await.unwrap();
    db.save_filter_preset(
        "my_preset",
        &["/home/user/a".to_string(), "/home/user/b".to_string()],
        "include",
    )
    .await
    .unwrap();
    db.delete_repo_path("/home/user/a").await.unwrap();
    let presets = db.list_filter_presets().await.unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0].0, "my_preset");
    assert_eq!(presets[0].1, vec!["/home/user/b".to_string()]);
}

#[tokio::test]
async fn delete_repo_path_removes_empty_preset() {
    let db = in_memory_db().await;
    db.save_repo_path("/home/user/solo").await.unwrap();
    db.save_filter_preset("solo_preset", &["/home/user/solo".to_string()], "include")
        .await
        .unwrap();
    db.delete_repo_path("/home/user/solo").await.unwrap();
    let presets = db.list_filter_presets().await.unwrap();
    assert!(presets.is_empty());
}

#[tokio::test]
async fn tips_state_defaults_on_fresh_db() {
    let db = in_memory_db().await;
    let (seen_up_to, show_mode) = db.get_tips_state().await.unwrap();
    assert_eq!(seen_up_to, 0);
    assert_eq!(show_mode, crate::models::TipsShowMode::Always);
}

#[tokio::test]
async fn tips_state_round_trip() {
    let db = in_memory_db().await;
    db.save_tips_state(7, crate::models::TipsShowMode::NewOnly)
        .await
        .unwrap();
    let (seen_up_to, show_mode) = db.get_tips_state().await.unwrap();
    assert_eq!(seen_up_to, 7);
    assert_eq!(show_mode, crate::models::TipsShowMode::NewOnly);
}

#[tokio::test]
async fn tips_state_overwrite() {
    let db = in_memory_db().await;
    db.save_tips_state(3, crate::models::TipsShowMode::NewOnly)
        .await
        .unwrap();
    db.save_tips_state(5, crate::models::TipsShowMode::Never)
        .await
        .unwrap();
    let (seen_up_to, show_mode) = db.get_tips_state().await.unwrap();
    assert_eq!(seen_up_to, 5);
    assert_eq!(show_mode, crate::models::TipsShowMode::Never);
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
async fn verify_command_empty_clears() {
    let db = in_memory_db().await;
    db.save_repo_path("/r").await.unwrap();
    db.set_verify_command("/r", Some("cargo test"))
        .await
        .unwrap();
    db.set_verify_command("/r", Some("")).await.unwrap();
    assert_eq!(db.get_verify_command("/r").await.unwrap(), None);
    db.set_verify_command("/r", Some("   ")).await.unwrap();
    assert_eq!(db.get_verify_command("/r").await.unwrap(), None);
    db.set_verify_command("/r", None).await.unwrap();
    assert_eq!(db.get_verify_command("/r").await.unwrap(), None);
}

#[tokio::test]
async fn verify_command_rejects_newline() {
    let db = in_memory_db().await;
    db.save_repo_path("/r").await.unwrap();
    let err = db.set_verify_command("/r", Some("a\nb")).await.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("newline"),
        "expected newline error, got: {err}"
    );
    let err = db.set_verify_command("/r", Some("a\rb")).await.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("carriage return"),
        "expected carriage return error, got: {err}"
    );
}

#[tokio::test]
async fn verify_command_set_some_creates_row() {
    let db = in_memory_db().await;
    db.set_verify_command("/new/path", Some("cargo test"))
        .await
        .unwrap();
    assert!(db
        .list_repo_paths()
        .await
        .unwrap()
        .iter()
        .any(|p| p == "/new/path"));
    assert_eq!(
        db.get_verify_command("/new/path").await.unwrap(),
        Some("cargo test".to_string())
    );
}

#[tokio::test]
async fn verify_command_set_none_on_unknown_path_is_noop() {
    let db = in_memory_db().await;
    db.set_verify_command("/unknown", None).await.unwrap();
    assert!(!db
        .list_repo_paths()
        .await
        .unwrap()
        .iter()
        .any(|p| p == "/unknown"));
}

#[tokio::test]
async fn verify_command_get_unknown_path_is_none() {
    let db = in_memory_db().await;
    assert_eq!(
        db.get_verify_command("/does/not/exist").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn list_filter_presets_errors_on_corrupt_json() {
    let db = in_memory_db().await;
    db.db_call(move |conn| {
        conn.execute(
            "INSERT INTO filter_presets (name, repo_paths, mode) VALUES (?1, ?2, ?3)",
            rusqlite::params!["bad_preset", "{not json", "all"],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let result = db.list_filter_presets().await;
    assert!(
        result.is_err(),
        "expected Err on corrupt filter preset JSON, got {:?}",
        result
    );
}

#[tokio::test]
async fn delete_repo_path_errors_on_corrupt_preset_json() {
    let db = in_memory_db().await;
    db.save_repo_path("/repo").await.unwrap();
    db.db_call(move |conn| {
        conn.execute(
            "INSERT INTO filter_presets (name, repo_paths, mode) VALUES (?1, ?2, ?3)",
            rusqlite::params!["bad_preset", "{not json", "all"],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let result = db.delete_repo_path("/repo").await;
    assert!(
        result.is_err(),
        "expected Err when corrupt preset JSON is encountered during delete"
    );
}
