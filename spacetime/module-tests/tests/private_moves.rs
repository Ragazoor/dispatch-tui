//! One rule for moving work under or out of a private epic, and the repo-group
//! and package rules around it (`workspaces.allium`: `ReParentingKeepsTheHolder`,
//! `RepoGroupsStayPublic`, `SuperAdminDeletesUnseenItem`;
//! `epics.allium`: `RegroupEpic`, `FlattenEpic`), run against the in-memory
//! store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// `root` is super-admin; `alice` and `bob` are members of workspace A,
/// `carol` of workspace B. Returns (store, A, B).
fn world() -> (Store, i64, i64) {
    let store = Store::bare();
    for who in ["root", "alice", "bob", "carol"] {
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
    for (ws, who) in [(ids[0], "alice"), (ids[0], "bob"), (ids[1], "carol")] {
        store
            .call("root", |ctx| {
                add_member(ctx, ws, identity_of(who), "member".into())
            })
            .expect("member");
    }
    (store, ids[0], ids[1])
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

fn reparent(store: &Store, who: &str, epic: i64, parent: i64) -> Result<(), String> {
    store.call(who, |ctx| {
        patch_epic(
            ctx,
            epic,
            EpicPatch {
                parent_epic_id: Some(parent),
                ..EpicPatch::default()
            },
        )
    })
}

fn move_task(store: &Store, who: &str, task: i64, epic: i64) -> Result<(), String> {
    store.call(who, |ctx| set_task_epic(ctx, task, epic, String::new()))
}

fn remove_membership(store: &Store, ws: i64, who: &str) {
    let id = store
        .db()
        .memberships()
        .iter()
        .find(|m| m.workspace_id == ws && m.user == identity_of(who))
        .expect("membership")
        .id;
    store
        .call("root", |ctx| remove_member(ctx, id))
        .expect("remove member");
}

// -- V2: the move rule --------------------------------------------------------

#[test]
fn moving_someone_elses_epic_under_your_private_epic_is_refused() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private");
    let bobs = make_epic(&store, "bob", a, 0, "public");
    assert!(reparent(&store, "alice", bobs, private).is_err());
    assert_eq!(store.epic(bobs).parent_epic_id, 0);
}

#[test]
fn moving_your_own_epic_under_your_private_epic_is_allowed() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private");
    let hers = make_epic(&store, "alice", a, 0, "public");
    reparent(&store, "alice", hers, private).expect("own epic");
    assert_eq!(store.epic(hers).parent_epic_id, private);
}

#[test]
fn moving_an_epic_out_of_a_private_epic_to_the_root_needs_the_same_authority() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private");
    let sub = make_epic(&store, "alice", a, private, "public");
    // Alice created it: she may expose it.
    reparent(&store, "alice", sub, 0).expect("creator exposes own epic");
    assert_eq!(store.epic(sub).parent_epic_id, 0);
}

#[test]
fn a_move_that_keeps_the_holder_needs_no_special_authority() {
    let (store, a, _) = world();
    let one = make_epic(&store, "alice", a, 0, "public");
    let two = make_epic(&store, "alice", a, 0, "public");
    let bobs = make_epic(&store, "bob", a, one, "public");
    reparent(&store, "alice", bobs, two).expect("public to public");
    assert_eq!(store.epic(bobs).parent_epic_id, two);
}

#[test]
fn moving_a_visible_task_into_your_private_epic_is_refused() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private");
    let shared = make_epic(&store, "bob", a, 0, "public");
    let task = store.create("bob", epic_task("t", shared, "backlog"));
    assert!(move_task(&store, "alice", task, private).is_err());
    assert_eq!(store.task(task).epic_id, shared);
}

#[test]
fn a_workspace_admin_may_move_a_task_under_a_private_epic_they_see() {
    let (store, a, _) = world();
    let membership = store
        .db()
        .memberships()
        .iter()
        .find(|m| m.workspace_id == a && m.user == identity_of("alice"))
        .expect("membership")
        .id;
    store
        .call("root", |ctx| {
            set_member_role(ctx, membership, "admin".into())
        })
        .expect("promote");
    let private = make_epic(&store, "alice", a, 0, "private");
    let shared = make_epic(&store, "bob", a, 0, "public");
    let task = store.create("bob", epic_task("t", shared, "backlog"));
    move_task(&store, "alice", task, private).expect("admin");
}

