//! Per-host settings (`docs/specs/settings.allium`), run against the
//! in-memory store. Twins of `tests/spacetime_module/settings.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn two_hosts() -> Store {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    store.register_host_for(HOST_B, "alice");
    store
}

fn save(store: &Store, host: &str, key: &str, value: &str) {
    store
        .call("alice", |ctx| {
            save_setting(ctx, host.into(), key.into(), value.into())
        })
        .expect("save_setting");
}

fn value_of(store: &Store, id: &str) -> Option<String> {
    store
        .db()
        .settings()
        .id()
        .find(id.to_string())
        .map(|s| s.value)
}

/// `save_setting` upserts by `(host, key)`, the same shape `register_host`
/// upserts by id.
#[test]
fn save_setting_upserts_by_host_and_key() {
    let store = two_hosts();
    save(&store, HOST_A, "theme", "dark");
    save(&store, HOST_A, "theme", "light");

    assert_eq!(store.db().settings().iter().count(), 1);
    assert_eq!(value_of(&store, "host-a/theme").as_deref(), Some("light"));
}

/// A setting written by one host does not affect, and is not returned as,
/// another host's row.
#[test]
fn a_setting_written_by_one_host_is_a_separate_row_from_anothers() {
    let store = two_hosts();
    save(&store, HOST_A, "theme", "dark");
    save(&store, HOST_B, "theme", "light");

    assert_eq!(store.db().settings().iter().count(), 2);
    assert_eq!(
        value_of(&store, "host-a/theme").as_deref(),
        Some("dark"),
        "host-a's own write must be unaffected by host-b's"
    );
    assert_eq!(value_of(&store, "host-b/theme").as_deref(), Some("light"));
}

#[test]
fn clear_setting_removes_the_row() {
    let store = two_hosts();
    save(&store, HOST_A, "theme", "dark");

    store
        .call("alice", |ctx| {
            clear_setting(ctx, HOST_A.into(), "theme".into())
        })
        .expect("clear_setting");

    assert_eq!(value_of(&store, "host-a/theme"), None);
}

/// Clearing a key that was never set is a no-op, not a refusal
/// (`settings.allium: ClearSetting`).
#[test]
fn clearing_an_unset_setting_is_a_no_op() {
    let store = two_hosts();
    store
        .call("alice", |ctx| {
            clear_setting(ctx, HOST_A.into(), "never-set".into())
        })
        .expect("clearing an unset key must succeed");
}

/// dispatch.allium: RecordBaseBranch, BranchHistoryCapped — `record_base_branch`
/// prunes the path's history to the most recently used
/// `MAX_BASE_BRANCHES_PER_REPO` rows, and touches no other path's rows.
#[test]
fn record_base_branch_prunes_the_paths_history_to_the_cap() {
    let store = two_hosts();
    let record = |path: &str, branch: &str, stamp: &str| {
        store
            .call("alice", |ctx| {
                record_base_branch(ctx, path.into(), branch.into(), stamp.into())
            })
            .expect("record_base_branch");
    };
    record("/other", "keep", "2026-01-01 00:00:00.000");
    for i in 0..12 {
        record(
            "/repo",
            &format!("b{i:02}"),
            &format!("2026-01-02 00:00:{i:02}.000"),
        );
    }
    // Re-recording an old branch makes it the most recent, so it survives.
    record("/repo", "b02", "2026-01-03 00:00:00.000");
    record("/repo", "b12", "2026-01-03 00:00:01.000");

    let mut kept: Vec<String> = store
        .db()
        .repo_base_branches()
        .iter()
        .filter(|r| r.repo_path == "/repo")
        .map(|r| r.branch)
        .collect();
    kept.sort();
    let mut expected: Vec<String> = (2..13)
        .filter(|i| *i != 3)
        .map(|i| format!("b{i:02}"))
        .collect();
    expected.sort();
    assert_eq!(kept, expected);
    assert_eq!(
        store
            .db()
            .repo_base_branches()
            .iter()
            .filter(|r| r.repo_path == "/other")
            .count(),
        1
    );
}
