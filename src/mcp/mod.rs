pub mod handlers;
pub mod identity;
pub mod middleware;
pub mod trajectory;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use uuid::Uuid;

use axum::{routing::post, Router};
use tokio::sync::mpsc;

use crate::board_event::BoardEvent;
use crate::embeddings::EmbeddingService;
use crate::models::{EpicId, TaskId};
use crate::process::ProcessRunner;
use crate::service::{EpicServiceApi, LearningServiceApi, Services, TaskServiceApi};
use crate::store;

/// Identifies a fire-and-forget background write performed by the MCP handler.
///
/// Production code never observes these; the variants exist so tests can await
/// a specific detached write deterministically (via `bg_write_done_tx`) instead
/// of sleeping. See `docs/conventions.md` ("No `tokio::time::sleep` in tests").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundWrite {
    /// A usage event was recorded.
    Usage,
    /// A trajectory entry was appended.
    Trajectory,
    /// `exit_session`'s detached tmux teardown (`kill_window`) ran to
    /// completion — fired whether or not a window existed to kill. See
    /// `close_persisted` in `docs/specs/pr-workflow.allium`.
    KillWindow,
}

/// The wrap-up action a task is being closed out with. Shared between
/// `wrap_up` (which issues an `ExitToken` recording it) and `exit_session`
/// (which validates the closing call's action against it), so it lives here
/// rather than in a handler submodule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum WrapUpAction {
    Rebase,
    Done,
    Pr,
}

impl WrapUpAction {
    pub(crate) const ALL: &'static [WrapUpAction] =
        &[WrapUpAction::Rebase, WrapUpAction::Done, WrapUpAction::Pr];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            WrapUpAction::Rebase => "rebase",
            WrapUpAction::Done => "done",
            WrapUpAction::Pr => "pr",
        }
    }
}

/// One-time token linking a wrap_up call to its exit_session close.
/// `action` records which wrap_up action issued it, so exit_session can
/// reject a call whose action doesn't match (e.g. issued for "rebase" but
/// closed with "pr").
pub(crate) struct ExitToken {
    pub(crate) token: String,
    pub(crate) action: WrapUpAction,
}

/// Shared dependencies threaded through the MCP entry points.
/// Bundles the four fields that appear in every signature so callers
/// construct one struct instead of passing a 5–6-argument list.
pub struct McpDeps {
    pub db: Arc<dyn store::TaskStore>,
    pub runner: Arc<dyn ProcessRunner>,
    pub embedding_service: Arc<EmbeddingService>,
    pub data_dir: std::path::PathBuf,
}

pub struct McpState {
    /// Read-only DB handle. Task/epic *mutations* must go through `task_svc` /
    /// `epic_svc` — calling a mutating method here is a compile error. See the
    /// mutation-boundary section of `docs/conventions.md`.
    pub db: Arc<dyn store::TaskReadStore>,
    pub task_svc: Arc<dyn TaskServiceApi>,
    pub epic_svc: Arc<dyn EpicServiceApi>,
    pub learning_svc: Arc<dyn LearningServiceApi>,
    /// When set, MCP sends events after mutations to trigger TUI updates.
    pub notify_tx: Option<mpsc::UnboundedSender<BoardEvent>>,
    // Deliberately no `runner`: no handler shells out. Every subprocess a
    // handler used to reach for — the dispatch provisioning, the wrap-up
    // rebase, the session-close tmux teardown — now happens behind
    // `TaskServiceApi`, which owns the runner `McpDeps` supplies. Keeping the
    // field would let the next handler quietly reach past the service layer
    // again; its absence makes that a compile error.
    /// Embedding service used for RAG-based query_learnings and for computing
    /// embeddings when a learning is recorded via MCP.
    pub embedding_service: Arc<EmbeddingService>,
    /// In-memory tokens issued by wrap_up, consumed by exit_session.
    pub(crate) exit_tokens: Arc<RwLock<HashMap<TaskId, ExitToken>>>,
    /// Dispatch data directory (where the store's local files live). Trajectory files are
    /// written here under `trajectories/<task_id>.jsonl`.
    pub data_dir: std::path::PathBuf,
    /// Fields that exist only to make async tests deterministic. See [`TestHooks`].
    pub(crate) test_hooks: TestHooks,
}

