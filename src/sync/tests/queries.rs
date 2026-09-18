//! What the board actually asks the store for.
//!
//! `docs/specs/sync.allium`:
//! `SubscriptionsCoverOnlyTheOwnBoardAndItsEpics`. The invariant is about the
//! ASK, because that is the half this system controls — a store that honours
//! the request sends nothing else, and asking for too much is the failure that
//! would be ours.

use crate::sync::sdk_connector::subscription_queries;
use crate::sync::SubscriptionRequest;

/// A well-formed identity: the store issues hex and nothing else is one.
const ID: &str = "c200e1f4bcae4a1b9f0e7d2a3c5b8e60";
const OTHER: &str = "ffee0011223344556677889900aabbcc";

fn queries(epics: Vec<i64>) -> Vec<String> {
    subscription_queries(&SubscriptionRequest::new(ID, epics)).unwrap()
}

#[test]
fn a_board_with_no_epics_asks_only_for_its_own() {
    let queries = queries(vec![]);

    assert!(queries.iter().any(|q| q.contains("FROM hosts")));
    assert!(queries
        .iter()
        .any(|q| q.contains("FROM tasks") && q.contains(&format!("owner = '{ID}'"))));
    assert!(
        !queries.iter().any(|q| q.contains("epic_id")),
        "nothing is followed, so no epic is asked for"
    );
}

#[test]
fn each_followed_epic_brings_its_own_tasks_and_the_epic_itself() {
    let queries = queries(vec![7, 9]);

    for epic in [7, 9] {
        assert!(
            queries
                .iter()
                .any(|q| q.contains("FROM epics") && q.contains(&format!("id = {epic}"))),
            "epic {epic} itself must arrive, or its tasks have no card to sit under"
        );
        assert!(queries
            .iter()
            .any(|q| q.contains("FROM tasks") && q.contains(&format!("epic_id = {epic}"))));
    }
}

/// The containment claim. Nothing in the ask can bring another person's user
/// board, whatever epics are followed.
#[test]
fn nothing_asked_for_can_reach_another_persons_user_board() {
    let queries = queries(vec![7, 9, 4096]);

    for query in &queries {
        assert!(
            !query.contains(OTHER),
            "a query named an identity that is not this board's: {query}"
        );
    }
    let owner_filters = queries.iter().filter(|q| q.contains("owner =")).count();
    assert_eq!(
        owner_filters, 1,
        "exactly one query selects by owner, and it selects by this one"
    );
    for query in queries.iter().filter(|q| q.contains("owner =")) {
        assert!(query.contains(&format!("owner = '{ID}'")), "{query}");
    }
}

/// The identity is validated against a closed alphabet rather than escaped.
///
/// A quoting rule is a thing to get subtly wrong; hex is not. The value reaches
/// this code from the store, so this is not the last line of defence — but it
/// is the one that would matter if a stored identity were ever tampered with on
/// disk, which is an ordinary text file in the settings table.
#[test]
fn an_identity_outside_the_hex_alphabet_is_refused() {
    for bad in [
        "",
        "not-hex",
        "c200'; DROP TABLE tasks; --",
        "c200e1f4 OR 1=1",
        "c200e1f4'",
    ] {
        let result = subscription_queries(&SubscriptionRequest::new(bad, vec![7]));
        assert!(
            result.is_err(),
            "{bad:?} should not have produced a subscription"
        );
    }
}

/// A board that follows nothing still asks for something. An empty ask would
/// subscribe to nothing at all, which is indistinguishable from a store with
/// no rows.
#[test]
fn the_ask_is_never_empty() {
    assert!(!queries(vec![]).is_empty());
}
