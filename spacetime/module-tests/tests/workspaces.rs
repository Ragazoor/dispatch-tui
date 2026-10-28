//! Super-admins, workspaces and membership (`workspaces.allium`: `ClaimSuperAdmin`,
//! `GrantSuperAdmin`, `RevokeSuperAdmin`, `CreateWorkspace`, `RenameWorkspace`,
//! `DeleteWorkspace`, `AddMember`, `SetMemberRole`, `RemoveMember`), run against
//! the in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn connect(store: &Store, who: &str) {
    store
        .call(who, |ctx| {
            stamp_rules_version_on_connect(ctx);
            Ok(())
        })
        .expect("connect");
}

fn is_admin(store: &Store, who: &str) -> bool {
    store
        .db()
        .user_profiles()
        .identity()
        .find(&identity_of(who))
        .is_some_and(|p| p.is_super_admin)
}

/// A store where `root` has claimed super-admin and `alice`, `bob` and `carol`
/// are connected.
fn store_with_root() -> Store {
    let store = Store::new();
    for who in ["root", "alice", "bob", "carol"] {
        connect(&store, who);
    }
    store
        .call("root", |ctx| claim_super_admin(ctx))
        .expect("claim");
    store
}

fn create_workspace_as(store: &Store, who: &str, name: &str) -> Result<i64, String> {
    store.call(who, |ctx| create_workspace(ctx, name.into()))?;
    Ok(store
        .db()
        .workspaces()
        .iter()
        .find(|w| w.name == name.trim())
        .expect("the workspace")
        .id)
}

fn membership_of(store: &Store, workspace: i64, who: &str) -> Option<Membership> {
    let user = identity_of(who);
    store
        .db()
        .memberships()
        .iter()
        .find(|m| m.workspace_id == workspace && m.user == user)
}

// -- super-admins -------------------------------------------------------------

#[test]
fn the_first_claim_makes_a_super_admin_and_a_second_is_refused() {
    let store = Store::new();
    connect(&store, "alice");
    connect(&store, "bob");
    store
        .call("alice", |ctx| claim_super_admin(ctx))
        .expect("claim");
    assert!(is_admin(&store, "alice"));
    assert!(store.call("bob", |ctx| claim_super_admin(ctx)).is_err());
    assert!(!is_admin(&store, "bob"));
}

#[test]
fn claiming_without_a_profile_is_refused() {
    let store = Store::new();
    assert!(store.call("alice", |ctx| claim_super_admin(ctx)).is_err());
}

#[test]
fn claiming_adopts_epic_less_tasks_with_no_owner_and_only_those() {
    let store = Store::new();
    connect(&store, "alice");
    connect(&store, "bob");
    let epic = store.create_epic("bob", "E", false);
    // A task with no epic and no owner predates owners; `write_task` refuses to
    // make one now, so strip the owner behind its back.
    let ownerless = store.create("bob", user_task("old", "bob"));
    store
        .call("bob", |ctx| {
            let row = ctx.db.tasks().id().find(ownerless).expect("task");
            ctx.db.tasks().id().update(Task {
                owner: String::new(),
                ..row
            });
            Ok(())
        })
        .expect("strip owner");
    let owned = store.create("bob", user_task("bobs", "bob"));
    let in_epic = store.create("bob", epic_task("inside", epic, "backlog"));
    store
        .call("alice", |ctx| claim_super_admin(ctx))
        .expect("claim");
    assert_eq!(store.task(ownerless).owner, identity_of("alice"));
    assert_eq!(store.task(owned).owner, identity_of("bob"));
    assert_eq!(store.task(in_epic).owner, "");
}

#[test]
fn only_a_super_admin_grants_and_the_grantee_needs_a_profile() {
    let store = store_with_root();
    let alice = identity_of("alice");
    assert!(store
        .call("bob", |ctx| grant_super_admin(ctx, alice.clone()))
        .is_err());
    assert!(store
        .call("root", |ctx| grant_super_admin(ctx, identity_of("nobody")))
        .is_err());
    store
        .call("root", |ctx| grant_super_admin(ctx, alice.clone()))
        .expect("grant");
    assert!(is_admin(&store, "alice"));
}

