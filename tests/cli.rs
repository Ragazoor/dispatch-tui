#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for the CLI commands (plan, verify-feed, repo, and the
//! argv boundary of the hook-* subcommands — their behaviour, and the PR
//! gate's, lives in `tests/hooks.rs`).
//!
//! Most tests invoke the compiled binary via `std::process::Command`. The
//! commands that read or write shared rows (`repo`, `prune-repo-paths`,
//! `plan`) need a shared store since task #4916, so their bodies are tested
//! in-process through `dispatch_tui::cli::commands` against an
//! in-memory store, and the binary is
//! checked only for refusing them without a store. Task creation is no longer
//! exposed via the CLI — tests seed tasks through the DB API directly.

mod common;

use std::io::Write;
use std::path::Path;
use std::process::Command;
use tempfile::NamedTempFile;

use common::seed_task;
use dispatch_tui::cli::commands;
use dispatch_tui::store::{Database, TaskRead};

/// A fresh in-memory store for the in-process command tests. The temp file is
/// only a path that nothing opens, for the tests that also pass `--db` to the
/// binary.
async fn sqlite() -> (NamedTempFile, Database) {
    let tmp = NamedTempFile::new().unwrap();
    let db = Database::open_in_memory().await.unwrap();
    (tmp, db)
}

/// A named host file in `dir`: the identity every store-backed one-shot
/// command needs before it reaches for the store (cli.allium:
/// CliCommandsNeedAHostFile). The commands never write it.
fn write_host_file(dir: &Path) {
    let identity = dispatch_tui::host_file::HostIdentity {
        host_id: "cli-test-host".to_string(),
        label: Some("cli-test".to_string()),
        user_identity: Some("c0ffee".to_string()),
        credential: Some("token".to_string()),
    };
    std::fs::write(
        dispatch_tui::host_file::host_file_path(dir),
        serde_json::to_vec(&identity).unwrap(),
    )
    .unwrap();
}

/// `dispatch repo list`'s output.
async fn list(db: &Database) -> String {
    let mut out = Vec::new();
    commands::list_repos(db, &mut out).await.unwrap();
    String::from_utf8(out).unwrap()
}

/// **Test 2 of Phase 12a, at the binary, as reshaped by task #12296.** Every
/// command that reads or writes shared rows fails cleanly -- without touching a
/// local database no board reads any more -- when the store it is pointed at
/// cannot be reached, and says why.
///
/// It used to assert the refusal for naming no store. With the managed store,
/// naming none means `http://127.0.0.1:3000`, which on a developer's machine is
/// a REAL store that `repo set-verify` would write to; so this points every
/// command at a port nothing listens on, which is also what makes the test
/// independent of whatever else the machine is running.
#[test]
fn store_backed_commands_fail_cleanly_when_the_store_is_unreachable() {
    let tmp = tempfile::tempdir().unwrap();
    write_host_file(tmp.path());
    let db = tmp.path().join("dispatch.db");
    let db = db.to_str().unwrap();
    let plan = make_plan_file("A plan", "Goal.");
    let plan = plan.path().to_str().unwrap();
    let dead = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    for args in [
        vec!["repo", "list"],
        vec!["repo", "set-verify", "/r", "true"],
        vec!["prune-repo-paths"],
        vec!["plan", "1", plan],
    ] {
        let out = binary()
            .env_remove("DISPATCH_SPACETIME_SERVER")
            .args(["--db", db, "--spacetime-server", &dead])
            .args(&args)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "{args:?} must fail against an unreachable store"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("Could not connect to the shared store"),
            "{args:?} must say the store could not be reached, got: {stderr}"
        );
    }
}

