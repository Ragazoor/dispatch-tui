//! The feed-request rules (`feed_request.rs`) as pure functions over rows.
//! The table writes are pinned by `spacetime/module-tests/tests/feed_requests.rs`.

use super::*;

fn task(host: &str, worktree: &str) -> Task {
    Task {
        id: 7,
        host: host.into(),
        worktree: worktree.into(),
        repo_path: "/repo".into(),
        ..blank_task()
    }
}

#[test]
fn teardown_is_owed_only_for_another_hosts_held_task() {
    assert!(teardown_request_owed(&task("b", "/wt"), "", "a"));
    assert!(teardown_request_owed(&task("b", ""), "win", "a"));
    assert!(!teardown_request_owed(&task("a", "/wt"), "win", "a"));
    assert!(!teardown_request_owed(&task("", "/wt"), "win", "a"));
    assert!(!teardown_request_owed(&task("b", ""), "", "a"));
}

#[test]
fn the_teardown_row_snapshots_the_task() {
    let row = new_teardown_request(&task("b", "/wt"), "win", "alice", "2026-01-01 00:00:00.000");
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
    let cutoff = "2026-01-01 00:10:00.000";
    assert!(validate_feed_request_claim(&row, "a", cutoff).is_ok());
    assert!(validate_feed_request_claim(&row, "b", cutoff).is_err());
    let running = claim_feed_request_row(row.clone(), "2026-01-01 00:00:00.000");
    // Claimed before the cutoff: left by a dead board. At or after: still served.
    assert!(validate_feed_request_claim(&running, "a", cutoff).is_ok());
    assert!(validate_feed_request_claim(&running, "a", "2026-01-01 00:00:00.000").is_err());
    assert!(validate_feed_request_report(&row, "a").is_err());
    assert!(validate_feed_request_report(&running, "a").is_ok());
    let done = done_feed_request_row(running, "t2");
    assert_eq!(done.state, FEED_DONE);
    assert!(validate_feed_request_claim(&done, "a", cutoff).is_err());
}

#[test]
fn an_ingest_is_refused_only_for_a_host_other_than_the_claim_holder() {
    assert!(check_ingest_claim(None, "a", 3).is_ok());
    assert!(check_ingest_claim(Some("a"), "a", 3).is_ok());
    assert!(check_ingest_claim(Some("b"), "a", 3).is_err());
}

fn owner_of(host: &str) -> Option<String> {
    match host {
        "mine" | "mine2" => Some("me".into()),
        "theirs" => Some("you".into()),
        _ => None,
    }
}

fn aimed(row_host: &str, patch: TaskPatch) -> Result<(), String> {
    let row = Task {
        id: 7,
        host: row_host.into(),
        repo_path: "/repo".into(),
        worktree: "/wt".into(),
        ..blank_task()
    };
    check_task_aim(&row, &patch, "me", owner_of)
}

#[test]
fn a_patch_that_changes_host_repo_or_worktree_is_refused_while_another_persons_host_holds_the_task()
{
    let worktree = |w: &str| TaskPatch {
        worktree: Some(w.into()),
        ..TaskPatch::default()
    };
    assert!(aimed("theirs", worktree("/x")).is_err());
    // A host nobody registered owns no teardown to aim.
    assert!(aimed("unknown", worktree("/x")).is_ok());
    assert!(aimed("mine", worktree("/x")).is_ok());
    assert!(aimed("", worktree("/x")).is_ok());
    // Repeating the current value is no change.
    assert!(aimed("theirs", worktree("/wt")).is_ok());
    assert!(aimed("theirs", TaskPatch::default()).is_ok());
}

#[test]
fn a_task_cannot_be_moved_onto_another_persons_host() {
    let to = |h: &str| TaskPatch {
        host: Some(h.into()),
        ..TaskPatch::default()
    };
    assert!(aimed("", to("mine")).is_ok());
    assert!(aimed("", to("theirs")).is_err());
    assert!(aimed("", to("nobody-registered")).is_ok());
    assert!(aimed("mine", to("mine2")).is_ok());
    assert!(aimed("mine", to("theirs")).is_err());
    assert!(aimed("mine", to("")).is_ok());
}

#[test]
fn naming_a_window_on_another_persons_host_is_refused_and_clearing_one_is_not() {
    let window = |w: &str| TaskPatch {
        tmux_window: Some(w.into()),
        ..TaskPatch::default()
    };
    assert!(aimed("theirs", window("dispatch:x")).is_err());
    assert!(aimed("theirs", window("")).is_ok());
    assert!(aimed("mine", window("dispatch:x")).is_ok());
}
