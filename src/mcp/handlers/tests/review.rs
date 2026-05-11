#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

// ---------------------------------------------------------------------------
// Fixtures for review/security tests
// ---------------------------------------------------------------------------

async fn insert_my_pr_fixture(state: &Arc<McpState>, number: i64, repo: &str) {
    use crate::db::PrKind;
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    let pr = ReviewPr {
        number,
        title: format!("My PR #{number}"),
        author: "alice".to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 5,
        deletions: 1,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    let mut existing = state.db.load_prs(PrKind::My).await.unwrap_or_default();
    existing.retain(|p| !(p.repo == repo && p.number == number));
    existing.push(pr);
    state.db.save_prs(PrKind::My, &existing).await.unwrap();
}

async fn insert_review_pr_fixture(state: &Arc<McpState>, number: i64, repo: &str) {
    use crate::db::PrKind;
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    let pr = ReviewPr {
        number,
        title: format!("PR #{number}"),
        author: "alice".to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 10,
        deletions: 2,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    // Load existing PRs and append to avoid batch-replace deleting prior inserts.
    let mut existing = state.db.load_prs(PrKind::Review).await.unwrap_or_default();
    existing.retain(|p| !(p.repo == repo && p.number == number));
    existing.push(pr);
    state.db.save_prs(PrKind::Review, &existing).await.unwrap();
}

async fn insert_security_alert_fixture(
    state: &Arc<McpState>,
    number: i64,
    repo: &str,
    kind: crate::models::AlertKind,
) {
    use crate::models::{AlertSeverity, SecurityAlert};
    let alert = SecurityAlert {
        number,
        repo: repo.to_string(),
        severity: AlertSeverity::High,
        kind,
        title: format!("Alert #{number}"),
        package: Some("some-pkg".to_string()),
        vulnerable_range: Some("< 1.0".to_string()),
        fixed_version: Some("1.0.0".to_string()),
        cvss_score: Some(7.5),
        url: format!("https://github.com/{repo}/security/dependabot/{number}"),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: "A vulnerability".to_string(),
    };
    // Load existing alerts and append to avoid batch-replace deleting prior inserts.
    let mut existing = state.db.load_security_alerts().await.unwrap_or_default();
    existing.retain(|a| !(a.repo == repo && a.number == number && a.kind == kind));
    existing.push(alert);
    state.db.save_security_alerts(&existing).await.unwrap();
}

// ---------------------------------------------------------------------------
// list_review_prs tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_review_prs_empty() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("No PRs found"));
}

#[tokio::test]
async fn list_review_prs_returns_stored_prs() {
    let state = test_state().await;
    insert_review_pr_fixture(&state, 42, "acme/app").await;
    insert_review_pr_fixture(&state, 99, "acme/app").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"mode": "reviewer"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("42"));
    assert!(text.contains("99"));
}

#[tokio::test]
async fn list_review_prs_filters_by_repo() {
    let state = test_state().await;
    insert_review_pr_fixture(&state, 1, "acme/app").await;
    insert_review_pr_fixture(&state, 2, "acme/other").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"repo": "acme/app"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/app"));
    assert!(!text.contains("acme/other"));
}

#[tokio::test]
async fn list_review_prs_mode_author() {
    let state = test_state().await;
    insert_my_pr_fixture(&state, 55, "acme/app").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"mode": "author"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("55"), "PR #55 should appear in author mode");
}

#[tokio::test]
async fn list_review_prs_mode_all() {
    let state = test_state().await;
    insert_review_pr_fixture(&state, 10, "acme/app").await;
    insert_my_pr_fixture(&state, 20, "acme/app").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"mode": "all"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("10"),
        "reviewer PR #10 should appear in all mode"
    );
    assert!(
        text.contains("20"),
        "author PR #20 should appear in all mode"
    );
}

// ---------------------------------------------------------------------------
// get_review_pr tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_review_pr_found() {
    let state = test_state().await;
    insert_review_pr_fixture(&state, 42, "acme/app").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_review_pr", "arguments": {"repo": "acme/app", "number": 42}})),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/app"));
    assert!(text.contains("42"));
}

