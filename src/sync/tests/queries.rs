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

fn queries() -> Vec<String> {
    subscription_queries(&SubscriptionRequest::new(ID, HOST)).unwrap()
}

/// The private tables a per-caller view serves. A board that asked for one
/// directly would be refused by the store.
const PRIVATE_TABLES: [&str; 22] = [
    "tasks",
    "epics",
    "work_packages",
    "work_package_links",
    "agents",
    "dispatch_requests",
    "session_requests",
    "unseen_tasks",
    "poll_owners",
    "learnings",
    "learning_retrievals",
    "learning_verdicts",
    "settings",
    "workspaces",
    "memberships",
    "usage_events",
    "task_subagents",
    "task_watchers",
    "todos",
    "task_shells",
    "retired_feed_items",
    "filter_presets",
];

fn asks_for(queries: &[String], source: &str) -> bool {
    queries
        .iter()
        .any(|q| q.split_whitespace().nth(3) == Some(source))
}

/// **No private table is asked for by name.** `sync.allium:
/// SubscribeOnceIdentityIsSettled`: the board reads through the views, and a
/// subscription to a private table is refused by the store, which would fail
/// the whole ask.
#[test]
fn no_private_table_is_asked_for_directly() {
    let queries = queries();
    for table in PRIVATE_TABLES {
        assert!(
            !asks_for(&queries, table),
            "`{table}` is private; the board reads it through a view: {queries:?}"
        );
    }
    for table in ["dispatch_request_timeouts", "session_request_timeouts"] {
        assert!(!asks_for(&queries, table), "{table} is the module's own");
    }
}

/// **The ask covers every source the board draws from.** The board has no
/// second copy to fall back on: a source nothing subscribes to renders empty,
/// in a way that looks exactly like having no rows.
#[test]
fn every_source_the_board_reads_is_asked_for() {
    let queries = queries();
    for source in [
        "hosts",
        "user_profiles",
        "repo_paths",
        "repo_base_branches",
        "own_tasks",
        "own_epics",
        "covered_tasks",
        "covered_epics",
        "own_packages",
        "visible_work_package_links",
        "visible_poll_owners",
        "visible_workspaces",
        "visible_memberships",
        "unseen_items",
        "own_unseen_tasks",
        "own_bookmarks",
        "own_subscriptions",
        "own_settings",
        "visible_learnings",
        "visible_learning_retrievals",
        "visible_learning_verdicts",
        "visible_dispatch_requests",
        "visible_session_requests",
        "visible_agents",
    ] {
        assert!(
            asks_for(&queries, source),
            "nothing asks for `{source}`, so the board would draw it empty: {queries:?}"
        );
    }
}

/// **The board no longer walks the epic tree.** `covered_epics` is walked by
/// the store, at any depth, so no ask names an epic by id or by parent.
#[test]
fn no_ask_names_an_epic_by_id() {
    for query in queries() {
        assert!(
            !query.contains("epic_id =")
                && !query.contains("parent_epic_id")
                && !query.contains("id = "),
            "the store walks the tree; an ask by epic would be the old walk: {query}"
        );
    }
}

/// The containment claim. Nothing in the ask can bring another person's user
/// board: the views answer for the connection's own identity, and no
/// ask names a person at all.
#[test]
fn nothing_asked_for_can_reach_another_persons_user_board() {
    let queries = queries();

    for query in &queries {
        assert!(
            !query.contains(OTHER),
            "a query named an identity that is not this board's: {query}"
        );
        assert!(
            !query.contains("owner =") && !query.contains("created_by ="),
            "a view ask needs no identity filter: {query}"
        );
    }
    let by_person: Vec<&String> = queries.iter().filter(|q| q.contains(ID)).collect();
    assert!(by_person.is_empty(), "{by_person:?}");
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
        let result = subscription_queries(&SubscriptionRequest::new(bad, HOST));
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
        let result = subscription_queries(&SubscriptionRequest::new(ID, bad));
        assert!(
            result.is_err(),
            "{bad:?} should not have produced a subscription"
        );
    }
}

/// An empty ask would subscribe to nothing at all, which is indistinguishable
/// from a store with no rows.
#[test]
fn the_ask_is_never_empty() {
    assert!(!queries().is_empty());
}

/// **Test 1 of task #4913**: a setting written by one host is not visible to
/// another host's board.
///
/// The view serves the settings of every host this person owns, so the ask
/// narrows to this host's own id: a store honouring the request never sends
/// another machine's settings here.
#[test]
fn settings_are_scoped_to_this_hosts_own_id() {
    let queries = queries();

    let asked: Vec<&String> = queries
        .iter()
        .filter(|q| q.contains("FROM own_settings"))
        .collect();
    assert_eq!(asked.len(), 1, "expected exactly one ask for settings");
    assert!(
        asked[0].contains(&format!("host = '{HOST}'")),
        "settings must be filtered to this host's own id: {}",
        asked[0]
    );
    assert!(
        !asked[0].contains(OTHER_HOST),
        "settings' ask must never name another host: {}",
        asked[0]
    );
}

/// The repo lists are asked for unfiltered, and that is the decision rather
/// than an oversight: a repo path and its base-branch history describe the
/// WORK, not the person, and carry no private content.
#[test]
fn the_repo_lists_are_shared_by_design() {
    let queries = queries();

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

/// `sync.allium: SubscribeToThisHostsDispatchRequests` and
/// `SubscribeToThisHostsSessionRequests`. The views serve every request on a
/// held task, and a request is provisioned only by the machine that made it,
/// so each ask narrows to THIS machine, in every state: a row leaving a
/// state-filtered query would arrive as a delete and its outcome would be lost.
#[test]
fn requests_are_asked_for_by_this_hosts_own_id_in_every_state() {
    let queries = queries();

    for view in ["visible_dispatch_requests", "visible_session_requests"] {
        let asked: Vec<&String> = queries
            .iter()
            .filter(|q| q.contains(&format!("FROM {view}")))
            .collect();
        assert_eq!(asked.len(), 1, "expected one ask for {view}: {queries:?}");
        assert!(
            asked[0].contains(&format!("host = '{HOST}'")),
            "{view} must be filtered to this host's own id: {}",
            asked[0]
        );
        assert!(
            !asked[0].contains(OTHER_HOST) && !asked[0].contains("state"),
            "the ask names only this host and filters on no state: {}",
            asked[0]
        );
    }
}

/// The name ledger is not asked for: only the module's picker reads it, and it
/// stays public for that reason alone.
#[test]
fn the_agent_name_ledger_is_not_asked_for() {
    assert!(!queries()
        .iter()
        .any(|q| q.contains("handed_out_agent_names")));
}
