//! `workspaces.allium: MigrateStoreToWorkspaces` (D12) and the two checks that
//! waited for `Epic.workspace`: `DeleteWorkspace` refused while occupied and
//! `RemoveMember`'s subscription and unseen-mark cleanup. Run against the
//! in-memory store.

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

/// Put the store back to a module that predates workspaces: rules version 16,
/// no workspace, nobody registered.
fn rewind_to_version_16(store: &Store) {
    store
        .call("operator", |ctx| {
            ctx.db
                .rules_version()
                .id()
                .update(RulesVersion { id: 1, version: 16 });
            Ok(())
        })
        .expect("rewind");
}

fn package(title: &str, created_by: &str) -> WorkPackage {
    WorkPackage {
        id: 0,
        title: title.into(),
        description: String::new(),
        created_by: created_by.into(),
        created_at: "2026-10-09 10:00:00".into(),
        updated_at: "2026-10-09 10:00:00".into(),
        sort_order: None,
        workspace_id: 0,
        visibility: "public".into(),
    }
}

fn learning(scope: &str) -> Learning {
    Learning {
        id: 0,
        kind: "pitfall".into(),
        summary: format!("a {scope} learning"),
        detail: None,
        scope: scope.into(),
        scope_ref: (scope != "user").then(|| "/repo".to_string()),
        tags: "[]".into(),
        status: "approved".into(),
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: "2026-09-19 10:00:00".into(),
        updated_at: "2026-09-19 10:00:00".into(),
        embedding: None,
        workspace_id: 0,
    }
}

fn default_workspace(store: &Store) -> Workspace {
    let all: Vec<Workspace> = store.db().workspaces().iter().collect();
    assert_eq!(all.len(), 1, "exactly one workspace: {all:?}");
    all.into_iter().next().expect("one workspace")
}

/// A store written by the previous module: a root epic, a sub-epic, a package,
/// one learning per scope, and rows naming people from every source the
/// migration reads. Nobody has connected yet.
fn old_store() -> Store {
    let store = Store::bare();
    store.register_host_for("laptop", "alice");
    let root = store.create_epic("bob", "Root", false);
    store.db().epics().insert(Epic {
        title: "Child".into(),
        parent_epic_id: root,
        created_by: identity_of("carol"),
        ..blank_epic()
    });
    store.create("dave", user_task("a user-board task", "dave"));
    store
        .db()
        .work_packages()
        .insert(package("Pack", &identity_of("erin")));
    // Written by the store itself: before workspaces nobody was refused.
    store
        .call_as_scheduler(|ctx| subscribe_to_epic(ctx, identity_of("frank"), root))
        .expect("subscribe");
    for scope in ["repo", "epic", "task", "user"] {
        store
            .call("operator", |ctx| create_learning(ctx, learning(scope)))
            .expect("learning");
    }
    store
}

#[test]
fn the_first_connection_after_the_upgrade_creates_the_default_workspace() {
    let store = old_store();
    connect(&store, "alice");
    let workspace = default_workspace(&store);
    assert_eq!(workspace.name, "Default");
}

#[test]
fn every_epic_and_package_lands_public_in_the_default_workspace() {
    let store = old_store();
    connect(&store, "alice");
    let workspace = default_workspace(&store).id;
    let epics: Vec<Epic> = store.db().epics().iter().collect();
    assert_eq!(epics.len(), 2);
    for epic in epics {
        assert_eq!(epic.workspace_id, workspace);
        assert_eq!(epic.visibility, "public");
    }
    let packages: Vec<WorkPackage> = store.db().work_packages().iter().collect();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].workspace_id, workspace);
    assert_eq!(packages[0].visibility, "public");
}

#[test]
fn non_user_learnings_join_the_workspace_and_user_learnings_stay_out() {
    let store = old_store();
    connect(&store, "alice");
    let workspace = default_workspace(&store).id;
    for l in store.db().learnings().iter() {
        let expected = if l.scope == "user" { 0 } else { workspace };
        assert_eq!(l.workspace_id, expected, "{} learning", l.scope);
    }
}

#[test]
fn every_known_identity_gets_a_profile_and_a_member_row() {
    let store = old_store();
    connect(&store, "alice");
    let workspace = default_workspace(&store).id;
    // alice: Host.owner. carol: Epic.created_by. dave: Task.owner and
    // Task.created_by. erin: WorkPackage.created_by. frank:
    // Subscription.subscriber.
    for who in ["alice", "carol", "dave", "erin", "frank"] {
        let id = identity_of(who);
        assert!(
            store.db().user_profiles().identity().find(&id).is_some(),
            "{who} has a profile"
        );
        let rows: Vec<Membership> = store
            .db()
            .memberships()
            .iter()
            .filter(|m| m.user == id)
            .collect();
        assert_eq!(rows.len(), 1, "{who} has one membership");
        assert_eq!(rows[0].workspace_id, workspace);
        assert_eq!(rows[0].role, "member");
    }
}

#[test]
fn nobody_is_a_super_admin_after_the_migration() {
    let store = old_store();
    connect(&store, "alice");
    assert!(store.db().user_profiles().iter().all(|p| !p.is_super_admin));
}

#[test]
fn an_empty_identity_is_not_a_person() {
    let store = old_store();
    // The ownerless epic-less task of an older store.
    store
        .call("operator", |ctx| {
            ctx.db.tasks().insert(Task {
                title: "ownerless".into(),
                repo_path: "/repo".into(),
                ..blank_task()
            });
            Ok(())
        })
        .expect("ownerless task");
    connect(&store, "alice");
    assert!(store
        .db()
        .user_profiles()
        .identity()
        .find(&String::new())
        .is_none());
    assert!(store.db().memberships().iter().all(|m| !m.user.is_empty()));
}