#[test]
fn a_super_admin_can_be_revoked_but_never_the_last() {
    let store = store_with_root();
    store
        .call("root", |ctx| grant_super_admin(ctx, identity_of("alice")))
        .expect("grant");
    // Not a super-admin: refused.
    assert!(store
        .call("bob", |ctx| revoke_super_admin(ctx, identity_of("root")))
        .is_err());
    // Revoking someone who is not one is refused.
    assert!(store
        .call("root", |ctx| revoke_super_admin(ctx, identity_of("bob")))
        .is_err());
    store
        .call("alice", |ctx| revoke_super_admin(ctx, identity_of("root")))
        .expect("revoke");
    assert!(!is_admin(&store, "root"));
    // Alice is now the last: even revoking herself is refused.
    assert!(store
        .call("alice", |ctx| revoke_super_admin(ctx, identity_of("alice")))
        .is_err());
    assert!(is_admin(&store, "alice"));
}

#[test]
fn a_super_admin_may_revoke_themselves_while_another_remains() {
    let store = store_with_root();
    store
        .call("root", |ctx| grant_super_admin(ctx, identity_of("alice")))
        .expect("grant");
    store
        .call("root", |ctx| revoke_super_admin(ctx, identity_of("root")))
        .expect("self revoke");
    assert!(!is_admin(&store, "root"));
}

#[test]
fn claiming_stays_closed_after_a_revoke() {
    let store = store_with_root();
    store
        .call("root", |ctx| grant_super_admin(ctx, identity_of("alice")))
        .expect("grant");
    store
        .call("root", |ctx| revoke_super_admin(ctx, identity_of("root")))
        .expect("revoke");
    assert!(store.call("bob", |ctx| claim_super_admin(ctx)).is_err());
}

// -- workspaces ---------------------------------------------------------------

#[test]
fn a_super_admin_creates_a_workspace_without_joining_it() {
    let store = store_with_root();
    let id = create_workspace_as(&store, "root", "  Platform ").expect("create");
    let row = store.db().workspaces().id().find(&id).expect("row");
    assert_eq!(row.name, "Platform");
    assert_eq!(row.created_by, identity_of("root"));
    assert!(!row.created_at.is_empty());
    assert!(membership_of(&store, id, "root").is_none());
}

#[test]
fn creating_a_workspace_needs_super_admin_and_a_good_unique_name() {
    let store = store_with_root();
    assert!(create_workspace_as(&store, "alice", "Nope").is_err());
    assert!(store
        .call("root", |ctx| create_workspace(ctx, "  ".into()))
        .is_err());
    assert!(store
        .call("root", |ctx| create_workspace(ctx, "x".repeat(65)))
        .is_err());
    store
        .call("root", |ctx| create_workspace(ctx, "x".repeat(64)))
        .expect("at the limit");
    create_workspace_as(&store, "root", "Platform").expect("create");
    assert!(store
        .call("root", |ctx| create_workspace(ctx, " Platform ".into()))
        .is_err());
    // The default workspace every store gets on its first connection, plus the
    // two created here.
    assert_eq!(store.db().workspaces().iter().count(), 3);
}

#[test]
fn a_workspace_name_with_control_characters_is_refused() {
    let store = store_with_root();
    assert!(store
        .call("root", |ctx| create_workspace(ctx, "a\nb".into()))
        .is_err());
    assert!(store
        .call("root", |ctx| create_workspace(ctx, "a\u{202E}b".into()))
        .is_err());
}

#[test]
fn renaming_is_for_super_admins_and_the_workspaces_own_admins() {
    let store = store_with_root();
    let ws = create_workspace_as(&store, "root", "Platform").expect("create");
    let other = create_workspace_as(&store, "root", "Other").expect("create");
    store
        .call("root", |ctx| {
            add_member(ctx, ws, identity_of("alice"), "admin".into())
        })
        .expect("add");
    store
        .call("root", |ctx| {
            add_member(ctx, ws, identity_of("bob"), "member".into())
        })
        .expect("add");
    // A plain member and an outsider are refused; so is an admin of ANOTHER workspace.
    assert!(store
        .call("bob", |ctx| rename_workspace(ctx, ws, "B".into()))
        .is_err());
    assert!(store
        .call("carol", |ctx| rename_workspace(ctx, ws, "C".into()))
        .is_err());
    store
        .call("root", |ctx| {
            add_member(ctx, other, identity_of("carol"), "admin".into())
        })
        .expect("add");
    assert!(store
        .call("carol", |ctx| rename_workspace(ctx, ws, "C".into()))
        .is_err());
    store
        .call("alice", |ctx| rename_workspace(ctx, ws, " Core ".into()))
        .expect("admin renames");
    assert_eq!(store.db().workspaces().id().find(&ws).unwrap().name, "Core");
    store
        .call("root", |ctx| rename_workspace(ctx, ws, "Core2".into()))
        .expect("super renames");
}

