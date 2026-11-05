//! Data rules the reducers enforce, run against the in-memory store. Twins of
//! the live-store cases in `tests/spacetime_module/data_rules.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// `mcp-task-tools.allium: UpdateTaskViaMcp`, enforced where the rows are.
#[test]
fn an_illegal_sub_status_is_refused_at_the_reducer_with_the_old_wording() {
    let store = Store::new();
    let epic = store.create_epic("alice", "epic", false);
    let id = store.create("alice", epic_task("task", epic, "backlog"));

    let bad = TaskPatch {
        sub_status: Some("active".into()),
        ..TaskPatch::default()
    };
    let why = store
        .call("alice", |ctx| patch_task(ctx, id, bad))
        .expect_err("active is not a backlog sub-status");
    assert!(
        why.contains("sub_status 'active' is not valid for status 'backlog'"),
        "{why}"
    );

    // Judged against the status the patch moves to, not the one the row has.
    let both = TaskPatch {
        sub_status: Some("active".into()),
        status: Some("running".into()),
        ..TaskPatch::default()
    };
    store
        .call("alice", |ctx| patch_task(ctx, id, both))
        .expect("running accepts active");
}

/// `epics.allium: UpdateEpicViaMcp` / `ReparentEpicFromTui`.
#[test]
fn a_parent_cycle_is_refused_at_the_reducer() {
    let store = Store::new();
    let a = store.create_epic("alice", "a", false);
    let b = store
        .call("alice", |ctx| {
            create_epic(
                ctx,
                Epic {
                    title: "b".into(),
                    parent_epic_id: a,
                    workspace_id: TEST_WORKSPACE,
                    ..blank_epic()
                },
            )
        })
        .map(|()| store.db().epics().iter().map(|e| e.id).max().expect("epic"))
        .expect("create_epic");

    let patch = EpicPatch {
        parent_epic_id: Some(b),
        ..EpicPatch::default()
    };
    let why = store
        .call("alice", |ctx| patch_epic(ctx, a, patch))
        .expect_err("a cycle is refused");
    assert!(
        why.contains("would create a cycle in the epic hierarchy"),
        "{why}"
    );
}

/// `epics.allium`'s routing note: the store places a task of a grouped epic.
#[test]
fn a_task_created_in_a_grouped_epic_lands_in_its_repo_sub_epic() {
    let store = Store::new();
    let root = store
        .call("alice", |ctx| {
            create_epic(
                ctx,
                Epic {
                    title: "root".into(),
                    group_by_repo: true,
                    workspace_id: TEST_WORKSPACE,
                    ..blank_epic()
                },
            )
        })
        .map(|()| store.db().epics().iter().map(|e| e.id).max().expect("epic"))
        .expect("create_epic");
    let task = Task {
        repo_path: "/x/alpha".into(),
        ..epic_task("task", root, "backlog")
    };
    let id = store.create("alice", task);

    let sub = store
        .db()
        .epics()
        .iter()
        .find(|e| e.origin == "repo-group")
        .expect("a sub-epic for the task's repo");
    assert!(sub.title.contains("alpha"), "{}", sub.title);
    assert_eq!(sub.parent_epic_id, root);
    assert_eq!(
        store.task(id).epic_id,
        sub.id,
        "in the sub-epic, not the root"
    );
}
fn repo_rows(store: &Store, path: &str) -> Vec<(String, String, String)> {
    let mut rows: Vec<_> = store
        .db()
        .repo_paths()
        .iter()
        .filter(|r| r.path == path)
        .map(|r| (r.host, r.github_path, r.verify_command))
        .collect();
    rows.sort();
    rows
}

fn save(store: &Store, path: &str, host: &str, github: &str) {
    store
        .call("u", |ctx| {
            save_repo_path(
                ctx,
                path.into(),
                "t".into(),
                host.into(),
                github.into(),
            )
        })
        .expect("save");
}

/// `core.allium: SavedRepoPath` — a path is one row per (path, host): a second
/// host saving the same path gets its own row and the first keeps its own.
#[test]
fn two_hosts_saving_one_path_each_own_a_row() {
    let store = Store::new();
    save(&store, "/repo", "host-a", "org/api");
    save(&store, "/repo", "host-b", "org/api");
    let hosts: Vec<String> = repo_rows(&store, "/repo").into_iter().map(|r| r.0).collect();
    assert_eq!(hosts, vec!["host-a", "host-b"]);
}

/// Saving again touches the host's own row; it does not add a second one.
#[test]
fn saving_twice_from_one_host_keeps_one_row() {
    let store = Store::new();
    save(&store, "/repo", "host-a", "org/api");
    save(&store, "/repo", "host-a", "org/api");
    assert_eq!(repo_rows(&store, "/repo").len(), 1);
}