/// Test-support fields grouped out of [`McpState`]'s field list. Not all of
/// these are `#[cfg(test)]` — `bg_write_done_tx` is read unconditionally by
/// production code (`handlers/dispatch.rs`) and is
/// simply always `None` outside tests, whereas `db_write` is compiled only
/// under `#[cfg(test)]`.
pub(crate) struct TestHooks {
    /// Fires with a [`BackgroundWrite`] tag after each fire-and-forget
    /// background write (usage, trajectory) lands, so tests can await it
    /// deterministically instead of sleeping.
    pub(crate) bg_write_done_tx: Option<mpsc::UnboundedSender<BackgroundWrite>>,
    /// Write-capable handle for seeding DB fixtures directly (production
    /// mutations go through `task_svc`/`epic_svc`). Reachable only via
    /// [`McpState::db_write`].
    #[cfg(test)]
    pub(crate) db_write: Arc<dyn store::TaskStore>,
}

impl McpState {
    /// A state with its own services, built from `deps`. Test-only: the
    /// board shares its own through [`with_services`](Self::with_services),
    /// and gating this keeps a production caller from building a second set.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new(deps: McpDeps, notify_tx: Option<mpsc::UnboundedSender<BoardEvent>>) -> Self {
        let services = Services::new(
            deps.db.clone(),
            deps.runner.clone(),
            deps.embedding_service.clone(),
        );
        Self::with_services(deps, services, notify_tx)
    }

    /// A state over services the caller already built — the board's, so the
    /// MCP server and the TUI share one set (`Services`).
    pub fn with_services(
        deps: McpDeps,
        services: Services,
        notify_tx: Option<mpsc::UnboundedSender<BoardEvent>>,
    ) -> Self {
        let Services {
            tasks: task_svc,
            epics: epic_svc,
            learnings: learning_svc,
        } = services;
        // Narrow the write-capable dependency handle to the read-only surface
        // consumers are allowed to touch. Mutations go through the services above.
        let db: Arc<dyn store::TaskReadStore> = deps.db.clone();
        Self {
            db,
            task_svc,
            epic_svc,
            learning_svc,
            notify_tx,
            embedding_service: deps.embedding_service,
            exit_tokens: Arc::new(RwLock::new(HashMap::new())),
            data_dir: deps.data_dir,
            test_hooks: TestHooks {
                bg_write_done_tx: None,
                #[cfg(test)]
                db_write: deps.db,
            },
        }
    }

    pub fn notify(&self) {
        if let Some(tx) = &self.notify_tx {
            let _ = tx.send(BoardEvent::Refresh);
        }
    }

    /// Test-only write handle for seeding DB fixtures directly. Not available in
    /// production builds, so handler code keeps going through the services.
    #[cfg(test)]
    pub(crate) fn db_write(&self) -> &Arc<dyn store::TaskStore> {
        &self.test_hooks.db_write
    }

    /// Notify the runtime that a single task changed. Prefer this over
    /// `notify()` whenever the affected `task_id` is known: it lets the
    /// runtime reload one row instead of all tasks.
    pub fn notify_task_changed(&self, task_id: TaskId) {
        if let Some(tx) = &self.notify_tx {
            let _ = tx.send(BoardEvent::TaskChanged(task_id));
        }
    }

    /// Notify the runtime that a single epic changed. Use this for epic
    /// updates and for feed-sync batches (one event per sync, not per task).
    pub fn notify_epic_changed(&self, epic_id: EpicId) {
        if let Some(tx) = &self.notify_tx {
            let _ = tx.send(BoardEvent::EpicChanged(epic_id));
        }
    }

    /// Notify the runtime that a `wrap_up(rebase)` fast-forwarded `repo_path`'s
    /// local base branch, so its drift measurement is now out of date
    /// (docs/specs/repo-sync.allium: rule RefreshRepoSyncStateAfterRebase).
    pub(crate) fn notify_branch_rebased(&self, repo_path: &str) {
        if let Some(tx) = &self.notify_tx {
            let _ = tx.send(BoardEvent::BranchRebased {
                repo_path: repo_path.to_string(),
            });
        }
    }

    /// Notify the runtime that an agent was just launched into a worktree under
    /// `repo_path`, so its drift measurement is now out of date
    /// (docs/specs/repo-sync.allium: rule RefreshRepoSyncStateAfterDispatch).
    /// Call this only after a dispatch that actually launched an agent — a failed
    /// provisioning moved nothing.
    pub(crate) fn notify_agent_launched(&self, repo_path: &str) {
        if let Some(tx) = &self.notify_tx {
            let _ = tx.send(BoardEvent::AgentLaunched {
                repo_path: repo_path.to_string(),
            });
        }
    }

    /// Issue a fresh exit token for a task, overwriting any existing one.
    /// Records which action issued it (validated against on exit_session).
    /// Returns the token string to embed in the response.
    pub(crate) fn issue_exit_token(&self, task_id: TaskId, action: WrapUpAction) -> String {
        let token = Uuid::new_v4().to_string();
        self.exit_tokens
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                task_id,
                ExitToken {
                    token: token.clone(),
                    action,
                },
            );
        token
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn router(deps: McpDeps, notify_tx: Option<mpsc::UnboundedSender<BoardEvent>>) -> Router {
    router_with_bg_done(deps, notify_tx, None)
}

