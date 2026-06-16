#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::models::TaskStatus;

fn test_task(id: i64, title: &str) -> Task {
    test_task_repo(id, title, "/repo")
}

fn test_task_repo(id: i64, title: &str, repo: &str) -> Task {
    let mut t = make_task(id, TaskStatus::Backlog);
    t.title = title.to_string();
    t.repo_path = repo.to_string();
    t
}

#[test]
fn new_app_has_inactive_search() {
    let app = App::new(vec![]);
    assert_eq!(app.search.query, "");
    assert!(!app.search_active());
}

#[test]
fn search_query_filters_by_title_fuzzy() {
    let mut app = App::new(vec![
        test_task(1, "Fix login bug"),
        test_task(2, "Add search feature"),
        test_task(3, "Refactor parser"),
    ]);
    app.search.query = "srch".to_string(); // subsequence of "Add search feature"
    let titles: Vec<&str> = app
        .tasks_for_current_view()
        .iter()
        .map(|t| t.title.as_str())
        .collect();
    assert_eq!(titles, vec!["Add search feature"]);
}

#[test]
fn empty_search_query_is_noop() {
    let mut app = App::new(vec![test_task(1, "alpha"), test_task(2, "beta")]);
    app.search.query = "".to_string();
    assert_eq!(app.tasks_for_current_view().len(), 2);
}

#[test]
fn search_composes_with_repo_filter() {
    let mut app = App::new(vec![
        test_task_repo(1, "alpha task", "/repo/a"),
        test_task_repo(2, "alpha task", "/repo/b"),
    ]);
    app.filter.repos.insert("/repo/a".to_string());
    app.filter.mode = RepoFilterMode::Include;
    app.search.query = "alpha".to_string();
    let ids: Vec<i64> = app
        .tasks_for_current_view()
        .iter()
        .map(|t| t.id.0)
        .collect();
    assert_eq!(ids, vec![1]); // repo filter AND title match
}
