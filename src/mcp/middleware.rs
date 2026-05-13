use axum::{body::Body, extract::Request, http::HeaderMap, middleware::Next, response::Response};

use crate::mcp::identity::{CallerIdentity, IdentityError, HEADER_KIND, HEADER_TASK_ID};

/// Parse the two caller-identity headers and attach the result to the
/// request extensions as `Result<CallerIdentity, IdentityError>`.
///
/// This is an **extractor**, not a gatekeeper: the request always flows
/// through to the handler. Gating happens at the handler boundary, after
/// the JSON-RPC body has been parsed and the request id is known —
/// because a JSON-RPC error response with `id: null` is rejected by
/// strict MCP clients (Claude Code) and would abort the handshake.
///
/// Methods that don't consume identity (`initialize`, `ping`,
/// `tools/list`, notifications) succeed regardless of header presence.
/// Methods that do (`tools/call`) return JSON-RPC -32600 with the
/// request's actual id when identity is missing or invalid.
pub async fn extract_caller_identity(
    headers: HeaderMap,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let task_id = headers.get(HEADER_TASK_ID).and_then(|v| v.to_str().ok());
    let kind = headers.get(HEADER_KIND).and_then(|v| v.to_str().ok());

    let result: Result<CallerIdentity, IdentityError> = CallerIdentity::from_headers(task_id, kind);
    req.extensions_mut().insert(result);
    next.run(req).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::{
        body::to_bytes, http::Request, http::StatusCode, middleware, routing::post, Extension,
        Router,
    };
    use tower::util::ServiceExt;

    use crate::models::TaskId;

    async fn echo(Extension(result): Extension<Result<CallerIdentity, IdentityError>>) -> String {
        format!("{result:?}")
    }

    fn app() -> Router {
        Router::new()
            .route("/mcp", post(echo))
            .layer(middleware::from_fn(extract_caller_identity))
    }

    async fn body_of(resp: Response) -> String {
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
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
        let body = body_of(resp).await;
        assert!(
            body.contains(&format!("Ok(Task({:?}))", TaskId(7))),
            "got {body}"
        );
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
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_of(resp).await;
        assert!(body.contains("Ok(Session)"), "got {body}");
    }

    #[tokio::test]
    async fn missing_header_passes_through_with_err() {
        let resp = app()
            .oneshot(Request::post("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_of(resp).await;
        assert!(body.contains("Err(Missing)"), "got {body}");
    }

    #[tokio::test]
    async fn conflict_passes_through_with_err() {
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
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_of(resp).await;
        assert!(body.contains("Err(Conflict)"), "got {body}");
    }
}