/// `uninstall --purge` works in the data directory `--db` names, not only the
/// default one: a purge against a throwaway `--db` forgets that directory's
/// identity and leaves the real one alone. `$HOME` and `$XDG_DATA_HOME` point
/// into a temp directory, so nothing real is reachable either way.
#[test]
fn uninstall_purge_honours_db() {
    let home = tempfile::tempdir().unwrap();
    let xdg = home.path().join("xdg");
    let default_dir = xdg.join("dispatch");
    std::fs::create_dir_all(&default_dir).unwrap();
    write_host_file(&default_dir);
    let chosen = home.path().join("chosen");
    std::fs::create_dir_all(&chosen).unwrap();
    write_host_file(&chosen);
    std::fs::write(chosen.join("app.log"), b"log").unwrap();

    let mut child = binary()
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &xdg)
        .env_remove("DISPATCH_DB")
        .args([
            "--db",
            chosen.join("tasks.db").to_str().unwrap(),
            "uninstall",
            "--yes",
            "--purge",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"y\ny\n").unwrap();
    let out = child.wait_with_output().unwrap();

    assert!(out.status.success(), "{out:?}");
    assert!(
        !chosen.exists(),
        "the --db directory is purged (and removed, being empty)"
    );
    assert!(
        default_dir.join("host.json").exists(),
        "the default data directory is not the one named, so it is left alone"
    );
}

/// An `http://` address nothing listens on: claimed from the OS, then released.
fn dead_store_address() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

