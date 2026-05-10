#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[tokio::test]
async fn save_and_load_review_prs() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    // Initially empty
    let prs = db.load_prs(super::super::PrKind::Review).await.unwrap();
    assert!(prs.is_empty());

    // Save some PRs
    let pr1 = ReviewPr {
        number: 42,
        title: "Fix bug".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec!["bug".to_string()],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    let pr2 = ReviewPr {
        number: 99,
        title: "Add feature".to_string(),
        author: "bob".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/99".to_string(),
        is_draft: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 200,
        deletions: 80,
        review_decision: ReviewDecision::Approved,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };

    db.save_prs(super::super::PrKind::Review, &[pr1, pr2])
        .await
        .unwrap();

    let loaded = db.load_prs(super::super::PrKind::Review).await.unwrap();
    assert_eq!(loaded.len(), 2);

    let p1 = loaded.iter().find(|p| p.number == 42).unwrap();
    assert_eq!(p1.title, "Fix bug");
    assert_eq!(p1.author, "alice");
    assert_eq!(p1.repo, "acme/app");
    assert!(!p1.is_draft);
    assert_eq!(p1.additions, 10);
    assert_eq!(p1.review_decision, ReviewDecision::ReviewRequired);
    assert_eq!(p1.labels, vec!["bug".to_string()]);

    let p2 = loaded.iter().find(|p| p.number == 99).unwrap();
    assert_eq!(p2.review_decision, ReviewDecision::Approved);
    assert!(p2.is_draft);
    assert!(p2.labels.is_empty());
}

#[tokio::test]
async fn get_review_pr_found_in_review_prs_table() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};

    let db = Database::open_in_memory().unwrap();
    let pr = ReviewPr {
        number: 42,
        title: "Fix bug".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/fix".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    db.save_prs(super::super::PrKind::Review, &[pr])
        .await
        .unwrap();

    let found = db.get_review_pr("acme/app", 42).await.unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.number, 42);
    assert_eq!(found.title, "Fix bug");
    assert_eq!(found.head_ref, "feature/fix");
}

#[tokio::test]
async fn get_review_pr_found_in_my_prs_table() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};

    let db = Database::open_in_memory().unwrap();
    let pr = ReviewPr {
        number: 99,
        title: "My authored PR".to_string(),
        author: "me".to_string(),
        repo: "acme/lib".to_string(),
        url: "https://github.com/acme/lib/pull/99".to_string(),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 5,
        deletions: 2,
        review_decision: ReviewDecision::Approved,
        labels: vec![],
        body: String::new(),
        head_ref: "my-branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    db.save_prs(super::super::PrKind::My, &[pr]).await.unwrap();

    // Not in review_prs — should fall back to my_prs
    let found = db.get_review_pr("acme/lib", 99).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().title, "My authored PR");
}

