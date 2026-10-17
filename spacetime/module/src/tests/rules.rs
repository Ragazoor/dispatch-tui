use super::*;
use std::collections::HashMap;

// Which pairs are legal is pinned pair by pair by the board's model tests
// (`src/models/tasks/model_tests.rs`), which read the board's `SubStatus`
// spellings through `validate_sub_status`; here, only the refusal's wording.

/// The wording reaches MCP callers unchanged from the service that used to
/// produce it.
#[test]
fn an_illegal_sub_status_is_refused_naming_both_values() {
    assert_eq!(
        validate_sub_status("backlog", "active").unwrap_err(),
        "sub_status 'active' is not valid for status 'backlog'"
    );
}

#[test]
fn an_unknown_sub_status_is_refused() {
    assert!(validate_sub_status("running", "bogus").is_err());
}

fn chain(edges: &[(i64, i64)]) -> impl Fn(i64) -> Option<i64> {
    let map: HashMap<i64, i64> = edges.iter().copied().collect();
    move |id| map.get(&id).copied()
}

const CYCLE: &str = "Setting this parent would create a cycle in the epic hierarchy";

#[test]
fn an_epic_cannot_be_its_own_parent() {
    assert_eq!(validate_epic_parent(1, 1, chain(&[])).unwrap_err(), CYCLE);
}

#[test]
fn an_epic_cannot_move_under_its_own_descendant() {
    // 3 -> 2 -> 1: moving 1 under 3 closes the loop.
    let parents = chain(&[(3, 2), (2, 1)]);
    assert_eq!(validate_epic_parent(1, 3, parents).unwrap_err(), CYCLE);
}

#[test]
fn an_epic_can_move_under_an_unrelated_epic() {
    assert!(validate_epic_parent(1, 3, chain(&[(3, 2)])).is_ok());
}

/// A stored `parent_epic_id` of 0 is a root, not epic 0.
#[test]
fn a_zero_parent_ends_the_walk() {
    assert!(validate_epic_parent(1, 3, chain(&[(3, 0), (0, 1)])).is_ok());
}

#[test]
fn a_walk_that_meets_an_existing_loop_terminates_and_refuses() {
    // 2 <-> 3 already loop; moving 1 under 2 must not spin forever, and must
    // not be waved through once the hop cap is spent: the answer is the cycle
    // error, not Ok.
    let parents = chain(&[(2, 3), (3, 2)]);
    assert_eq!(validate_epic_parent(1, 2, parents).unwrap_err(), CYCLE);
}

fn task_with(status: &str, sub_status: &str) -> Task {
    Task {
        id: 1,
        status: status.into(),
        sub_status: sub_status.into(),
        ..blank_task()
    }
}

#[test]
fn a_write_that_moves_the_sub_status_to_an_illegal_one_is_refused() {
    let prior = task_with("backlog", "none");
    let row = task_with("backlog", "approved");
    assert_eq!(
        check_sub_status_write(Some(&prior), &row).unwrap_err(),
        "sub_status 'approved' is not valid for status 'backlog'"
    );
}

#[test]
fn a_write_that_moves_the_sub_status_to_a_legal_one_passes() {
    let prior = task_with("running", "active");
    let row = task_with("running", "stale");
    assert!(check_sub_status_write(Some(&prior), &row).is_ok());
}

#[test]
fn a_write_that_leaves_the_sub_status_alone_is_not_judged() {
    // A row already holding an odd pair stays writable for its other fields.
    let prior = task_with("backlog", "approved");
    let row = Task {
        title: "renamed".into(),
        ..prior.clone()
    };
    assert!(check_sub_status_write(Some(&prior), &row).is_ok());
}

#[test]
fn an_insert_is_not_judged() {
    assert!(check_sub_status_write(None, &task_with("backlog", "approved")).is_ok());
}

#[test]
fn a_task_patch_naming_epic_id_is_refused() {
    let row = task_with("backlog", "none");
    let patch = TaskPatch {
        epic_id: Some(7),
        ..TaskPatch::default()
    };
    assert!(check_task_patch(&row, &patch)
        .unwrap_err()
        .contains("set_task_epic"));
}

fn subtree(edges: &[(i64, i64)]) -> impl Fn(i64) -> Vec<i64> {
    let edges = edges.to_vec();
    move |id| {
        edges
            .iter()
            .filter(|(parent, _)| *parent == id)
            .map(|(_, child)| *child)
            .collect()
    }
}

#[test]
fn a_subtree_within_the_depth_cap_is_not_too_deep() {
    assert!(!subtree_exceeds_depth(1, subtree(&[(1, 2), (2, 3)]), 2));
}

#[test]
fn a_subtree_below_the_depth_cap_is_too_deep() {
    assert!(subtree_exceeds_depth(1, subtree(&[(1, 2), (2, 3)]), 1));
}

