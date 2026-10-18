//! The SpacetimeDB module publishes, and keeps publishing, without losing data.
//!
//! `spacetime/module/` is outside the workspace and compiles to wasm, so
//! nothing in `cargo test` normally touches it. These tests do: they stand up a
//! throwaway standalone instance and publish the real module into it.
//!
//! **What this covers that a unit test cannot.** Whether a schema change is
//! *automigratable* is a judgement only the store makes, and it is not obvious
//! from the source. Appending a column looks like the safe case and is still
//! refused without a `#[default(..)]` annotation; that refusal was found by
//! running this, not by reading the code. Every publish below passes
//! `--delete-data=never`, which is what turns "it migrated" into an assertion
//! rather than a hope: with it, a change the store cannot automigrate aborts
//! instead of quietly destroying the database and reporting success.
//!
//! **Skipped when `spacetime` is not on `PATH`, except under CI**, where a
//! missing CLI fails instead (`spacetime_available_or_skip`, the same rule as
//! `tests/tmux_harness/mod.rs`). CI's Test and Coverage jobs both install it
//! and hard-fail if the install doesn't land at the pinned version (see
//! `.github/workflows/ci.yml`'s "Pin and verify the spacetime CLI version"
//! step) — that is docs/specs/spacetime-memory-store.allium's
//! `ConformanceIsCiGated` guarantee. These tests also run under tarpaulin:
//! `spacetime_command` drops the coverage flags a module build would otherwise
//! inherit (task #4909), and this is the only place the SDK glue in
//! `src/sync/sdk_connector/` is measured.
//! The gate script `scripts/check-spacetime-module.sh` runs the parts that need
//! no server, and runs everywhere.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::spacetime_instance::{
    column, describe, instance_tests_should_run, is_transport_error, module_path, no_rows,
    spacetime_available_or_skip, spacetime_command, Instance, COVERAGE_ENV_VARS,
};
use dispatch_tui::process::{ProcessRunner, RealProcessRunner};
use dispatch_tui::sync::{SharedRows, SpacetimeSdkConnector, StoreConnector, SubscriptionRequest};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A wasm release build of the module runs inside `spacetime publish`, and a
/// cold one compiles the whole `spacetimedb` crate tree.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(600);

/// A build directory for the scratch module that OUTLIVES the test.
///
/// The scratch copy lives in a tempdir, so without this every run compiles the
/// whole dependency tree for wasm in release and then deletes the result.
/// Pointing it at the module's own (gitignored) target tree means a second run
/// recompiles only the module crate, because the committed and working-tree
/// manifests have identical dependencies in the normal case.
///
/// Created here rather than left to the builder: cargo writes its lock and
/// temp files directly into the directory it is handed and does not make one,
/// so an absent path fails the publish with a bare `No such file or directory`
/// naming a random temp sibling — which reads as anything but a missing target
/// dir.
fn scratch_target_dir() -> PathBuf {
    let dir = module_path().join("target").join("committed");
    std::fs::create_dir_all(&dir).expect("scratch target dir");
    dir
}

