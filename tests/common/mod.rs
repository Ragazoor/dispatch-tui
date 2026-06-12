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