#[tokio::test]
async fn get_review_pr_not_found() {
    let db = Database::open_in_memory().unwrap();
    let result = db.get_review_pr("acme/app", 999).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn save_review_prs_replaces_all() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr, Reviewer};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    let pr1 = ReviewPr {
        number: 1,
        title: "Old PR".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/1".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: "Initial body".to_string(),
        head_ref: "feature/old-branch".to_string(),
        ci_status: CiStatus::Pending,
        reviewers: vec![Reviewer {
            login: "carol".to_string(),
            decision: None,
        }],
    };
    db.save_prs(super::super::PrKind::Review, &[pr1])
        .await
        .unwrap();
    assert_eq!(
        db.load_prs(super::super::PrKind::Review)
            .await
            .unwrap()
            .len(),
        1
    );

    // Verify new fields round-trip on the first save
    let loaded_first = db.load_prs(super::super::PrKind::Review).await.unwrap();
    assert_eq!(loaded_first[0].body, "Initial body");
    assert_eq!(loaded_first[0].head_ref, "feature/old-branch");
    assert_eq!(loaded_first[0].ci_status, CiStatus::Pending);
    assert_eq!(loaded_first[0].reviewers.len(), 1);
    assert_eq!(loaded_first[0].reviewers[0].login, "carol");
    assert_eq!(loaded_first[0].reviewers[0].decision, None);

    // Save new set — old ones should be gone
    let pr2 = ReviewPr {
        number: 2,
        title: "New PR".to_string(),
        author: "bob".to_string(),
        repo: "acme/other".to_string(),
        url: "https://github.com/acme/other/pull/2".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 5,
        deletions: 3,
        review_decision: ReviewDecision::ChangesRequested,
        labels: vec!["urgent".to_string()],
        body: "Needs more work".to_string(),
        head_ref: "fix/new-branch".to_string(),
        ci_status: CiStatus::Failure,
        reviewers: vec![Reviewer {
            login: "dave".to_string(),
            decision: Some(ReviewDecision::ChangesRequested),
        }],
    };
    db.save_prs(super::super::PrKind::Review, &[pr2])
        .await
        .unwrap();

    let loaded = db.load_prs(super::super::PrKind::Review).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].number, 2);
    assert_eq!(loaded[0].repo, "acme/other");
    assert_eq!(loaded[0].body, "Needs more work");
    assert_eq!(loaded[0].head_ref, "fix/new-branch");
    assert_eq!(loaded[0].ci_status, CiStatus::Failure);
    assert_eq!(loaded[0].reviewers.len(), 1);
    assert_eq!(loaded[0].reviewers[0].login, "dave");
    assert_eq!(
        loaded[0].reviewers[0].decision,
        Some(ReviewDecision::ChangesRequested)
    );
}

#[tokio::test]
async fn save_review_prs_preserves_agent_fields() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    // Insert a PR and manually set agent fields
    let pr = ReviewPr {
        number: 42,
        title: "Initial".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature-branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    db.save_prs(super::super::PrKind::Review, &[pr])
        .await
        .unwrap();

    // Simulate agent dispatch via the proper set_pr_agent method
    db.set_pr_agent(
        super::super::PrKind::Review,
        "acme/app",
        42,
        "dispatch:review-42",
        "/tmp/wt",
    )
    .await
    .unwrap();

    // Now save a refreshed version of the same PR (as if GitHub API returned it)
    let refreshed_pr = ReviewPr {
        number: 42,
        title: "Updated title".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 15,
        deletions: 8,
        review_decision: ReviewDecision::Approved,
        labels: vec![],
        body: String::new(),
        head_ref: "feature-branch".to_string(),
        ci_status: CiStatus::Success,
        reviewers: vec![],
    };
    db.save_prs(super::super::PrKind::Review, &[refreshed_pr])
        .await
        .unwrap();

    // Agent fields in DB should be preserved, GitHub fields should be updated
    let loaded = db.load_prs(super::super::PrKind::Review).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].title, "Updated title");
    assert_eq!(loaded[0].review_decision, ReviewDecision::Approved);

    // Agent status should still be present after refresh
    let status = db
        .pr_agent_status("review_prs", "acme/app", 42)
        .await
        .unwrap();
    assert!(status.is_some(), "agent status should be preserved");
}

#[tokio::test]
async fn save_review_prs_removes_stale_prs() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    let make_pr = |number: i64, repo: &str| ReviewPr {
        number,
        title: format!("PR {number}"),
        author: "alice".to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };

    // Save two PRs
    db.save_prs(
        super::super::PrKind::Review,
        &[make_pr(1, "acme/app"), make_pr(2, "acme/other")],
    )
    .await
    .unwrap();
    assert_eq!(
        db.load_prs(super::super::PrKind::Review)
            .await
            .unwrap()
            .len(),
        2
    );

    // Refresh with only one — the other should be removed
    db.save_prs(super::super::PrKind::Review, &[make_pr(1, "acme/app")])
        .await
        .unwrap();
    let loaded = db.load_prs(super::super::PrKind::Review).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].number, 1);
}

