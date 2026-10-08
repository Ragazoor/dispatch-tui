use super::*;
use crate::cli::agent_tree::test_repo::TestRepo;
use crate::process::{MockProcessRunner, RealProcessRunner};
use serde_json::{json, Value};

fn run(repo: &TestRepo, base: &str, commit: Option<&str>) -> Value {
    let report = report(repo.root(), base, commit, &RealProcessRunner::default());
    serde_json::to_value(report).expect("serialise")
}

/// Rows come in display order, depth first, a folder's own files before its
/// subfolders, single-child chains merged into one row, and only the folder
/// nearest the files draws counts (AgentTreeModRow).
#[test]
fn rows_are_the_companion_panes_tree_flattened() {
    let repo = TestRepo::new();
    repo.write("src/tui/ui/a.rs", "a\nb\n");
    repo.write("src/lib.rs", "x\n");
    repo.commit_all("seed files");
    repo.write("src/tui/ui/a.rs", "a\nB\nc\n");
    repo.write("src/lib.rs", "y\n");
    repo.write("new.txt", "n\n");

    let out = run(&repo, "main", None);
    assert_eq!(
        out["tree"],
        json!({"rows": [
            {"depth": 0, "name": "new.txt", "path": "new.txt", "kind": "file",
             "badge": "added", "counts": null},
            {"depth": 0, "name": "src", "path": "src", "kind": "directory",
             "badge": null, "counts": null},
            {"depth": 1, "name": "lib.rs", "path": "src/lib.rs", "kind": "file",
             "badge": "modified", "counts": {"added": 1, "removed": 1}},
            {"depth": 1, "name": "tui/ui", "path": "src/tui/ui", "kind": "directory",
             "badge": null, "counts": {"added": 2, "removed": 1}},
            {"depth": 2, "name": "a.rs", "path": "src/tui/ui/a.rs", "kind": "file",
             "badge": "modified", "counts": {"added": 2, "removed": 1}},
        ]})
    );
}

/// The commits half lists the agent's commits since the fork point, newest
/// first, as (id, subject).
#[test]
fn commits_since_the_fork_point_are_listed_newest_first() {
    let repo = TestRepo::new();
    repo.write("a.txt", "a\n");
    let first = repo.commit_all("first");
    repo.write("b.txt", "b\n");
    let second = repo.commit_all("second");

    let out = run(&repo, "main", None);
    assert_eq!(
        out["commits"],
        json!({"commits": [
            {"id": second, "subject": "second"},
            {"id": first, "subject": "first"},
        ]})
    );
}

/// A selected commit is shown against its parent.
#[test]
fn a_selected_commit_shows_that_commit() {
    let repo = TestRepo::new();
    repo.write("a.txt", "a\n");
    let id = repo.commit_all("first");
    repo.write("b.txt", "unstaged\n");

    let out = run(&repo, "main", Some(&id));
    assert_eq!(
        out["tree"],
        json!({"rows": [
            {"depth": 0, "name": "a.txt", "path": "a.txt", "kind": "file",
             "badge": "added", "counts": {"added": 1, "removed": 0}},
        ]})
    );
}

/// A base branch that will not resolve fails the commits half and leaves the
/// tree working (RefreshAgentTreeModPane: the halves fail apart).
#[test]
fn an_unknown_base_fails_only_the_commits() {
    let repo = TestRepo::new();
    repo.write("a.txt", "a\n");

    let out = run(&repo, "no-such-branch", None);
    assert_eq!(out["tree"]["rows"].as_array().map(Vec::len), Some(1));
    let reason = out["commits"]["error"].as_str().expect("commits error");
    assert!(reason.contains("no-such-branch"), "{reason}");
    assert!(out["commits"].get("commits").is_none());
}

/// A git that cannot run fails the tree half with git's reason.
#[test]
fn a_failing_git_fails_the_tree_with_its_reason() {
    // Every git call fails, the commits half's too.
    let runner = MockProcessRunner::new(
        (0..8)
            .map(|_| {
                MockProcessRunner::fail("fatal: Unable to create '.git/index.lock': File exists.")
            })
            .collect(),
    );
    let report = report(std::path::Path::new("/nowhere"), "main", None, &runner);
    let out = serde_json::to_value(report).expect("serialise");
    let reason = out["tree"]["error"].as_str().expect("tree error");
    assert!(reason.contains("index.lock"), "{reason}");
}
