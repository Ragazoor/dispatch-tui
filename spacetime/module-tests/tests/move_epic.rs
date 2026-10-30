//! `workspaces.allium: MoveEpicToWorkspace` (D13), run against the in-memory
//! store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// `root` is super-admin; `alice` is admin of A and B; `bob` admin of A only;
/// `carol` member of A; `dave` admin of B only. Returns (store, A, B).
fn world() -> (Store, i64, i64) {
    let store = Store::bare();
    for who in ["root", "alice", "bob", "carol", "dave"] {
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
    let (a, b) = (ids[0], ids[1]);
    for (ws, who, role) in [
        (a, "alice", "admin"),
        (b, "alice", "admin"),
        (a, "bob", "admin"),
        (a, "carol", "member"),
        (b, "dave", "admin"),
    ] {
        store
            .call("root", |ctx| {
                add_member(ctx, ws, identity_of(who), role.into())
            })
            .expect("member");
    }
    (store, a, b)
}

fn make_epic(store: &Store, who: &str, ws: i64, parent: i64, visibility: &str) -> i64 {
    let row = Epic {
        title: "e".into(),
        workspace_id: ws,
        parent_epic_id: parent,
        visibility: visibility.into(),
        ..blank_epic()
    };
    store
        .call(who, |ctx| create_epic(ctx, row))
        .expect("create_epic");
    store.db().epics().iter().map(|e| e.id).max().expect("epic")
}

fn mv(store: &Store, who: &str, epic: i64, to: i64) -> Result<(), String> {
    store.call(who, |ctx| move_epic_to_workspace(ctx, epic, to))
}

#[test]
fn an_admin_of_both_moves_a_root_epic_and_its_tree() {
    let (store, a, b) = world();
    let root = make_epic(&store, "alice", a, 0, "public");
    let sub = make_epic(&store, "alice", a, root, "public");
    mv(&store, "alice", root, b).expect("move");
    assert_eq!(store.epic(root).workspace_id, b);
    assert_eq!(store.epic(sub).workspace_id, b);
}

#[test]
fn an_admin_of_only_one_side_is_refused() {
    let (store, a, b) = world();
    let root = make_epic(&store, "alice", a, 0, "public");
    assert!(mv(&store, "bob", root, b).is_err(), "admin of source only");
    let in_b = make_epic(&store, "alice", b, 0, "public");
    assert!(mv(&store, "dave", in_b, a).is_err(), "admin of dest only");
    assert!(mv(&store, "carol", root, b).is_err(), "member");
    assert_eq!(store.epic(root).workspace_id, a);
}

#[test]
fn a_super_admin_moves_without_belonging_to_either_workspace() {
    let (store, a, b) = world();
    let root = make_epic(&store, "alice", a, 0, "public");
    mv(&store, "root", root, b).expect("move");
    assert_eq!(store.epic(root).workspace_id, b);
}

#[test]
fn only_a_super_admin_moves_an_epic_out_of_a_workspace_that_does_not_exist() {
    let (store, _, b) = world();
    store.db().epics().insert(Epic {
        title: "stranded".into(),
        workspace_id: 0,
        ..blank_epic()
    });
    let stranded = store.db().epics().iter().map(|e| e.id).max().expect("epic");
    assert!(mv(&store, "alice", stranded, b).is_err());
    assert!(mv(&store, "dave", stranded, b).is_err());
    mv(&store, "root", stranded, b).expect("rescue");
    assert_eq!(store.epic(stranded).workspace_id, b);
}

#[test]
fn a_sub_epic_the_same_workspace_and_a_missing_destination_are_refused() {
    let (store, a, _) = world();
    let root = make_epic(&store, "alice", a, 0, "public");
    let sub = make_epic(&store, "alice", a, root, "public");
    assert!(mv(&store, "root", sub, a).is_err(), "sub-epic");
    assert!(mv(&store, "root", root, a).is_err(), "same workspace");
    assert!(mv(&store, "root", root, 9999).is_err(), "missing destination");
    assert!(mv(&store, "root", root, 0).is_err(), "workspace 0");
}

#[test]
fn a_private_epic_moves_only_at_its_creators_hand() {
    let (store, a, b) = world();
    let private = make_epic(&store, "alice", a, 0, "private");
    assert!(mv(&store, "root", private, b).is_err(), "super-admin");
    assert!(mv(&store, "bob", private, b).is_err(), "other admin");
    mv(&store, "alice", private, b).expect("creator, admin of both");
    assert_eq!(store.epic(private).workspace_id, b);
}

#[test]
fn a_refusal_for_an_unseen_private_epic_matches_one_for_a_missing_epic() {
    let (store, a, b) = world();
    let private = make_epic(&store, "alice", a, 0, "private");
    let unseen = mv(&store, "root", private, b).expect_err("refused");
    let missing = mv(&store, "root", 9999, b).expect_err("refused");
    assert_eq!(
        unseen.replace(&private.to_string(), "N"),
        missing.replace("9999", "N")
    );
}

#[test]
fn learnings_of_the_tree_follow_and_repo_learnings_stay() {
    let (store, a, b) = world();
    let root = make_epic(&store, "alice", a, 0, "public");
    let sub = make_epic(&store, "alice", a, root, "public");
    let task = store.create("alice", epic_task("t", sub, "backlog"));
    let learning = |scope: &str, scope_ref: Option<String>, source: Option<i64>| Learning {
        id: 0,
        kind: "pitfall".into(),
        summary: "s".into(),
        detail: None,
        scope: scope.into(),
        scope_ref,
        tags: "[]".into(),
        status: "approved".into(),
        source_task_id: source,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: "2026-10-10 12:00:00".into(),
        updated_at: "2026-10-10 12:00:00".into(),
        embedding: None,
        workspace_id: 0,
        author: String::new(),
    };
    for row in [
        learning("epic", Some(sub.to_string()), None),
        learning("task", Some(task.to_string()), None),
        learning("repo", Some("r".into()), Some(task)),
    ] {
        store
            .call("alice", |ctx| create_learning(ctx, row))
            .expect("learning");
    }
    mv(&store, "alice", root, b).expect("move");
    let ws = |scope: &str| {
        store
            .db()
            .learnings()
            .iter()
            .find(|l| l.scope == scope)
            .expect("learning")
            .workspace_id
    };
    assert_eq!(ws("epic"), b);
    assert_eq!(ws("task"), b);
    assert_eq!(ws("repo"), a);
}

#[test]
fn package_links_into_the_tree_are_dropped_and_other_links_stay() {
    let (store, a, b) = world();
    let root = make_epic(&store, "alice", a, 0, "public");
    let other = make_epic(&store, "alice", a, 0, "public");
    let task = store.create("alice", epic_task("t", root, "backlog"));
    store
        .call("alice", |ctx| {
            create_work_package(
                ctx,
                WorkPackage {
                    title: "p".into(),
                    workspace_id: a,
                    visibility: "public".into(),
                    created_by: identity_of("alice"),
                    ..blank_work_package()
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
    for (kind, id) in [("epic", root), ("task", task), ("epic", other)] {
        store
            .call("alice", |ctx| {
                link_work_package_item(ctx, package, kind.into(), id)
            })
            .expect("link");
    }
    mv(&store, "alice", root, b).expect("move");
    let left: Vec<i64> = store
        .db()
        .work_package_links()
        .iter()
        .map(|l| l.item_id)
        .collect();
    assert_eq!(left, vec![other]);
    assert_eq!(store.db().work_packages().id().find(package).expect("p").workspace_id, a);
}

#[test]
fn follows_of_people_who_are_not_in_the_destination_are_dropped() {
    let (store, a, b) = world();
    let root = make_epic(&store, "alice", a, 0, "public");
    for who in ["alice", "carol"] {
        store
            .call(who, |ctx| {
                subscribe_to_epic(ctx, identity_of(who), root)
            })
            .expect("follow");
    }
    mv(&store, "alice", root, b).expect("move");
    let who: Vec<String> = store
        .db()
        .subscriptions()
        .iter()
        .map(|s| s.subscriber)
        .collect();
    assert_eq!(who, vec![identity_of("alice")]);
}

#[test]
fn a_private_sub_epic_of_someone_else_blocks_the_move_of_its_root() {
    let (store, a, b) = world();
    let root = make_epic(&store, "alice", a, 0, "public");
    let secret = make_epic(&store, "carol", a, root, "private");
    assert!(mv(&store, "alice", root, b).is_err(), "admin of both");
    assert!(mv(&store, "root", root, b).is_err(), "super-admin");
    assert_eq!(store.epic(root).workspace_id, a);
    assert_eq!(store.epic(secret).workspace_id, a);
}
