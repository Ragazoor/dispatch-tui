pub mod handlers;

use std::sync::Arc;

use axum::{Router, routing::post};

use crate::db::Database;

pub struct McpState {
    pub db: Arc<Database>,
}

pub fn router(db: Arc<Database>) -> Router {
    let state = Arc::new(McpState { db });
    Router::new()
        .route("/mcp", post(handlers::handle_mcp))
        .with_state(state)
}

pub async fn serve(db: Arc<Database>, port: u16) -> anyhow::Result<()> {
    let app = router(db);
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
