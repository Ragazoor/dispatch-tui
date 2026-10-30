//! Visibility on create and change, and write checks (`workspaces.allium`:
//! `CreateEpicInWorkspace`, `SetEpicVisibility`, `EpicStaysInItsWorkspace`,
//! `TaskEntersAnEpicTheCallerCanSee`, `CreateWorkPackageInWorkspace`,
//! `SetWorkPackageVisibility`, `LinkOnlyInsideTheWorkspace`,
//! `SuperAdminDeletesUnseenItem`, `WritesRequireSight`), run against the
//! in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// `root` is super-admin; `alice` and `bob` are members of workspace A,
/// `carol` of workspace B, `dave` of neither. Returns (store, A, B).
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
    for (ws, who) in [(ids[0], "alice"), (ids[0], "bob"), (ids[1], "carol")] {
        store
            .call("root", |ctx| {
                add_member(ctx, ws, identity_of(who), "member".into())
            })
            .expect("member");
    }
    (store, ids[0], ids[1])
}

fn epic_row(title: &str, workspace_id: i64, parent: i64, visibility: &str) -> Epic {
    Epic {
        title: title.into(),
        workspace_id,
        parent_epic_id: parent,
        visibility: visibility.into(),
        ..blank_epic()
    }
}

fn make_epic(
    store: &Store,
    who: &str,
    workspace_id: i64,
    parent: i64,
    visibility: &str,
) -> Result<i64, String> {
    let row = epic_row("e", workspace_id, parent, visibility);
    store.call(who, |ctx| create_epic(ctx, row))?;
    Ok(store.db().epics().iter().map(|e| e.id).max().expect("epic"))
}

fn package_row(workspace_id: i64, visibility: &str) -> WorkPackage {
    WorkPackage {
        title: "p".into(),
        workspace_id,
        visibility: visibility.into(),
        created_by: String::new(),
        ..blank_work_package()
    }
}

fn make_package(
    store: &Store,
    who: &str,
    workspace_id: i64,
    visibility: &str,
) -> Result<i64, String> {
    let mut row = package_row(workspace_id, visibility);
    row.created_by = identity_of(who);
    store.call(who, |ctx| create_work_package(ctx, row))?;
    Ok(store
        .db()
        .work_packages()
        .iter()
        .map(|p| p.id)
        .max()
        .expect("package"))
}

// -- CreateEpicInWorkspace ----------------------------------------------------

#[test]
fn a_member_creates_a_root_epic_in_their_workspace() {
    let (store, a, _) = world();
    let id = make_epic(&store, "alice", a, 0, "public").expect("create");
    let epic = store.epic(id);
    assert_eq!(epic.workspace_id, a);
    assert_eq!(epic.created_by, identity_of("alice"));
}

#[test]
fn a_non_member_cannot_create_an_epic_in_a_workspace() {
    let (store, a, _) = world();
    assert!(make_epic(&store, "carol", a, 0, "public").is_err());
    assert!(make_epic(&store, "dave", a, 0, "public").is_err());
}

#[test]
fn a_root_epic_needs_a_workspace_that_exists() {
    let (store, _, _) = world();
    assert!(make_epic(&store, "alice", 0, 0, "public").is_err());
    assert!(make_epic(&store, "alice", 9999, 0, "public").is_err());
}

#[test]
fn a_sub_epic_takes_its_parents_workspace_whatever_the_row_names() {
    let (store, a, b) = world();
    let parent = make_epic(&store, "alice", a, 0, "public").expect("parent");
    let sub = make_epic(&store, "alice", b, parent, "public").expect("sub");
    assert_eq!(store.epic(sub).workspace_id, a);
}

#[test]
fn a_sub_epic_under_an_epic_the_caller_cannot_see_is_refused() {
    let (store, a, _) = world();
    let parent = make_epic(&store, "alice", a, 0, "private").expect("parent");
    assert!(make_epic(&store, "bob", a, parent, "public").is_err());
    assert!(make_epic(&store, "alice", a, parent, "public").is_ok());
}

#[test]
fn an_unknown_visibility_is_refused() {
    let (store, a, _) = world();
    assert!(make_epic(&store, "alice", a, 0, "secret").is_err());
}

// -- SetEpicVisibility --------------------------------------------------------

