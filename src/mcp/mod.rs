pub mod handlers;
pub mod identity;
pub mod middleware;
pub mod trajectory;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use uuid::Uuid;

use axum::{routing::post, Router};
use tokio::sync::mpsc;

use crate::db;
use crate::models::{EpicId, TaskId};
use crate::process::ProcessRunner;
use crate::service::embeddings::EmbeddingService;
use crate::service::{EpicService, EpicServiceApi, TaskService, TaskServiceApi};

/// Events sent from the MCP server to the TUI runtime.
#[derive(Debug)]
pub enum McpEvent {
    /// Catch-all "I don't know what changed" — full reload of tasks, epics, and usage.
    /// Prefer the targeted variants below when the changed entity is known.
    Refresh,
    /// A single task changed — reload just that row.
    TaskChanged(TaskId),
    /// A single epic changed — reload just that row (and the epic's task list,
    /// since feed-sync changes appear here as a batch update for the epic).
    EpicChanged(EpicId),
    /// A message was sent to an agent — flash the target task's card.
    MessageSent { to_task_id: TaskId },
}

/// One-time token linking a wrap_up call to its exit_session close.
/// `reflected` tracks whether the reflection prompt has been shown (first call).
pub(crate) struct ExitToken {
    pub(crate) token: String,
    pub(crate) reflected: bool,
}

pub struct McpState {
    pub db: Arc<dyn db::TaskStore>,
    pub task_svc: Arc<dyn TaskServiceApi>,
    pub epic_svc: Arc<dyn EpicServiceApi>,
    /// When set, MCP sends events after mutations to trigger TUI updates.
    pub notify_tx: Option<mpsc::UnboundedSender<McpEvent>>,
    /// Process runner shared with TuiRuntime for executing git/tmux operations.
    pub runner: Arc<dyn ProcessRunner>,
    /// Embedding service used for RAG-based query_learnings and for computing
    /// embeddings when a learning is recorded via MCP.
    pub embedding_service: Arc<EmbeddingService>,
    /// In-memory tokens issued by wrap_up, consumed by exit_session.
    pub(crate) exit_tokens: Arc<RwLock<HashMap<TaskId, ExitToken>>>,
    /// Dispatch data directory (parent of the SQLite DB). Trajectory files are
    /// written here under `trajectories/<task_id>.jsonl`.
    pub data_dir: std::path::PathBuf,
}

impl McpState {
    pub fn new(
        db: Arc<dyn db::TaskStore>,
        notify_tx: Option<mpsc::UnboundedSender<McpEvent>>,
        runner: Arc<dyn ProcessRunner>,
        embedding_service: Arc<EmbeddingService>,
        data_dir: std::path::PathBuf,
    ) -> Self {
        let task_svc: Arc<dyn TaskServiceApi> = Arc::new(TaskService::new(db.clone()));
        let epic_svc: Arc<dyn EpicServiceApi> = Arc::new(EpicService::new(db.clone()));
        Self {
            db,
            task_svc,
            epic_svc,
            notify_tx,
            runner,
            embedding_service,
            exit_tokens: Arc::new(RwLock::new(HashMap::new())),
            data_dir,
        }
    }

    pub fn notify(&self) {
        if let Some(tx) = &self.notify_tx {
            let _ = tx.send(McpEvent::Refresh);
        }
    }

    pub fn notify_message_sent(&self, to_task_id: TaskId) {
        if let Some(tx) = &self.notify_tx {
            let _ = tx.send(McpEvent::MessageSent { to_task_id });
        }
    }

    /// Notify the runtime that a single task changed. Prefer this over
    /// `notify()` whenever the affected `task_id` is known: it lets the
    /// runtime reload one row instead of all tasks.
    pub fn notify_task_changed(&self, task_id: TaskId) {
        if let Some(tx) = &self.notify_tx {
            let _ = tx.send(McpEvent::TaskChanged(task_id));
        }
    }

    /// Notify the runtime that a single epic changed. Use this for epic
    /// updates and for feed-sync batches (one event per sync, not per task).
    pub fn notify_epic_changed(&self, epic_id: EpicId) {
        if let Some(tx) = &self.notify_tx {
            let _ = tx.send(McpEvent::EpicChanged(epic_id));
        }
    }

    /// Issue a fresh exit token for a task, overwriting any existing one.
    /// Returns the token string to embed in the response.
    pub fn issue_exit_token(&self, task_id: TaskId) -> String {
        let token = Uuid::new_v4().to_string();
        self.exit_tokens
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                task_id,
                ExitToken {
                    token: token.clone(),
                    reflected: false,
                },
            );
        token
    }
}

pub fn router(
    db: Arc<dyn db::TaskStore>,
    notify_tx: Option<mpsc::UnboundedSender<McpEvent>>,
    runner: Arc<dyn ProcessRunner>,
    embedding_service: Arc<EmbeddingService>,
    data_dir: std::path::PathBuf,
) -> Router {
    let state = Arc::new(McpState::new(
        db,
        notify_tx,
        runner,
        embedding_service,
        data_dir,
    ));
    Router::new()
        .route("/mcp", post(handlers::handle_mcp))
        .layer(axum::middleware::from_fn(
            middleware::extract_caller_identity,
        ))
        .with_state(state)
}

pub async fn serve(
    db: Arc<dyn db::TaskStore>,
    port: u16,
    notify_tx: mpsc::UnboundedSender<McpEvent>,
    runner: Arc<dyn ProcessRunner>,
    embedding_service: Arc<EmbeddingService>,
    data_dir: std::path::PathBuf,
) -> anyhow::Result<()> {
    let app = router(db, Some(notify_tx), runner, embedding_service, data_dir);
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
