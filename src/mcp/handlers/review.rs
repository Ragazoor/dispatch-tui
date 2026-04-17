use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::PrKind;
use crate::mcp::McpState;
use crate::models::{AlertKind, AlertSeverity, ReviewAgentStatus};

use super::types::{deserialize_flexible_i64, parse_args, JsonRpcResponse};

// ---------------------------------------------------------------------------
// Arg structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct ListReviewPrsArgs {
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default)]
    pub(super) repo: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct GetReviewPrArgs {
    pub(super) repo: String,
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) number: i64,
}

#[derive(Deserialize)]
pub(super) struct ListSecurityAlertsArgs {
    #[serde(default)]
    pub(super) repo: Option<String>,
    #[serde(default)]
    pub(super) severity: Option<String>,
    #[serde(default)]
    pub(super) kind: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct GetSecurityAlertArgs {
    pub(super) repo: String,
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) number: i64,
    pub(super) kind: String,
}

#[derive(Deserialize)]
pub(super) struct DispatchReviewAgentArgs {
    pub(super) repo: String,
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) number: i64,
    pub(super) local_repo: String,
}

#[derive(Deserialize)]
pub(super) struct DispatchFixAgentArgs {
    pub(super) repo: String,
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) number: i64,
    pub(super) kind: String,
    pub(super) local_repo: String,
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_review_pr(pr: &crate::models::ReviewPr) -> String {
    let draft = if pr.is_draft { " [draft]" } else { "" };
    format!(
        "#{} {}{}\n  repo: {}\n  author: {} | decision: {} | ci: {}\n  url: {}",
        pr.number,
        pr.title,
        draft,
        pr.repo,
        pr.author,
        pr.review_decision.as_str(),
        pr.ci_status.as_str(),
        pr.url,
    )
}

fn format_security_alert(alert: &crate::models::SecurityAlert) -> String {
    let pkg = alert
        .package
        .as_deref()
        .map(|p| format!(" pkg:{p}"))
        .unwrap_or_default();
    format!(
        "#{} {}{}\n  repo: {} | severity: {} | kind: {}\n  url: {}",
        alert.number,
        alert.title,
        pkg,
        alert.repo,
        alert.severity.as_str(),
        alert.kind.as_db_str(),
        alert.url,
    )
}

// ---------------------------------------------------------------------------
// Read handlers
// ---------------------------------------------------------------------------

pub(super) fn handle_list_review_prs(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ListReviewPrsArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };

    let mode = parsed.mode.as_deref().unwrap_or("all");

    if !matches!(mode, "reviewer" | "author" | "all") {
        return JsonRpcResponse::err(
            id,
            -32602,
            format!("Invalid mode: {mode}. Use reviewer, author, or all"),
        );
    }

    let mut prs: Vec<crate::models::ReviewPr> = Vec::new();
    if mode == "reviewer" || mode == "all" {
        match state.db.load_prs(PrKind::Review) {
            Ok(loaded) => prs.extend(loaded),
            Err(e) => return JsonRpcResponse::err(id, -32603, format!("DB error: {e}")),
        }
    }
    if mode == "author" || mode == "all" {
        match state.db.load_prs(PrKind::My) {
            Ok(loaded) => prs.extend(loaded),
            Err(e) => return JsonRpcResponse::err(id, -32603, format!("DB error: {e}")),
        }
    }

    if let Some(ref repo_filter) = parsed.repo {
        prs.retain(|p| &p.repo == repo_filter);
    }

    if prs.is_empty() {
        return JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": "No PRs found"}]}),
        );
    }

    let lines: Vec<String> = prs.iter().map(format_review_pr).collect();
    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": lines.join("\n\n")}]}),
    )
}

pub(super) fn handle_get_review_pr(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<GetReviewPrArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };

    match state.db.get_review_pr(&parsed.repo, parsed.number) {
        Ok(Some(pr)) => JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": format_review_pr(&pr)}]}),
        ),
        Ok(None) => JsonRpcResponse::err(
            id,
            -32602,
            format!("PR not found: {}#{}", parsed.repo, parsed.number),
        ),
        Err(e) => JsonRpcResponse::err(id, -32603, format!("DB error: {e}")),
    }
}

