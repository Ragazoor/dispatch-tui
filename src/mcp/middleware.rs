use axum::{
    body::Body,
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::mcp::identity::{CallerIdentity, IdentityError, HEADER_KIND, HEADER_TASK_ID};

/// Parse the two caller-identity headers and attach the result to the
/// request extensions. On error, short-circuit with a JSON-RPC -32600
/// reply (id=null because the request body hasn't been parsed yet).
pub async fn extract_caller_identity(
    headers: HeaderMap,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let task_id = headers.get(HEADER_TASK_ID).and_then(|v| v.to_str().ok());
    let kind = headers.get(HEADER_KIND).and_then(|v| v.to_str().ok());

    match CallerIdentity::from_headers(task_id, kind) {
        Ok(identity) => {
            req.extensions_mut().insert(identity);
            next.run(req).await
        }
        Err(e) => {
            let msg = match e {
                IdentityError::Missing => "missing X-Caller-* identity header".to_string(),
                IdentityError::Conflict => {
                    "both X-Caller-Task-Id and X-Caller-Kind set".to_string()
                }
                IdentityError::UnknownKind(k) => format!("unknown X-Caller-Kind: {k}"),
                IdentityError::InvalidTaskId(s) => format!("invalid X-Caller-Task-Id: {s}"),
            };
            let body = json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32600, "message": msg },
            });
            (StatusCode::OK, Json(body)).into_response()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::Request, middleware, routing::post, Extension, Router};
    use serde_json::Value;
    use tower::util::ServiceExt;

    async fn echo(Extension(id): Extension<CallerIdentity>) -> String {
        format!("{id:?}")
    }

    fn app() -> Router {
        Router::new()
            .route("/mcp", post(echo))
            .layer(middleware::from_fn(extract_caller_identity))
    }

    #[tokio::test]
    async fn task_header_populates_extension() {
        let resp = app()
            .oneshot(
                Request::post("/mcp")
                    .header(HEADER_TASK_ID, "7")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(body.contains("Task(TaskId(7))"), "got {body}");
    }

    #[tokio::test]
    async fn session_kind_populates_extension() {
        let resp = app()
            .oneshot(
                Request::post("/mcp")
                    .header(HEADER_KIND, "session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(body.contains("Session"), "got {body}");
    }

    #[tokio::test]
    async fn missing_header_returns_jsonrpc_error() {
        let resp = app()
            .oneshot(Request::post("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32600);
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing"));
    }

    #[tokio::test]
    async fn conflict_returns_jsonrpc_error() {
        let resp = app()
            .oneshot(
                Request::post("/mcp")
                    .header(HEADER_TASK_ID, "1")
                    .header(HEADER_KIND, "session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], -32600);
    }
}
