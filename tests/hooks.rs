#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for the `hook-*` subcommands and the PR gate, after they
//! stopped opening the database.
//!
//! Every hook now delivers to the running board's HTTP server (see
//! `HookDelivery` in `docs/specs/agent-health.allium`). These tests therefore
//! stand a real board up on a free port, point a real `dispatch` process at it
//! with `--port`, and assert the state the board wrote — the same state the
//! hook used to write itself.

mod common;

use std::path::Path;

use tokio::process::Command;

use common::{dead_port, repo_file, seed_running_task, seed_task, spawn_board, Board};
use dispatch_tui::db::{Database, TaskCrud, TaskPatch};
use dispatch_tui::mcp::BackgroundWrite;
use dispatch_tui::models::{SubStatus, TaskStatus};

/// Run `dispatch <args…> --port <port>`. Returns the exit status and stderr.
async fn run(port: u16, args: &[&str]) -> (std::process::Output, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dispatch"))
        .args(args)
        .args(["--port", &port.to_string()])
        .output()
        .await
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (out, stderr)
}

/// [`run`], reduced to what the observer hooks assert: success and stderr.
async fn run_hook(port: u16, args: &[&str]) -> (bool, String) {
    let (out, stderr) = run(port, args).await;
    (out.status.success(), stderr)
}

/// [`run`], reduced to what the gate asserts: the exit code (2 blocks) and
/// lowercased stderr, which is where the reminder goes.
async fn run_pr_gate(port: u16, id: &str) -> (Option<i32>, String) {
    let (out, stderr) = run(port, &["pr-gate", id]).await;
    (out.status.code(), stderr.to_lowercase())
}

/// Seed a running task on `board` and return its id as the string the CLI
/// takes.
async fn seed(board: &Board, title: &str) -> (dispatch_tui::models::TaskId, String) {
    let id = seed_running_task(&board.db_path(), title, SubStatus::Active).await;
    (id, id.0.to_string())
}

// ---------------------------------------------------------------------------
// 1. Each hook kind posts to the board and mutates the state it used to
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hook_pre_tool_use_stamps_activity_through_the_board() {
    let board = spawn_board().await;
    let id = seed_running_task(&board.db_path(), "PreToolUse", SubStatus::Active).await;

    let (ok, stderr) = run_hook(board.port, &["hook", &id.0.to_string(), "pre_tool_use"]).await;
    assert!(ok, "stderr: {stderr}");

    let task = board.task(id).await;
    assert!(
        task.last_pre_tool_use_at.is_some(),
        "the board must have stamped last_pre_tool_use_at"
    );
}

#[tokio::test]
async fn hook_notification_sets_needs_input_through_the_board() {
    let board = spawn_board().await;
    let id = seed_running_task(&board.db_path(), "Notification", SubStatus::Active).await;

    let (ok, stderr) = run_hook(board.port, &["hook", &id.0.to_string(), "notification"]).await;
    assert!(ok, "stderr: {stderr}");

    let task = board.task(id).await;
    assert_eq!(task.sub_status, SubStatus::NeedsInput);
    assert!(task.last_notification_at.is_some());
}

#[tokio::test]
async fn hook_notification_kind_reaches_the_board() {
    let board = spawn_board().await;
    let id = seed_running_task(&board.db_path(), "Notification kind", SubStatus::Active).await;

    // `auth_success` is the one subtype that is a deliberate no-op, so it
    // proves the subtype travelled: a dropped `--kind` would raise instead.
    let (ok, stderr) = run_hook(
        board.port,
        &[
            "hook",
            &id.0.to_string(),
            "notification",
            "--kind",
            "auth_success",
        ],
    )
    .await;
    assert!(ok, "stderr: {stderr}");

    let task = board.task(id).await;
    assert_eq!(
        task.sub_status,
        SubStatus::Active,
        "auth_success must not raise needs_input"
    );
}