/// The module as the last commit has it, unpacked into a scratch directory.
///
/// `git show` rather than a checked-in copy so the fixture cannot go stale: it
/// is always the previous shape, whatever that currently is. Once this branch
/// is committed the two sides are identical and the test degrades into the
/// republish case — which is the right behaviour, because from then on it
/// guards the *next* change.
fn committed_module(into: &Path) -> PathBuf {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let git = |args: &[&str]| {
        let mut argv = vec!["-C".to_string(), repo_root.display().to_string()];
        argv.extend(args.iter().map(|a| a.to_string()));
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        // `git -C <dir>` rather than a cwd on the child: it is the form the
        // rest of the repo uses (`src/git.rs`, `src/repo_sync.rs`), and
        // `run_with_timeout` has no argument for a working directory.
        let out = RealProcessRunner::default()
            .run_with_timeout("git", &argv, PUBLISH_TIMEOUT)
            .expect("git");
        assert!(out.status.success(), "{}", describe(&out));
        out.stdout
    };
    // Every tracked source file, not just `lib.rs`: the module is split by
    // domain, so the committed shape is a directory.
    let listing = git(&[
        "ls-tree",
        "-r",
        "--name-only",
        "HEAD",
        "spacetime/module/src",
    ]);
    let mut tracked: Vec<(String, String)> = String::from_utf8_lossy(&listing)
        .lines()
        .map(|path| {
            let rel = path.trim_start_matches("spacetime/module/");
            (path.to_string(), rel.to_string())
        })
        .collect();
    // The lockfile too: without it cargo resolves every dependency afresh, and
    // a newer `spacetimedb` can need a newer rustc than the pinned toolchain.
    for file in ["Cargo.toml", "Cargo.lock"] {
        tracked.push((format!("spacetime/module/{file}"), file.to_string()));
    }
    for (tracked, dest) in tracked {
        let bytes = git(&["show", &format!("HEAD:{tracked}")]);
        let dest = into.join(dest);
        // The module's sources nest (`src/tests/`), so make each file's own
        // directory, not just `src/`.
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).expect("scratch module subdir");
        }
        std::fs::write(dest, bytes).expect("write scratch module file");
    }
    into.to_path_buf()
}

/// The `schema_version` row, as `[id, version, module_version]`.
fn schema_version_row(instance: &Instance) -> Vec<i64> {
    let json = instance.sql("SELECT * FROM schema_version");
    let line = json
        .lines()
        .find(|l| l.trim_start().starts_with('['))
        .unwrap_or_else(|| panic!("no JSON in sql output:\n{json}"));
    let parsed: serde_json::Value = serde_json::from_str(line).expect("parse sql output");
    parsed[0]["rows"][0]
        .as_array()
        .unwrap_or_else(|| panic!("no schema_version row in {parsed}"))
        .iter()
        .map(|v| v.as_i64().expect("integer column"))
        .collect()
}

#[test]
fn a_missing_cli_is_a_skip_outside_ci_only() {
    assert!(instance_tests_should_run(true, false));
    assert!(instance_tests_should_run(true, true));
    assert!(!instance_tests_should_run(false, false));
}

/// Under CI a skip would be a silent pass, so a missing CLI fails instead.
#[test]
#[should_panic(expected = "spacetime is required in CI")]
fn a_missing_cli_is_a_failure_in_ci() {
    instance_tests_should_run(false, true);
}

/// Tarpaulin builds the tests with `RUSTFLAGS=-Cinstrument-coverage`. Inherited
/// by the wasm build `spacetime publish -p` runs, that flag fails it with
/// "can't find crate for `profiler_builtins`".
#[test]
fn the_cli_does_not_inherit_the_tests_coverage_flags() {
    let command = spacetime_command(None, &["--version"]);
    let removed: Vec<_> = command
        .get_envs()
        .filter(|(_, value)| value.is_none())
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .collect();
    for var in COVERAGE_ENV_VARS {
        assert!(
            removed.iter().any(|r| r == var),
            "{var} not removed: {removed:?}"
        );
    }
}

#[test]
fn only_transport_errors_are_retried() {
    assert!(is_transport_error(
        "Error: Connection reset by peer (os error 104)"
    ));
    assert!(is_transport_error(
        "error sending request: connection refused"
    ));
    // The `sql` flake of 2026-10-08: the request never got a response.
    assert!(is_transport_error(
        "Error: error sending request for url (http://127.0.0.1:40123/v1/database/db/sql): connection error"
    ));
    assert!(!is_transport_error(
        "Error: no such table: `nope`. Please check the table name"
    ));
    assert!(!is_transport_error(
        "Error: schema migration refused: column removed"
    ));
    assert!(!is_transport_error(""));
}

