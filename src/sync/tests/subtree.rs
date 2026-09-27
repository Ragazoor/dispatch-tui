//! `docs/specs/sync.allium`: `ASubEpicOfAFollowedEpicIsAskedForToo` and
//! `BoardConnection.covered_epics` — which epics a board widens its
//! subscription to as sub-epic rows arrive.

use crate::sync::subtree::SubtreeCover;

/// A sub-epic of a followed epic is covered the moment it arrives.
#[test]
fn a_child_of_a_followed_epic_is_covered() {
    let mut cover = SubtreeCover::new([1]);

    assert_eq!(cover.delivered(2, 1, &[(1, 0), (2, 1)]), vec![2]);
}

/// Coverage walks down one level per arrival, to any depth.
#[test]
fn coverage_reaches_any_depth() {
    let mut cover = SubtreeCover::new([1]);
    let mut known = vec![(1, 0)];
    for (id, parent) in [(2, 1), (3, 2), (4, 3), (5, 4)] {
        known.push((id, parent));
        assert_eq!(
            cover.delivered(id, parent, &known),
            vec![id],
            "depth of {id}"
        );
    }
}

/// An epic whose parent is not covered widens nothing — its own tree is not
/// something this board follows.
#[test]
fn an_epic_outside_every_followed_tree_is_not_covered() {
    let mut cover = SubtreeCover::new([1]);

    assert!(cover.delivered(8, 7, &[(7, 0), (8, 7)]).is_empty());
    assert!(cover.delivered(7, 0, &[(7, 0), (8, 7)]).is_empty());
}

/// A sub-epic delivered earlier by something else (own_creations) is covered
/// along with its parent, because the store will not deliver it again.
#[test]
fn a_descendant_already_held_is_covered_with_its_parent() {
    let mut cover = SubtreeCover::new([1]);
    // 3 (a grandchild) arrived first, through own_creations, while its parent
    // 2 was not yet held; nothing was covered then.
    assert!(cover.delivered(3, 2, &[(1, 0), (3, 2)]).is_empty());

    let covered = cover.delivered(2, 1, &[(1, 0), (2, 1), (3, 2)]);

    assert_eq!(covered, vec![2, 3]);
}

/// Delivering an already-covered epic again (an update) widens nothing twice.
#[test]
fn a_covered_epic_is_not_covered_twice() {
    let mut cover = SubtreeCover::new([1]);
    let known = [(1, 0), (2, 1)];
    assert_eq!(cover.delivered(2, 1, &known), vec![2]);

    assert!(cover.delivered(2, 1, &known).is_empty());
    assert!(
        cover.delivered(1, 0, &known).is_empty(),
        "a followed epic is covered from the start, by its own asks"
    );
}

/// A reparent under a covered epic is a delivery like any other.
#[test]
fn an_epic_moved_under_a_followed_one_is_covered() {
    let mut cover = SubtreeCover::new([1]);
    assert!(cover.delivered(5, 9, &[(1, 0), (5, 9), (9, 0)]).is_empty());

    assert_eq!(cover.delivered(5, 1, &[(1, 0), (5, 1), (9, 0)]), vec![5]);
}

// ---------------------------------------------------------------------------
// Following an epic mid-connection (task #4916)
// ---------------------------------------------------------------------------

/// A `Subscription` row that arrives covers its epic. This is how a board's
/// followed epics reach its ask: the first subscription carries the
/// subscription rows, and each one widens the ask from there — so an epic
/// followed on another machine, or a moment ago on this one, is asked for
/// without reconnecting.
#[test]
fn following_an_epic_covers_it() {
    let mut cover = SubtreeCover::new([]);

    assert_eq!(cover.follow(5, &[]), vec![5]);
    assert!(cover.covers(5));
}

/// Following an epic already covered asks for nothing new — the same row
/// delivered twice (an update, a reconnect's re-delivery) must not stack a
/// second identical subscription.
#[test]
fn following_a_covered_epic_widens_nothing() {
    let mut cover = SubtreeCover::new([5]);

    assert!(cover.follow(5, &[]).is_empty());
}

/// A followed epic's descendants already held (own_creations) are covered with
/// it, for the reason `a_descendant_already_held_is_covered_with_its_parent`
/// gives: the store will not deliver them a second time.
#[test]
fn following_an_epic_covers_its_descendants_already_held() {
    let mut cover = SubtreeCover::new([]);

    let mut newly = cover.follow(5, &[(5, 0), (6, 5), (7, 6), (9, 0)]);
    newly.sort_unstable();
    assert_eq!(newly, vec![5, 6, 7]);
    assert!(!cover.covers(9));
}