#[test]
fn a_person_with_a_profile_already_keeps_it() {
    let store = old_store();
    store
        .call("dave", |ctx| {
            stamp_rules_version_on_connect(ctx);
            set_display_name(ctx, "Dave".into())
        })
        .expect("name");
    rewind_to_version_16(&store);
    store
        .call("operator", |ctx| {
            for w in ctx.db.workspaces().iter().collect::<Vec<_>>() {
                ctx.db.workspaces().id().delete(w.id);
            }
            Ok(())
        })
        .expect("clear");
    connect(&store, "alice");
    let profile = store
        .db()
        .user_profiles()
        .identity()
        .find(&identity_of("dave"))
        .expect("profile");
    assert_eq!(profile.display_name, "Dave");
}

#[test]
fn the_migration_runs_once() {
    let store = old_store();
    connect(&store, "alice");
    connect(&store, "bob");
    connect(&store, "alice");
    assert_eq!(store.db().workspaces().iter().count(), 1);
    let members = store.db().memberships().iter().count();
    connect(&store, "alice");
    assert_eq!(store.db().memberships().iter().count(), members);
}

// -- DeleteWorkspace and RemoveMember ----------------------------------------

fn store_with_root_and_default() -> (Store, i64) {
    let store = Store::bare();
    for who in ["root", "alice"] {
        connect(&store, who);
    }
    store
        .call("root", |ctx| claim_super_admin(ctx))
        .expect("claim");
    let workspace = default_workspace(&store).id;
    store
        .call("root", |ctx| {
            add_member(ctx, workspace, identity_of("alice"), "member".into())
        })
        .expect("alice joins");
    (store, workspace)
}

#[test]
fn deleting_a_workspace_that_holds_an_epic_is_refused() {
    let (store, workspace) = store_with_root_and_default();
    let epic = store.create_epic("alice", "Held", false);
    store
        .call("operator", |ctx| {
            let row = ctx.db.epics().id().find(epic).expect("epic");
            ctx.db.epics().id().update(Epic {
                workspace_id: workspace,
                ..row
            });
            Ok(())
        })
        .expect("place");
    assert!(store
        .call("root", |ctx| delete_workspace(ctx, workspace))
        .is_err());
    assert!(store.db().workspaces().id().find(workspace).is_some());
}

#[test]
fn deleting_a_workspace_that_holds_a_package_is_refused() {
    let (store, workspace) = store_with_root_and_default();
    store
        .call("alice", |ctx| {
            create_work_package(
                ctx,
                WorkPackage {
                    workspace_id: workspace,
                    ..package("Held", &identity_of("alice"))
                },
            )
        })
        .expect("package");
    assert!(store
        .call("root", |ctx| delete_workspace(ctx, workspace))
        .is_err());
}

#[test]
fn deleting_an_empty_workspace_removes_its_learnings() {
    let (store, workspace) = store_with_root_and_default();
    store
        .call("alice", |ctx| {
            create_learning(
                ctx,
                Learning {
                    workspace_id: workspace,
                    ..learning("repo")
                },
            )
        })
        .expect("learning");
    store
        .call("root", |ctx| delete_workspace(ctx, workspace))
        .expect("delete");
    assert_eq!(store.db().learnings().iter().count(), 0);
}

#[test]
fn removing_a_member_ends_their_subscriptions_and_unseen_marks_in_that_workspace_only() {
    let (store, workspace) = store_with_root_and_default();
    let other = store
        .call("root", |ctx| create_workspace(ctx, "Other".into()))
        .map(|_| {
            store
                .db()
                .workspaces()
                .iter()
                .find(|w| w.name == "Other")
                .expect("other")
                .id
        })
        .expect("create");
    let here = store.create_epic("alice", "Here", false);
    let there = store.create_epic("alice", "There", false);
    store
        .call("operator", |ctx| {
            for (epic, ws) in [(here, workspace), (there, other)] {
                let row = ctx.db.epics().id().find(epic).expect("epic");
                ctx.db.epics().id().update(Epic {
                    workspace_id: ws,
                    ..row
                });
            }
            Ok(())
        })
        .expect("place");
    let alice = identity_of("alice");
    for epic in [here, there] {
        store
            .call_as_scheduler(|ctx| subscribe_to_epic(ctx, alice.clone(), epic))
            .expect("subscribe");
    }
    store
        .call_as_scheduler(|ctx| create_task(ctx, epic_task("here", here, "backlog")))
        .expect("task");
    let task_here = store.db().tasks().iter().map(|t| t.id).max().expect("task");
    store
        .call_as_scheduler(|ctx| create_task(ctx, epic_task("there", there, "backlog")))
        .expect("task");
    let task_there = store.db().tasks().iter().map(|t| t.id).max().expect("task");
    for task in [task_here, task_there] {
        store
            .call("operator", |ctx| {
                ctx.db.unseen_tasks().insert(UnseenTask {
                    id: format!("{alice}/{task}"),
                    task_id: task,
                    subscriber: alice.clone(),
                });
                Ok(())
            })
            .expect("mark");
    }
    let membership = store
        .db()
        .memberships()
        .iter()
        .find(|m| m.workspace_id == workspace && m.user == alice)
        .expect("membership")
        .id;
    store
        .call("root", |ctx| remove_member(ctx, membership))
        .expect("remove");

    let subs: Vec<i64> = store
        .db()
        .subscriptions()
        .iter()
        .map(|s| s.epic_id)
        .collect();
    assert_eq!(subs, vec![there]);
    let marks: Vec<i64> = store
        .db()
        .unseen_tasks()
        .iter()
        .map(|u| u.task_id)
        .collect();
    assert_eq!(marks, vec![task_there]);
}