#[tokio::test]
async fn get_review_pr_not_found() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_review_pr", "arguments": {"repo": "acme/app", "number": 999}})),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn get_review_pr_found_in_my_prs() {
    let state = test_state().await;
    insert_my_pr_fixture(&state, 55, "acme/app").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_review_pr", "arguments": {"repo": "acme/app", "number": 55}})),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/app"));
    assert!(text.contains("55"));
}

// ---------------------------------------------------------------------------
// list_security_alerts tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_security_alerts_empty() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("No alerts found"));
}

#[tokio::test]
async fn list_security_alerts_returns_stored_alerts() {
    use crate::models::AlertKind;
    let state = test_state().await;
    insert_security_alert_fixture(&state, 1, "acme/api", AlertKind::Dependabot).await;
    insert_security_alert_fixture(&state, 2, "acme/api", AlertKind::CodeScanning).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Alert #1"));
    assert!(text.contains("Alert #2"));
}

#[tokio::test]
async fn list_security_alerts_filters_by_kind() {
    use crate::models::AlertKind;
    let state = test_state().await;
    insert_security_alert_fixture(&state, 1, "acme/api", AlertKind::Dependabot).await;
    insert_security_alert_fixture(&state, 2, "acme/api", AlertKind::CodeScanning).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {"kind": "dependabot"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Alert #1"));
    assert!(!text.contains("Alert #2"));
}

#[tokio::test]
async fn list_security_alerts_filters_by_severity() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};
    let state = test_state().await;

    let high_alert = SecurityAlert {
        number: 1,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "High Alert".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com/1".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: String::new(),
    };
    let critical_alert = SecurityAlert {
        number: 2,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::Critical,
        kind: AlertKind::Dependabot,
        title: "Critical Alert".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com/2".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: String::new(),
    };
    state
        .db
        .save_security_alerts(&[high_alert, critical_alert])
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {"severity": "high"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("High Alert"), "High alert should appear");
    assert!(
        !text.contains("Critical Alert"),
        "Critical alert should not appear"
    );
}

#[tokio::test]
async fn list_security_alerts_filters_by_repo() {
    use crate::models::AlertKind;
    let state = test_state().await;
    insert_security_alert_fixture(&state, 1, "acme/api", AlertKind::Dependabot).await;
    insert_security_alert_fixture(&state, 2, "acme/web", AlertKind::Dependabot).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {"repo": "acme/api"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/api"), "acme/api alert should appear");
    assert!(
        !text.contains("acme/web"),
        "acme/web alert should not appear"
    );
}

// ---------------------------------------------------------------------------
// get_security_alert tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_security_alert_found() {
    use crate::models::AlertKind;
    let state = test_state().await;
    insert_security_alert_fixture(&state, 7, "acme/api", AlertKind::Dependabot).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_security_alert",
            "arguments": {"repo": "acme/api", "number": 7, "kind": "dependabot"}
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/api"));
    assert!(text.contains("Alert #7"));
}

#[tokio::test]
async fn get_security_alert_not_found() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_security_alert",
            "arguments": {"repo": "acme/api", "number": 999, "kind": "dependabot"}
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

// ---------------------------------------------------------------------------
// dispatch_review_agent tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatch_review_agent_pr_not_found() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_review_agent",
            "arguments": {"repo": "acme/app", "number": 999, "local_repo": "/tmp/repo"}
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_review_agent_already_reviewing() {
    use crate::db::PrKind;
    use crate::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr};
    let state = test_state().await;
    let pr = ReviewPr {
        number: 42,
        title: "PR #42".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 10,
        deletions: 2,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    state.db.save_prs(PrKind::Review, &[pr]).await.unwrap();
    // Persist the agent tracking fields (save_prs does not write these).
    state
        .db
        .set_pr_agent(
            PrKind::Review,
            "acme/app",
            42,
            "review-42",
            "/repo/.worktrees/review-42",
        )
        .await
        .unwrap();
    let _ = ReviewAgentStatus::Reviewing; // confirm variant exists

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_review_agent",
            "arguments": {"repo": "acme/app", "number": 42, "local_repo": "/tmp/repo"}
        })),
    )
    .await;
    assert_error(&resp, "already has an active review agent");
}