/// Task #4982, cli.allium: CliCommandsReachTheStoreWithoutManagingIt. With
/// nothing named, a command reaches the address a board on the same `--db`
/// recorded beside it (`store-server`, startup.allium: StoreAddressRecord) --
/// and when that board has gone without removing it, the command fails naming
/// THAT address rather than falling back to the managed one.
///
/// Only `repo list`, which is read-only: on a machine running its own store on
/// 127.0.0.1:3000, a command that ignored the record would reach that real
/// store, and this test must never write to it.
#[test]
fn a_command_with_nothing_named_reaches_the_store_recorded_beside_its_database() {
    let dir = tempfile::tempdir().unwrap();
    write_host_file(dir.path());
    let db = dir.path().join("dispatch.db");
    let recorded = dead_store_address();
    std::fs::write(dir.path().join("store-server"), format!("{recorded}\n")).unwrap();

    let out = binary()
        .env_remove("DISPATCH_SPACETIME_SERVER")
        .env_remove("DISPATCH_BOARD_STORE")
        .args(["--db", db.to_str().unwrap(), "repo", "list"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "the recorded store is unreachable, so the command must fail, got stderr: {stderr}"
    );
    let port = recorded.rsplit(':').next().unwrap();
    assert!(
        stderr.contains(port),
        "the failure must name the recorded address ({recorded}), got: {stderr}"
    );
    assert!(
        !stderr.contains("127.0.0.1:3000"),
        "a stale record must not fall back to the managed address, got: {stderr}"
    );
}

/// The environment comes before the record: a command started inside the
/// board's own tmux session follows the session's address.
#[test]
fn the_environment_wins_over_the_recorded_store() {
    let dir = tempfile::tempdir().unwrap();
    write_host_file(dir.path());
    let db = dir.path().join("dispatch.db");
    let recorded = dead_store_address();
    let from_env = dead_store_address();
    std::fs::write(dir.path().join("store-server"), format!("{recorded}\n")).unwrap();

    let out = binary()
        .env("DISPATCH_SPACETIME_SERVER", &from_env)
        .args(["--db", db.to_str().unwrap(), "repo", "list"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "stderr: {stderr}");
    let env_port = from_env.rsplit(':').next().unwrap();
    let recorded_port = recorded.rsplit(':').next().unwrap();
    assert!(
        stderr.contains(env_port) && !stderr.contains(recorded_port),
        "the environment's address ({from_env}) must be the one tried, not the \
         record's ({recorded}), got: {stderr}"
    );
}

/// The board's address, published on its tmux session, is followed by a
/// command run there, and comes before the record.
#[test]
fn the_boards_published_address_wins_over_the_recorded_store() {
    let dir = tempfile::tempdir().unwrap();
    write_host_file(dir.path());
    let db = dir.path().join("dispatch.db");
    let recorded = dead_store_address();
    let from_board = dead_store_address();
    std::fs::write(dir.path().join("store-server"), format!("{recorded}\n")).unwrap();

    let out = binary()
        .env_remove("DISPATCH_SPACETIME_SERVER")
        .env("DISPATCH_BOARD_STORE", &from_board)
        .args(["--db", db.to_str().unwrap(), "repo", "list"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "stderr: {stderr}");
    let board_port = from_board.rsplit(':').next().unwrap();
    let recorded_port = recorded.rsplit(':').next().unwrap();
    assert!(
        stderr.contains(board_port) && !stderr.contains(recorded_port),
        "the board's address ({from_board}) must be the one tried, got: {stderr}"
    );
}

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
// Removed subcommands
//
// `create`, `list` and `update` were CLI task-mutation surfaces. Tasks are
// created and mutated via MCP; the installed Claude Code hooks forward to the
// dedicated `hook-*` subcommands. Each must now be rejected by clap outright,
// so a stale hook script or muscle-memory invocation fails loudly instead of
// silently doing something.
// ---------------------------------------------------------------------------

/// A `--db` path that deliberately does not exist. Every assertion in this
/// section is about clap rejecting argv *before* anything opens a database, so
/// pointing at a path no `Database::open` could succeed on makes that claim
/// structural rather than merely asserted — and saves the tests a temp file
/// none of them ever reads.
const UNOPENABLE_DB: &str = "/nonexistent-dir/dispatch-test.db";

/// Assert `subcommand` is not a recognised `dispatch` subcommand.
fn assert_subcommand_removed(subcommand: &str) {
    let out = binary()
        .args(["--db", UNOPENABLE_DB, subcommand])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "{subcommand} must no longer be a recognised subcommand"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unrecognized subcommand") || stderr.contains("invalid value"),
        "expected clap rejection for {subcommand}, got stderr: {stderr}"
    );
}

#[test]
fn create_subcommand_removed() {
    assert_subcommand_removed("create");
}

#[test]
fn list_subcommand_removed() {
    assert_subcommand_removed("list");
}

#[test]
fn update_subcommand_removed() {
    assert_subcommand_removed("update");
}

// ---------------------------------------------------------------------------
// plan
// ---------------------------------------------------------------------------

#[tokio::test]
async fn plan_attaches_to_existing_task() {
    let db = std::sync::Arc::new(Database::open_in_memory().await.unwrap());
    let id = seed_task(&db, "Plan Target").await;
    let attach_plan = make_plan_file("Detailed Plan", "Step by step.");
    let plan_path = commands::resolve_plan_path(attach_plan.path()).unwrap();

    let mut out = Vec::new();
    commands::attach_plan(db.clone(), id.0, &plan_path, &mut out)
        .await
        .unwrap();
    let stdout = String::from_utf8(out).unwrap();
    assert!(
        stdout.contains(&format!("Plan attached to task #{}", id.0)),
        "Expected confirmation, got: {stdout}"
    );

    // The plan must actually be persisted (routing through the service path
    // writes it), not just echoed.
    let task = db.get_task(id).await.unwrap().unwrap();
    assert!(
        task.plan_path.is_some(),
        "Expected plan_path to be persisted, got None"
    );
}

#[tokio::test]
async fn plan_nonexistent_task_fails() {
    let db = std::sync::Arc::new(Database::open_in_memory().await.unwrap());
    let attach_plan = make_plan_file("Orphan Plan", "No task.");
    let plan_path = commands::resolve_plan_path(attach_plan.path()).unwrap();

    let err = commands::attach_plan(db.clone(), 9999, &plan_path, &mut Vec::new())
        .await
        .expect_err("attaching a plan to a missing task must fail");
    assert!(
        err.to_string().contains("not found"),
        "Expected 'not found' error, got: {err}"
    );
}

#[tokio::test]
async fn plan_nonexistent_file_fails() {
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
    assert_subcommand_removed("fetch-reviews");
}

#[test]
fn fetch_security_subcommand_removed() {
    assert_subcommand_removed("fetch-security");
}

// ---------------------------------------------------------------------------
// hook-*
//
// The hook subcommands no longer open a database: each delivers its event to
// the running board (see `HookDelivery` in `docs/specs/agent-health.allium`),
// and `tests/hooks.rs` owns their behaviour end to end. What stays here is the
// part that is still purely about argv — clap rejecting an action before
// anything is built or delivered, which is why these run against
// [`UNOPENABLE_DB`] and need no board at all.
// ---------------------------------------------------------------------------

/// Assert `subcommand` rejects `bogus` as its action argument with clap's own
/// invalid-value message, and that the message enumerates `valid`.
///
/// The action is parsed at the boundary by clap (`ValueEnum`), not by a
/// hand-rolled `match` inside the handler, so this happens before any request
/// is built — hence [`UNOPENABLE_DB`] and no board.
fn assert_action_rejected_by_clap(subcommand: &str, bogus: &str, valid: &[&str]) {
    let out = binary()
        .args(["--db", UNOPENABLE_DB, subcommand, "1", bogus])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "{subcommand} must reject `{bogus}` as an action"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("invalid value") && stderr.contains("possible values"),
        "expected clap's invalid-value message, got stderr: {stderr}"
    );
    for v in valid {
        assert!(
            stderr.contains(v),
            "clap must list `{v}` as a possible value, got stderr: {stderr}"
        );
    }
}

#[test]
fn hook_subagent_unknown_action_is_rejected_by_clap() {
    assert_action_rejected_by_clap("hook-subagent", "bogus", &["start", "stop", "clear"]);
}

// ---------------------------------------------------------------------------
// verify-feed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn verify_feed_empty_array_fails() {
    // An empty feed almost always means the command is misconfigured
    // (e.g. fetch-cve.sh with no repos). Treat it as a failure so the
    // operator notices, rather than silently passing.
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
        !out.status.success(),
        "Expected failure when feed command returns an empty array"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("0 items") || stderr.contains("empty"),
        "Expected empty-feed error message, got stderr: {stderr}"
    );
}