#[tokio::test]
async fn set_pr_agent_updates_fields() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    let pr = ReviewPr {
        number: 42,
        title: "Test".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    db.save_prs(super::super::PrKind::Review, &[pr])
        .await
        .unwrap();

    db.set_pr_agent(
        super::super::PrKind::Review,
        "acme/app",
        42,
        "dispatch:review-42",
        "/tmp/wt",
    )
    .await
    .unwrap();

    let status = db
        .pr_agent_status("review_prs", "acme/app", 42)
        .await
        .unwrap();
    assert_eq!(
        status,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        "agent should be marked as reviewing"
    );
}

#[tokio::test]
async fn update_agent_status_finds_review_pr() {
    use crate::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    let pr = ReviewPr {
        number: 42,
        title: "Test".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    db.save_prs(super::super::PrKind::Review, &[pr])
        .await
        .unwrap();
    db.set_pr_agent(
        super::super::PrKind::Review,
        "acme/app",
        42,
        "dispatch:review-42",
        "/tmp/wt",
    )
    .await
    .unwrap();

    let table = db
        .update_agent_status("acme/app", 42, Some("findings_ready"))
        .await
        .unwrap();
    assert_eq!(table, "review_prs");

    let status = db
        .pr_agent_status("review_prs", "acme/app", 42)
        .await
        .unwrap();
    assert_eq!(status, Some(ReviewAgentStatus::FindingsReady));
}

#[tokio::test]
async fn update_agent_status_finds_bot_pr() {
    use crate::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    let pr = ReviewPr {
        number: 10,
        title: "Bump dep".to_string(),
        author: "dependabot".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/10".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 1,
        deletions: 1,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    db.save_prs(super::super::PrKind::Bot, &[pr]).await.unwrap();
    db.set_pr_agent(
        super::super::PrKind::Bot,
        "acme/app",
        10,
        "dispatch:review-10",
        "/tmp/wt",
    )
    .await
    .unwrap();

    let table = db
        .update_agent_status("acme/app", 10, Some("idle"))
        .await
        .unwrap();
    assert_eq!(table, "bot_prs");

    let status = db.pr_agent_status("bot_prs", "acme/app", 10).await.unwrap();
    assert_eq!(status, Some(ReviewAgentStatus::Idle));
}

#[tokio::test]
async fn update_agent_status_errors_when_no_match() {
    let db = Database::open_in_memory().unwrap();
    let result = db
        .update_agent_status("acme/unknown", 999, Some("idle"))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn update_agent_status_skips_pr_without_tmux() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    let pr = ReviewPr {
        number: 42,
        title: "Test".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    db.save_prs(super::super::PrKind::Review, &[pr])
        .await
        .unwrap();

    // PR has no tmux_window, so update should fail
    let result = db
        .update_agent_status("acme/app", 42, Some("findings_ready"))
        .await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Query coverage: my_prs / bot_prs round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn save_and_load_my_prs() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    assert!(db
        .load_prs(super::super::PrKind::My)
        .await
        .unwrap()
        .is_empty());

    let pr = ReviewPr {
        number: 7,
        title: "My feature".to_string(),
        author: "me".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/7".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 42,
        deletions: 10,
        review_decision: ReviewDecision::Approved,
        labels: vec!["feature".to_string()],
        body: "Add new feature".to_string(),
        head_ref: "feature/my-branch".to_string(),
        ci_status: CiStatus::Success,
        reviewers: vec![],
    };
    db.save_prs(super::super::PrKind::My, &[pr]).await.unwrap();

    let loaded = db.load_prs(super::super::PrKind::My).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].number, 7);
    assert_eq!(loaded[0].title, "My feature");
    assert_eq!(loaded[0].author, "me");
    assert_eq!(loaded[0].review_decision, ReviewDecision::Approved);
    assert_eq!(loaded[0].labels, vec!["feature".to_string()]);
    assert_eq!(loaded[0].body, "Add new feature");
    assert_eq!(loaded[0].ci_status, CiStatus::Success);
}

#[tokio::test]
async fn save_and_load_bot_prs() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    assert!(db
        .load_prs(super::super::PrKind::Bot)
        .await
        .unwrap()
        .is_empty());

    let pr = ReviewPr {
        number: 55,
        title: "Bump lodash".to_string(),
        author: "dependabot[bot]".to_string(),
        repo: "acme/lib".to_string(),
        url: "https://github.com/acme/lib/pull/55".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 3,
        deletions: 3,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec!["dependencies".to_string()],
        body: "Bumps lodash".to_string(),
        head_ref: "dependabot/npm_and_yarn/lodash-4.17.21".to_string(),
        ci_status: CiStatus::Pending,
        reviewers: vec![],
    };
    db.save_prs(super::super::PrKind::Bot, &[pr]).await.unwrap();

    let loaded = db.load_prs(super::super::PrKind::Bot).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].number, 55);
    assert_eq!(loaded[0].title, "Bump lodash");
    assert_eq!(loaded[0].author, "dependabot[bot]");
    assert_eq!(loaded[0].ci_status, CiStatus::Pending);
}

