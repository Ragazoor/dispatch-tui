//! Work packages (`docs/specs/work-packages.allium`), run against the
//! in-memory store. Twins of `tests/spacetime_module/work_packages.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn package_row(title: &str) -> WorkPackage {
    WorkPackage {
        id: 0,
        title: title.into(),
        description: String::new(),
        sort_order: None,
        created_by: "user-a".into(),
        created_at: "2026-10-09 10:00:00".into(),
        updated_at: "2026-10-09 10:00:00".into(),
    }
}

fn new_package(store: &Store, title: &str) -> i64 {
    store
        .call("alice", |ctx| create_work_package(ctx, package_row(title)))
        .expect("create_work_package");
    store
        .db()
        .work_packages()
        .iter()
        .map(|p| p.id)
        .max()
        .expect("a package")
}

fn link(store: &Store, package: i64, kind: &str, item: i64) -> Result<(), String> {
    store.call("alice", |ctx| {
        link_work_package_item(ctx, package, kind.into(), item)
    })
}

fn link_ids(store: &Store) -> Vec<String> {
    store
        .db()
        .work_package_links()
        .iter()
        .map(|l| l.id)
        .collect()
}

/// A done epic holding one done task: both can be deleted.
fn done_epic(store: &Store, title: &str) -> (i64, i64) {
    let epic = store.create_epic("alice", title, false);
    let task = store.create("alice", epic_task(title, epic, "done"));
    assert_eq!(store.epic(epic).status, "done");
    (epic, task)
}

/// `work-packages.allium: CreateWorkPackage`: a blank title is refused and
/// nothing is created; a padded one is trimmed.
#[test]
fn creating_a_work_package_trims_its_title_and_refuses_a_blank_one() {
    let store = Store::new();
    let blank = store.call("alice", |ctx| create_work_package(ctx, package_row("   ")));
    assert!(blank.is_err());
    assert_eq!(store.db().work_packages().iter().count(), 0);

    let id = new_package(&store, "  Window 1  ");
    assert_eq!(
        store.db().work_packages().id().find(id).expect("row").title,
        "Window 1"
    );
}

/// `work-packages.allium: UpdateWorkPackage`: a missing package is refused,
/// a blank title is refused, a description change leaves the title alone.
#[test]
fn patching_a_work_package_refuses_a_missing_package_and_a_blank_title() {
    let store = Store::new();
    let id = new_package(&store, "Window 1");
    let patch = |title: Option<&str>, description: Option<&str>| WorkPackagePatch {
        title: title.map(Into::into),
        description: description.map(Into::into),
        sort_order: None,
    };

    let missing = store.call("alice", |ctx| {
        patch_work_package(ctx, 99, patch(Some("x"), None))
    });
    assert!(missing.is_err());
    let blank = store.call("alice", |ctx| {
        patch_work_package(ctx, id, patch(Some(" "), None))
    });
    assert!(blank.is_err());
    store
        .call("alice", |ctx| {
            patch_work_package(ctx, id, patch(None, Some("the plan")))
        })
        .expect("a description change");

    let row = store.db().work_packages().id().find(id).expect("row");
    assert_eq!(row.title, "Window 1");
    assert_eq!(row.description, "the plan");
}

/// `work-packages.allium: UpdateWorkPackage`: a sort order is written when
/// given and left alone by a patch that omits it.
#[test]
fn patching_a_work_package_writes_the_sort_order_and_omitting_it_keeps_it() {
    let store = Store::new();
    let id = new_package(&store, "Window 1");
    assert_eq!(
        store.db().work_packages().id().find(id).expect("row").sort_order,
        None
    );

    store
        .call("alice", |ctx| {
            patch_work_package(
                ctx,
                id,
                WorkPackagePatch {
                    sort_order: Some(3),
                    ..Default::default()
                },
            )
        })
        .expect("a sort order change");
    store
        .call("alice", |ctx| {
            patch_work_package(
                ctx,
                id,
                WorkPackagePatch {
                    description: Some("plan".into()),
                    ..Default::default()
                },
            )
        })
        .expect("a description change");

    let row = store.db().work_packages().id().find(id).expect("row");
    assert_eq!((row.sort_order, row.description.as_str()), (Some(3), "plan"));
}

