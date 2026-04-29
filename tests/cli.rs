//! Integration tests for the CLI commands (list, update, create).
//!
//! Each test spins up a fresh temp-file DB and invokes the compiled binary
//! via `std::process::Command`.

use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;

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

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

#[test]
fn create_from_plan() {
    let db = NamedTempFile::new().unwrap();
    let plan = make_plan_file("My Feature", "Implement X.");

    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "create",
            "--from-plan",
            plan.path().to_str().unwrap(),
            "--repo-path",
            "/tmp/test-repo",
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
        stdout.contains("Created task #"),
        "Expected task creation, got: {stdout}"
    );
    assert!(
        stdout.contains("My Feature"),
        "Expected title in output, got: {stdout}"
    );
    assert!(
        stdout.contains("[backlog]"),
        "Expected [backlog] status, got: {stdout}"
    );
}

#[test]
fn create_then_list() {
    let db = NamedTempFile::new().unwrap();
    let plan = make_plan_file("Auth Bug Fix", "Fix login.");
    let db_path = db.path().to_str().unwrap();

    let create_out = binary()
        .args([
            "--db",
            db_path,
            "create",
            "--from-plan",
            plan.path().to_str().unwrap(),
            "--repo-path",
            "/tmp/test-repo",
        ])
        .output()
        .unwrap();
    assert!(
        create_out.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&create_out.stderr)
    );

    let out = binary().args(["--db", db_path, "list"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Auth Bug Fix"),
        "Expected task title in list, got: {stdout}"
    );
    assert!(
        stdout.contains("backlog"),
        "Expected backlog status in list, got: {stdout}"
    );
}

