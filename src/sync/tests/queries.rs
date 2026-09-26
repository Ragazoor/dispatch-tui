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

/// A well-formed host id: `uuid::Uuid::new_v4().to_string()`'s shape.
const HOST: &str = "c200e1f4-bcae-4a1b-9f0e-7d2a3c5b8e60";
const OTHER_HOST: &str = "ffee0011-2233-4455-6677-889900aabbcc";

fn queries(epics: Vec<i64>) -> Vec<String> {
    subscription_queries(&SubscriptionRequest::new(ID, epics, HOST)).unwrap()
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

/// `sync.allium: SubscribeOnceIdentityIsSettled`'s `own_creations` — asked for
/// unconditionally, unlike `epics` above, because the whole point is finding a
/// task or epic this identity just made regardless of which epic it landed in
/// or whether anyone follows it yet. See `src/sync/sdk_connector.rs::generated_id`
/// for why the read-back needs this.
#[test]
fn own_creations_are_asked_for_even_with_nothing_followed() {
    let queries = queries(vec![]);

    assert!(
        queries
            .iter()
            .any(|q| q.contains("FROM tasks") && q.contains(&format!("created_by = '{ID}'"))),
        "a task this identity created must be findable regardless of its epic: {queries:?}"
    );
    assert!(
        queries
            .iter()
            .any(|q| q.contains("FROM epics") && q.contains(&format!("created_by = '{ID}'"))),
        "a brand-new epic must be findable before anyone follows it: {queries:?}"
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
        owner_filters, 2,
        "two queries select by owner — the user board and the checklist — and both select by this one"
    );
    for query in queries.iter().filter(|q| q.contains("owner =")) {
        assert!(query.contains(&format!("owner = '{ID}'")), "{query}");
    }

    let creator_filters = queries
        .iter()
        .filter(|q| q.contains("created_by ="))
        .count();
    assert_eq!(
        creator_filters, 2,
        "two queries select by created_by — own_creations for tasks and for epics — and both select by this one"
    );
    for query in queries.iter().filter(|q| q.contains("created_by =")) {
        assert!(query.contains(&format!("created_by = '{ID}'")), "{query}");
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
        let result = subscription_queries(&SubscriptionRequest::new(bad, vec![7], HOST));
        assert!(
            result.is_err(),
            "{bad:?} should not have produced a subscription"
        );
    }
}

/// A host id outside its own alphabet is refused the same way an identity is.
#[test]
fn a_host_id_outside_its_alphabet_is_refused() {
    for bad in ["", "not valid", "c200'; DROP TABLE settings; --"] {
        let result = subscription_queries(&SubscriptionRequest::new(ID, vec![7], bad));
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

/// **The ask covers every shared table the board draws from.**
///
/// The board has no second copy to fall back on: a table nothing subscribes to
/// is a table that renders empty, in a way that looks exactly like having no
/// rows. So this is asserted per table rather than left to whoever notices the
/// gap on screen.
///
/// `task_watchers`, `task_shells` and `task_subagents` are deliberately absent.
/// Nothing on the read surface reaches them — they are written and consumed by
/// the mutation path, which is Phase 6 — and asking for rows nothing renders is
/// the "asking for too much" this file's header warns about.
#[test]
fn every_table_the_board_reads_is_asked_for() {
    let queries = queries(vec![7]);

    for table in [
        "hosts",
        "subscriptions",
        "tasks",
        "epics",
        "todos",
        "repo_paths",
        "repo_base_branches",
        "settings",
        "filter_presets",
    ] {
        assert!(
            queries.iter().any(|q| q.contains(&format!("FROM {table}"))),
            "nothing asks for `{table}`, so the board would draw it empty"
        );
    }
}

/// **Test 1 of task #4913**: a setting written by one host is not visible to
/// another host's board.
///
/// Asserted at the ask, per this file's own header: the query this board sends
/// names only its own host id, so a store honouring the request never sends a
/// colleague's settings or filter presets here in the first place.
#[test]
fn settings_and_filter_presets_are_scoped_to_this_hosts_own_id() {
    let queries = subscription_queries(&SubscriptionRequest::new(ID, vec![], HOST)).unwrap();

    for table in ["settings", "filter_presets"] {
        let asked: Vec<&String> = queries
            .iter()
            .filter(|q| q.contains(&format!("FROM {table}")))
            .collect();
        assert_eq!(asked.len(), 1, "expected exactly one ask for {table}");
        assert!(
            asked[0].contains(&format!("host = '{HOST}'")),
            "{table} must be filtered to this host's own id: {}",
            asked[0]
        );
        assert!(
            !asked[0].contains(OTHER_HOST),
            "{table}'s ask must never name another host: {}",
            asked[0]
        );
    }
}

/// A checklist is personal, so the ask for it names one person.
///
/// `todo.allium: Todo.owner` exists for exactly this query. Without the filter
/// the only query a shared store can answer is "every todo", which puts a
/// colleague's checklist on this board.
#[test]
fn the_checklist_asked_for_is_this_persons_only() {
    let queries = queries(vec![7]);

    let todos: Vec<&String> = queries
        .iter()
        .filter(|q| q.contains("FROM todos"))
        .collect();
    assert_eq!(todos.len(), 1);
    assert!(
        todos[0].contains(&format!("owner = '{ID}'")),
        "a todo query with no owner filter is every person's checklist: {}",
        todos[0]
    );
}

/// The repo lists are asked for unfiltered, and that is the decision rather
/// than an oversight.
///
/// Neither table has an owner column to filter on, and neither wants one: a
/// repo path and its base-branch history describe the WORK, not the person, and
/// a colleague adding a repo to the shared board is a colleague telling
/// everybody where the code lives. They carry no private content — a path and a
/// branch name — so the containment claim is not weakened by them.
#[test]
fn the_repo_lists_are_shared_by_design() {
    let queries = queries(vec![]);

    for table in ["repo_paths", "repo_base_branches"] {
        let asked: Vec<&String> = queries
            .iter()
            .filter(|q| q.contains(&format!("FROM {table}")))
            .collect();
        assert_eq!(asked.len(), 1);
        assert!(
            !asked[0].contains("WHERE"),
            "{table} is deliberately unfiltered: {}",
            asked[0]
        );
    }
}