pub(super) fn handle_list_security_alerts(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ListSecurityAlertsArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };

    let mut alerts = match state.db.load_security_alerts() {
        Ok(a) => a,
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("DB error: {e}")),
    };

    if let Some(ref repo_filter) = parsed.repo {
        alerts.retain(|a| &a.repo == repo_filter);
    }
    if let Some(ref sev_filter) = parsed.severity {
        let sev = match sev_filter.as_str() {
            "critical" => AlertSeverity::Critical,
            "high" => AlertSeverity::High,
            "medium" => AlertSeverity::Medium,
            "low" => AlertSeverity::Low,
            other => {
                return JsonRpcResponse::err(
                    id,
                    -32602,
                    format!("Invalid severity: {other}. Use critical, high, medium, or low"),
                );
            }
        };
        alerts.retain(|a| a.severity == sev);
    }
    if let Some(ref kind_filter) = parsed.kind {
        let kind = match kind_filter.as_str() {
            "dependabot" => AlertKind::Dependabot,
            "code_scanning" => AlertKind::CodeScanning,
            other => {
                return JsonRpcResponse::err(
                    id,
                    -32602,
                    format!("Invalid kind: {other}. Use dependabot or code_scanning"),
                );
            }
        };
        alerts.retain(|a| a.kind == kind);
    }

    if alerts.is_empty() {
        return JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": "No alerts found"}]}),
        );
    }

    let lines: Vec<String> = alerts.iter().map(format_security_alert).collect();
    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": lines.join("\n\n")}]}),
    )
}

pub(super) fn handle_get_security_alert(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<GetSecurityAlertArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };

    let kind = match parsed.kind.as_str() {
        "dependabot" => AlertKind::Dependabot,
        "code_scanning" => AlertKind::CodeScanning,
        other => {
            return JsonRpcResponse::err(
                id,
                -32602,
                format!("Invalid kind: {other}. Use dependabot or code_scanning"),
            );
        }
    };

    match state
        .db
        .get_security_alert(&parsed.repo, parsed.number, kind)
    {
        Ok(Some(alert)) => JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": format_security_alert(&alert)}]}),
        ),
        Ok(None) => JsonRpcResponse::err(
            id,
            -32602,
            format!(
                "Security alert not found: {}#{} ({})",
                parsed.repo, parsed.number, parsed.kind
            ),
        ),
        Err(e) => JsonRpcResponse::err(id, -32603, format!("DB error: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Dispatch handlers
// ---------------------------------------------------------------------------

pub(super) async fn handle_dispatch_review_agent(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<DispatchReviewAgentArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(repo = %parsed.repo, number = parsed.number, "MCP dispatch_review_agent");

    let pr = match state.db.get_review_pr(&parsed.repo, parsed.number) {
        Ok(Some(pr)) => pr,
        Ok(None) => {
            return JsonRpcResponse::err(
                id,
                -32602,
                format!("PR not found: {}#{}", parsed.repo, parsed.number),
            );
        }
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("DB error: {e}")),
    };

    // FindingsReady is intentionally excluded: it means the agent completed successfully
    // and re-dispatching is allowed (e.g. to do a fresh review pass).
    let pr_key = crate::models::PrRef::new(parsed.repo.clone(), parsed.number);
    let has_active_agent = state
        .db
        .load_pr_agent_states()
        .ok()
        .and_then(|m| m.get(&pr_key).cloned())
        .map(|h| h.status == ReviewAgentStatus::Reviewing)
        .unwrap_or(false);
    if has_active_agent {
        return JsonRpcResponse::err(
            id,
            -32602,
            format!(
                "PR {}#{} already has an active review agent",
                parsed.repo, parsed.number
            ),
        );
    }

    let req = crate::tui::ReviewAgentRequest {
        repo: parsed.local_repo.clone(),
        github_repo: parsed.repo.clone(),
        number: parsed.number,
        head_ref: pr.head_ref.clone(),
        is_dependabot: false,
    };

    let runner = state.runner.clone();
    let db = state.db.clone();
    let repo = parsed.repo.clone();
    let number = parsed.number;

    let result = match tokio::task::spawn_blocking(move || {
        crate::dispatch::dispatch_review_agent(&req, &*runner)
    })
    .await
    {
        Ok(r) => r,
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("internal error: {e}")),
    };

    match result {
        Ok(dispatch_result) => {
            // Try both PR table kinds — set_pr_agent uses WHERE clause so only the
            // matching table will be updated. Warn only if neither table was updated.
            let mut persisted = false;
            for kind in [PrKind::Review, PrKind::My] {
                match db.set_pr_agent(
                    kind,
                    &repo,
                    number,
                    &dispatch_result.tmux_window,
                    &dispatch_result.worktree_path,
                ) {
                    Ok(true) => {
                        persisted = true;
                        break;
                    }
                    Ok(false) => {} // PR not in this table
                    Err(e) => {
                        tracing::warn!(
                            "dispatch_review_agent: failed to persist agent ({kind:?}): {e}"
                        )
                    }
                }
            }
            if !persisted {
                tracing::warn!(
                    "dispatch_review_agent: could not find {repo}#{number} in any PR table to persist agent"
                );
            }
            state.notify();
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "Review agent dispatched for {}#{} (window: {}, worktree: {})",
                    repo, number, dispatch_result.tmux_window, dispatch_result.worktree_path
                )}]}),
            )
        }
        Err(e) => JsonRpcResponse::err(id, -32603, format!("dispatch failed: {e}")),
    }
}