#[test]
fn list_filter_by_status() {
    let db = NamedTempFile::new().unwrap();
    let plan = make_plan_file("Filter Test", "Filter tasks.");
    let db_path = db.path().to_str().unwrap();

    binary()
        .args([
            "--db",
            db_path,
            "create",
            "--from-plan",
            plan.path().to_str().unwrap(),
            "--repo-path",
            "/tmp/test-repo",
        ])
        .output()
        .unwrap();

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

#[test]
fn create_idempotent_for_same_plan() {
    let db = NamedTempFile::new().unwrap();
    let plan = make_plan_file("Idempotent Task", "No duplicates.");
    let db_path = db.path().to_str().unwrap();
    let plan_path = plan.path().to_str().unwrap();

    // First create
    binary()
        .args([
            "--db",
            db_path,
            "create",
            "--from-plan",
            plan_path,
            "--repo-path",
            "/tmp/test-repo",
        ])
        .output()
        .unwrap();

    // Second create with same plan
    let out = binary()
        .args([
            "--db",
            db_path,
            "create",
            "--from-plan",
            plan_path,
            "--repo-path",
            "/tmp/test-repo",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("already exists"),
        "Expected idempotency message, got: {stdout}"
    );

    // Only one task in list
    let out = binary().args(["--db", db_path, "list"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let task_lines: Vec<_> = stdout
        .lines()
        .filter(|l| l.contains("Idempotent Task"))
        .collect();
    assert_eq!(
        task_lines.len(),
        1,
        "Expected exactly one task, got: {stdout}"
    );
}

#[test]
fn create_with_title_and_description_overrides() {
    let db = NamedTempFile::new().unwrap();
    let plan = make_plan_file("Original Title", "Original goal.");
    let db_path = db.path().to_str().unwrap();

    let out = binary()
        .args([
            "--db",
            db_path,
            "create",
            "--from-plan",
            plan.path().to_str().unwrap(),
            "--repo-path",
            "/tmp/test-repo",
            "--title",
            "Custom Title",
            "--description",
            "Custom description",
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
        stdout.contains("Custom Title"),
        "Expected custom title, got: {stdout}"
    );

    // Verify in list output
    let out = binary().args(["--db", db_path, "list"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Custom Title"),
        "Expected custom title in list, got: {stdout}"
    );
}

#[test]
fn create_missing_plan_file_fails() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "create",
            "--from-plan",
            "/tmp/nonexistent-plan-file-12345.md",
            "--repo-path",
            "/tmp/test-repo",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Expected failure for missing plan file"
    );
}

// ---------------------------------------------------------------------------
// update
// ---------------------------------------------------------------------------

#[test]
fn update_changes_status() {
    let db = NamedTempFile::new().unwrap();
    let plan = make_plan_file("Update Test", "Test update.");
    let db_path = db.path().to_str().unwrap();

    binary()
        .args([
            "--db",
            db_path,
            "create",
            "--from-plan",
            plan.path().to_str().unwrap(),
            "--repo-path",
            "/tmp/test-repo",
        ])
        .output()
        .unwrap();

    // Update to running
    let out = binary()
        .args(["--db", db_path, "update", "1", "running"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Task 1 updated to running"),
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

// ---------------------------------------------------------------------------
// plan
// ---------------------------------------------------------------------------

#[test]
fn plan_attaches_to_existing_task() {
    let db = NamedTempFile::new().unwrap();
    let plan = make_plan_file("Plan Target", "Attach a plan.");
    let db_path = db.path().to_str().unwrap();

    // Create a task first
    binary()
        .args([
            "--db",
            db_path,
            "create",
            "--from-plan",
            plan.path().to_str().unwrap(),
            "--repo-path",
            "/tmp/test-repo",
        ])
        .output()
        .unwrap();

    // Write a separate plan file to attach
    let attach_plan = make_plan_file("Detailed Plan", "Step by step.");

    let out = binary()
        .args([
            "--db",
            db_path,
            "plan",
            "1",
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
        stdout.contains("Plan attached to task #1"),
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

#[test]
fn update_unknown_status_fails() {
    let db = NamedTempFile::new().unwrap();
    let plan = make_plan_file("Error Test", "Test errors.");
    let db_path = db.path().to_str().unwrap();

    binary()
        .args([
            "--db",
            db_path,
            "create",
            "--from-plan",
            plan.path().to_str().unwrap(),
            "--repo-path",
            "/tmp/test-repo",
        ])
        .output()
        .unwrap();

    let out = binary()
        .args(["--db", db_path, "update", "1", "bogus-status"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "Expected failure for unknown status");
}

// ---------------------------------------------------------------------------
// fetch-reviews
// ---------------------------------------------------------------------------

#[test]
fn fetch_reviews_no_settings_prints_empty_array() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args(["--db", db.path().to_str().unwrap(), "fetch-reviews"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let items: Vec<serde_json::Value> =
        serde_json::from_str(stdout.trim()).expect("output must be valid JSON array");
    assert!(items.is_empty(), "expected empty array, got: {stdout}");
}

// ---------------------------------------------------------------------------
// fetch-security
// ---------------------------------------------------------------------------

#[test]
fn fetch_security_no_settings_prints_empty_array() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args(["--db", db.path().to_str().unwrap(), "fetch-security"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let items: Vec<serde_json::Value> =
        serde_json::from_str(stdout.trim()).expect("output must be valid JSON array");
    assert!(items.is_empty(), "expected empty array, got: {stdout}");
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
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("0 valid items"),
        "Expected '0 valid items', got: {stdout}"
    );
}

#[test]
fn verify_feed_valid_items_succeeds() {
    let db = NamedTempFile::new().unwrap();
    let json = r#"[{"external_id":"dependabot:org/repo#1","title":"[HIGH] lodash RCE","description":"desc","url":"https://example.com","status":"backlog"}]"#;
    let cmd = format!("echo '{json}'");
    let out = binary()
        .args(["--db", db.path().to_str().unwrap(), "verify-feed", &cmd])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1 valid item"),
        "Expected '1 valid item', got: {stdout}"
    );
    assert!(
        stdout.contains("dependabot:org/repo#1"),
        "Expected external_id in output, got: {stdout}"
    );
    assert!(
        stdout.contains("[HIGH] lodash RCE"),
        "Expected title in output, got: {stdout}"
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
            "echo 'not valid json'",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Expected failure for invalid JSON"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to parse"),
        "Expected parse error in stderr, got: {stderr}"
    );
}

#[test]
fn verify_feed_command_failure_exits_nonzero() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "verify-feed",
            "exit 1",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Expected failure when command exits non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("command exited"),
        "Expected 'command exited' in stderr, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// tui — tmux guard
// ---------------------------------------------------------------------------

#[test]
fn tui_fails_without_tmux() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args(["--db", db.path().to_str().unwrap(), "tui"])
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Expected failure when TMUX is not set"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("tmux"),
        "Expected error mentioning tmux, got: {stderr}"
    );
}
