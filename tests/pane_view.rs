#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Task #4982: the companion panes read through the running board.
//!
//! `agent-tree.allium`'s `BoardPaneView` contract and the
//! `PanesReadThroughTheBoard` guarantee, with `cli.allium`'s
//! `PaneRenderersAskTheBoard`. Like `tests/hooks.rs`, these stand a real board
//! up on a free port and talk to it the way a pane would
//! (`dispatch_tui::hooks::fetch_pane_view`), so the route, the wire types and
//! the client are exercised together.

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use common::{dead_port, repo_file, seed_task, spawn_board};
use dispatch_tui::hooks::fetch_pane_view;
use dispatch_tui::hooks::wire::{PaneTask, PaneViewRequest, PANE_VIEW_PATH};
use dispatch_tui::models::{test_tmux_window, TaskId, TaskStatus};
use dispatch_tui::store::{Store, TaskCrud, TaskPatch, TaskRead};

/// Seed a task with `status`, and a tmux window when `window` is set.
async fn seed_with(db: &Store, title: &str, status: TaskStatus, window: bool) -> TaskId {
    let id = seed_task(db, title).await;
    let mut patch = TaskPatch::new().status(status);
    let tmux = test_tmux_window(&format!("task-{}", id.0));
    if window {
        patch = patch.tmux_window(Some(&tmux));
    }
    db.patch_task(id, &patch).await.unwrap();
    id
}

// ---------------------------------------------------------------------------
// 1. BoardPaneView: what the board answers
// ---------------------------------------------------------------------------

/// The two things only the task knows, which the pane needs before it can run
/// git: its worktree and its base branch.
#[tokio::test]
async fn the_board_answers_with_the_tasks_worktree_and_base_branch() {
    let board = spawn_board().await;
    let id = seed_task(&board.db, "With a worktree").await;
    let db = board.db.clone();
    db.patch_task(id, &TaskPatch::new().worktree(Some("/wt/4982-x")))
        .await
        .unwrap();

    let view = fetch_pane_view(board.port, id)
        .await
        .expect("a running board must answer a pane's read");

    assert_eq!(
        view.task,
        Some(PaneTask {
            worktree: Some("/wt/4982-x".into()),
            base_branch: "main".into(),
        })
    );
}

/// AnsweredFromTheBoardsRows: a task whose row the board does not hold yet is
/// an ANSWER with a null task -- the board does not wait for it, and it is not
/// a failed read. The pane does the waiting.
#[tokio::test]
async fn a_task_the_board_does_not_hold_is_answered_with_a_null_task() {
    let board = spawn_board().await;

    let view = fetch_pane_view(board.port, TaskId(999_999))
        .await
        .expect("a missing row is an answer, not a failure");

    assert_eq!(view.task, None);
}

/// "Not arrived yet" and "arrived without a worktree" stay two answers.
#[tokio::test]
async fn a_task_without_a_worktree_is_answered_as_present_without_one() {
    let board = spawn_board().await;
    let id = seed_task(&board.db, "No worktree").await;

    let view = fetch_pane_view(board.port, id).await.expect("answered");

    assert_eq!(
        view.task,
        Some(PaneTask {
            worktree: None,
            base_branch: "main".into(),
        })
    );
}

/// LiveIsTheBoardsOwnDefinition: exactly the tasks `is_live_agent` holds for
/// -- Running or Review with a tmux window -- and no others, by id, including
/// the asking pane's own task.
#[tokio::test]
async fn live_agents_are_exactly_the_boards_live_agents_by_id() {
    let board = spawn_board().await;
    let data_dir = &board.db;
    let review = seed_with(data_dir, "review, window", TaskStatus::Review, true).await;
    let running = seed_with(data_dir, "running, window", TaskStatus::Running, true).await;
    seed_with(data_dir, "running, no window", TaskStatus::Running, false).await;
    seed_with(data_dir, "backlog, window", TaskStatus::Backlog, true).await;
    seed_with(data_dir, "done, window", TaskStatus::Done, true).await;

    let view = fetch_pane_view(board.port, running)
        .await
        .expect("answered");

    let ids: Vec<TaskId> = view.live_agents.iter().map(|a| a.id).collect();
    assert_eq!(ids, vec![review, running]);
    let first = &view.live_agents[0];
    assert_eq!(first.title, "review, window");
    assert_eq!(first.tmux_window, format!("task-{}", review.0));
}

/// AskingChangesNothing: a pane asks once a second for as long as it is
/// shown, and the board's rows are exactly as they would have been.
#[tokio::test]
async fn asking_changes_no_row() {
    let board = spawn_board().await;
    let id = seed_with(&board.db, "asked about", TaskStatus::Running, true).await;
    let before = board.db.list_all().await.unwrap();

    for _ in 0..3 {
        fetch_pane_view(board.port, id).await.expect("answered");
    }

    let after = board.db.list_all().await.unwrap();
    assert_eq!(before, after, "answering a pane must write nothing");
}