#[tokio::test]
async fn my_prs_and_review_prs_are_independent() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    let make_pr = |number: i64, title: &str| ReviewPr {
        number,
        title: title.to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: format!("https://github.com/acme/app/pull/{number}"),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };

    db.save_prs(super::super::PrKind::My, &[make_pr(1, "My PR")])
        .await
        .unwrap();
    db.save_prs(super::super::PrKind::Review, &[make_pr(2, "Review PR")])
        .await
        .unwrap();
    db.save_prs(super::super::PrKind::Bot, &[make_pr(3, "Bot PR")])
        .await
        .unwrap();

    assert_eq!(
        db.load_prs(super::super::PrKind::My).await.unwrap().len(),
        1
    );
    assert_eq!(
        db.load_prs(super::super::PrKind::Review)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.load_prs(super::super::PrKind::Bot).await.unwrap().len(),
        1
    );

    // Saving empty to one table doesn't affect others
    db.save_prs(super::super::PrKind::My, &[]).await.unwrap();
    assert!(db
        .load_prs(super::super::PrKind::My)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        db.load_prs(super::super::PrKind::Review)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.load_prs(super::super::PrKind::Bot).await.unwrap().len(),
        1
    );
}

// ---------------------------------------------------------------------------
// PrWorkflowStore tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pr_workflow_insert_if_absent_is_idempotent() {
    let db = in_memory_db();
    use crate::models::WorkflowItemKind::*;

    db.insert_pr_workflow_if_absent("org/repo", 42, ReviewerPr)
        .await
        .unwrap();
    db.insert_pr_workflow_if_absent("org/repo", 42, ReviewerPr)
        .await
        .unwrap(); // no-op

    let row = db
        .get_pr_workflow("org/repo", 42, ReviewerPr)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "backlog");
    assert!(row.sub_state.is_none());
}

#[tokio::test]
async fn pr_workflow_upsert_updates_state_and_sub_state() {
    let db = in_memory_db();
    use crate::models::WorkflowItemKind::*;

    db.insert_pr_workflow_if_absent("org/repo", 1, ReviewerPr)
        .await
        .unwrap();
    db.upsert_pr_workflow("org/repo", 1, ReviewerPr, "ongoing", Some("reviewing"))
        .await
        .unwrap();

    let row = db
        .get_pr_workflow("org/repo", 1, ReviewerPr)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "ongoing");
    assert_eq!(row.sub_state.as_deref(), Some("reviewing"));
}

