use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

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

/// Split a `Host` value (`host[:port]`) or an `Origin` value
/// (`scheme://host[:port]`) into its lowercased host and optional port.
///
/// Strict: an authority carrying userinfo (`localhost:1@evil.com`), a path
/// that is not empty, or a port that is not plain digits yields `None`.
/// `Origin: null` has no authority and so yields `None` too.
fn parse_authority(value: &str) -> Option<(String, Option<u16>)> {
    let authority = match value.split_once("://") {
        Some((scheme, rest))
            if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") =>
        {
            rest
        }
        Some(_) => return None,
        None => value,
    };
    if authority.is_empty() || authority.contains(['/', '?', '#', '@', '\\']) {
        return None;
    }
    let (host, port) = match authority.strip_prefix('[') {
        Some(rest) => {
            let (host, after) = rest.split_once(']')?;
            (host, after.strip_prefix(':'))
        }
        None => match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };
    let port = match port {
        None => None,
        Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
            Some(p.parse::<u16>().ok()?)
        }
        Some(_) => return None,
    };
    Some((host.to_ascii_lowercase(), port))
}

/// Whether `value` names this machine's loopback interface on the bound
/// port. `bound` of `None` accepts any port (test routers have no listener).
/// `require_port` is set for `Origin`, which a browser always sends with the
/// port, so a missing one means the default port, not ours. `Host` may omit
/// it: the hook client sends `127.0.0.1` bare.
fn is_loopback(value: &str, bound: Option<u16>, require_port: bool) -> bool {
    let Some((host, port)) = parse_authority(value) else {
        return false;
    };
    if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
        return false;
    }
    match (bound, port) {
        (None, _) => true,
        (Some(b), Some(p)) => b == p,
        (Some(_), None) => !require_port,
    }
}

/// Refuse a request whose `Host` or `Origin` is not loopback on the bound
/// port — the defence against DNS rebinding and cross-origin pages
/// (`AgentPortRejectsNonLoopbackOrigins` in `docs/specs/mcp-task-tools.allium`).
/// Runs before identity extraction, so a rejected request never reaches a
/// handler. A missing header passes: a browser always sends `Host`, and local
/// clients may omit `Origin`.
pub async fn require_loopback_origin(
    State(bound): State<Option<u16>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let headers = req.headers();
    let foreign = |name, require_port| {
        headers.get(name).is_some_and(|v| {
            !v.to_str()
                .is_ok_and(|v| is_loopback(v, bound, require_port))
        })
    };
    if foreign(header::HOST, false) || foreign(header::ORIGIN, true) {
        return (StatusCode::FORBIDDEN, "non-loopback Host or Origin").into_response();
    }
    next.run(req).await
}

#[cfg(test)]
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

    fn guarded() -> Router {
        Router::new()
            .route("/mcp", post(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                Some(3142u16),
                require_loopback_origin,
            ))
    }

    async fn status_with(header_name: &str, value: &str) -> StatusCode {
        guarded()
            .oneshot(
                Request::post("/mcp")
                    .header(header_name, value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn foreign_host_is_rejected() {
        assert_eq!(
            status_with("host", "evil.example").await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status_with("host", "evil.example:3142").await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status_with("host", "127.0.0.1.evil.example").await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn foreign_origin_is_rejected() {
        assert_eq!(
            status_with("origin", "http://evil.example").await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn loopback_host_and_origin_pass() {
        for (name, value) in [
            ("host", "127.0.0.1"),
            ("host", "127.0.0.1:3142"),
            ("host", "localhost:3142"),
            ("host", "[::1]:3142"),
            ("origin", "http://localhost:3142"),
            ("origin", "http://127.0.0.1:3142"),
        ] {
            assert_eq!(
                status_with(name, value).await,
                StatusCode::OK,
                "{name}: {value}"
            );
        }
    }

    #[tokio::test]
    async fn origin_edge_cases_are_rejected() {
        for (name, value) in [
            ("origin", "null"),
            ("origin", "http://localhost.:3142"),
            ("origin", "http://localhost:9999"),
            ("origin", "http://localhost"),
            ("origin", "http://localhost:3142@evil.example"),
            ("origin", "http://localhost:x@evil.com"),
            ("origin", "http://localhost:3142.evil.example"),
            ("origin", "ftp://localhost:3142"),
            ("host", "localhost.:3142"),
            ("host", "localhost:9999"),
            ("host", "localhost:3142@evil.example"),
        ] {
            assert_eq!(
                status_with(name, value).await,
                StatusCode::FORBIDDEN,
                "{name}: {value}"
            );
        }
    }

    #[tokio::test]
    async fn upper_case_loopback_passes() {
        for (name, value) in [
            ("host", "LOCALHOST:3142"),
            ("origin", "HTTP://LocalHost:3142"),
        ] {
            assert_eq!(
                status_with(name, value).await,
                StatusCode::OK,
                "{name}: {value}"
            );
        }
    }

    #[tokio::test]
    async fn missing_host_and_origin_pass() {
        let resp = guarded()
            .oneshot(Request::post("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