#[tokio::test]
async fn hook_stop_defers_or_flips_through_the_board() {
    let board = spawn_board().await;
    let id = seed_running_task(&board.db_path(), "Stop", SubStatus::Active).await;

    let (ok, stderr) = run_hook(board.port, &["hook", &id.0.to_string(), "stop"]).await;
    assert!(ok, "stderr: {stderr}");

    let task = board.task(id).await;
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "a Stop with no live subagent flips straight to review"
    );
}

#[tokio::test]
async fn hook_user_prompt_submit_returns_to_running_through_the_board() {
    let board = spawn_board().await;
    let id = seed_task(&board.db_path(), "UserPromptSubmit").await;
    let conn = Database::open(&board.db_path()).await.unwrap();
    conn.patch_task(id, &TaskPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();
    drop(conn);

    let (ok, stderr) = run_hook(
        board.port,
        &["hook", &id.0.to_string(), "user_prompt_submit"],
    )
    .await;
    assert!(ok, "stderr: {stderr}");

    let task = board.task(id).await;
    assert_eq!(task.status, TaskStatus::Running);
}

#[tokio::test]
async fn hook_subagent_start_then_stop_round_trips_through_the_board() {
    let board = spawn_board().await;
    let id = seed_running_task(&board.db_path(), "Subagent", SubStatus::Active).await;
    let task_id = id.0.to_string();

    let (ok, stderr) = run_hook(
        board.port,
        &[
            "hook-subagent",
            &task_id,
            "start",
            "--agent-id",
            "a1",
            "--session-id",
            "s1",
        ],
    )
    .await;
    assert!(ok, "stderr: {stderr}");
    assert_eq!(board.task(id).await.live_subagents, 1);

    let (ok, stderr) = run_hook(
        board.port,
        &[
            "hook-subagent",
            &task_id,
            "stop",
            "--agent-id",
            "a1",
            "--session-id",
            "s1",
        ],
    )
    .await;
    assert!(ok, "stderr: {stderr}");
    assert_eq!(board.task(id).await.live_subagents, 0);
}

#[tokio::test]
async fn hook_subagent_clear_voids_a_pending_stop_through_the_board() {
    let board = spawn_board().await;
    let id = seed_running_task(&board.db_path(), "Subagent clear", SubStatus::Active).await;
    let task_id = id.0.to_string();

    for args in [
        vec![
            "hook-subagent",
            &task_id,
            "start",
            "--agent-id",
            "a1",
            "--session-id",
            "s1",
        ],
        vec!["hook", &task_id, "stop"],
    ] {
        let (ok, stderr) = run_hook(board.port, &args).await;
        assert!(ok, "stderr: {stderr}");
    }
    let task = board.task(id).await;
    assert!(task.stop_pending, "precondition: the Stop must be deferred");

    let (ok, stderr) = run_hook(board.port, &["hook-subagent", &task_id, "clear"]).await;
    assert!(ok, "stderr: {stderr}");

    let task = board.task(id).await;
    assert_eq!(task.status, TaskStatus::Running);
    assert!(!task.stop_pending);
    assert_eq!(task.live_subagents, 0);
}

#[tokio::test]
async fn hook_shell_start_then_stop_round_trips_through_the_board() {
    let board = spawn_board().await;
    let id = seed_running_task(&board.db_path(), "Shell", SubStatus::Active).await;
    let task_id = id.0.to_string();

    let (ok, stderr) = run_hook(
        board.port,
        &[
            "hook-shell",
            &task_id,
            "start",
            "--shell-id",
            "bash_1",
            "--session-id",
            "s1",
        ],
    )
    .await;
    assert!(ok, "stderr: {stderr}");
    assert_eq!(board.task(id).await.live_shells, 1);

    let (ok, stderr) = run_hook(
        board.port,
        &[
            "hook-shell",
            &task_id,
            "stop",
            "--shell-id",
            "bash_1",
            "--session-id",
            "s1",
        ],
    )
    .await;
    assert!(ok, "stderr: {stderr}");
    assert_eq!(board.task(id).await.live_shells, 0);
}

#[tokio::test]
async fn hook_peer_message_stamps_sender_and_target_through_the_board() {
    let board = spawn_board().await;
    let sender = seed_running_task(&board.db_path(), "Sender", SubStatus::Active).await;
    let target = seed_running_task(&board.db_path(), "Target", SubStatus::Active).await;

    let (ok, stderr) = run_hook(
        board.port,
        &[
            "hook-peer-message",
            &sender.0.to_string(),
            "--target",
            &format!("task-{}", target.0),
            "--body",
            "ping",
        ],
    )
    .await;
    assert!(ok, "stderr: {stderr}");

    assert!(board.task(sender).await.last_peer_message_sent_at.is_some());
    assert!(board
        .task(target)
        .await
        .last_peer_message_received_at
        .is_some());
}

/// The `SendMessage` trajectory entry is the only audit record a native
/// peer message gets. It used to be written by the hook process, into the
/// data dir beside the database; it is now written by the board, into the
/// board's own data dir. The hook process has no data dir any more.
#[tokio::test]
async fn hook_peer_message_trajectory_is_written_by_the_board() {
    let board = spawn_board().await;
    let sender = seed_running_task(&board.db_path(), "Sender", SubStatus::Active).await;

    let (ok, stderr) = run_hook(
        board.port,
        &[
            "hook-peer-message",
            &sender.0.to_string(),
            "--target",
            "task-999999",
            "--body",
            "ping",
        ],
    )
    .await;
    assert!(ok, "stderr: {stderr}");

    // The append is detached from the response, so wait on the board's own
    // completion signal rather than the clock.
    board.await_bg_write(BackgroundWrite::Trajectory).await;

    let path = board
        .dir
        .path()
        .join(dispatch_tui::mcp::trajectory::TRAJECTORIES_SUBDIR)
        .join(format!("{}.jsonl", sender.0));
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("expected {} to exist: {e}", path.display()));
    assert!(body.contains("SendMessage"), "got: {body}");
}