#[test]
fn the_creator_makes_an_epic_private_and_public_again() {
    let (store, a, _) = world();
    let id = make_epic(&store, "alice", a, 0, "public").expect("create");
    store
        .call("alice", |ctx| {
            set_epic_visibility(ctx, id, "private".into())
        })
        .expect("private");
    assert_eq!(store.epic(id).visibility, "private");
    store
        .call("alice", |ctx| set_epic_visibility(ctx, id, "public".into()))
        .expect("public");
    assert_eq!(store.epic(id).visibility, "public");
}

#[test]
fn another_member_cannot_change_an_epics_visibility() {
    let (store, a, _) = world();
    let id = make_epic(&store, "alice", a, 0, "public").expect("create");
    assert!(store
        .call("bob", |ctx| set_epic_visibility(ctx, id, "private".into()))
        .is_err());
    assert_eq!(store.epic(id).visibility, "public");
}

#[test]
fn a_workspace_admin_changes_a_public_epics_visibility_but_cannot_reach_a_private_one() {
    let (store, a, _) = world();
    let membership = store
        .db()
        .memberships()
        .iter()
        .find(|m| m.workspace_id == a && m.user == identity_of("bob"))
        .expect("membership")
        .id;
    store
        .call("root", |ctx| {
            set_member_role(ctx, membership, "admin".into())
        })
        .expect("promote");
    let public = make_epic(&store, "alice", a, 0, "public").expect("public");
    store
        .call("bob", |ctx| {
            set_epic_visibility(ctx, public, "private".into())
        })
        .expect("admin acts on a public epic");
    // Now private to alice: bob, the admin, no longer sees it.
    assert!(store
        .call("bob", |ctx| set_epic_visibility(
            ctx,
            public,
            "public".into()
        ))
        .is_err());
    assert_eq!(store.epic(public).visibility, "private");
}

#[test]
fn a_refusal_for_an_unseen_epic_is_the_refusal_for_a_missing_one() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private").expect("private");
    let unseen = store
        .call("bob", |ctx| {
            set_epic_visibility(ctx, private, "public".into())
        })
        .expect_err("refused");
    let missing = store
        .call("bob", |ctx| set_epic_visibility(ctx, 9999, "public".into()))
        .expect_err("refused");
    assert_eq!(
        unseen.replace(&private.to_string(), "N"),
        missing.replace("9999", "N")
    );
}

// -- EpicStaysInItsWorkspace --------------------------------------------------

#[test]
fn an_epic_moves_inside_its_workspace_but_never_to_another() {
    let (store, a, b) = world();
    store
        .call("root", |ctx| {
            add_member(ctx, b, identity_of("alice"), "member".into())
        })
        .expect("alice joins B");
    let mine = make_epic(&store, "alice", a, 0, "public").expect("mine");
    let same = make_epic(&store, "alice", a, 0, "public").expect("same");
    let other = make_epic(&store, "alice", b, 0, "public").expect("other");
    let patch = |parent| EpicPatch {
        parent_epic_id: Some(parent),
        ..EpicPatch::default()
    };
    assert!(store
        .call("alice", |ctx| patch_epic(ctx, mine, patch(other)))
        .is_err());
    store
        .call("alice", |ctx| patch_epic(ctx, mine, patch(same)))
        .expect("same workspace");
    assert_eq!(store.epic(mine).parent_epic_id, same);
}

// -- TaskEntersAnEpicTheCallerCanSee ------------------------------------------

#[test]
fn a_task_cannot_be_created_in_or_moved_into_an_unseen_epic() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private").expect("private");
    let public = make_epic(&store, "alice", a, 0, "public").expect("public");
    let task = epic_task("t", private, "backlog");
    assert!(store
        .call("bob", |ctx| create_task(ctx, task.clone()))
        .is_err());
    assert!(store.call("alice", |ctx| create_task(ctx, task)).is_ok());

    let own = store.create("bob", user_task("mine", "bob"));
    assert!(store
        .call("bob", |ctx| set_task_epic(ctx, own, private, String::new()))
        .is_err());
    store
        .call("bob", |ctx| set_task_epic(ctx, own, public, String::new()))
        .expect("public epic");
}

// -- WritesRequireSight -------------------------------------------------------