#[test]
fn renaming_keeps_names_unique_and_non_empty_but_may_keep_its_own() {
    let store = store_with_root();
    let ws = create_workspace_as(&store, "root", "Platform").expect("create");
    create_workspace_as(&store, "root", "Other").expect("create");
    assert!(store
        .call("root", |ctx| rename_workspace(ctx, ws, "Other".into()))
        .is_err());
    assert!(store
        .call("root", |ctx| rename_workspace(ctx, ws, " ".into()))
        .is_err());
    assert!(store
        .call("root", |ctx| rename_workspace(ctx, ws, "x".repeat(65)))
        .is_err());
    store
        .call("root", |ctx| rename_workspace(ctx, ws, "Platform".into()))
        .expect("same name");
}

#[test]
fn renaming_an_unknown_workspace_is_refused() {
    let store = store_with_root();
    assert!(store
        .call("root", |ctx| rename_workspace(ctx, 99, "X".into()))
        .is_err());
}

#[test]
fn deleting_is_for_super_admins_only_and_removes_its_memberships() {
    let store = store_with_root();
    let ws = create_workspace_as(&store, "root", "Platform").expect("create");
    let keep = create_workspace_as(&store, "root", "Keep").expect("create");
    for w in [ws, keep] {
        store
            .call("root", |ctx| {
                add_member(ctx, w, identity_of("alice"), "admin".into())
            })
            .expect("add");
    }
    assert!(store
        .call("alice", |ctx| delete_workspace(ctx, ws))
        .is_err());
    assert!(store.call("root", |ctx| delete_workspace(ctx, 99)).is_err());
    store
        .call("root", |ctx| delete_workspace(ctx, ws))
        .expect("delete");
    assert!(store.db().workspaces().id().find(&ws).is_none());
    assert!(membership_of(&store, ws, "alice").is_none());
    assert!(membership_of(&store, keep, "alice").is_some());
}

// -- membership ---------------------------------------------------------------

#[test]
fn a_super_admin_or_workspace_admin_adds_members_with_a_role() {
    let store = store_with_root();
    let ws = create_workspace_as(&store, "root", "Platform").expect("create");
    store
        .call("root", |ctx| {
            add_member(ctx, ws, identity_of("alice"), "admin".into())
        })
        .expect("add");
    store
        .call("alice", |ctx| {
            add_member(ctx, ws, identity_of("bob"), "member".into())
        })
        .expect("admin adds");
    assert_eq!(membership_of(&store, ws, "alice").unwrap().role, "admin");
    let bob = membership_of(&store, ws, "bob").unwrap();
    assert_eq!(bob.role, "member");
    assert!(!bob.created_at.is_empty());
    // A plain member and an outsider cannot add.
    assert!(store
        .call("bob", |ctx| add_member(
            ctx,
            ws,
            identity_of("carol"),
            "member".into()
        ))
        .is_err());
    assert!(store
        .call("carol", |ctx| add_member(
            ctx,
            ws,
            identity_of("carol"),
            "admin".into()
        ))
        .is_err());
}

#[test]
fn adding_refuses_an_unknown_person_a_duplicate_a_bad_role_and_an_unknown_workspace() {
    let store = store_with_root();
    let ws = create_workspace_as(&store, "root", "Platform").expect("create");
    assert!(store
        .call("root", |ctx| add_member(
            ctx,
            ws,
            identity_of("nobody"),
            "member".into()
        ))
        .is_err());
    assert!(store
        .call("root", |ctx| add_member(
            ctx,
            ws,
            identity_of("alice"),
            "owner".into()
        ))
        .is_err());
    assert!(store
        .call("root", |ctx| add_member(
            ctx,
            99,
            identity_of("alice"),
            "member".into()
        ))
        .is_err());
    store
        .call("root", |ctx| {
            add_member(ctx, ws, identity_of("alice"), "member".into())
        })
        .expect("add");
    assert!(store
        .call("root", |ctx| add_member(
            ctx,
            ws,
            identity_of("alice"),
            "admin".into()
        ))
        .is_err());
    assert_eq!(store.db().memberships().iter().count(), 1);
}