// ---------------------------------------------------------------------------
// 2. No board: the event is dropped, loudly, and nothing is written
// ---------------------------------------------------------------------------

/// Every observer hook fails when no board answers, and the event is gone:
/// nothing reaches the database by any other route. The board here exists
/// only so there is a real row the dropped event could have touched.
#[tokio::test]
async fn every_hook_kind_fails_when_the_board_is_down_and_writes_nothing() {
    let board = spawn_board().await;
    let (id, task_id) = seed(&board, "No board").await;
    let dead = dead_port().await;

    for args in [
        vec!["hook", &task_id, "pre_tool_use"],
        vec!["hook", &task_id, "stop"],
        vec![
            "hook-subagent",
            &task_id,
            "start",
            "--agent-id",
            "a1",
            "--session-id",
            "s1",
        ],
        vec!["hook-subagent", &task_id, "clear"],
        vec![
            "hook-shell",
            &task_id,
            "start",
            "--shell-id",
            "b1",
            "--session-id",
            "s1",
        ],
        vec![
            "hook-peer-message",
            &task_id,
            "--target",
            "task-1",
            "--body",
            "x",
        ],
    ] {
        let (ok, stderr) = run_hook(dead, &args).await;
        assert!(!ok, "{args:?} must fail with no board, stderr: {stderr}");
        assert!(
            stderr.to_lowercase().contains("board"),
            "{args:?} must name the board, got stderr: {stderr}"
        );
    }

    let task = board.task(id).await;
    assert!(
        task.last_pre_tool_use_at.is_none(),
        "a dropped event must not reach the database by any other route"
    );
    assert_eq!(task.live_subagents, 0);
    assert_eq!(task.live_shells, 0);
}

// ---------------------------------------------------------------------------
// 2b. A missing task stays a silent skip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hook_unknown_task_is_a_silent_skip() {
    let board = spawn_board().await;

    for kind in ["notification", "stop", "user_prompt_submit"] {
        let (ok, stderr) = run_hook(board.port, &["hook", "99999", kind]).await;
        assert!(
            ok,
            "{kind} must exit 0 for a missing task, stderr: {stderr}"
        );
        assert!(
            !stderr.contains("not found"),
            "{kind} must skip silently, got stderr: {stderr}"
        );
    }
}