/// AskingChangesNothing, the display half: answering pushes no refresh to the
/// board's own screen, which a hook event does (HookEventsPushALiveRefresh).
/// Driven through the router in-process so the notify channel is observable.
#[tokio::test]
async fn asking_pushes_no_refresh_to_the_board() {
    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let db: std::sync::Arc<dyn dispatch_tui::store::TaskStore> =
        std::sync::Arc::new(Store::open_in_memory().await.unwrap());
    let router = dispatch_tui::mcp::router(
        dispatch_tui::mcp::McpDeps {
            db,
            runner: std::sync::Arc::new(dispatch_tui::process::MockProcessRunner::new(vec![])),
            embedding_service: dispatch_tui::embeddings::EmbeddingService::new_noop(),
            data_dir: std::env::temp_dir(),
        },
        Some(notify_tx),
    );
    let body = serde_json::to_vec(&PaneViewRequest { task_id: TaskId(1) }).unwrap();

    let response = router
        .oneshot(
            Request::post(PANE_VIEW_PATH)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the board must serve {PANE_VIEW_PATH}"
    );
    let bytes = to_bytes(response.into_body(), 65_536).await.unwrap();
    serde_json::from_slice::<dispatch_tui::hooks::wire::PaneView>(&bytes)
        .expect("the answer must be a PaneView");
    assert!(
        notify_rx.try_recv().is_err(),
        "answering a pane must not push a refresh"
    );
}

/// BoardPaneView: a board that cannot be reached gives no PaneView at all --
/// a failed read naming the address tried, never an empty view.
#[tokio::test]
async fn an_unreachable_board_is_a_failed_read_naming_the_address() {
    let port = dead_port().await;

    let err = fetch_pane_view(port, TaskId(1))
        .await
        .expect_err("no board, no view");

    let message = format!("{err:#}");
    assert!(
        message.contains(&port.to_string()),
        "the failure must name the board it tried, got: {message}"
    );
}

// ---------------------------------------------------------------------------
// 2. PaneRenderersAskTheBoard: the renderers find the board like a hook does
// ---------------------------------------------------------------------------

/// cli.allium: PaneRenderersAskTheBoard -- "the address the agent's session
/// was launched with, else the default port", which is `hooks::BoardAddress`.
#[test]
fn pane_renderers_take_the_board_address_like_a_hook() {
    for subcommand in ["agent-tree", "agent-diff"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_dispatch"))
            .args([subcommand, "--help"])
            .output()
            .unwrap();
        let help = String::from_utf8_lossy(&out.stdout);
        assert!(
            help.contains("--port"),
            "`dispatch {subcommand}` must take the board's port, got:\n{help}"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. PanesReadThroughTheBoard: no pane renderer opens the store
// ---------------------------------------------------------------------------

/// The cheap half, like `no_hook_source_names_the_database` in
/// `tests/hooks.rs`: the renderers' sources name neither the store nor the
/// database. A lint on identifiers, not a proof; the behavioural test below
/// holds the guarantee.
#[test]
fn no_pane_renderer_source_opens_the_store() {
    const FILES: &[&str] = &[
        "src/cli/mod.rs",
        "src/cli/agent_tree.rs",
        "src/cli/agent_diff.rs",
        "src/cli/agent_tree_agents.rs",
        "src/cli/agent_tree_commits.rs",
    ];
    const FORBIDDEN: &[&str] = &["open_cli_store", "crate::store::", "TaskRead", "Store"];
    for file in FILES {
        let source = repo_file(file);
        for needle in FORBIDDEN {
            assert!(
                !source.contains(needle),
                "{file} names `{needle}` -- a pane renderer reads through the board, \
                 never the store"
            );
        }
    }
}

/// The behavioural half: run each renderer as a real process pointed at a
/// database path that does not exist, a store nothing listens on and a board
/// nothing listens on. None may bring a database into being -- opening the
/// store is what creates it.
///
/// Run under `setsid` so the renderer has no controlling terminal: one that
/// got as far as taking the terminal fails there at once instead of drawing
/// on the terminal running the tests.
#[tokio::test]
async fn no_pane_renderer_creates_a_database() {
    if std::process::Command::new("setsid")
        .arg("--version")
        .output()
        .is_err()
    {
        assert!(
            std::env::var_os("CI").is_none(),
            "setsid must be on PATH in CI"
        );
        return;
    }
    let dead_board = dead_port().await;
    let dead_store = format!("http://127.0.0.1:{}", dead_port().await);
    for subcommand in ["agent-tree", "agent-diff"] {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");

        let run = tokio::process::Command::new("setsid")
            .arg(env!("CARGO_BIN_EXE_dispatch"))
            .args(["--data-dir", data_dir.to_str().unwrap()])
            .args([subcommand, "1"])
            .env("DISPATCH_SPACETIME_SERVER", &dead_store)
            .env("DISPATCH_PORT", dead_board.to_string())
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        let out = tokio::time::timeout(std::time::Duration::from_secs(60), run)
            .await
            .unwrap_or_else(|_| panic!("`dispatch {subcommand}` did not exit"))
            .unwrap();

        let stderr = String::from_utf8_lossy(&out.stderr);
        let created: Vec<_> = std::fs::read_dir(&data_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name())
                    .filter(|name| name != "app.log")
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            created.is_empty(),
            "`dispatch {subcommand}` brought a database into existence -- a pane \
             renderer must ask the board, never open the store. stderr: {stderr}"
        );
        assert!(
            !stderr.contains("Could not connect to the shared store"),
            "`dispatch {subcommand}` tried to connect to the store: {stderr}"
        );
    }
}