/// `work-packages.allium: LinkItem` / `UnlinkItem`: both idempotent; a link
/// to an unknown item, an unknown package or an unknown kind is refused.
#[test]
fn linking_is_idempotent_and_refuses_unknown_targets() {
    let store = Store::new();
    let package = new_package(&store, "Window 1");
    let epic = store.create_epic("alice", "E", false);
    let task = store.create("alice", epic_task("t", epic, "backlog"));

    for _ in 0..2 {
        link(&store, package, "task", task).expect("link a task");
    }
    link(&store, package, "epic", epic).expect("link an epic");
    assert_eq!(
        link_ids(&store).len(),
        2,
        "a repeated link must not duplicate"
    );

    for (pkg, kind, item) in [
        (package, "task", 99),
        (package, "epic", 99),
        (99, "task", task),
        (package, "board", task),
    ] {
        assert!(
            link(&store, pkg, kind, item).is_err(),
            "{pkg} {kind} {item}"
        );
    }

    for _ in 0..2 {
        store
            .call("alice", |ctx| {
                unlink_work_package_item(ctx, package, "task".into(), task)
            })
            .expect("unlink");
    }
    let left: Vec<String> = store
        .db()
        .work_package_links()
        .iter()
        .map(|l| l.kind)
        .collect();
    assert_eq!(left, vec!["epic".to_string()]);
}

/// `work-packages.allium: DeleteWorkPackage`: the package and its links go;
/// the linked items stay.
#[test]
fn deleting_a_work_package_removes_its_links_and_keeps_the_items() {
    let store = Store::new();
    let package = new_package(&store, "Window 1");
    let epic = store.create_epic("alice", "E", false);
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    link(&store, package, "task", task).expect("link");

    store
        .call("alice", |ctx| delete_work_package(ctx, package))
        .expect("delete_work_package");
    assert_eq!(store.db().work_packages().iter().count(), 0);
    assert!(link_ids(&store).is_empty());
    assert_eq!(store.task(task).title, "t");
}

/// `work-packages.allium: DeletingAnItemRemovesItsLinks`: deleting a task,
/// or an epic with its subtree, removes every link naming a deleted row, and
/// only those: a task and an epic sharing an id are different items.
#[test]
fn deleting_linked_items_removes_their_links_and_only_theirs() {
    let store = Store::new();
    let package = new_package(&store, "Window 1");
    let (epic_e, task_in_e) = done_epic(&store, "E");
    let epic_f = store.create_epic("alice", "F", false);
    let task_in_f = store.create("alice", epic_task("in F", epic_f, "done"));
    let kept = store.create("alice", epic_task("kept", epic_f, "backlog"));
    for (kind, id) in [
        ("epic", epic_e),
        ("task", task_in_e),
        ("task", task_in_f),
        ("task", kept),
        ("epic", epic_f),
    ] {
        link(&store, package, kind, id).expect("link");
    }

    store
        .call("alice", |ctx| delete_task(ctx, task_in_f))
        .expect("delete_task");
    store
        .call("alice", |ctx| delete_epic(ctx, epic_e))
        .expect("delete_epic");

    let mut left = link_ids(&store);
    left.sort();
    let mut want = vec![
        format!("{package}/task/{kept}"),
        format!("{package}/epic/{epic_f}"),
    ];
    want.sort();
    assert_eq!(left, want);
}

/// The batch path owes the same cascade (`tasks.allium: BatchDelete`).
#[test]
fn batch_delete_removes_the_links_of_everything_it_deletes() {
    let store = Store::new();
    let package = new_package(&store, "Window 1");
    let (epic_e, _) = done_epic(&store, "E");
    let (epic_f, task_in_f) = done_epic(&store, "F");
    link(&store, package, "epic", epic_e).expect("link");
    link(&store, package, "task", task_in_f).expect("link");

    store
        .call("alice", |ctx| {
            batch_delete(ctx, vec![task_in_f], vec![epic_e])
        })
        .expect("batch_delete");
    assert!(link_ids(&store).is_empty());
    assert!(store.db().epics().id().find(epic_f).is_some());
}