/// verify-feed is where a script author finds out they broke the wire format,
/// so the cross-field rule has to surface there with a message that names the
/// item — not just fail. See AReviewTaggedFeedItemNamesItsPr in
/// docs/specs/feeds.allium.
#[tokio::test]
async fn verify_feed_rejects_a_review_tagged_item_that_names_no_pr() {
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
        !out.status.success(),
        "a review-tagged item naming no PR must fail verify-feed"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("x1") && stderr.contains("pr-review"),
        "the failure must name the offending item and its tag; stderr: {stderr}"
    );
}

#[tokio::test]
async fn verify_feed_valid_items_succeeds() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "verify-feed",
            r#"echo '[{"external_id":"x1","title":"T","description":"","url":"https://github.com/o/r/pull/1","status":"backlog","tag":"pr-review"}]'"#,
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

#[tokio::test]
async fn verify_feed_reports_dropped_unrecognised_signal() {
    // feeds.allium (FeedItem.signals): an unrecognised signal is DROPPED, not
    // fatal — so the item still counts as valid and the exit status stays 0.
    // But the drop must be REPORTED. verify-feed is the only feed entry point
    // with no app.log sink, so without a stderr tracing subscriber the
    // deserialize_lenient_signals warning goes to a no-op dispatcher and a user
    // debugging a typo'd signal sees "✓ 1 valid item" with no hint anything was
    // discarded — the tool whose whole job is printing evidence losing the
    // evidence.
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        // Hermetic: the report must not depend on the developer's shell. Before
        // the filter was pinned to a fixed `warn`, RUST_LOG=dispatch_tui=error
        // suppressed the warning and this test failed.
        .env_remove("RUST_LOG")
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "verify-feed",
            r#"echo '[{"external_id":"x1","title":"T","description":"","url":"https://github.com/o/r/pull/1","status":"backlog","tag":"pr-review","signals":["reviewed","bogus"]}]'"#,
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "A dropped signal is non-fatal per feeds.allium; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("x1") && stdout.contains("1 valid item"),
        "Expected the item to still count as valid, got stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("dropping unrecognised feed signal"),
        "Expected the dropped-signal warning on stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("bogus"),
        "Expected the offending signal value on stderr, got: {stderr}"
    );
}