#[tokio::test]
async fn pr_workflow_get_returns_none_when_absent() {
    let db = in_memory_db();
    use crate::models::WorkflowItemKind::*;
    let result = db
        .get_pr_workflow("org/repo", 99, ReviewerPr)
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn pr_workflow_list_returns_all_rows() {
    let db = in_memory_db();
    use crate::models::WorkflowItemKind::*;

    db.insert_pr_workflow_if_absent("org/a", 1, ReviewerPr)
        .await
        .unwrap();
    db.insert_pr_workflow_if_absent("org/b", 2, DependabotAlert)
        .await
        .unwrap();
    db.insert_pr_workflow_if_absent("org/a", 1, CodeScanAlert)
        .await
        .unwrap();

    let rows = db.list_pr_workflows().await.unwrap();
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn pr_workflow_prune_removes_done_older_than_threshold() {
    let db = in_memory_db();

    // Insert seed rows synchronously, scoping the MutexGuard so it does not
    // straddle a later `.await` (clippy::await_holding_lock).
    {
        let conn = db.conn().unwrap();
        conn.execute(
            "INSERT INTO pr_workflow_states (repo, number, kind, state, updated_at)
             VALUES ('org/repo', 1, 'reviewer_pr', 'done', '2020-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        // Insert a recent done row — should NOT be pruned
        conn.execute(
            "INSERT INTO pr_workflow_states (repo, number, kind, state, updated_at)
             VALUES ('org/repo', 2, 'reviewer_pr', 'done', ?1)",
            rusqlite::params![chrono::Utc::now().to_rfc3339()],
        )
        .unwrap();
        // Insert an ongoing row — should NOT be pruned regardless of age
        conn.execute(
            "INSERT INTO pr_workflow_states (repo, number, kind, state, updated_at)
             VALUES ('org/repo', 3, 'reviewer_pr', 'ongoing', '2020-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
    }

    db.prune_done_pr_workflows(chrono::Duration::days(7))
        .await
        .unwrap();

    let rows = db.list_pr_workflows().await.unwrap();
    // Only old done row removed; recent done and ongoing remain
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| !(r.state == "done" && r.number == 1)));
}

#[tokio::test]
async fn pr_workflow_kind_roundtrip_in_db() {
    let db = in_memory_db();
    use crate::models::WorkflowItemKind::*;

    for kind in [ReviewerPr, DependabotPr, DependabotAlert, CodeScanAlert] {
        db.insert_pr_workflow_if_absent("org/repo", kind as i64, kind)
            .await
            .unwrap();
        let row = db
            .get_pr_workflow("org/repo", kind as i64, kind)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.kind, kind);
    }
}

#[tokio::test]
async fn find_pr_workflow_kind_returns_kind_when_row_exists() {
    let db = in_memory_db();
    use crate::models::WorkflowItemKind::*;

    // Insert a workflow row for ReviewerPr
    db.insert_pr_workflow_if_absent("org/repo", 42, ReviewerPr)
        .await
        .unwrap();

    // find_pr_workflow_kind should return the kind
    let kind = db.find_pr_workflow_kind("org/repo", 42).await.unwrap();
    assert_eq!(kind, Some(ReviewerPr));
}

#[tokio::test]
async fn find_pr_workflow_kind_returns_none_when_no_row_exists() {
    let db = in_memory_db();

    // No workflow row exists for this (repo, number) pair
    let kind = db.find_pr_workflow_kind("org/repo", 99).await.unwrap();
    assert_eq!(kind, None);
}

#[tokio::test]
async fn find_pr_workflow_kind_with_multiple_kinds_returns_first() {
    let db = in_memory_db();
    use crate::models::WorkflowItemKind::*;

    // Insert multiple workflow rows for the same (repo, number) with different kinds
    db.insert_pr_workflow_if_absent("org/repo", 5, ReviewerPr)
        .await
        .unwrap();
    db.insert_pr_workflow_if_absent("org/repo", 5, DependabotAlert)
        .await
        .unwrap();

    // find_pr_workflow_kind uses LIMIT 1, so it returns one of them
    // (the one from the LIMIT 1 query — typically the first inserted)
    let kind = db.find_pr_workflow_kind("org/repo", 5).await.unwrap();
    assert!(kind.is_some(), "Should find one of the kinds");
    // We don't assert which one because LIMIT 1 order is undefined without ORDER BY
}