#[test]
fn a_task_leaves_your_private_epic_for_a_public_one() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private");
    let public = make_epic(&store, "alice", a, 0, "public");
    let task = store.create("alice", epic_task("t", private, "backlog"));
    move_task(&store, "alice", task, public).expect("holder exposes own work");
    assert_eq!(store.task(task).epic_id, public);
}

#[test]
fn an_epicless_task_of_yours_moves_into_your_private_epic() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private");
    let task = store.create("alice", user_task("t", "alice"));
    move_task(&store, "alice", task, private).expect("own task, own holder");
    assert_eq!(store.task(task).epic_id, private);
}

// -- V3: repo groups ----------------------------------------------------------

/// A grouped root in A and the repo-group sub-epic one task created.
fn grouped(store: &Store, a: i64) -> (i64, i64, i64) {
    let root = make_epic(store, "alice", a, 0, "public");
    store
        .call("alice", |ctx| {
            patch_epic(
                ctx,
                root,
                EpicPatch {
                    group_by_repo: Some(true),
                    ..EpicPatch::default()
                },
            )
        })
        .expect("group");
    let task = store.create(
        "alice",
        Task {
            repo_path: "/x/alpha".into(),
            ..epic_task("t", root, "backlog")
        },
    );
    let sub = store
        .db()
        .epics()
        .iter()
        .find(|e| e.origin == "repo-group")
        .expect("repo group")
        .id;
    (root, sub, task)
}

#[test]
fn a_repo_group_cannot_be_made_private() {
    let (store, a, _) = world();
    let (_, sub, _) = grouped(&store, a);
    assert!(store
        .call("alice", |ctx| set_epic_visibility(
            ctx,
            sub,
            "private".into()
        ))
        .is_err());
    assert_eq!(store.epic(sub).visibility, "public");
}

#[test]
fn a_repo_group_cannot_be_created_private() {
    let (store, a, _) = world();
    let (root, _, _) = grouped(&store, a);
    let row = Epic {
        title: "x".into(),
        workspace_id: a,
        parent_epic_id: root,
        visibility: "private".into(),
        origin: "repo-group".into(),
        ..blank_epic()
    };
    assert!(store.call("alice", |ctx| create_epic(ctx, row)).is_err());
}

#[test]
fn a_repo_groups_origin_cannot_be_changed() {
    let (store, a, _) = world();
    let (_, sub, _) = grouped(&store, a);
    let r = store.call("alice", |ctx| {
        patch_epic(
            ctx,
            sub,
            EpicPatch {
                origin: Some(String::new()),
                ..EpicPatch::default()
            },
        )
    });
    assert!(r.is_err());
    assert_eq!(store.epic(sub).origin, "repo-group");
}

#[test]
fn a_hand_made_epic_cannot_be_given_the_repo_group_origin() {
    let (store, a, _) = world();
    let plain = make_epic(&store, "alice", a, 0, "public");
    let r = store.call("alice", |ctx| {
        patch_epic(
            ctx,
            plain,
            EpicPatch {
                origin: Some("repo-group".into()),
                ..EpicPatch::default()
            },
        )
    });
    assert!(r.is_err());
}

// -- R2: relinks end the unseen mark ------------------------------------------