#[tokio::test]
async fn verify_feed_recognised_signals_produce_no_warning() {
    // Guards against a subscriber configured so noisily that a clean feed nags
    // on every run — the warning must fire for a dropped signal only.
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .env_remove("RUST_LOG")
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "verify-feed",
            r#"echo '[{"external_id":"x1","title":"T","description":"","url":"https://github.com/o/r/pull/1","status":"backlog","tag":"pr-review","signals":["reviewed"]}]'"#,
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("dropping unrecognised"),
        "A fully-recognised signal list must not warn, got stderr: {stderr}"
    );
}

#[tokio::test]
async fn verify_feed_missing_tag_fails() {
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

#[tokio::test]
async fn verify_feed_invalid_tag_fails() {
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

#[tokio::test]
async fn verify_feed_invalid_json_fails() {
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

#[tokio::test]
async fn verify_feed_surfaces_stderr_written_on_zero_exit() {
    // feeds.allium: FeedCommandStderrOnSuccess. A command that writes to
    // stderr internally but still exits 0 must have that stderr visible in
    // verify-feed's own output, not silently discarded — this is the third
    // exec of a feed command and the one debugging path a user has left
    // once app.log points them at a failure.
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "verify-feed",
            "echo 'boom' >&2; printf '[]'",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("boom"),
        "Expected the command's stderr to be surfaced, got stderr: {stderr}"
    );
}

#[tokio::test]
async fn verify_feed_command_failure_exits_nonzero() {
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

// ---------------------------------------------------------------------------
// prune-repo-paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn prune_repo_paths_removes_nonexistent_paths() {
    let (_tmp, db) = sqlite().await;
    // A path that exists on disk, and one that does not.
    let real_dir = tempfile::tempdir().unwrap();
    let real_path = real_dir.path().to_str().unwrap();
    let fake_path = "/tmp/dispatch-test-nonexistent-path-99999";
    seed_repo_path(&db, real_path).await;
    seed_repo_path(&db, fake_path).await;

    let mut out = Vec::new();
    commands::prune_repo_paths(&db, &mut out).await.unwrap();
    let stdout = String::from_utf8(out).unwrap();
    assert!(
        stdout.contains(fake_path),
        "expected removed path in output, got: {stdout}"
    );
    assert!(
        stdout.contains("1 path(s) removed"),
        "expected removal count in output, got: {stdout}"
    );

    let listed = list(&db).await;
    assert!(
        listed.contains(real_path),
        "real path must remain after prune, got: {listed}"
    );
    assert!(
        !listed.contains(fake_path),
        "fake path must be removed after prune, got: {listed}"
    );
}

#[tokio::test]
async fn prune_repo_paths_empty_db_succeeds() {
    let (_tmp, db) = sqlite().await;
    let mut out = Vec::new();
    commands::prune_repo_paths(&db, &mut out).await.unwrap();
    let stdout = String::from_utf8(out).unwrap();
    assert!(
        stdout.contains("0 path(s) removed"),
        "expected zero removals for empty DB, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// repo set-verify / clear-verify / list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatch_repo_set_verify_writes_command() {
    let (_tmp, db) = sqlite().await;
    commands::set_verify(&db, "/r", "cargo test", &mut Vec::new())
        .await
        .unwrap();

    let stdout = list(&db).await;
    assert!(stdout.contains("/r"), "path must appear in list");
    assert!(stdout.contains("cargo test"), "command must appear in list");
}

#[tokio::test]
async fn dispatch_repo_clear_verify_removes_command() {
    let (_tmp, db) = sqlite().await;
    commands::set_verify(&db, "/r", "cargo test", &mut Vec::new())
        .await
        .unwrap();
    commands::clear_verify(&db, "/r", &mut Vec::new())
        .await
        .unwrap();

    let stdout = list(&db).await;
    assert!(
        stdout.contains("/r"),
        "path row must still appear after clear"
    );
    assert!(!stdout.contains("cargo test"), "command must be cleared");
}

#[tokio::test]
async fn dispatch_repo_set_verify_rejects_newline() {
    let (_tmp, db) = sqlite().await;
    let err = commands::set_verify(&db, "/r", "a\nb", &mut Vec::new())
        .await
        .expect_err("expected failure for newline command");
    assert!(
        format!("{err:#}").to_lowercase().contains("single line"),
        "expected single-line error: {err:#}"
    );
}

#[tokio::test]
async fn dispatch_repo_set_verify_expands_tilde_in_path() {
    let (_tmp, db) = sqlite().await;
    commands::set_verify(&db, "~/r", "cargo test", &mut Vec::new())
        .await
        .unwrap();

    // list should show the expanded path, not the literal `~/r`
    let stdout = list(&db).await;
    let home = std::env::var("HOME").unwrap();
    let expanded = format!("{home}/r");
    assert!(
        stdout.contains(&expanded),
        "expected expanded path {expanded} in list output, got: {stdout}"
    );
    assert!(
        !stdout.contains("~/r"),
        "tilde path must NOT appear verbatim in list output, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// doctor subcommand removed (self-diagnosis surface retired)
// ---------------------------------------------------------------------------

/// The `doctor` self-diagnosis surface was retired. Its only remediation worth
/// keeping — pointing git at `.githooks` — is now the documented one-liner
/// `git config core.hooksPath .githooks` in CLAUDE.md's "First-time setup".
#[test]
fn doctor_subcommand_removed() {
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args(["--db", db.path().to_str().unwrap(), "doctor"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "doctor must no longer be a recognised subcommand, stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unrecognized subcommand") || stderr.contains("unexpected argument"),
        "expected clap to reject `doctor` as an unknown subcommand, stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// toggle-agent-tree-pane
// ---------------------------------------------------------------------------

#[tokio::test]
async fn toggle_agent_tree_pane_never_fails_without_a_real_tmux_session() {
    // This process has no real tmux session (or, if it happens to run inside
    // one, "task-999999999" doesn't name a real window in it either way).
    // The command must swallow the resulting tmux failure and exit 0 —
    // best-effort, matching the companion pane's decorative, non-critical
    // role everywhere else it's touched.
    let db = NamedTempFile::new().unwrap();
    let out = binary()
        .args([
            "--db",
            db.path().to_str().unwrap(),
            "toggle-agent-tree-pane",
            "task-999999999",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------------------
// repo status / repo sync (docs/specs/repo-sync.allium)
// ---------------------------------------------------------------------------

/// Seed a repo path via `repo set-verify`, which creates the row.
async fn seed_repo_path(db: &Database, path: &str) {
    commands::set_verify(db, path, "true", &mut Vec::new())
        .await
        .unwrap_or_else(|e| panic!("seeding {path} should succeed: {e:#}"));
}

async fn status(db: &Database, no_fetch: bool) -> String {
    let mut out = Vec::new();
    commands::repo_status(db, no_fetch, &mut out)
        .await
        .expect("measuring is read-only and never fails the command");
    String::from_utf8(out).unwrap()
}

// surface-provides.RepoStatusCli — the command exists and is read-only.
#[tokio::test]
async fn repo_status_reports_no_paths_for_an_empty_db() {
    let (_tmp, db) = sqlite().await;
    let stdout = status(&db, false).await;
    assert!(
        stdout.contains("No repo paths configured."),
        "got: {stdout}"
    );
}

// @guarantee UnmeasuredRowsShowNoCounts + UnmeasuredIsNeverPresentedAsClean: a
// repository that cannot be measured shows no ahead/behind figures and reports
// its fetch error instead.
#[tokio::test]
async fn repo_status_row_for_an_unmeasurable_repo_shows_no_counts() {
    let (_tmp, db) = sqlite().await;
    let missing = "/tmp/dispatch-test-not-a-repo-77777";
    seed_repo_path(&db, missing).await;

    let stdout = status(&db, false).await;
    assert!(stdout.contains(missing), "the row names the repo: {stdout}");
    assert!(
        stdout.contains("unknown"),
        "an unmeasurable repo reads as unknown, never as in sync: {stdout}"
    );
    assert!(
        !stdout.contains('\u{2191}') && !stdout.contains('\u{2193}'),
        "no ahead/behind figures may be quoted: {stdout}"
    );
}

// @guarantee FetchesUnlessSuppressed — the default fetches, so a repository
// whose fetch fails reports that error.
#[tokio::test]
async fn repo_status_fetches_by_default_and_reports_the_fetch_error() {
    let (_tmp, db) = sqlite().await;
    let missing = "/tmp/dispatch-test-not-a-repo-77778";
    seed_repo_path(&db, missing).await;

    let stdout = status(&db, false).await;
    assert!(
        stdout.to_lowercase().contains("fetch"),
        "expected the fetch failure in the row, got: {stdout}"
    );
}

// @guarantee FetchesUnlessSuppressed — --no-fetch skips the fetch, so there is
// no fetch error to report.
#[tokio::test]
async fn repo_status_no_fetch_skips_the_fetch() {
    let (_tmp, db) = sqlite().await;
    let missing = "/tmp/dispatch-test-not-a-repo-77779";
    seed_repo_path(&db, missing).await;

    let stdout = status(&db, true).await;
    assert!(stdout.contains(missing), "got: {stdout}");
    assert!(
        !stdout.to_lowercase().contains("fetch"),
        "--no-fetch performs no fetch, so no fetch error exists: {stdout}"
    );
}

// rule-failure.SyncRepoViaCli.1 — `requires: targets.count > 0`.
#[tokio::test(flavor = "multi_thread")]
async fn repo_sync_fails_when_there_are_no_saved_paths() {
    let (_tmp, db) = sqlite().await;
    assert!(
        commands::repo_sync(&db, None, &mut Vec::new(), &mut Vec::new())
            .await
            .is_err(),
        "no targets means nothing to sync, which is an error"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_sync_fails_for_an_unknown_path() {
    let (_tmp, db) = sqlite().await;
    seed_repo_path(&db, "/tmp/dispatch-test-saved-77780").await;

    assert!(
        commands::repo_sync(
            &db,
            Some("/tmp/dispatch-test-never-saved-77781".to_string()),
            &mut Vec::new(),
            &mut Vec::new()
        )
        .await
        .is_err(),
        "a path that is not a saved repo path is not a target"
    );
}

// @guarantee FailureIsVisibleInTheExitCode + EveryTargetAttempted: every target
// is attempted and the exit code is non-zero when any of them failed.
#[tokio::test(flavor = "multi_thread")]
async fn repo_sync_attempts_every_target_and_fails_the_exit_code() {
    let (_tmp, db) = sqlite().await;
    let a = "/tmp/dispatch-test-not-a-repo-77782";
    let b = "/tmp/dispatch-test-not-a-repo-77783";
    seed_repo_path(&db, a).await;
    seed_repo_path(&db, b).await;

    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert!(
        commands::repo_sync(&db, None, &mut out, &mut err)
            .await
            .is_err(),
        "a failed target must fail the command"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(&err)
    );
    assert!(
        combined.contains(a) && combined.contains(b),
        "one failure must not abandon the rest, got: {combined}"
    );
}

// ---------------------------------------------------------------------------
// statusline
// ---------------------------------------------------------------------------

const STATUS_PAYLOAD: &[u8] =
    br#"{"rate_limits":{"five_hour":{"used_percentage":5.0,"resets_at":9}}}"#;

/// Run the decorator the way Claude Code does — payload on stdin — and return
/// its output.
fn run_statusline(db: &Path, snapshot: &Path, chain: Option<&str>) -> std::process::Output {
    let mut command = binary();
    command.args([
        "--db",
        db.to_str().unwrap(),
        "statusline",
        "--snapshot",
        snapshot.to_str().unwrap(),
    ]);
    if let Some(chain) = chain {
        command.args(["--chain", chain]);
    }
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(STATUS_PAYLOAD)
        .unwrap();
    child.wait_with_output().unwrap()
}

/// The decorator runs several times a second in every dispatch-spawned Claude
/// session, so any database work there would be pure waste. The module keeps no
/// `Database` import, but that is a source property; this asserts the observable
/// one — running the subcommand brings no database into existence. See
/// docs/specs/dispatch.allium: StatusLineDecorator
/// (`@guarantee NeverReadsOrWritesTheDatabase`).
#[test]
fn statusline_creates_no_database_file() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("tasks.db");
    let snapshot = tmp.path().join("rate-limits.json");

    let out = run_statusline(&db_path, &snapshot, None);

    assert!(out.status.success(), "the decorator must always exit 0");
    assert!(
        snapshot.exists(),
        "the snapshot must have been published, or this proves nothing about the DB"
    );
    assert!(
        !db_path.exists(),
        "the statusline subcommand must not create a database file"
    );
}

/// The decorator does no async work, so it must start none of the machinery for
/// it — a multi-thread tokio runtime would spin up one worker per core plus the
/// reactor on every 300 ms debounce tick, in every session. See
/// docs/specs/dispatch.allium: StatusLineDecorator (`@guarantee
/// StartsNoAsyncRuntime`).
///
/// Observed rather than asserted from source: the chained command's parent *is*
/// the `dispatch` process, so `/proc/$PPID/task` is that process's live thread
/// count while the decorator waits on the chain. `run_bounded` accounts for at
/// most three threads there (the stdin writer plus the two output drains) on top
/// of `main`, hence the bound of 4 — an inequality, because the stdin writer may
/// already have finished. Add a background thread to `run_bounded` and this
/// bound needs revisiting.
#[test]
#[cfg(target_os = "linux")]
fn statusline_starts_no_worker_thread_pool() {
    let tmp = tempfile::tempdir().unwrap();
    let out = run_statusline(
        &tmp.path().join("tasks.db"),
        &tmp.path().join("rate-limits.json"),
        Some("ls /proc/$PPID/task | wc -l"),
    );

    assert!(out.status.success(), "the decorator must always exit 0");
    let printed = String::from_utf8_lossy(&out.stdout);
    let threads: usize = printed
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("expected a thread count, got {printed:?}: {e}"));
    assert!(
        threads <= 4,
        "statusline must run without a worker-thread pool, saw {threads} threads"
    );
}

// ---------------------------------------------------------------------------
// caller-headers
// ---------------------------------------------------------------------------

/// The `headersHelper` runs on every MCP session start and reconnect. Its whole
/// contract is one line of JSON on stdout and exit 0; these lock it at the
/// process level, where the unit tests on `resolve_headers_for_path` cannot see
/// it. See docs/specs/mcp-task-tools.allium (CreateTaskViaMcp guidance).
#[test]
fn caller_headers_emits_session_header_outside_a_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let out = binary()
        .arg("caller-headers")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "caller-headers must exit 0");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["X-Caller-Kind"], "session");
}

/// The helper has one answer. Standing inside a worktree — the shape it used to
/// read a task identity out of — changes nothing, because Claude Code never runs
/// it there: a user-global helper runs from Claude Code's own configuration
/// directory. A dispatched agent's identity comes from its launch instead.
#[test]
fn caller_headers_emits_session_even_inside_a_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let wt = tmp.path().join(".worktrees").join("3840-some-slug");
    std::fs::create_dir_all(&wt).unwrap();
    let out = binary()
        .arg("caller-headers")
        .current_dir(&wt)
        .output()
        .unwrap();
    assert!(out.status.success(), "caller-headers must exit 0");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["X-Caller-Kind"], "session");
    assert!(
        v.get("X-Caller-Task-Id").is_none(),
        "the helper must never claim a task identity: {v}"
    );
}

/// startup.allium: AbortWhenTheStoreIsNotTheOneThisInstallUses — the flag
/// that accepts a store switch is part of the launch command (surface
/// BoardLaunchCommand), documented where the operator will look for it.
#[test]
fn tui_accepts_the_store_switch_flag() {
    let out = binary().args(["tui", "--help"]).output().unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(
        help.contains("--accept-store-switch"),
        "`dispatch tui --help` must offer --accept-store-switch:\n{help}"
    );
}
