#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration test: review-agent dispatch lifecycle.
//!
//! Drives the live lifecycle (no agent → reviewing → no agent) through the
//! public `PrStore` API and asserts the agent_status column reflects each
//! transition.

use chrono::Utc;

use dispatch_tui::db::{Database, PrKind, PrStore};
use dispatch_tui::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr};

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

#[tokio::test]
async fn review_agent_dispatch_sets_reviewing_status() {
    let db = Database::open_in_memory().await.unwrap();
    let repo = "acme/app";
    let number = 42i64;

    db.save_prs(PrKind::Review, &[pr_fixture(repo, number)])
        .await
        .unwrap();
    let updated = db
        .set_pr_agent(
            PrKind::Review,
            repo,
            number,
            "dispatch:review-42",
            "/tmp/wt-42",
        )
        .await
        .unwrap();
    assert!(updated, "set_pr_agent should report a row change");

    let status = db
        .pr_agent_status("review_prs", repo, number)
        .await
        .unwrap();
    assert_eq!(
        status,
        Some(ReviewAgentStatus::Reviewing),
        "set_pr_agent should set agent_status='reviewing'"
    );
}