/// The events whose handler must first confirm the row exists report that it
/// is gone, and still exit 0. Both halves matter: the report is the
/// deliberate outcome, and the zero exit is the contract
/// `MissingTaskSucceeds` holds.
#[tokio::test]
async fn hook_unknown_task_is_named_by_the_events_that_read_the_row_first() {
    let board = spawn_board().await;

    for args in [
        vec!["hook", "99999", "pre_tool_use"],
        vec![
            "hook-subagent",
            "99999",
            "start",
            "--agent-id",
            "a1",
            "--session-id",
            "s1",
        ],
        vec!["hook-subagent", "99999", "clear"],
        vec![
            "hook-shell",
            "99999",
            "start",
            "--shell-id",
            "b1",
            "--session-id",
            "s1",
        ],
        vec![
            "hook-peer-message",
            "99999",
            "--target",
            "task-1",
            "--body",
            "x",
        ],
    ] {
        let (ok, stderr) = run_hook(board.port, &args).await;
        assert!(ok, "{args:?} must exit 0, stderr: {stderr}");
        assert!(
            stderr.contains("not found"),
            "{args:?} must name the missing task, got stderr: {stderr}"
        );
    }
}

/// An event carrying no identifiers is dropped before any delivery is
/// attempted, so it neither reaches the board nor fails when none is running.
#[tokio::test]
async fn hook_subagent_missing_agent_id_is_a_noop_even_with_no_board() {
    let dead = dead_port().await;
    let (ok, stderr) = run_hook(dead, &["hook-subagent", "1", "start"]).await;
    assert!(
        ok,
        "an information-free event must not attempt delivery, stderr: {stderr}"
    );
}

/// An unparseable hook kind is rejected by the hook process itself, before
/// any delivery — so it fails the same way whether or not a board is up.
#[tokio::test]
async fn hook_unknown_kind_fails_before_delivery() {
    let dead = dead_port().await;
    let (ok, _) = run_hook(dead, &["hook", "1", "bogus"]).await;
    assert!(!ok);
}

// ---------------------------------------------------------------------------
// 2c. The PR gate — the one hook that answers the tool call
// ---------------------------------------------------------------------------

/// Seed a task and take the gate's one-shot reminder, which only the first
/// call produces. Returns the task id alongside it, for the tests that go on
/// to make a second attempt.
async fn pr_gate_first_reminder(board: &Board) -> (String, String) {
    let id = seed_task(&board.db_path(), "gate me").await.0.to_string();
    let (code, stderr) = run_pr_gate(board.port, &id).await;
    assert_eq!(code, Some(2), "first call must block, got: {stderr}");
    (id, stderr)
}

#[tokio::test]
async fn pr_gate_blocks_first_then_allows_through_the_board() {
    let board = spawn_board().await;
    let (id, stderr) = pr_gate_first_reminder(&board).await;
    assert!(
        stderr.contains("query_learnings"),
        "expected the reminder, got: {stderr}"
    );

    let (code, stderr) = run_pr_gate(board.port, &id).await;
    assert_eq!(code, Some(0), "second call must allow, got: {stderr}");
}

#[tokio::test]
async fn pr_gate_missing_task_allows_through_the_board() {
    let board = spawn_board().await;
    let (code, stderr) = run_pr_gate(board.port, "999999").await;
    assert_eq!(code, Some(0), "got: {stderr}");
}

/// The gate is a reminder, not enforcement, so a board it cannot reach must
/// not stand between the agent and its PR. It fails **open**: it reports the
/// unreachable board and exits non-zero with a code that is not 2, which
/// Claude Code shows without blocking the tool call. This is the one place
/// the hook family's "drop it and fail" rule has to choose a side, and the
/// side it chooses is letting the work through.
#[tokio::test]
async fn pr_gate_with_no_board_fails_open() {
    let dead = dead_port().await;
    let (code, stderr) = run_pr_gate(dead, "1").await;
    assert_ne!(code, Some(2), "a missing board must never block a PR");
    assert_ne!(code, Some(0), "it must still report, got: {stderr}");
    assert!(
        stderr.contains("board"),
        "the failure must name the board, got: {stderr}"
    );
}