/// An empty patch, to be filled by the caller. Every field named, because a
/// missing one is a deserialisation failure rather than a `None`.
fn empty_task_patch() -> serde_json::Value {
    let mut patch = serde_json::Map::new();
    for field in [
        "title",
        "description",
        "repo_path",
        "status",
        "worktree",
        "tmux_window",
        "plan_path",
        "epic_id",
        "sub_status",
        "tag",
        "sort_order",
        "base_branch",
        "external_id",
        "labels",
        "last_pre_tool_use_at",
        "last_notification_at",
        "wrap_up_mode",
        "url",
        "url_type",
        "pr_learnings_gate_shown_at",
        "auto_run_plan",
        "live_subagents",
        "stop_pending",
        "stop_pending_at",
        "last_peer_message_sent_at",
        "last_peer_message_received_at",
        "phoenix",
        "host",
        "owner",
        "completed_at",
    ] {
        patch.insert(field.into(), serde_json::json!({"none": []}));
    }
    serde_json::Value::Object(patch)
}

/// A blank epic row, as `seed_epics` and `create_epic` both take it.
fn epic_json(id: i64, title: &str, status: &str, parent: i64) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": title,
        "description": "",
        "status": status,
        "plan_path": "",
        "sort_order": {"none": []},
        "created_at": "2026-09-19 10:00:00",
        "updated_at": "2026-09-19 10:00:00",
        "auto_dispatch": false,
        "parent_epic_id": parent,
        "feed_command": "",
        "feed_interval_secs": 0,
        "group_by_repo": false,
        "feed_role": "none",
        "origin": "manual",
        "feed_append_only": false,
        "completed_at": "",
        "created_by": "",
    })
}

/// A patch that sets one string column.
fn patch_setting(field: &str, value: &str) -> serde_json::Value {
    let mut patch = empty_task_patch();
    patch[field] = serde_json::json!({"some": value});
    patch
}

fn published_instance() -> Instance {
    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));
    instance
}

/// A `learnings` row, JSON-encoded for `seed_learnings`'s CLI call — a real
/// id and an explicit `source_task_id`, unlike `learning_json` in `subscription.rs` (which
/// `create_learning` always overwrites to 0/none on the way in).
fn seeded_learning_json(id: i64, summary: &str, source_task_id: Option<i64>) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "kind": "convention",
        "summary": summary,
        "detail": {"none": []},
        "scope": "user",
        "scope_ref": {"none": []},
        "tags": "[]",
        "status": "approved",
        "source_task_id": match source_task_id {
            Some(t) => serde_json::json!({"some": t}),
            None => serde_json::json!({"none": []}),
        },
        "upvote_count": 0,
        "last_upvoted_at": {"none": []},
        "created_at": "2026-09-19 10:00:00",
        "updated_at": "2026-09-19 10:00:00",
        "embedding": {"none": []},
    })
}

/// A task row owned by `host`, in `epic`, at `status`.
///
/// `host` is what makes these tests about two machines rather than about two
/// calls: a task's host is the machine holding its worktree, and it is the only
/// thing in the row that says which board's work it is.
fn task_json(id: i64, title: &str, status: &str, epic: i64, host: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": title,
        "description": "",
        "repo_path": "/repo",
        "status": status,
        "worktree": "",
        "tmux_window": "",
        "plan_path": "",
        "epic_id": epic,
        "sub_status": "none",
        "tag": "",
        "sort_order": {"none": []},
        "created_at": "2026-09-19 10:00:00",
        "updated_at": "2026-09-19 10:00:00",
        "base_branch": "main",
        "external_id": "",
        "labels": "[]",
        "last_pre_tool_use_at": "",
        "last_notification_at": "",
        "wrap_up_mode": "",
        "url": "",
        "url_type": "",
        "pr_learnings_gate_shown_at": "",
        "auto_run_plan": false,
        "live_subagents": 0,
        "stop_pending": false,
        "stop_pending_at": "",
        "live_shells": 0,
        "oldest_live_shell_started_at": "",
        "last_peer_message_sent_at": "",
        "last_peer_message_received_at": "",
        "phoenix": false,
        "host": host,
        "owner": "",
        "completed_at": "",
        "created_by": "",
        "alias": "",
    })
}