#[test]
fn a_member_cannot_write_to_a_task_in_a_private_epic() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private").expect("private");
    let task = store.create("alice", epic_task("t", private, "backlog"));
    store
        .call("bob", |ctx| patch_task(ctx, task, status_patch("running")))
        .expect("a refusal reveals nothing: it is a no-op");
    assert_eq!(store.task(task).status, "backlog");
    assert!(store
        .call("bob", |ctx| update_task(
            ctx,
            task,
            None,
            String::new(),
            status_patch("running")
        ))
        .is_err());
    store
        .call("bob", |ctx| delete_task(ctx, task))
        .expect("no-op");
    assert!(store.db().tasks().id().find(task).is_some());
}

#[test]
fn a_hook_for_a_task_in_a_private_epic_is_answered_as_for_a_missing_task() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private").expect("private");
    let task = store.create("alice", epic_task("t", private, "running"));
    let unseen = store
        .call("bob", |ctx| {
            try_record_stop(ctx, task, "2026-09-19T10:00:00.000".into())
        })
        .expect_err("refused");
    let missing = store
        .call("bob", |ctx| {
            try_record_stop(ctx, 999_999, "2026-09-19T10:00:00.000".into())
        })
        .expect_err("refused");
    assert_eq!(
        unseen.replace(&task.to_string(), "N"),
        missing.replace("999999", "N")
    );
    assert_eq!(store.task(task).status, "running");
}

#[test]
fn a_user_board_task_is_written_by_its_owner_alone() {
    let (store, _, _) = world();
    let task = store.create("alice", user_task("mine", "alice"));
    store
        .call("bob", |ctx| patch_task(ctx, task, status_patch("running")))
        .expect("no-op");
    assert_eq!(store.task(task).status, "backlog");
    store
        .call("alice", |ctx| {
            patch_task(ctx, task, status_patch("running"))
        })
        .expect("owner");
    assert_eq!(store.task(task).status, "running");
}

#[test]
fn a_member_deletes_a_public_epic_but_not_one_holding_a_private_sub_epic() {
    let (store, a, _) = world();
    let public = make_epic(&store, "alice", a, 0, "public").expect("public");
    let hidden = make_epic(&store, "alice", a, public, "private").expect("hidden");
    assert!(store.call("bob", |ctx| delete_epic(ctx, public)).is_err());
    assert!(store.db().epics().id().find(hidden).is_some());
    store
        .call("alice", |ctx| delete_epic(ctx, public))
        .expect("creator sees the whole subtree");
    assert!(store.db().epics().id().find(public).is_none());
}

#[test]
fn the_refusal_to_delete_names_nothing_held_by_a_private_sub_epic() {
    let (store, a, _) = world();
    let public = make_epic(&store, "alice", a, 0, "public").expect("public");
    let hidden = make_epic(&store, "alice", a, public, "private").expect("hidden");
    store.create("alice", epic_task("secret", hidden, "backlog"));
    let refusal = store
        .call("bob", |ctx| delete_epic(ctx, public))
        .expect_err("refused");
    assert!(!refusal.contains("task"), "{refusal}");
}

#[test]
fn a_non_member_cannot_follow_an_epic() {
    let (store, a, _) = world();
    let epic = make_epic(&store, "alice", a, 0, "public").expect("epic");
    assert!(store
        .call("dave", |ctx| subscribe_to_epic(
            ctx,
            identity_of("dave"),
            epic
        ))
        .is_err());
    assert!(store
        .call("bob", |ctx| subscribe_to_epic(
            ctx,
            identity_of("bob"),
            epic
        ))
        .is_ok());
}

// -- work packages ------------------------------------------------------------

#[test]
fn a_member_creates_a_package_in_their_workspace_a_stranger_cannot() {
    let (store, a, _) = world();
    assert!(make_package(&store, "alice", a, "public").is_ok());
    assert!(make_package(&store, "carol", a, "public").is_err());
    assert!(make_package(&store, "alice", 0, "public").is_err());
    assert!(make_package(&store, "alice", a, "secret").is_err());
}

