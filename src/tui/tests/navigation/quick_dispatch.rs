use super::*;

#[test]
fn repo_cursor_resets_on_entering_repo_path_mode() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.repo_cursor = 2; // cursor was left at position 2
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.set_buffer("some desc".to_string());
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(app.input.repo_cursor, 0, "cursor should reset to top");
}

#[test]
fn fuzzy_matches_empty_query_matches_anything() {
    assert!(super::fuzzy_matches("/some/path", ""));
    assert!(super::fuzzy_matches("", ""));
}

#[test]
fn fuzzy_matches_subsequence() {
    assert!(super::fuzzy_matches("/tmp", "tmp"));
    assert!(super::fuzzy_matches("/home/tmp", "tmp"));
    assert!(super::fuzzy_matches("/home/ragge/proj", "ragge"));
}

#[test]
fn fuzzy_matches_case_insensitive() {
    assert!(super::fuzzy_matches("/TMP", "tmp"));
    assert!(super::fuzzy_matches("/tmp", "TMP"));
}

#[test]
fn fuzzy_matches_no_match() {
    assert!(!super::fuzzy_matches("/var", "tmp"));
}

#[test]
fn fuzzy_matches_chars_must_be_in_order() {
    // "tp" is a valid subsequence of "/tmp" (t at 1, p at 3)
    assert!(super::fuzzy_matches("/tmp", "tp"));
    // "pt" requires p before t: p is at 3, t is at 1 -> false
    assert!(!super::fuzzy_matches("/tmp", "pt"));
}

#[test]
fn filtered_repos_empty_query_returns_all() {
    let paths = vec!["/a".to_string(), "/b".to_string()];
    assert_eq!(super::filtered_repos(&paths, ""), vec!["/a", "/b"]);
}

#[test]
fn filtered_repos_narrows_by_query() {
    let paths = vec![
        "/home/ragge/proj".to_string(),
        "/var/log".to_string(),
        "/home/other".to_string(),
    ];
    let result = super::filtered_repos(&paths, "home");
    assert_eq!(result, vec!["/home/ragge/proj", "/home/other"]);
}

#[test]
fn filtered_repos_no_matches_returns_empty() {
    let paths = vec!["/tmp".to_string(), "/var".to_string()];
    assert_eq!(super::filtered_repos(&paths, "xyz"), Vec::<String>::new());
}

#[test]
fn repo_cursor_resets_on_copy_task() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.repo_cursor = 2; // cursor was left at position 2
                               // Copy the first task (select it and press 'c')
    app.handle_key(make_key(KeyCode::Char('c')));
    // The copy opens on the tag step (CopyTask in docs/specs/tasks.allium);
    // Enter keeps the copied tag and hands over to the repo-path step.
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(
        app.input.repo_cursor, 0,
        "cursor should reset to top on copy"
    );
}

#[test]
fn move_repo_cursor_wraps_within_filtered_list() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/tmp".to_string(), "/var".to_string(), "/home".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    // Type "tmp" to filter — only /tmp matches (1 item)
    for c in "tmp".chars() {
        app.handle_key(make_key(KeyCode::Char(c)));
    }
    assert_eq!(app.input.repo_cursor, 0);
    // j wraps in a list of 1 — cursor stays at 0
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn typing_resets_repo_cursor_to_zero() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    // Navigate to position 2
    app.handle_key(make_key(KeyCode::Down));
    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.input.repo_cursor, 2);
    // Type a character — cursor should reset
    app.handle_key(make_key(KeyCode::Char('/')));
    assert_eq!(app.input.repo_cursor, 0);
}
