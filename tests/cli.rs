#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for the CLI commands (list, update, plan, verify-feed).
//!
//! Each test spins up a fresh temp-file DB and invokes the compiled binary
//! via `std::process::Command`. Task creation is no longer exposed via the
//! CLI — tests seed tasks through the DB API directly.

use std::io::Write;
use std::path::Path;
use std::process::Command;
use tempfile::NamedTempFile;

use dispatch_tui::db::{CreateTaskRequest, Database, ProjectCrud, TaskCrud};
use dispatch_tui::models::{TaskId, TaskStatus};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_dispatch"))
}

fn make_plan_file(title: &str, goal: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(
        f,
        "# {title} \u{2014} Implementation Plan\n\n**Goal:** {goal}"
    )
    .unwrap();
    f
}

/// Seed a backlog task directly via the DB API so we can drive the
/// `update` / `list` / `plan` subcommands without the (removed) `create`
/// subcommand.
fn seed_task(db_path: &Path, title: &str) -> TaskId {
    let db = Database::open(db_path).unwrap();
    let project_id = db.get_default_project().unwrap().id;
    db.create_task(CreateTaskRequest {
        title,
        description: "",
        repo_path: "/tmp/test-repo",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        project_id,
    })
    .unwrap()
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[test]
fn list_empty_db() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args(["--db", db.path().to_str().unwrap(), "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No tasks found."),
        "Expected 'No tasks found.', got: {stdout}"
    );
}

#[test]
fn list_unknown_status_fails() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "list",
            "--status",
            "bogus",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success(), "Expected failure for unknown status");
}

#[test]
fn list_filter_by_status() {
    let db = NamedTempFile::new().unwrap();
    let db_path = db.path().to_str().unwrap();
    seed_task(db.path(), "Filter Test");

    // list --status backlog -> shows the task
    let out = binary()
        .args(["--db", db_path, "list", "--status", "backlog"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Filter Test"),
        "Expected task in backlog list, got: {stdout}"
    );

    // list --status running -> empty
    let out = binary()
        .args(["--db", db_path, "list", "--status", "running"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No tasks found."),
        "Expected no running tasks, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// create subcommand removed (tasks are created via MCP only)
// ---------------------------------------------------------------------------

#[test]
fn create_subcommand_removed() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args(["--db", db.path().to_str().unwrap(), "create"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "create must no longer be a recognised subcommand"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unrecognized subcommand") || stderr.contains("invalid value"),
        "expected clap rejection, got stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// update
// ---------------------------------------------------------------------------

#[test]
fn update_changes_status() {
    let db = NamedTempFile::new().unwrap();
    let db_path = db.path().to_str().unwrap();
    let id = seed_task(db.path(), "Update Test");

    // Update to running
    let out = binary()
        .args(["--db", db_path, "update", &id.0.to_string(), "running"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("Task {} updated to running", id.0)),
        "Expected update confirmation, got: {stdout}"
    );

    // Verify via list
    let out = binary()
        .args(["--db", db_path, "list", "--status", "running"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Update Test"),
        "Expected task in running list, got: {stdout}"
    );
}

#[test]
fn update_unknown_status_fails() {
    let db = NamedTempFile::new().unwrap();
    let db_path = db.path().to_str().unwrap();
    let id = seed_task(db.path(), "Error Test");

    let out = binary()
        .args(["--db", db_path, "update", &id.0.to_string(), "bogus-status"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "Expected failure for unknown status");
}

// ---------------------------------------------------------------------------
// plan
// ---------------------------------------------------------------------------

#[test]
fn plan_attaches_to_existing_task() {
    let db = NamedTempFile::new().unwrap();
    let db_path = db.path().to_str().unwrap();
    let id = seed_task(db.path(), "Plan Target");

    let attach_plan = make_plan_file("Detailed Plan", "Step by step.");

    let out = binary()
        .args([
            "--db",
            db_path,
            "plan",
            &id.0.to_string(),
            attach_plan.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("Plan attached to task #{}", id.0)),
        "Expected confirmation, got: {stdout}"
    );
}

#[test]
fn plan_nonexistent_file_fails() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "plan",
            "1",
            "/tmp/nonexistent-plan-99999.md",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Expected failure for missing plan file"
    );
}

// ---------------------------------------------------------------------------
// fetch-reviews / fetch-security have been removed; users wire their own
// shell scripts as feed_command. These tests pin the removal so a future
// re-introduction has to opt back in deliberately.
// ---------------------------------------------------------------------------

#[test]
fn fetch_reviews_subcommand_removed() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args(["--db", db.path().to_str().unwrap(), "fetch-reviews"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "fetch-reviews must no longer be a recognised subcommand"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unrecognized subcommand") || stderr.contains("invalid value"),
        "expected clap rejection, got stderr: {stderr}"
    );
}

#[test]
fn fetch_security_subcommand_removed() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args(["--db", db.path().to_str().unwrap(), "fetch-security"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "fetch-security must no longer be a recognised subcommand"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unrecognized subcommand") || stderr.contains("invalid value"),
        "expected clap rejection, got stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// verify-feed
// ---------------------------------------------------------------------------

#[test]
fn verify_feed_valid_empty_array_succeeds() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "verify-feed",
            "echo '[]'",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn verify_feed_valid_items_succeeds() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "verify-feed",
            r#"echo '[{"external_id":"x1","title":"T","description":"","status":"backlog","tag":"pr-review"}]'"#,
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("x1"),
        "Expected feed item id in output, got: {stdout}"
    );
    assert!(
        stdout.contains("TAG"),
        "Expected TAG header in output, got: {stdout}"
    );
    assert!(
        stdout.contains("pr-review"),
        "Expected tag value in output, got: {stdout}"
    );
}

#[test]
fn verify_feed_missing_tag_fails() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "verify-feed",
            r#"echo '[{"external_id":"x1","title":"T","description":"","status":"backlog"}]'"#,
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Expected failure when feed item is missing tag"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to parse") && stderr.contains("tag"),
        "Expected parse error mentioning tag, got stderr: {stderr}"
    );
}

#[test]
fn verify_feed_invalid_tag_fails() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "verify-feed",
            r#"echo '[{"external_id":"x1","title":"T","description":"","status":"backlog","tag":"nonsense"}]'"#,
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Expected failure when feed item has unknown tag value"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to parse"),
        "Expected parse error, got stderr: {stderr}"
    );
}

#[test]
fn verify_feed_invalid_json_fails() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "verify-feed",
            "echo 'not json'",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Expected failure for invalid JSON output"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to parse"),
        "Expected parse error, got stderr: {stderr}"
    );
}

#[test]
fn verify_feed_command_failure_exits_nonzero() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args(["--db", db.path().to_str().unwrap(), "verify-feed", "exit 7"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Expected failure when feed command exits non-zero"
    );
}