#[test]
fn a_looping_subtree_terminates() {
    assert!(!subtree_exceeds_depth(1, subtree(&[(1, 2), (2, 1)]), 64));
}

#[test]
fn append_only_is_refused_with_grouping_or_a_feed_role() {
    assert!(validate_feed_append_only(true, true, "none").is_err());
    assert!(validate_feed_append_only(true, false, "reviews").is_err());
    assert!(validate_feed_append_only(true, false, "none").is_ok());
    assert!(validate_feed_append_only(false, true, "reviews").is_ok());
}

#[test]
fn the_append_only_refusal_says_what_to_do_instead() {
    let why = validate_feed_append_only(true, true, "none").unwrap_err();
    assert!(
        why.starts_with("feed_append_only cannot be combined"),
        "{why}"
    );
    assert!(why.contains("one flat append-only epic per repo"), "{why}");
}

#[test]
fn only_a_grouped_non_feed_epic_auto_groups() {
    assert!(can_auto_group(true, ""));
    assert!(!can_auto_group(true, "gh pr list"));
    assert!(!can_auto_group(false, ""));
}

#[test]
fn a_repo_group_is_named_after_the_final_path_component() {
    assert_eq!(repo_name_from_path("/home/u/dispatch"), "dispatch");
    assert_eq!(repo_name_from_path("/home/u/dispatch/"), "dispatch");
    assert_eq!(repo_name_from_path(""), "other");
    assert_eq!(repo_name_from_path("/"), "other");
}

fn sig(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// `feeds.allium: RouteSignals`. Total, first match wins.
#[test]
fn signals_route_by_first_match() {
    let cases: &[(&[&str], &str)] = &[
        (&["direct-request"], ROLE_MY_REVIEWS),
        (&["team-request"], ROLE_TEAM_REVIEWS),
        (&["reviewed"], ROLE_MY_REVIEWS),
        (&["commented"], ROLE_MY_REVIEWS),
        (&["author-bot"], ROLE_BOTS),
        // engagement beats the bot rule
        (&["reviewed", "author-bot"], ROLE_MY_REVIEWS),
        // but the user's own comment on their own PR is not engagement
        (&["commented", "author-me", "author-bot"], ROLE_BOTS),
        (&["team-request", "reviewed"], ROLE_MY_REVIEWS),
        (&[], ROLE_MY_REVIEWS),
        (&["org-review"], ROLE_MY_REVIEWS),
        (&["org-review", "author-bot"], ROLE_BOTS),
        (&["org-review", "author-bot", "reviewed"], ROLE_MY_REVIEWS),
    ];
    for (signals, role) in cases {
        assert_eq!(route_signals(&sig(signals)), *role, "{signals:?}");
    }
}

/// `feeds.allium: ExcludeFromReviews`.
#[test]
fn own_authored_and_settled_approved_prs_are_excluded() {
    let others = [
        "direct-request",
        "team-request",
        "reviewed",
        "commented",
        "author-bot",
        "org-review",
    ];
    assert!(!excluded_from_reviews(&[]));
    assert!(excluded_from_reviews(&sig(&["author-me"])));
    for s in others.iter().chain(["approved"].iter()) {
        assert!(
            !excluded_from_reviews(&sig(&[s])) || *s == "approved",
            "{s}"
        );
        assert!(
            excluded_from_reviews(&sig(&["author-me", s])),
            "author-me + {s}"
        );
    }
    // approved is excluded unless a request rescues it
    assert!(excluded_from_reviews(&sig(&["approved"])));
    for s in ["reviewed", "commented", "author-bot", "org-review"] {
        assert!(excluded_from_reviews(&sig(&["approved", s])), "{s}");
    }
    for s in ["direct-request", "team-request"] {
        assert!(!excluded_from_reviews(&sig(&["approved", s])), "{s}");
        assert!(
            !excluded_from_reviews(&sig(&["approved", "reviewed", s])),
            "{s}"
        );
    }
    // an own-authored PR is never rescued
    assert!(excluded_from_reviews(&sig(&[
        "author-me",
        "approved",
        "direct-request"
    ])));
}

#[test]
fn a_feed_item_url_names_its_repo_group() {
    assert_eq!(
        repo_name_from_url("https://github.com/org/my-repo/pull/42"),
        "my-repo"
    );
    assert_eq!(
        repo_name_from_url("https://github.com/org/my-repo"),
        "my-repo"
    );
    assert_eq!(repo_name_from_url(""), "other");
    assert_eq!(repo_name_from_url("https://gitlab.com/org/repo"), "other");
    assert_eq!(extract_github_repo("https://github.com/org"), None);
}

#[test]
fn the_rules_version_is_positive() {
    // 0 is what a module that predates the gate reads as.
    assert!(RULES_VERSION > 0);
}