/// Like [`router`], but installs a test-only completion signal that fires after
/// each fire-and-forget background write (usage, trajectory). Lets integration
/// tests await detached writes deterministically instead of sleeping.
#[cfg(any(test, feature = "test-support"))]
pub fn router_with_bg_done(
    deps: McpDeps,
    notify_tx: Option<mpsc::UnboundedSender<BoardEvent>>,
    bg_write_done_tx: Option<mpsc::UnboundedSender<BackgroundWrite>>,
) -> Router {
    let mut state = McpState::new(deps, notify_tx);
    state.test_hooks.bg_write_done_tx = bg_write_done_tx;
    router_over(state)
}

fn router_over(state: McpState) -> Router {
    let state = Arc::new(state);
    Router::new()
        .route("/mcp", post(handlers::handle_mcp))
        // Claude Code hooks deliver here rather than opening the database
        // themselves — see `HookDelivery` in `docs/specs/agent-health.allium`.
        .route(crate::hooks::wire::HOOK_PATH, post(handlers::handle_hook))
        // The companion panes read here instead of opening the store
        // themselves -- `PanesReadThroughTheBoard` in `docs/specs/agent-tree.allium`.
        .route(
            crate::hooks::wire::PANE_VIEW_PATH,
            post(handlers::handle_pane_view),
        )
        .layer(axum::middleware::from_fn(
            middleware::extract_caller_identity,
        ))
        .with_state(state)
}

/// Claim the port agents reach this board on.
///
/// Separate from [`serve_on`] so the claim happens on the startup path, where a
/// failure can still abort before the board takes the screen — see
/// `startup.allium`'s `AbortWhenTheAgentPortIsTaken`. Bound inside `serve` (as
/// it once was) the failure lands on a stderr the drawn board has already
/// covered, leaving a board no agent can reach and nothing saying so.
///
/// The operator-facing wording for a taken port belongs to the caller
/// (`startup::StartupAbort::AgentPortUnavailable`), which is where every other
/// startup abort is worded; this reports the port and the underlying error.
pub async fn bind(port: u16) -> anyhow::Result<tokio::net::TcpListener> {
    use anyhow::Context;
    tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .with_context(|| format!("binding agent port {port}"))
}

/// Serve the MCP API on a listener [`bind`] already claimed, through the
/// board's own `services`.
pub async fn serve_on(
    listener: tokio::net::TcpListener,
    deps: McpDeps,
    services: Services,
    notify_tx: mpsc::UnboundedSender<BoardEvent>,
) -> anyhow::Result<()> {
    let app = router_over(McpState::with_services(deps, services, Some(notify_tx)));
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod port_tests {
    use super::*;

    #[tokio::test]
    async fn bind_fails_when_the_port_is_already_held() {
        let held = bind(0).await.expect("a free port binds");
        let port = held
            .local_addr()
            .expect("bound listener has an address")
            .port();

        let err = bind(port)
            .await
            .expect_err("AbortWhenTheAgentPortIsTaken: a port another process holds must not bind");
        assert!(
            err.to_string().contains(&port.to_string()),
            "the operator must be told which port is taken, got: {err}"
        );
    }
}

#[cfg(test)]
mod services_tests {
    use super::*;

    /// The board builds its services once and hands the same ones to the MCP
    /// server, so an agent's call and a keypress go through one `TaskService`
    /// (one clock, one runner) rather than two built from the same handles.
    #[tokio::test]
    async fn with_services_uses_the_services_it_is_given() {
        let db: Arc<dyn store::TaskStore> = Arc::new(store::Store::open_in_memory().unwrap());
        let runner = crate::process::MockProcessRunner::unused();
        let embedding_service = EmbeddingService::new_test();
        let services =
            crate::service::Services::new(db.clone(), runner.clone(), embedding_service.clone());

        let state = McpState::with_services(
            McpDeps {
                db,
                runner,
                embedding_service,
                data_dir: std::env::temp_dir(),
            },
            services.clone(),
            None,
        );

        assert!(Arc::ptr_eq(&state.task_svc, &services.tasks));
        assert!(Arc::ptr_eq(&state.epic_svc, &services.epics));
        assert!(Arc::ptr_eq(&state.learning_svc, &services.learnings));
    }
}