#[test]
fn only_the_creator_or_an_admin_changes_a_packages_visibility() {
    let (store, a, _) = world();
    let id = make_package(&store, "alice", a, "public").expect("package");
    assert!(store
        .call("bob", |ctx| set_work_package_visibility(
            ctx,
            id,
            "private".into()
        ))
        .is_err());
    store
        .call("alice", |ctx| {
            set_work_package_visibility(ctx, id, "private".into())
        })
        .expect("creator");
    assert_eq!(
        store
            .db()
            .work_packages()
            .id()
            .find(id)
            .expect("p")
            .visibility,
        "private"
    );
    // A private package is invisible to everyone else, so nobody else writes it.
    assert!(store
        .call("bob", |ctx| patch_work_package(
            ctx,
            id,
            WorkPackagePatch::default()
        ))
        .is_err());
}

#[test]
fn a_package_links_only_inside_its_workspace_except_a_main_board_task() {
    let (store, a, b) = world();
    store
        .call("root", |ctx| {
            add_member(ctx, b, identity_of("alice"), "member".into())
        })
        .expect("alice joins B");
    let package = make_package(&store, "alice", a, "public").expect("package");
    let here = make_epic(&store, "alice", a, 0, "public").expect("here");
    let there = make_epic(&store, "alice", b, 0, "public").expect("there");
    let own = store.create("alice", user_task("mine", "alice"));
    let link = |item: i64, kind: &str| {
        store.call("alice", |ctx| {
            link_work_package_item(ctx, package, kind.into(), item)
        })
    };
    assert!(link(here, "epic").is_ok());
    assert!(link(there, "epic").is_err());
    assert!(link(own, "task").is_ok());
}

#[test]
fn a_package_never_links_an_item_the_caller_cannot_see() {
    let (store, a, _) = world();
    let package = make_package(&store, "bob", a, "public").expect("package");
    let private = make_epic(&store, "alice", a, 0, "private").expect("private");
    assert!(store
        .call("bob", |ctx| link_work_package_item(
            ctx,
            package,
            "epic".into(),
            private
        ))
        .is_err());
    let alices_task = store.create("alice", user_task("hers", "alice"));
    assert!(store
        .call("bob", |ctx| link_work_package_item(
            ctx,
            package,
            "task".into(),
            alices_task
        ))
        .is_err());
}

// -- SuperAdminDeletesUnseenItem ----------------------------------------------

#[test]
fn the_escape_hatch_keeps_the_every_task_done_guard() {
    let (store, a, _) = world();
    let epic = make_epic(&store, "alice", a, 0, "private").expect("epic");
    store.create("alice", epic_task("running", epic, "running"));
    assert!(store
        .call("root", |ctx| {
            super_admin_delete_unseen_item(ctx, "epic".into(), epic)
        })
        .is_err());
    assert!(store.db().epics().id().find(epic).is_some());
}

#[test]
fn a_super_admin_deletes_a_private_epic_and_package_without_seeing_them() {
    let (store, a, _) = world();
    let epic = make_epic(&store, "alice", a, 0, "private").expect("epic");
    let package = make_package(&store, "alice", a, "private").expect("package");
    // The super-admin is no member, so a normal delete is a silent no-op.
    store
        .call("root", |ctx| delete_epic(ctx, epic))
        .expect("no-op");
    assert!(store.db().epics().id().find(epic).is_some());

    store
        .call("root", |ctx| {
            super_admin_delete_unseen_item(ctx, "epic".into(), epic)
        })
        .expect("delete epic");
    store
        .call("root", |ctx| {
            super_admin_delete_unseen_item(ctx, "work_package".into(), package)
        })
        .expect("delete package");
    assert!(store.db().epics().id().find(epic).is_none());
    assert!(store.db().work_packages().id().find(package).is_none());
}

#[test]
fn the_escape_hatch_is_for_super_admins_and_private_items_only() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private").expect("private");
    let public = make_epic(&store, "alice", a, 0, "public").expect("public");
    assert!(store
        .call("bob", |ctx| super_admin_delete_unseen_item(
            ctx,
            "epic".into(),
            private
        ))
        .is_err());
    assert!(store
        .call("root", |ctx| super_admin_delete_unseen_item(
            ctx,
            "epic".into(),
            public
        ))
        .is_err());
}

// -- module-made sub-epics ----------------------------------------------------

#[test]
fn the_operator_is_not_held_to_membership() {
    let (store, a, _) = world();
    let private = make_epic(&store, "alice", a, 0, "private").expect("private");
    store
        .call_as_scheduler(|ctx| patch_epic(ctx, private, EpicPatch::default()))
        .expect("the store itself sees everything");
}
