#![allow(clippy::unwrap_used, dead_code)]

use std::path::Path;
use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::Value;
use tower::ServiceExt;

use dispatch_tui::db::{self, Database};
use dispatch_tui::process::{MockProcessRunner, ProcessRunner};
use dispatch_tui::service::embeddings::EmbeddingService;

pub async fn test_router() -> (axum::Router, Arc<dyn db::TaskStore>) {
    let tmp = tempfile::TempDir::new().unwrap();
    test_router_with_data_dir(tmp.into_path().as_path()).await
}

pub async fn test_router_with_data_dir(data_dir: &Path) -> (axum::Router, Arc<dyn db::TaskStore>) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let router = dispatch_tui::mcp::router(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_noop(),
        data_dir.to_path_buf(),
    );
    (router, db)
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