/// `host-a` and `host-b`, both owned by the identity this instance calls as,
/// so the calls below may act for either (`store-split.allium`, D11).
fn register_own_hosts(instance: &Instance) {
    let me = instance.identity();
    register_host(instance, "host-a", &me);
    register_host(instance, "host-b", &me);
}

/// A host owned by a SECOND identity, "teammate" — somebody else's machine,
/// registered by the teammate as theirs. The teammate's CLI logs in on its
/// first call, so a throwaway call (refused: a new host must name its owner)
/// comes first to make their identity known. Returns the teammate's identity.
fn register_teammates_host(instance: &Instance, host: &str) -> String {
    instance.call_as("teammate", "register_host", &["scratch", "", ""]);
    let them = instance.identity_as("teammate");
    let owned = instance.call_as("teammate", "register_host", &[host, "", &them]);
    assert!(owned.status.success(), "{}", describe(&owned));
    them
}

/// A running task with no live subagents or shells and no deferred Stop —
/// the ordinary "task exists" seed these tests build on.
fn running_task(instance: &Instance, id: i64) {
    let mut row = task_json(id, "t", "running", 0, "");
    row["owner"] = serde_json::json!("user-me");
    // The hook facts are the current agent's (agents A4), and a task with a
    // worktree gets one from the backfill.
    row["worktree"] = serde_json::json!(format!("/wt/{id}-t"));
    let seeded = instance.call("seed_tasks", &[&serde_json::json!([row]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
    backfill_agents(instance);
}

/// `BackfillCurrentAgents`: a current agent for every task with a worktree and
/// none, taking the session facts a seeded Task row still carries.
fn backfill_agents(instance: &Instance) {
    let backfilled = instance.call("backfill_agents", &[]);
    assert!(backfilled.status.success(), "{}", describe(&backfilled));
}

/// A feed item, already resolved the way the client resolves one before
/// sending — see `FeedTaskUpsertItem`'s doc comment in the module.
/// Wrap plain feed items as `ingest_feed` takes them: no signals.
fn ingest_items(items: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Array(
        items
            .as_array()
            .expect("feed items are an array")
            .iter()
            .map(|item| serde_json::json!({ "item": item, "signals": [] }))
            .collect(),
    )
}

fn feed_item_json(external_id: &str, title: &str, status: &str) -> serde_json::Value {
    serde_json::json!({
        "external_id": external_id,
        "title": title,
        "description": "",
        "repo_path": "/repo",
        "status": status,
        "sub_status": "none",
        "base_branch": "main",
        "tag": "chore",
        "labels": "[]",
        "sort_order": {"none": []},
        "url": "",
        "url_type": "",
        "wrap_up_mode": "",
    })
}

fn current_agent_column(instance: &Instance, field: &str, task: i64) -> String {
    column(
        instance,
        &format!("SELECT {field} FROM agents WHERE task_id = {task} AND state = 'current'"),
    )
}

fn task_column(instance: &Instance, field: &str, task: i64) -> String {
    column(
        instance,
        &format!("SELECT {field} FROM tasks WHERE id = {task}"),
    )
}

/// Register `host` in the host registry with `owner`, which a new host must
/// name (`store-split.allium`, D11).
fn register_host(instance: &Instance, host: &str, owner: &str) {
    let registered = instance.call("register_host", &[host, "", owner]);
    assert!(registered.status.success(), "{}", describe(&registered));
}

#[path = "spacetime_module/agent_state.rs"]
mod agent_state;
#[path = "spacetime_module/agents.rs"]
mod agents;
#[path = "spacetime_module/data_rules.rs"]
mod data_rules;
#[path = "spacetime_module/feeds.rs"]
mod feeds;
#[path = "spacetime_module/mutations.rs"]
mod mutations;
#[path = "spacetime_module/poll_owner.rs"]
mod poll_owner;
#[path = "spacetime_module/publish.rs"]
mod publish;
#[path = "spacetime_module/settings.rs"]
mod settings;
#[path = "spacetime_module/subscription.rs"]
mod subscription;