#[tokio::test]
async fn dispatch_review_agent_success() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    // Pre-create worktree dir so git worktree add is skipped.
    std::fs::create_dir_all(dir.path().join(".worktrees").join("review-42")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (has_window → false, empty stdout)
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git fetch origin feature/branch
        // git worktree add skipped (dir pre-exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux send-keys -l (claude cmd)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    insert_review_pr_fixture(&state, 42, "acme/app").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_review_agent",
            "arguments": {"repo": "acme/app", "number": 42, "local_repo": repo_path}
        })),
    )
    .await;

    assert!(
        resp.error.is_none(),
        "expected success, got error: {:?}",
        resp.error
    );
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Review agent dispatched"),
        "expected dispatch confirmation: {text}"
    );

    let status = db
        .pr_agent_status("review_prs", "acme/app", 42)
        .await
        .unwrap();
    assert_eq!(
        status,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        "agent should be reviewing after dispatch"
    );
}

// ---------------------------------------------------------------------------
// dispatch_fix_agent tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatch_fix_agent_alert_not_found() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_fix_agent",
            "arguments": {
                "repo": "acme/api", "number": 999,
                "kind": "dependabot", "local_repo": "/tmp/repo"
            }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_fix_agent_already_reviewing() {
    use crate::models::{AlertKind, AlertSeverity, ReviewAgentStatus, SecurityAlert};
    let state = test_state().await;
    let alert = SecurityAlert {
        number: 7,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-9999".to_string(),
        package: Some("pkg".to_string()),
        vulnerable_range: None,
        fixed_version: Some("1.0.0".to_string()),
        cvss_score: None,
        url: "https://example.com".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: "A vuln".to_string(),
    };
    state.db.save_security_alerts(&[alert]).await.unwrap();
    // Persist the agent tracking fields (save_security_alerts does not write these).
    state
        .db
        .set_alert_agent(
            "acme/api",
            7,
            AlertKind::Dependabot,
            "fix-7",
            "/repo/.worktrees/fix-vuln-7",
        )
        .await
        .unwrap();
    let _ = ReviewAgentStatus::Reviewing; // confirm variant exists

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_fix_agent",
            "arguments": {
                "repo": "acme/api", "number": 7,
                "kind": "dependabot", "local_repo": "/tmp/repo"
            }
        })),
    )
    .await;
    assert_error(&resp, "already has an active fix agent");
}

#[tokio::test]
async fn dispatch_fix_agent_success() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    // Pre-create worktree dir so git worktree add is skipped.
    std::fs::create_dir_all(dir.path().join(".worktrees").join("fix-vuln-7")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (has_window)
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"), // git symbolic-ref (detect default branch)
        MockProcessRunner::ok(),                                          // git fetch origin main
        // git worktree add skipped (dir pre-exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux send-keys -l (claude cmd)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let alert = SecurityAlert {
        number: 7,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-0001".to_string(),
        package: Some("lodash".to_string()),
        vulnerable_range: None,
        fixed_version: Some("4.17.21".to_string()),
        cvss_score: None,
        url: "https://example.com/7".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: "Prototype pollution".to_string(),
    };
    db.save_security_alerts(&[alert]).await.unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_fix_agent",
            "arguments": {
                "repo": "acme/api", "number": 7,
                "kind": "dependabot", "local_repo": repo_path
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_none(),
        "expected success, got error: {:?}",
        resp.error
    );
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Fix agent dispatched"),
        "expected dispatch confirmation: {text}"
    );

    let status = db
        .alert_agent_status("acme/api", 7, AlertKind::Dependabot)
        .await
        .unwrap();
    assert_eq!(
        status,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        "agent should be reviewing after dispatch"
    );
}
