//! The decisions the row callbacks make, run without a connection.
//!
//! The callbacks themselves need the SDK's event context, which only a live
//! store produces. What they decide — which queries a row widens the ask by —
//! is a pure function of the cover and the rows already held, tested here.

use super::wiring::{follow_queries, moved_under_new_parent, widen_queries, Subtree};

#[test]
fn an_epic_under_an_uncovered_parent_widens_nothing() {
    let mut subtree = Subtree::default();
    assert!(widen_queries(&mut subtree, 5, 1, &[(5, 1)]).is_empty());
}

#[test]
fn an_epic_under_a_covered_parent_asks_for_its_subtree() {
    let mut subtree = Subtree::default();
    subtree.cover.follow(1, &[]);
    let queries = widen_queries(&mut subtree, 5, 1, &[(5, 1)]);
    assert_eq!(
        queries,
        vec![
            "SELECT * FROM tasks WHERE epic_id = 5".to_string(),
            "SELECT * FROM epics WHERE parent_epic_id = 5".to_string(),
        ]
    );
}

#[test]
fn a_covered_epic_is_not_asked_for_twice() {
    let mut subtree = Subtree::default();
    subtree.cover.follow(1, &[]);
    widen_queries(&mut subtree, 5, 1, &[(5, 1)]);
    assert!(widen_queries(&mut subtree, 5, 1, &[(5, 1)]).is_empty());
}

#[test]
fn a_followed_epic_asks_for_itself_and_its_tree() {
    let mut subtree = Subtree::default();
    let queries = follow_queries(&mut subtree, 9, &[]);
    assert_eq!(queries[0], "SELECT * FROM epics WHERE id = 9");
    assert!(queries.contains(&"SELECT * FROM tasks WHERE epic_id = 9".to_string()));
}

#[test]
fn a_followed_epic_already_covered_asks_for_nothing() {
    let mut subtree = Subtree::default();
    follow_queries(&mut subtree, 9, &[]);
    assert!(follow_queries(&mut subtree, 9, &[]).is_empty());
}

#[test]
fn following_pulls_in_descendants_already_held() {
    let mut subtree = Subtree::default();
    let queries = follow_queries(&mut subtree, 9, &[(10, 9)]);
    assert!(queries.contains(&"SELECT * FROM tasks WHERE epic_id = 10".to_string()));
}

#[test]
fn only_a_change_of_parent_widens_on_update() {
    assert!(moved_under_new_parent(1, 2));
    assert!(!moved_under_new_parent(2, 2));
}
