//! Integration tests: review board lifecycle through App::update() with a real (in-memory) DB.
#![allow(dead_code, unused_imports)]

use std::time::Duration;

use chrono::Utc;
use dispatch_tui::models::{CiStatus, ReviewDecision, ReviewPr};
use dispatch_tui::tui::{App, Command, Message, PrListKind, ReviewAgentRequest};

fn make_app() -> App {
    App::new(vec![], Duration::from_secs(300))
}

fn make_pr(number: i64, repo: &str) -> ReviewPr {
    ReviewPr {
        number,
        title: format!("PR {number}"),
        author: "alice".to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feat/thing".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    }
}

// ---------------------------------------------------------------------------
// Tick wiring: App emits FetchPrs when review lists are stale
// ---------------------------------------------------------------------------

#[test]
fn tick_triggers_fetch_when_review_list_stale() {
    let mut app = make_app();
    // Both lists have last_fetch = None (never fetched) — needs_fetch returns true
    let cmds = app.update(Message::Tick);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::FetchPrs(PrListKind::Review))),
        "Tick should emit FetchPrs(Review) when list is stale"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::FetchPrs(PrListKind::Authored))),
        "Tick should emit FetchPrs(Authored) when list is stale"
    );
}

// ---------------------------------------------------------------------------
// Bug: security alerts never auto-refresh on tick
// ---------------------------------------------------------------------------

#[test]
fn tick_triggers_security_fetch_when_stale() {
    let mut app = make_app();
    // security.last_fetch = None (default) — needs_fetch(SECURITY_POLL_INTERVAL) returns true
    let cmds = app.update(Message::Tick);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::FetchSecurityAlerts)),
        "Tick should emit FetchSecurityAlerts when security list is stale"
    );
}

// ---------------------------------------------------------------------------
// Dispatch review agent
// ---------------------------------------------------------------------------

#[test]
fn dispatch_review_agent_emits_command() {
    let mut app = make_app();
    // Set a local repo path so resolve_repo_path can match "org/app"
    app.update(Message::RepoPathsUpdated(
        vec!["/repos/org/app".to_string()],
    ));
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![make_pr(42, "org/app")],
    ));

    let req = ReviewAgentRequest {
        repo: "org/app".to_string(),
        github_repo: "org/app".to_string(),
        number: 42,
        head_ref: "feat/thing".to_string(),
        is_dependabot: false,
    };
    let cmds = app.update(Message::DispatchReviewAgent(req));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::DispatchReviewAgent(_))),
        "DispatchReviewAgent message should emit Command::DispatchReviewAgent"
    );
}

#[test]
fn review_agent_dispatched_registers_handle() {
    let mut app = make_app();
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![make_pr(42, "org/app")],
    ));

    app.update(Message::ReviewAgentDispatched {
        github_repo: "org/app".to_string(),
        number: 42,
        tmux_window: "win-42".to_string(),
        worktree: "/wt/42".to_string(),
    });

    let handle = app
        .review_agent_handle("org/app", 42)
        .expect("handle should be registered after ReviewAgentDispatched");
    assert_eq!(handle.tmux_window, "win-42");
    assert_eq!(handle.worktree, "/wt/42");
}

// ---------------------------------------------------------------------------
// Status update
// ---------------------------------------------------------------------------

#[test]
fn review_status_update_reflects_on_handle() {
    use dispatch_tui::models::ReviewAgentStatus;

    let mut app = make_app();
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![make_pr(42, "org/app")],
    ));
    app.update(Message::ReviewAgentDispatched {
        github_repo: "org/app".to_string(),
        number: 42,
        tmux_window: "win-42".to_string(),
        worktree: "/wt/42".to_string(),
    });

    app.update(Message::ReviewStatusUpdated {
        repo: "org/app".to_string(),
        number: 42,
        status: ReviewAgentStatus::FindingsReady,
    });

    let handle = app
        .review_agent_handle("org/app", 42)
        .expect("handle should still exist after status update");
    assert_eq!(handle.status, ReviewAgentStatus::FindingsReady);
}

// ---------------------------------------------------------------------------
// PR merged: missing PR with active agent triggers cleanup
// ---------------------------------------------------------------------------

#[test]
fn prs_loaded_without_tracked_pr_triggers_cleanup() {
    let mut app = make_app();
    // Load PR #42 and register a review agent for it
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![make_pr(42, "org/app")],
    ));
    app.update(Message::ReviewAgentDispatched {
        github_repo: "org/app".to_string(),
        number: 42,
        tmux_window: "win-42".to_string(),
        worktree: "/wt/42".to_string(),
    });

    // PR #42 disappears from next fetch (it was merged)
    let cmds = app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![], // no PRs
    ));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::KillTmuxWindow { window } if window == "win-42")),
        "KillTmuxWindow should be emitted when a tracked PR disappears from the board"
    );
    assert!(
        app.review_agent_handle("org/app", 42).is_none(),
        "Agent handle should be removed after PR disappears"
    );
}

// ---------------------------------------------------------------------------
// PR approved moves to Approved decision
// ---------------------------------------------------------------------------

#[test]
fn pr_approved_updates_review_decision() {
    let mut app = make_app();

    // Load PR as ReviewRequired first
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![make_pr(42, "org/app")],
    ));
    assert_eq!(
        app.review_prs()[0].review_decision,
        ReviewDecision::ReviewRequired
    );

    // Reload with approved decision
    let mut approved_pr = make_pr(42, "org/app");
    approved_pr.review_decision = ReviewDecision::Approved;
    app.update(Message::PrsLoaded(PrListKind::Review, vec![approved_pr]));

    assert_eq!(
        app.review_prs()[0].review_decision,
        ReviewDecision::Approved,
        "PR decision should be updated to Approved after PrsLoaded"
    );
}

// ---------------------------------------------------------------------------
// Fetch failure: error state set, existing PRs preserved
// ---------------------------------------------------------------------------

#[test]
fn pr_fetch_failed_sets_error_state_and_preserves_prs() {
    let mut app = make_app();
    // Load some PRs first
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![make_pr(42, "org/app")],
    ));
    assert_eq!(app.review_prs().len(), 1);

    // Simulate fetch failure
    app.update(Message::PrsFetchFailed(
        PrListKind::Review,
        "network timeout".to_string(),
    ));

    assert_eq!(
        app.last_review_error(),
        Some("network timeout"),
        "Error message should be stored on fetch failure"
    );
    assert!(
        !app.review_board_loading(),
        "loading flag should be cleared on failure"
    );
    assert_eq!(
        app.review_prs().len(),
        1,
        "Existing PRs should be preserved on failure — board does not go blank"
    );
}
