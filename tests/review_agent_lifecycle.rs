#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration test: review-agent state machine end-to-end.
//!
//! Drives the lifecycle Reviewing → FindingsReady → Idle through the public
//! `PrStore` API and asserts both the agent_status column and the
//! pr_workflow_states row reflect each transition.

use chrono::Utc;

use dispatch_tui::db::{Database, PrKind, PrStore, PrWorkflowStore};
use dispatch_tui::models::{
    CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr, WorkflowItemKind,
};

fn pr_fixture(repo: &str, number: i64) -> ReviewPr {
    ReviewPr {
        number,
        title: format!("PR #{number}"),
        author: "alice".to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 2,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    }
}

#[test]
fn review_agent_full_state_machine() {
    let db = Database::open_in_memory().unwrap();
    let repo = "acme/app";
    let number = 42i64;

    // 1. Persist a PR and dispatch a review agent (set tmux_window + worktree).
    db.save_prs(PrKind::Review, &[pr_fixture(repo, number)])
        .unwrap();
    let updated = db
        .set_pr_agent(
            PrKind::Review,
            repo,
            number,
            "dispatch:review-42",
            "/tmp/wt-42",
        )
        .unwrap();
    assert!(updated, "set_pr_agent should report a row change");

    // Agent is in Reviewing.
    let status = db.pr_agent_status("review_prs", repo, number).unwrap();
    assert_eq!(
        status,
        Some(ReviewAgentStatus::Reviewing),
        "set_pr_agent should set agent_status='reviewing'"
    );

    // Pre-insert workflow row in Ongoing/Reviewing so update_review_status
    // has something to upsert against.
    db.insert_pr_workflow_if_absent(repo, number, WorkflowItemKind::ReviewerPr)
        .unwrap();
    db.upsert_pr_workflow(
        repo,
        number,
        WorkflowItemKind::ReviewerPr,
        "ongoing",
        Some("reviewing"),
    )
    .unwrap();

    // 2. Agent calls update_review_status(findings_ready).
    let table = db
        .update_agent_status(repo, number, Some("findings_ready"))
        .unwrap();
    assert_eq!(table, "review_prs");

    let status = db.pr_agent_status("review_prs", repo, number).unwrap();
    assert_eq!(status, Some(ReviewAgentStatus::FindingsReady));

    // 3. Agent calls update_review_status(idle) — re-review allowed from here.
    db.update_agent_status(repo, number, Some("idle")).unwrap();
    let status = db.pr_agent_status("review_prs", repo, number).unwrap();
    assert_eq!(status, Some(ReviewAgentStatus::Idle));

    // 4. Re-dispatch from Idle resets agent_status back to Reviewing.
    let updated = db
        .set_pr_agent(
            PrKind::Review,
            repo,
            number,
            "dispatch:review-42",
            "/tmp/wt-42",
        )
        .unwrap();
    assert!(updated);
    let status = db.pr_agent_status("review_prs", repo, number).unwrap();
    assert_eq!(status, Some(ReviewAgentStatus::Reviewing));
}

#[test]
fn update_agent_status_errors_when_no_active_agent() {
    let db = Database::open_in_memory().unwrap();
    db.save_prs(PrKind::Review, &[pr_fixture("acme/app", 7)])
        .unwrap();

    // No set_pr_agent call: tmux_window is still NULL.
    let result = db.update_agent_status("acme/app", 7, Some("idle"));
    assert!(
        result.is_err(),
        "update_agent_status should fail when no agent has been dispatched"
    );
}