pub(super) async fn handle_dispatch_fix_agent(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<DispatchFixAgentArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(repo = %parsed.repo, number = parsed.number, kind = %parsed.kind, "MCP dispatch_fix_agent");

    let kind = match parsed.kind.as_str() {
        "dependabot" => AlertKind::Dependabot,
        "code_scanning" => AlertKind::CodeScanning,
        other => {
            return JsonRpcResponse::err(
                id,
                -32602,
                format!("Invalid kind: {other}. Use dependabot or code_scanning"),
            );
        }
    };

    let alert = match state
        .db
        .get_security_alert(&parsed.repo, parsed.number, kind)
    {
        Ok(Some(a)) => a,
        Ok(None) => {
            return JsonRpcResponse::err(
                id,
                -32602,
                format!(
                    "Security alert not found: {}#{} ({})",
                    parsed.repo, parsed.number, parsed.kind
                ),
            );
        }
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("DB error: {e}")),
    };

    // FindingsReady is intentionally excluded: it means the agent completed successfully
    // and re-dispatching is allowed (e.g. to do a fresh fix pass).
    let alert_key =
        crate::tui::types::FixDispatchKey::new(parsed.repo.clone(), parsed.number, kind);
    let has_active_fix_agent = state
        .db
        .load_alert_agent_states()
        .ok()
        .and_then(|m| m.get(&alert_key).cloned())
        .map(|h| h.status == ReviewAgentStatus::Reviewing)
        .unwrap_or(false);
    if has_active_fix_agent {
        return JsonRpcResponse::err(
            id,
            -32602,
            format!(
                "Alert {}#{} ({}) already has an active fix agent",
                parsed.repo, parsed.number, parsed.kind
            ),
        );
    }

    let req = crate::tui::FixAgentRequest {
        repo: parsed.local_repo.clone(),
        github_repo: parsed.repo.clone(),
        number: parsed.number,
        kind: alert.kind,
        title: alert.title.clone(),
        description: alert.description.clone(),
        package: alert.package.clone(),
        fixed_version: alert.fixed_version.clone(),
    };

    let runner = state.runner.clone();
    let db = state.db.clone();
    let repo = parsed.repo.clone();
    let number = parsed.number;

    let result = match tokio::task::spawn_blocking(move || {
        crate::dispatch::dispatch_fix_agent(req, &*runner)
    })
    .await
    {
        Ok(r) => r,
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("internal error: {e}")),
    };

    match result {
        Ok(dispatch_result) => {
            // set_alert_agent also sets agent_status = 'reviewing'
            match db.set_alert_agent(
                &repo,
                number,
                kind,
                &dispatch_result.tmux_window,
                &dispatch_result.worktree_path,
            ) {
                Ok(true) => {}
                Ok(false) => tracing::warn!(
                    "dispatch_fix_agent: alert {repo}#{number} not found in DB to persist agent"
                ),
                Err(e) => tracing::warn!("dispatch_fix_agent: failed to persist agent: {e}"),
            }
            state.notify();
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "Fix agent dispatched for {}#{} ({}) (window: {}, worktree: {})",
                    repo, number, kind.as_db_str(),
                    dispatch_result.tmux_window, dispatch_result.worktree_path
                )}]}),
            )
        }
        Err(e) => JsonRpcResponse::err(id, -32603, format!("dispatch failed: {e}")),
    }
}
