//! The store serves one person, possibly on several hosts
//! (`docs/plans/2026-11-23-remove-workspaces.md`, R1-R7): no workspace,
//! membership, role or private item stands between two identities. Every test
//! runs on a bare store, with no workspace at all.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// A bare store where `alice` and `bob` have connected (so each has a
/// profile) and own the hosts `h-alice` and `h-bob`.
fn world() -> Store {
    let store = Store::bare();
    for who in ["alice", "bob"] {
        store
            .call(who, |ctx| {
                stamp_rules_version_on_connect(ctx);
                Ok(())
            })
            .expect("connect");
        store.register_host_for(&format!("h-{who}"), who);
    }
    store
}

fn alices_epic(store: &Store) -> i64 {
    store
        .call("alice", |ctx| {
            create_epic(
                ctx,
                Epic {
                    title: "alice's".into(),
                    feed_command: "echo alice".into(),
                    ..blank_epic()
                },
            )
        })
        .expect("an epic needs no workspace");
    store.db().epics().iter().map(|e| e.id).max().expect("epic")
}

#[test]
fn an_epic_is_created_with_no_workspace() {
    let store = world();
    let id = alices_epic(&store);
    assert_eq!(store.epic(id).created_by, identity_of("alice"));
}

#[test]
fn anyone_may_detach_a_task_and_it_lands_on_their_board() {
    let store = world();
    let epic = alices_epic(&store);
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    store
        .call("bob", |ctx| set_task_epic(ctx, task, 0, String::new()))
        .expect("R7: anyone may detach");
    let row = store.task(task);
    assert_eq!(row.epic_id, 0);
    assert_eq!(row.owner, identity_of("bob"));
}

#[test]
fn a_signalled_task_marks_every_known_person() {
    let store = world();
    let epic = alices_epic(&store);
    let task = store.create_signalled("alice", epic_task("t", epic, "backlog"));
    let mut marked: Vec<String> = store
        .db()
        .unseen_tasks()
        .iter()
        .filter(|u| u.task_id == task)
        .map(|u| u.subscriber)
        .collect();
    marked.sort();
    let mut everyone = vec![identity_of("alice"), identity_of("bob")];
    everyone.sort();
    assert_eq!(marked, everyone, "R5: one mark per UserProfile");
}

#[test]
fn a_follower_receives_an_epic_someone_else_created() {
    let store = world();
    let epic = alices_epic(&store);
    store
        .call("bob", |ctx| subscribe_to_epic(ctx, identity_of("bob"), epic))
        .expect("R2: bob may follow it");
    let ctx = store.call("bob", |ctx| Ok(covered_epics_of(ctx, &identity_of("bob"))))
        .expect("read");
    assert!(ctx.iter().any(|e| e.id == epic), "R4: the view narrows by follows only");
}

/// Rows an older store left behind, planted straight into the tables: alice's
/// private epic in workspace 7, another epic in workspace 9, and a private
/// work package. The dead columns stand in nobody's way (R2, R8, R11): bob
/// follows and receives the private epic, moves a task between the two epics,
/// re-parents the private epic under the other and links the task to the
/// private package.
#[test]
fn old_private_and_workspace_rows_stand_in_nobodys_way() {
    let store = world();
    let private = store
        .db()
        .epics()
        .insert(Epic {
            title: "alice's private".into(),
            created_by: identity_of("alice"),
            workspace_id: 7,
            visibility: "private".into(),
            ..blank_epic()
        })
        .id;
    let other = store
        .db()
        .epics()
        .insert(Epic {
            title: "elsewhere".into(),
            created_by: identity_of("alice"),
            workspace_id: 9,
            ..blank_epic()
        })
        .id;
    let package = store
        .db()
        .work_packages()
        .insert(WorkPackage {
            title: "alice's private package".into(),
            created_by: identity_of("alice"),
            workspace_id: 7,
            visibility: "private".into(),
            ..blank_work_package()
        })
        .id;
    let task = store.create("alice", epic_task("t", private, "backlog"));

    store
        .call("bob", |ctx| subscribe_to_epic(ctx, identity_of("bob"), private))
        .expect("bob may follow a private epic");
    let covered = store
        .call("bob", |ctx| Ok(covered_epics_of(ctx, &identity_of("bob"))))
        .expect("read");
    assert!(
        covered.iter().any(|e| e.id == private),
        "the view serves bob the private epic he follows"
    );

    store
        .call("bob", |ctx| set_task_epic(ctx, task, other, String::new()))
        .expect("bob moves the task across workspaces");
    assert_eq!(store.task(task).epic_id, other);

    store
        .call("bob", |ctx| {
            patch_epic(
                ctx,
                private,
                EpicPatch {
                    parent_epic_id: Some(other),
                    ..EpicPatch::default()
                },
            )
        })
        .expect("bob re-parents the private epic");
    assert_eq!(store.epic(private).parent_epic_id, other);

    store
        .call("bob", |ctx| {
            link_work_package_item(ctx, package, "task".into(), task)
        })
        .expect("bob links into the private package");
    assert!(store
        .db()
        .work_package_links()
        .iter()
        .any(|l| l.work_package_id == package && l.item_id == task));
}