/// A save by a host adopts a row no one owns instead of adding a second one.
#[test]
fn saving_a_path_adopts_a_row_that_has_no_host() {
    let store = Store::new();
    save(&store, "/repo", "", "");
    save(&store, "/repo", "host-b", "org/api");
    assert_eq!(
        repo_rows(&store, "/repo"),
        vec![("host-b".into(), "org/api".into(), String::new())]
    );
}

/// `core.allium: SavedRepoPath.github_path` — the verify command belongs to the
/// repository: setting it through one host's path reaches every row that
/// shares the GitHub path, even at a different local path.
#[test]
fn the_verify_command_is_shared_across_rows_with_one_github_path() {
    let store = Store::new();
    save(&store, "/home/a/api", "host-a", "org/api");
    save(&store, "/home/b/code/api", "host-b", "org/api");
    save(&store, "/home/a/other", "host-a", "org/other");
    store
        .call("u", |ctx| {
            set_verify_command(ctx, "/home/a/api".into(), "make ci".into())
        })
        .expect("set");
    assert_eq!(repo_rows(&store, "/home/b/code/api")[0].2, "make ci");
    assert_eq!(repo_rows(&store, "/home/a/other")[0].2, "");
}

/// A host that saves the repo later starts with the command already set.
#[test]
fn a_new_row_starts_with_its_repositorys_verify_command() {
    let store = Store::new();
    save(&store, "/home/a/api", "host-a", "org/api");
    store
        .call("u", |ctx| {
            set_verify_command(ctx, "/home/a/api".into(), "make ci".into())
        })
        .expect("set");
    save(&store, "/home/b/api", "host-b", "org/api");
    assert_eq!(repo_rows(&store, "/home/b/api")[0].2, "make ci");
}

/// Without a GitHub path the path itself is the repository.
#[test]
fn a_row_without_a_github_path_shares_only_with_its_own_path() {
    let store = Store::new();
    save(&store, "/repo", "host-a", "");
    save(&store, "/repo", "host-b", "");
    save(&store, "/elsewhere", "host-a", "");
    store
        .call("u", |ctx| {
            set_verify_command(ctx, "/repo".into(), "make".into())
        })
        .expect("set");
    assert!(repo_rows(&store, "/repo").iter().all(|r| r.2 == "make"));
    assert_eq!(repo_rows(&store, "/elsewhere")[0].2, "");
}

/// `cli.allium: PruneRepoPathsViaCli` — adoption stamps only an unowned row.
#[test]
fn adopting_a_path_stamps_only_an_unowned_row() {
    let store = Store::new();
    save(&store, "/legacy", "", "");
    save(&store, "/owned", "host-a", "");
    for p in ["/legacy", "/owned", "/missing"] {
        store
            .call("bob", |ctx| adopt_repo_path(ctx, p.into(), "host-b".into()))
            .expect("adopt never fails");
    }
    assert_eq!(repo_rows(&store, "/legacy")[0].0, "host-b");
    assert_eq!(repo_rows(&store, "/owned")[0].0, "host-a");
    assert!(repo_rows(&store, "/missing").is_empty());
}

/// A delete removes the caller's row or an unowned one, and refuses a path only
/// another host has, naming that host.
#[test]
fn deleting_a_path_removes_own_or_unowned_rows_and_refuses_other_hosts() {
    let store = Store::new();
    save(&store, "/repo", "host-a", "");
    save(&store, "/legacy", "", "");

    let err = store
        .call("bob", |ctx| {
            delete_repo_path(ctx, "/repo".into(), "host-b".into())
        })
        .expect_err("another host's row is refused");
    assert!(err.to_string().contains("host-a"), "{err}");
    assert_eq!(repo_rows(&store, "/repo").len(), 1);

    store
        .call("bob", |ctx| {
            delete_repo_path(ctx, "/legacy".into(), "host-b".into())
        })
        .expect("an unowned row may be removed");
    assert!(repo_rows(&store, "/legacy").is_empty());

    store
        .call("alice", |ctx| {
            delete_repo_path(ctx, "/repo".into(), "host-a".into())
        })
        .expect("owner delete");
    assert!(repo_rows(&store, "/repo").is_empty());
}

/// Deleting one host's row leaves the other host's row of the same path.
#[test]
fn deleting_a_path_leaves_another_hosts_row_of_it() {
    let store = Store::new();
    save(&store, "/repo", "host-a", "");
    save(&store, "/repo", "host-b", "");
    store
        .call("alice", |ctx| {
            delete_repo_path(ctx, "/repo".into(), "host-a".into())
        })
        .expect("delete");
    assert_eq!(repo_rows(&store, "/repo")[0].0, "host-b");
}

/// A path nobody has saved is not an error (a stale view, as before).
#[test]
fn deleting_an_unknown_path_is_ok() {
    let store = Store::new();
    store
        .call("alice", |ctx| {
            delete_repo_path(ctx, "/nope".into(), "host-a".into())
        })
        .expect("nothing to delete");
}