#[test]
fn one_person_may_belong_to_many_workspaces() {
    let store = store_with_root();
    let a = create_workspace_as(&store, "root", "A").expect("create");
    let b = create_workspace_as(&store, "root", "B").expect("create");
    for w in [a, b] {
        store
            .call("root", |ctx| {
                add_member(ctx, w, identity_of("alice"), "member".into())
            })
            .expect("add");
    }
    assert_eq!(store.db().memberships().iter().count(), 2);
}

#[test]
fn setting_a_role_is_for_super_admins_and_that_workspaces_admins() {
    let store = store_with_root();
    let ws = create_workspace_as(&store, "root", "Platform").expect("create");
    store
        .call("root", |ctx| {
            add_member(ctx, ws, identity_of("alice"), "admin".into())
        })
        .expect("add");
    store
        .call("root", |ctx| {
            add_member(ctx, ws, identity_of("bob"), "member".into())
        })
        .expect("add");
    let bob = membership_of(&store, ws, "bob").unwrap().id;
    assert!(store
        .call("bob", |ctx| set_member_role(ctx, bob, "admin".into()))
        .is_err());
    assert!(store
        .call("carol", |ctx| set_member_role(ctx, bob, "admin".into()))
        .is_err());
    assert!(store
        .call("alice", |ctx| set_member_role(ctx, bob, "owner".into()))
        .is_err());
    assert!(store
        .call("alice", |ctx| set_member_role(ctx, 999, "admin".into()))
        .is_err());
    store
        .call("alice", |ctx| set_member_role(ctx, bob, "admin".into()))
        .expect("admin promotes");
    assert_eq!(membership_of(&store, ws, "bob").unwrap().role, "admin");
}

#[test]
fn an_admin_may_demote_themselves_and_leave_no_admin() {
    let store = store_with_root();
    let ws = create_workspace_as(&store, "root", "Platform").expect("create");
    store
        .call("root", |ctx| {
            add_member(ctx, ws, identity_of("alice"), "admin".into())
        })
        .expect("add");
    let alice = membership_of(&store, ws, "alice").unwrap().id;
    store
        .call("alice", |ctx| set_member_role(ctx, alice, "member".into()))
        .expect("demote self");
    assert_eq!(membership_of(&store, ws, "alice").unwrap().role, "member");
    store
        .call("root", |ctx| set_member_role(ctx, alice, "admin".into()))
        .expect("a super-admin still manages it");
}

#[test]
fn removing_a_member_is_for_super_admins_and_that_workspaces_admins() {
    let store = store_with_root();
    let ws = create_workspace_as(&store, "root", "Platform").expect("create");
    store
        .call("root", |ctx| {
            add_member(ctx, ws, identity_of("alice"), "admin".into())
        })
        .expect("add");
    store
        .call("root", |ctx| {
            add_member(ctx, ws, identity_of("bob"), "member".into())
        })
        .expect("add");
    let bob = membership_of(&store, ws, "bob").unwrap().id;
    assert!(store.call("carol", |ctx| remove_member(ctx, bob)).is_err());
    assert!(store.call("bob", |ctx| remove_member(ctx, bob)).is_err());
    assert!(store.call("alice", |ctx| remove_member(ctx, 999)).is_err());
    store
        .call("alice", |ctx| remove_member(ctx, bob))
        .expect("remove");
    assert!(membership_of(&store, ws, "bob").is_none());
    assert!(membership_of(&store, ws, "alice").is_some());
}

#[test]
fn removing_a_member_keeps_their_tasks_and_their_other_workspaces() {
    let store = store_with_root();
    let a = create_workspace_as(&store, "root", "A").expect("create");
    let b = create_workspace_as(&store, "root", "B").expect("create");
    for w in [a, b] {
        store
            .call("root", |ctx| {
                add_member(ctx, w, identity_of("bob"), "member".into())
            })
            .expect("add");
    }
    let task = store.create("bob", user_task("mine", "bob"));
    let in_a = membership_of(&store, a, "bob").unwrap().id;
    store
        .call("root", |ctx| remove_member(ctx, in_a))
        .expect("remove");
    assert!(membership_of(&store, b, "bob").is_some());
    assert_eq!(store.task(task).owner, identity_of("bob"));
}