/// See `PrLearningsGate` in `docs/specs/pr-workflow.allium` for why the scope
/// is the whole submission. Asserting both halves is what makes this a
/// regression guard: banning the old "PR conventions" phrasing would pass on
/// "conventions for this pull request", which is the same defect reworded.
#[tokio::test]
async fn pr_gate_reminder_covers_the_diff_not_just_the_pr_body() {
    let board = spawn_board().await;
    let (_, stderr) = pr_gate_first_reminder(&board).await;
    assert!(
        stderr.contains("diff"),
        "reminder must name the diff, got: {stderr}"
    );
    assert!(
        stderr.contains("title and body"),
        "reminder must still name the PR body, got: {stderr}"
    );
}

/// See `PrLearningsGate` in `docs/specs/pr-workflow.allium` for why no tag is
/// suggested.
#[tokio::test]
async fn pr_gate_reminder_suggests_no_tag_filter() {
    let board = spawn_board().await;
    let (_, stderr) = pr_gate_first_reminder(&board).await;
    assert!(
        !stderr.contains("tag_filter"),
        "reminder must not suggest a tag_filter, got: {stderr}"
    );
    // `query_learnings` contains "query", so drop the tool name before looking
    // for the argument the reminder is supposed to steer the agent towards.
    assert!(
        stderr.replace("query_learnings", "").contains("query"),
        "reminder must point the agent at the `query` argument, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// 3. Structural: no hook path opens the database
// ---------------------------------------------------------------------------

/// The cheap half of `HookNeverOpensTheDatabase`
/// (`docs/specs/agent-health.allium`): the hook implementations live in one
/// subtree, and nothing in it may name the database. Catches the literal
/// reintroduction at the point of writing it, which is worth having — but it
/// is a lint on identifiers, not a proof, so the behavioural test below is
/// what actually holds the guarantee.
#[test]
fn no_hook_source_names_the_database() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hooks");
    let mut files = Vec::new();
    collect_rs_files(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "expected hook sources under src/hooks");

    const FORBIDDEN: &[&str] = &["db::", "Database", "rusqlite", "TaskService", "TaskStore"];
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for needle in FORBIDDEN {
            assert!(
                !source.contains(needle),
                "{} names `{needle}` — a hook must reach the board, never the database",
                file.display()
            );
        }
    }
}

/// Recursive, unlike a bare `read_dir`: a hook added in a subdirectory is
/// still a hook.
fn collect_rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("expected {} to exist: {e}", dir.display()));
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// Every `dispatch <subcommand>` the installed hook scripts actually invoke.
///
/// Read from the scripts rather than listed here, because the point is to
/// notice a hook entry point nobody remembered to check. Only command
/// positions count — `dispatch` at the start of a line or after `exec` —
/// since the scripts also say the word in prose, and comment lines are
/// skipped for the same reason.
fn hook_entry_point_subcommands() -> Vec<String> {
    let hooks_json = repo_file("plugin/hooks/hooks.json");
    let scripts: std::collections::BTreeSet<String> = hooks_json
        .match_indices("hooks/scripts/")
        .map(|(i, m)| {
            let rest = &hooks_json[i + m.len()..];
            let end = rest.find('"').expect("a quoted script path");
            rest[..end].to_string()
        })
        .collect();
    assert!(
        scripts.len() >= 2,
        "expected hooks.json to register the hook scripts, found {scripts:?}"
    );

    let mut found = std::collections::BTreeSet::new();
    for script in &scripts {
        let body = repo_file(&format!("plugin/hooks/scripts/{script}"));
        for line in body.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let after = line
                .strip_prefix("dispatch ")
                .or_else(|| line.strip_prefix("exec dispatch "));
            if let Some(rest) = after {
                let word = rest.split_whitespace().next().unwrap_or_default();
                if !word.is_empty() {
                    found.insert(word.to_string());
                }
            }
        }
    }
    assert!(
        !found.is_empty(),
        "expected the hook scripts to invoke dispatch subcommands"
    );
    found.into_iter().collect()
}

