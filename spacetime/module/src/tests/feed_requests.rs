//! The feed-request rules (`feed_request.rs`) as pure functions over rows.
//! The table writes are pinned by `spacetime/module-tests/tests/feed_requests.rs`.

use super::*;

fn task(host: &str, worktree: &str, window: &str) -> Task {
    Task {
        id: 7,
        host: host.into(),
        worktree: worktree.into(),
        tmux_window: window.into(),
        repo_path: "/repo".into(),
        ..blank_task()
    }
}

#[test]
fn teardown_is_owed_only_for_another_hosts_held_task() {
    assert!(teardown_request_owed(&task("b", "/wt", ""), "a"));
    assert!(teardown_request_owed(&task("b", "", "win"), "a"));
    assert!(!teardown_request_owed(&task("a", "/wt", "win"), "a"));
    assert!(!teardown_request_owed(&task("", "/wt", "win"), "a"));
    assert!(!teardown_request_owed(&task("b", "", ""), "a"));
}

#[test]
fn the_teardown_row_snapshots_the_task() {
    let row = new_teardown_request(&task("b", "/wt", "win"), "alice", "2026-01-01 00:00:00.000");
    assert_eq!(row.kind, FEED_REQUEST_TEARDOWN);
    assert_eq!((row.host.as_str(), row.task_id), ("b", 7));
    assert_eq!(row.worktree, "/wt");
    assert_eq!(row.tmux_window, "win");
    assert_eq!(row.state, FEED_REQUESTED);
}

fn epic(command: &str) -> Epic {
    Epic {
        id: 3,
        feed_command: command.into(),
        ..blank_epic()
    }
}

#[test]
fn a_refresh_request_is_decided_in_the_order_a_caller_is_told() {
    let e = epic("echo");
    assert!(decide_feed_refresh(None, "b", Some("a"), Some("me"), "me", false).is_err());
    assert!(decide_feed_refresh(Some(&epic("")), "b", Some("a"), Some("me"), "me", false).is_err());
    assert!(decide_feed_refresh(Some(&e), "b", None, None, "me", false).is_err());
    assert!(decide_feed_refresh(Some(&e), "a", Some("a"), Some("me"), "me", false).is_err());
    assert!(decide_feed_refresh(Some(&e), "b", Some("a"), Some("you"), "me", false).is_err());
    assert_eq!(
        decide_feed_refresh(Some(&e), "b", Some("a"), Some("me"), "me", false),
        Ok(RefreshDecision::Write)
    );
    assert_eq!(
        decide_feed_refresh(Some(&e), "b", Some("a"), Some("me"), "me", true),
        Ok(RefreshDecision::Coalesced)
    );
}

#[test]
fn claim_and_report_rules() {
    let row = FeedRequest {
        id: 1,
        host: "a".into(),
        state: FEED_REQUESTED.into(),
        ..blank_feed_request()
    };
    assert!(validate_feed_request_claim(&row, "a").is_ok());
    assert!(validate_feed_request_claim(&row, "b").is_err());
    let running = claim_feed_request_row(row.clone(), "t");
    assert!(validate_feed_request_claim(&running, "a").is_ok());
    assert!(validate_feed_request_report(&row, "a").is_err());
    assert!(validate_feed_request_report(&running, "a").is_ok());
    let done = done_feed_request_row(running, "t2");
    assert_eq!(done.state, FEED_DONE);
    assert!(validate_feed_request_claim(&done, "a").is_err());
}