#[test]
fn flattening_a_grouped_epic_clears_the_unseen_marks_of_its_tasks() {
    let (store, a, _) = world();
    let (root, sub, _) = grouped(&store, a);
    let signalled = {
        let row = Task {
            repo_path: "/x/alpha".into(),
            ..epic_task("s", sub, "backlog")
        };
        store
            .call("alice", |ctx| create_task_signalled(ctx, row))
            .expect("signalled");
        store.db().tasks().iter().map(|t| t.id).max().expect("task")
    };
    assert!(
        store
            .db()
            .unseen_tasks()
            .iter()
            .any(|u| u.task_id == signalled),
        "the signal marks the task"
    );
    store
        .call("alice", |ctx| flatten_epic(ctx, root))
        .expect("flatten");
    assert_eq!(store.task(signalled).epic_id, root);
    assert!(
        !store
            .db()
            .unseen_tasks()
            .iter()
            .any(|u| u.task_id == signalled),
        "the task left its epic, so its mark ends"
    );
}

// -- V10: cross-workspace task moves ------------------------------------------

#[test]
fn a_task_moved_to_another_workspace_drops_its_links_and_takes_its_learnings_along() {
    let (store, a, b) = world();
    for who in ["alice"] {
        store
            .call("root", |ctx| {
                add_member(ctx, b, identity_of(who), "member".into())
            })
            .expect("alice joins B");
    }
    let from = make_epic(&store, "alice", a, 0, "public");
    let to = make_epic(&store, "alice", b, 0, "public");
    let task = store.create("alice", epic_task("t", from, "backlog"));
    store
        .call("alice", |ctx| {
            create_work_package(
                ctx,
                WorkPackage {
                    title: "p".into(),
                    workspace_id: a,
                    visibility: "public".into(),
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
    store
        .call("alice", |ctx| {
            link_work_package_item(ctx, package, "task".into(), task)
        })
        .expect("link");
    assert_eq!(store.db().work_package_links().iter().count(), 1);

    move_task(&store, "alice", task, to).expect("move");

    assert_eq!(
        store.db().work_package_links().iter().count(),
        0,
        "a link never crosses workspaces"
    );
}

// -- V11: unlinking an item you cannot see ------------------------------------

#[test]
fn unlinking_an_item_the_caller_cannot_see_does_nothing() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private");
    store
        .call("alice", |ctx| {
            create_work_package(
                ctx,
                WorkPackage {
                    title: "p".into(),
                    workspace_id: a,
                    visibility: "public".into(),
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
    store
        .call("alice", |ctx| {
            link_work_package_item(ctx, package, "epic".into(), private)
        })
        .expect("link");

    store
        .call("bob", |ctx| {
            unlink_work_package_item(ctx, package, "epic".into(), private)
        })
        .expect("answered as no link");
    assert_eq!(store.db().work_package_links().iter().count(), 1);

    store
        .call("alice", |ctx| {
            unlink_work_package_item(ctx, package, "epic".into(), private)
        })
        .expect("the creator unlinks");
    assert_eq!(store.db().work_package_links().iter().count(), 0);
}

// -- V12: the hatch is for items whose creator is gone ------------------------

#[test]
fn the_hatch_refuses_a_private_epic_whose_creator_is_still_a_member() {
    let (store, a, _) = world();
    let epic = make_epic(&store, "alice", a, 0, "private");
    assert!(store
        .call("root", |ctx| super_admin_delete_unseen_item(
            ctx,
            "epic".into(),
            epic
        ))
        .is_err());
    assert!(store.db().epics().id().find(epic).is_some());
}

#[test]
fn the_hatch_refuses_a_private_package_whose_creator_is_still_a_member() {
    let (store, a, _) = world();
    store
        .call("alice", |ctx| {
            create_work_package(
                ctx,
                WorkPackage {
                    title: "p".into(),
                    workspace_id: a,
                    visibility: "private".into(),
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
    assert!(store
        .call("root", |ctx| super_admin_delete_unseen_item(
            ctx,
            "work_package".into(),
            package
        ))
        .is_err());
}

#[test]
fn the_hatch_deletes_a_private_epic_once_its_creator_has_left() {
    let (store, a, _) = world();
    let epic = make_epic(&store, "alice", a, 0, "private");
    remove_membership(&store, a, "alice");
    store
        .call("root", |ctx| {
            super_admin_delete_unseen_item(ctx, "epic".into(), epic)
        })
        .expect("hatch");
    assert!(store.db().epics().id().find(epic).is_none());
}