/// A representative invocation of each hook entry point, for the behavioural
/// scan below. Asserted to cover exactly the set the scripts invoke, so a new
/// entry point fails here — deliberately, because the author is the one who
/// knows what argv it takes.
fn hook_entry_point_invocations() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("hook", vec!["hook", "1", "pre_tool_use"]),
        (
            "hook-subagent",
            vec![
                "hook-subagent",
                "1",
                "start",
                "--agent-id",
                "a1",
                "--session-id",
                "s1",
            ],
        ),
        (
            "hook-shell",
            vec![
                "hook-shell",
                "1",
                "start",
                "--shell-id",
                "b1",
                "--session-id",
                "s1",
            ],
        ),
        (
            "hook-peer-message",
            vec![
                "hook-peer-message",
                "1",
                "--target",
                "task-1",
                "--body",
                "x",
            ],
        ),
        ("pr-gate", vec!["pr-gate", "1"]),
    ]
}

#[test]
fn the_invocation_table_covers_every_installed_entry_point() {
    let covered: std::collections::BTreeSet<&str> = hook_entry_point_invocations()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let installed: std::collections::BTreeSet<String> =
        hook_entry_point_subcommands().into_iter().collect();
    let installed: std::collections::BTreeSet<&str> =
        installed.iter().map(String::as_str).collect();
    assert_eq!(
        covered, installed,
        "the hook scripts invoke a subcommand the invocation table below does not exercise \
         (or vice versa) — add it, so the no-database scan covers it too"
    );
}

/// The behavioural half, and the one that actually holds
/// `HookNeverOpensTheDatabase`: run every installed hook entry point as a real
/// process, pointed at a database path that does not exist and a port nothing
/// is listening on. None may bring a database into being.
///
/// Unlike the source scan this does not care *how* a hook would have reached a
/// database — a renamed import, a helper in another module, a new file in a
/// subdirectory. It only cares that none appears, which is the guarantee as
/// written.
#[tokio::test]
async fn no_installed_hook_entry_point_creates_a_database() {
    let dead = dead_port().await;
    for (name, argv) in hook_entry_point_invocations() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("dispatch.db");

        Command::new(env!("CARGO_BIN_EXE_dispatch"))
            .args(["--db", db_path.to_str().unwrap()])
            .args(&argv)
            .args(["--port", &dead.to_string()])
            .output()
            .await
            .unwrap();

        assert!(
            !db_path.exists(),
            "`dispatch {name}` brought a database into existence — a hook must reach the \
             board, never the database"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Concurrent hooks cannot desync a denormalised counter
// ---------------------------------------------------------------------------

/// `live_subagents` is denormalised onto the task row. Two hook processes
/// firing at the same instant used to be two processes racing for one
/// database file; they are now two requests to one board. The count must end
/// at exactly the number of starts.
#[tokio::test]
async fn concurrent_subagent_hooks_do_not_desync_the_counter() {
    let board = spawn_board().await;
    let (id, task_id) = seed(&board, "Concurrent").await;

    const N: usize = 8;
    // Starts then stops, through the same fan-out. The expected count is the
    // only thing that differs, which is what the test is about.
    for (action, want) in [("start", N as i64), ("stop", 0)] {
        let mut handles = Vec::new();
        for i in 0..N {
            let task_id = task_id.clone();
            let port = board.port;
            handles.push(tokio::spawn(async move {
                run_hook(
                    port,
                    &[
                        "hook-subagent",
                        &task_id,
                        action,
                        "--agent-id",
                        &format!("a{i}"),
                        "--session-id",
                        "s1",
                    ],
                )
                .await
            }));
        }
        for h in handles {
            let (ok, stderr) = h.await.unwrap();
            assert!(ok, "stderr: {stderr}");
        }

        assert_eq!(
            board.task(id).await.live_subagents,
            want,
            "every concurrent {action} must be counted exactly once"
        );
    }
}
