#![allow(clippy::unwrap_used, dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::Value;
use tokio::sync::mpsc;
use tower::ServiceExt;

use dispatch_tui::db::{self, Database};
use dispatch_tui::mcp::BackgroundWrite;
use dispatch_tui::process::{MockProcessRunner, ProcessRunner};
use dispatch_tui::service::embeddings::EmbeddingService;

pub async fn test_router() -> (axum::Router, Arc<dyn db::TaskStore>) {
    test_router_with_data_dir(std::env::temp_dir().as_path()).await
}

pub async fn test_router_with_data_dir(data_dir: &Path) -> (axum::Router, Arc<dyn db::TaskStore>) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let router = dispatch_tui::mcp::router(
        dispatch_tui::mcp::McpDeps {
            db: db.clone(),
            runner,
            embedding_service: EmbeddingService::new_noop(),
            data_dir: data_dir.to_path_buf(),
        },
        None,
    );
    (router, db)
}

/// Like [`test_router_with_data_dir`], but installs a completion signal that
/// fires after each fire-and-forget background write. Returns the receiver so
/// tests can await a specific write (e.g. trajectory) deterministically instead
/// of sleeping.
pub async fn test_router_with_bg_done(
    data_dir: &Path,
) -> (
    axum::Router,
    Arc<dyn db::TaskStore>,
    mpsc::UnboundedReceiver<BackgroundWrite>,
) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (tx, rx) = mpsc::unbounded_channel();
    let router = dispatch_tui::mcp::router_with_bg_done(
        dispatch_tui::mcp::McpDeps {
            db: db.clone(),
            runner,
            embedding_service: EmbeddingService::new_noop(),
            data_dir: data_dir.to_path_buf(),
        },
        None,
        Some(tx),
    );
    (router, db, rx)
}

/// Await a specific fire-and-forget background write, draining any other write
/// signals (e.g. usage) that arrive first. Fails if none arrives within 5s.
pub async fn await_bg_write(
    rx: &mut mpsc::UnboundedReceiver<BackgroundWrite>,
    want: BackgroundWrite,
) {
    loop {
        let got = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for background write")
            .expect("bg_done channel closed");
        if got == want {
            return;
        }
    }
}

pub async fn post_mcp(router: axum::Router, headers: &[(&str, &str)], body: Value) -> Value {
    let mut builder = Request::post("/mcp").header("content-type", "application/json");
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let resp = router
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), 65_536).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// ---------------------------------------------------------------------------
// Task fixtures
// ---------------------------------------------------------------------------

/// Seed a backlog task directly via the DB API. Task creation is not exposed
/// over the CLI, so every suite that needs a row to act on starts here.
pub async fn seed_task(db_path: &Path, title: &str) -> dispatch_tui::models::TaskId {
    use dispatch_tui::db::{CreateTaskRequest, TaskCrud};

    let db = Database::open(db_path).await.unwrap();
    db.create_task(CreateTaskRequest {
        title,
        description: "",
        repo_path: "/tmp/test-repo",
        plan: None,
        status: dispatch_tui::models::TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        wrap_up_mode: None,
        auto_run_plan: false,
        phoenix: false,
    })
    .await
    .unwrap()
}

/// [`seed_task`], then moved to Running with the given sub-status — the state
/// most hook rules require before they do anything.
pub async fn seed_running_task(
    db_path: &Path,
    title: &str,
    sub: dispatch_tui::models::SubStatus,
) -> dispatch_tui::models::TaskId {
    use dispatch_tui::db::{TaskCrud, TaskPatch};

    let id = seed_task(db_path, title).await;
    let conn = Database::open(db_path).await.unwrap();
    conn.patch_task(
        id,
        &TaskPatch::new()
            .status(dispatch_tui::models::TaskStatus::Running)
            .sub_status(sub),
    )
    .await
    .unwrap();
    id
}

// ---------------------------------------------------------------------------
// A real board on a real port
// ---------------------------------------------------------------------------

/// A board serving `/mcp` and `/hook` against an on-disk database, plus the
/// temp directory holding both the database file and the data dir the board
/// writes trajectories into.
///
/// On-disk rather than in-memory because the suites that want this drive a
/// real `dispatch` child process, which has to open the same file.
pub struct Board {
    pub port: u16,
    pub dir: tempfile::TempDir,
    bg_writes: tokio::sync::Mutex<mpsc::UnboundedReceiver<BackgroundWrite>>,
}

impl Board {
    pub fn db_path(&self) -> std::path::PathBuf {
        self.dir.path().join("dispatch.db")
    }

    pub async fn task(&self, id: dispatch_tui::models::TaskId) -> dispatch_tui::models::Task {
        use dispatch_tui::db::TaskRead;

        let conn = Database::open(&self.db_path()).await.unwrap();
        conn.get_task(id).await.unwrap().unwrap()
    }

    /// Await one detached background write of `want`. See [`await_bg_write`].
    pub async fn await_bg_write(&self, want: BackgroundWrite) {
        await_bg_write(&mut *self.bg_writes.lock().await, want).await
    }
}

/// Stand a board up on an OS-allocated port. The listener is claimed before
/// the server is spawned, so the returned port is serving by the time this
/// returns and no caller has to poll for readiness.
pub async fn spawn_board() -> Board {
    let dir = tempfile::tempdir().unwrap();
    let db: Arc<dyn db::TaskStore> = Arc::new(
        Database::open(&dir.path().join("dispatch.db"))
            .await
            .unwrap(),
    );
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (bg_tx, bg_rx) = mpsc::unbounded_channel();
    let router = dispatch_tui::mcp::router_with_bg_done(
        dispatch_tui::mcp::McpDeps {
            db,
            runner,
            embedding_service: EmbeddingService::new_noop(),
            data_dir: dir.path().to_path_buf(),
        },
        None,
        Some(bg_tx),
    );
    let listener = dispatch_tui::mcp::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Board {
        port,
        dir,
        bg_writes: tokio::sync::Mutex::new(bg_rx),
    }
}

/// A port nothing is listening on: claimed from the OS, then released.
pub async fn dead_port() -> u16 {
    let listener = dispatch_tui::mcp::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Read a file by its path relative to the repository root.
pub fn repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}
