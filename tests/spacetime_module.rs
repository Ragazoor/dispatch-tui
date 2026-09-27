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
//! **Skipped when `spacetime` is not on `PATH`.** CI's Test job installs it
//! and hard-fails the job if the install doesn't land at the pinned version
//! (see `.github/workflows/ci.yml`'s "Pin and verify the spacetime CLI
//! version" step) — that is docs/specs/spacetime-memory-store.allium's
//! `ConformanceIsCiGated` guarantee. The Coverage job deliberately does not
//! install it (tarpaulin conflicts with `spacetime publish`, task #4909), so
//! this test still takes its skip there. Unlike tmux, this file's own
//! availability check has no hard-fail arm of its own — the enforcement
//! lives in that earlier CI step rather than in this test — see
//! `tests/tmux_harness/mod.rs` for the pattern this deliberately departs from.
//! The gate script `scripts/check-spacetime-module.sh` runs the parts that need
//! no server, and runs everywhere.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::spacetime_instance::{
    column, describe, module_path, no_rows, spacetime_available_or_skip, Instance,
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
    std::fs::create_dir_all(into.join("src")).expect("scratch module dir");
    for (tracked, dest) in [
        ("spacetime/module/Cargo.toml", "Cargo.toml"),
        ("spacetime/module/src/lib.rs", "src/lib.rs"),
    ] {
        // `git -C <dir>` rather than a cwd on the child: it is the form the rest
        // of the repo uses (`src/git.rs`, `src/repo_sync.rs`), and
        // `run_with_timeout` has no argument for a working directory.
        let out = RealProcessRunner::default()
            .run_with_timeout(
                "git",
                &[
                    "-C",
                    &repo_root.display().to_string(),
                    "show",
                    &format!("HEAD:{tracked}"),
                ],
                PUBLISH_TIMEOUT,
            )
            .expect("git show");
        assert!(out.status.success(), "{}", describe(&out));
        std::fs::write(into.join(dest), &out.stdout).expect("write scratch module file");
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

/// Publishing the same module over itself is accepted and changes nothing.
#[test]
fn publishing_the_module_twice_automigrates() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let module = module_path();

    let first = instance.publish(&module, None);
    assert!(
        first.status.success(),
        "first publish: {}",
        describe(&first)
    );

    let second = instance.publish(&module, None);
    assert!(
        second.status.success(),
        "republishing an unchanged module must automigrate: {}",
        describe(&second)
    );
}

/// The shape in the last commit migrates into the shape in the working tree,
/// without destroying data.
///
/// This is the one that catches a column added anywhere but the end. It is also
/// what caught `#[default(..)]` being required on an appended column, which
/// nothing in the source hints at.
#[test]
fn the_committed_module_automigrates_into_the_working_tree_module() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let scratch = tempfile::tempdir().expect("temp dir");
    let previous = committed_module(scratch.path());

    let first = instance.publish(&previous, Some(&scratch_target_dir()));
    assert!(
        first.status.success(),
        "publishing the committed module: {}",
        describe(&first)
    );
    let before = schema_version_row(&instance);
    assert_eq!(before[0], 1, "init stamps exactly one schema_version row");

    let migrated = instance.publish(&module_path(), None);
    assert!(
        migrated.status.success(),
        "the working tree's module must automigrate over the committed one, \
         without --delete-data. A column added anywhere but the end fails here: {}",
        describe(&migrated)
    );

    // The row survived rather than being recreated. A publish does not re-run
    // `init`, so the SQLite version coming back unchanged is what says the
    // database was migrated rather than dropped and rebuilt.
    let after = schema_version_row(&instance);
    assert_eq!(
        after[1], before[1],
        "the SQLite schema version must survive the migration untouched"
    );

    // Re-stamping is what moves `module_version` off its migration default, and
    // it is a separate step from publishing on purpose: a publish cannot run
    // `init`, so nothing else would ever update it.
    let restamp = instance.call("set_schema_version", &["97"]);
    assert!(restamp.status.success(), "{}", describe(&restamp));
    let stamped = schema_version_row(&instance);
    assert_eq!(
        stamped[2], 1,
        "re-stamping must record the running module's own version"
    );
}

/// What a cold start costs: process start to a subscription the board could
/// draw from.
///
/// **This is a measurement, not a gate.** The number is what Phase 4 of the
/// migration plan was asked to produce, and it is recorded in the design doc;
/// the assertion here is a ceiling so loose that only a genuine regression —
/// a hang, a retry storm, a synchronous round trip that was not there before —
/// can trip it. A tight bound would fail on a loaded CI box and teach people to
/// ignore it.
///
/// It lives beside the module harness because that is where a real standalone
/// instance already exists, and a measurement against a fake would measure the
/// fake.
///
/// **What it deliberately does NOT measure**: time to the board drawing. The
/// board draws before the connection opens (`sync.allium:
/// OpenBoardConnection`), so that number is unchanged by any of this, and
/// conflating the two is how a design that costs nothing gets reported as
/// costing a round trip.
#[test]
fn a_cold_start_reaches_a_live_subscription_promptly() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(
        published.status.success(),
        "publishing for the cold-start measurement: {}",
        describe(&published)
    );

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (connect, subscribe, identity) = runtime.block_on(async {
        let connector =
            SpacetimeSdkConnector::new(instance.database(), Arc::new(SharedRows::new()));

        // allow-test-sleep: this test's entire purpose is to measure elapsed
        // time against a real server. It asserts a loose ceiling, not a
        // duration, and removing the measurement removes the test.
        let started = Instant::now();
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("cold-start connect: {e}"));
        // allow-test-sleep: see above.
        let connect = started.elapsed();

        // allow-test-sleep: see above.
        let before_subscribe = Instant::now();
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-cold-start",
            ))
            .await
            .unwrap_or_else(|e| panic!("cold-start subscribe: {e}"));
        // allow-test-sleep: see above.
        (connect, before_subscribe.elapsed(), accepted.identity)
    });

    // Printed rather than only asserted: the number is the deliverable, and
    // `cargo test -- --nocapture` is how it is read again later.
    println!(
        "cold start: connect {connect:?}, subscribe {subscribe:?}, \
         total {:?} (identity {identity})",
        connect + subscribe
    );

    assert!(
        connect + subscribe < COLD_START_CEILING,
        "a cold start took {:?}, past the {COLD_START_CEILING:?} ceiling — \
         this is a regression, not a slow machine",
        connect + subscribe
    );
}

/// The loose ceiling for the cold-start measurement above.
///
/// Set well above anything a healthy run produces, because the failure worth
/// catching is a hang or a retry storm rather than a hundred milliseconds of
/// drift. The real numbers live in the migration design doc.
const COLD_START_CEILING: Duration = Duration::from_secs(10);

/// **Test 2 of Phase 5, against a real server.** A row somebody else writes
/// reaches this board's `SharedRows` without anything here asking for it.
///
/// The in-process tests drive `SharedRows` directly, which proves the decoding
/// and the wake-up but not that the SDK ever calls it. This one closes that
/// gap: the row is written through the CLI — a different process, standing in
/// for a teammate's board — and the only thing connecting the two is the
/// subscription.
#[test]
fn a_row_written_elsewhere_arrives_through_the_subscription() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows = Arc::new(SharedRows::new());
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    runtime.block_on(async {
        let connector = SpacetimeSdkConnector::new(instance.database(), rows.clone());
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![1],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));

        assert!(
            rows.epics().is_empty(),
            "nothing has been written yet, so nothing may have arrived"
        );

        let mut woken = rows.changed();
        woken.mark_unchanged();

        // A different process writes the row. Nothing below asks for it.
        let seeded = instance.call(
            "seed_epics",
            &[&serde_json::json!([{
                "id": 1,
                "title": "Written by somebody else",
                "description": "",
                "status": "backlog",
                "plan_path": "",
                "sort_order": {"none": []},
                "created_at": "2026-09-19 10:00:00",
                "updated_at": "2026-09-19 10:00:00",
                "auto_dispatch": false,
                "parent_epic_id": 0,
                "feed_command": "",
                "feed_interval_secs": 0,
                "group_by_repo": false,
                "feed_role": "none",
                "origin": "manual",
                "feed_append_only": false,
                "completed_at": "",
                "created_by": "",
            }])
            .to_string()],
        );
        assert!(seeded.status.success(), "{}", describe(&seeded));

        // No sleep and no poll: the wake-up is the assertion. If the row never
        // arrives this hangs and the harness kills it, which is louder than a
        // slept-through comparison.
        woken
            .changed()
            .await
            .expect("the subscription must deliver");

        let epics = rows.epics();
        assert_eq!(epics.len(), 1);
        assert_eq!(epics[0].title, "Written by somebody else");
    });
}

/// **Task #4927**: following an epic brings its whole sub-epic tree, at any
/// depth, and keeps bringing it as the tree grows.
///
/// `sync.allium: ASubEpicOfAFollowedEpicIsAskedForToo`. Two sub-epic levels
/// exist before the board subscribes, which proves the walk on a cold start;
/// a third level and a task inside it are written by another process once the
/// board is up, which proves it live. Nothing in the ask names epics 2, 3 or 4
/// — the board only follows 1.
#[test]
fn a_followed_epics_whole_sub_epic_tree_arrives_live() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([
            epic_json(1, "Followed", "backlog", 0),
            epic_json(2, "Child", "backlog", 1),
            epic_json(3, "Grandchild", "backlog", 2),
            epic_json(9, "Unfollowed", "backlog", 0),
        ])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let rows = Arc::new(SharedRows::new());
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    runtime.block_on(async {
        let connector = SpacetimeSdkConnector::new(instance.database(), rows.clone());
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![1],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));

        // The loop checks before it waits, so a change that landed before a
        // wait began is never missed and nothing needs marking as seen.
        let woken = rows.changed();
        let epic_ids = |rows: &SharedRows| {
            let mut ids: Vec<i64> = rows.epics().iter().map(|e| e.id.0).collect();
            ids.sort_unstable();
            ids
        };
        // Bounded by a timeout rather than a sleep: each widening is a round
        // trip, and the wait ends the moment the rows are there.
        let wait_for = |want: fn(&SharedRows) -> bool| {
            let rows = rows.clone();
            let mut woken = woken.clone();
            async move {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while !want(&rows) {
                        woken
                            .changed()
                            .await
                            .expect("the subscription must deliver");
                    }
                })
                .await
            }
        };

        wait_for(|rows| rows.epics().len() == 3)
            .await
            .unwrap_or_else(|_| {
                panic!("cold start: the tree never arrived: {:?}", epic_ids(&rows))
            });
        assert_eq!(epic_ids(&rows), vec![1, 2, 3], "epic 9 is not followed");

        let seeded = instance.call(
            "seed_epics",
            &[&serde_json::json!([epic_json(4, "Great-grandchild", "backlog", 3)]).to_string()],
        );
        assert!(seeded.status.success(), "{}", describe(&seeded));
        let seeded = instance.call(
            "seed_tasks",
            &[
                &serde_json::json!([task_json(1, "deep inside", "backlog", 4, "host-b")])
                    .to_string(),
            ],
        );
        assert!(seeded.status.success(), "{}", describe(&seeded));

        wait_for(|rows| rows.epics().len() == 4 && rows.tasks().len() == 1)
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "live: the new level never arrived: epics {:?}, tasks {}",
                    epic_ids(&rows),
                    rows.tasks().len()
                )
            });
        assert_eq!(epic_ids(&rows), vec![1, 2, 3, 4]);
        assert_eq!(rows.tasks()[0].title, "deep inside");
    });
}

/// **Tests 1 and 2 of Phase 10 (task #4914), against a real server.** A
/// learning recorded on one host's board is retrievable and RAG-ranked from
/// another's — and `rag_rank_learnings` produces the same ranking over rows
/// sourced from SpacetimeDB that it does over SQLite rows, because the
/// ranking algorithm itself never changed; only where the rows came from did.
///
/// Host A is the `create_learning` reducer call — a different process,
/// standing in for a teammate's board, the same convention
/// `a_row_written_elsewhere_arrives_through_the_subscription` uses. Host B is
/// this test's own `SharedRows`, subscribed with nothing followed: learnings
/// need no `epic_id` to see one, because they are unconditionally subscribed
/// (`docs/specs/learnings.allium`'s Storage Backend section).
#[test]
fn a_learning_recorded_elsewhere_is_retrievable_and_rag_ranked_from_here() {
    if !spacetime_available_or_skip() {
        return;
    }
    use dispatch_tui::db::LearningFilter;
    use dispatch_tui::service::embeddings::{
        rag_rank_learnings, serialize_embedding, RagRankParams,
    };

    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    // The query direction, and two candidate embeddings: one nearly parallel
    // to the query (high cosine) and one mostly orthogonal (low cosine) — the
    // same shape `rag_rank_learnings_orders_by_score` uses in-process, so a
    // pass here is the same property proven over the real transport.
    let query = [1.0f32, 0.0, 0.0];
    let high_sim_bytes = serialize_embedding(&[1.0, 0.0, 0.0]);
    let low_sim_bytes = serialize_embedding(&[0.26, 0.97, 0.0]);

    let rows = Arc::new(SharedRows::new());
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    runtime.block_on(async {
        let connector = SpacetimeSdkConnector::new(instance.database(), rows.clone());
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-b",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));

        assert!(
            rows.learnings_matching(&LearningFilter::default())
                .is_empty(),
            "nothing has been written yet, so nothing may have arrived"
        );

        let mut woken = rows.changed();
        woken.mark_unchanged();

        // Host A records both learnings directly through the reducer.
        let created = instance.call(
            "create_learning",
            &[&learning_json(
                "High similarity",
                "user",
                &serde_json::json!({"none": []}),
                &high_sim_bytes,
            )
            .to_string()],
        );
        assert!(created.status.success(), "{}", describe(&created));

        woken
            .changed()
            .await
            .expect("the subscription must deliver the first learning");

        let mut woken = rows.changed();
        woken.mark_unchanged();
        let created = instance.call(
            "create_learning",
            &[&learning_json(
                "Low similarity",
                "repo",
                &serde_json::json!({"some": "/repo/a"}),
                &low_sim_bytes,
            )
            .to_string()],
        );
        assert!(created.status.success(), "{}", describe(&created));

        woken
            .changed()
            .await
            .expect("the subscription must deliver the second learning");

        // Host B's read: the RAG candidate pool, sourced entirely from the
        // subscription — this is `SharedLearningReader::list_all_approved_non_task_learnings`'s
        // backing, `SharedRows::approved_non_task_learnings_with_embedding`.
        let candidates = rows.approved_non_task_learnings_with_embedding();
        assert_eq!(candidates.len(), 2, "both learnings must have arrived");

        let decoded: Vec<(dispatch_tui::models::Learning, Vec<f32>)> = candidates
            .into_iter()
            .map(|(l, bytes)| {
                (
                    l,
                    dispatch_tui::service::embeddings::deserialize_embedding(&bytes),
                )
            })
            .collect();

        let ranked = rag_rank_learnings(
            &decoded,
            &RagRankParams {
                query_vec: &query,
                task_epic_id: None,
                task_repo: Some("/repo/a"),
                threshold: 0.0,
                tag_filter: &[],
                limit: 10,
            },
        );

        assert_eq!(ranked.len(), 2);
        assert_eq!(
            ranked[0].summary, "High similarity",
            "the high-cosine candidate must rank first, over rows sourced from the store"
        );
        assert_eq!(ranked[1].summary, "Low similarity");
    });
}

/// A `learnings` row, JSON-encoded for `create_learning`'s CLI call.
///
/// `embedding` is passed pre-serialized bytes rather than a float vector: the
/// module treats it as opaque, exactly as `serialize_embedding` produces it.
fn learning_json(
    summary: &str,
    scope: &str,
    scope_ref: &serde_json::Value,
    embedding: &[u8],
) -> serde_json::Value {
    serde_json::json!({
        "id": 0,
        "kind": "convention",
        "summary": summary,
        "detail": {"none": []},
        "scope": scope,
        "scope_ref": scope_ref,
        "tags": "[]",
        "status": "approved",
        "source_task_id": {"none": []},
        "upvote_count": 0,
        "last_upvoted_at": {"none": []},
        "created_at": "2026-09-26 10:00:00",
        "updated_at": "2026-09-26 10:00:00",
        "embedding": {"some": embedding},
    })
}

// ---------------------------------------------------------------------------
// Phase 6 — the mutations, and the two things only a real store can show
// ---------------------------------------------------------------------------
//
// Everything below writes through a REDUCER and reads back over SQL. The point
// is not that the reducers compile — `check-spacetime-module.sh` covers that —
// it is the two properties that need more than one writer to exist at all:
// an epic status derived once from every host's children, and two hosts
// changing one task without undoing each other.

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
    })
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

/// THE FLAPPING TEST. An epic whose children live on two machines has ONE
/// status, and it is the one every child agrees on.
///
/// The set-up is the exact disagreement the 2026-09-13 design named: host-a
/// sees only a done child and would write `done`; host-b sees only a running
/// one and would write `backlog`. Neither is wrong about what it can see. The
/// store sees both, and the answer below is the only one that is right about
/// the epic.
#[test]
fn an_epics_status_is_derived_from_every_hosts_children() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "Shared", "backlog", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let seeded = instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "finished on host-a", "done", 1, "host-a"),
            task_json(2, "still going on host-b", "running", 1, "host-b"),
        ])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let recalculated = instance.call("recalculate_epic_status", &["1"]);
    assert!(recalculated.status.success(), "{}", describe(&recalculated));
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog",
        "one child is still running on another host, so the epic is not done"
    );

    // The other host finishes. Now — and only now — the epic is done, and the
    // transition is driven by the child's own patch rather than by anybody
    // remembering to recalculate.
    let patched = instance.call(
        "patch_task",
        &["2", &patch_setting("status", "done").to_string()],
    );
    assert!(patched.status.success(), "{}", describe(&patched));
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "done"
    );
}

/// The completion stamp is the store's clock, and it lands on the transition
/// into done rather than on every write afterwards.
#[test]
fn finishing_an_epic_stamps_a_completion_from_the_stores_clock() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "running", 1, "host-a")]).to_string()],
    );

    let patched = instance.call(
        "patch_task",
        &["1", &patch_setting("status", "done").to_string()],
    );
    assert!(patched.status.success(), "{}", describe(&patched));

    let stamped = column(&instance, "SELECT completed_at FROM epics WHERE id = 1");
    assert_ne!(stamped, "", "finishing an epic must stamp a completion");
    // The format both stores write: `YYYY-MM-DD HH:MM:SS.mmm`.
    assert_eq!(stamped.len(), 23, "unexpected timestamp shape: {stamped}");

    // Reopening does not unmake a completion. `completed_at` records the last
    // one, and the regression writes only the status.
    let reopened = instance.call(
        "patch_task",
        &["1", &patch_setting("status", "running").to_string()],
    );
    assert!(reopened.status.success(), "{}", describe(&reopened));
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
    assert_eq!(
        column(&instance, "SELECT completed_at FROM epics WHERE id = 1"),
        stamped,
        "a regression must not clear the last completion"
    );
}

/// TWO HOSTS WRITING ONE TASK CONVERGE. Each names only the field it changed,
/// so the later write does not undo the earlier one.
///
/// This is the whole reason the patch reducers take one `Option` per field
/// rather than a row. With a full-row write each host would send a complete
/// task built from what it last saw, and the second would silently revert the
/// first's change to a field it never touched.
#[test]
fn two_hosts_patching_different_fields_of_one_task_converge() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "original", "backlog", 1, "")]).to_string()],
    );

    // Host A renames it.
    let a = instance.call(
        "patch_task",
        &[
            "1",
            &patch_setting("title", "renamed by host-a").to_string(),
        ],
    );
    assert!(a.status.success(), "{}", describe(&a));

    // Host B, which still believes the title is "original", moves it.
    let b = instance.call(
        "patch_task",
        &["1", &patch_setting("worktree", "/wt/host-b").to_string()],
    );
    assert!(b.status.success(), "{}", describe(&b));

    assert_eq!(
        column(&instance, "SELECT title FROM tasks WHERE id = 1"),
        "renamed by host-a",
        "host B's write must not revert a field it never named"
    );
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        "/wt/host-b"
    );
}

/// The validator runs at the store, and its refusal changes nothing.
///
/// `core.allium: OwnerTracksUserBoardTask`, enforced by the module's
/// `write_task`. Asserted here rather than only as a unit test because what
/// matters is that a REDUCER refuses — a validator that lived on the client
/// could be skipped by any other client.
#[test]
fn the_store_refuses_a_user_board_task_with_no_owner() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let refused = instance.call(
        "create_task",
        &[&task_json(0, "nobody's task", "backlog", 0, "").to_string()],
    );
    assert!(
        !refused.status.success(),
        "a task with no epic and no owner must be refused, got {}",
        describe(&refused)
    );

    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "a refused create must leave no row behind"
    );

    // The same row WITH an owner is accepted, so the refusal is the rule rather
    // than the reducer being broken.
    let accepted = instance.call(
        "create_task",
        &[&{
            let mut row = task_json(0, "my task", "backlog", 0, "");
            row["owner"] = serde_json::json!("user-me");
            row
        }
        .to_string()],
    );
    assert!(accepted.status.success(), "{}", describe(&accepted));
}

/// ONE CLAIM, ONE WINNER — and the loser is TOLD.
///
/// `dispatch.allium: DispatchClaimExclusive`. A reducer returns no value, so
/// "did I win?" is carried by the only channel it has: the second claim is
/// REFUSED. Reading the row back instead would not work — a claim does not
/// stamp the winner's name on it, so both hosts would see `running` and both
/// would believe they won.
///
/// Sequential here rather than concurrent, and that is the honest limit of this
/// test: it shows the claim excludes an already-claimed task, not that two
/// simultaneous calls cannot interleave. The latter is the reducer's
/// transaction, which is the store's guarantee rather than something a test on
/// this side can observe.
#[test]
fn a_claimed_task_is_not_claimed_twice() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "the only one", "backlog", 1, "")]).to_string()],
    );

    let first = instance.call("claim_backlog_task", &["1", "host-a"]);
    assert!(first.status.success(), "{}", describe(&first));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );

    let second = instance.call("claim_backlog_task", &["1", "host-b"]);
    assert!(
        !second.status.success(),
        "the second claim must be refused, not silently ignored: {}",
        describe(&second)
    );
    assert!(
        describe(&second).contains("already claimed"),
        "the refusal must say why: {}",
        describe(&second)
    );
}

/// A foreign-owned backlog task is refused, not claimed. Its worktree is on
/// another machine, so dispatching an agent here would give it nowhere to work.
#[test]
fn a_task_owned_by_another_host_cannot_be_claimed() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "host-a's", "backlog", 1, "host-a")]).to_string()],
    );

    let refused = instance.call("claim_backlog_task", &["1", "host-b"]);
    assert!(
        !refused.status.success(),
        "a task whose worktree is elsewhere must be refused: {}",
        describe(&refused)
    );
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog",
        "a refused claim must change nothing"
    );

    // Its owner can still take it, so the refusal is the rule rather than the
    // reducer being broken.
    let won = instance.call("claim_backlog_task", &["1", "host-a"]);
    assert!(won.status.success(), "{}", describe(&won));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );
}

/// Releasing a claim that already has a worktree is refused. That claim is a
/// dispatch in progress, and releasing it would put a running agent's task back
/// in the backlog for another host to claim underneath it.
#[test]
fn a_claim_with_a_worktree_cannot_be_released() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "provisioned", "running", 1, "host-a"),
            task_json(2, "not yet", "running", 1, "host-a"),
        ])
        .to_string()],
    );
    let gave_it_a_worktree = instance.call(
        "patch_task",
        &["1", &patch_setting("worktree", "/wt/1").to_string()],
    );
    assert!(
        gave_it_a_worktree.status.success(),
        "{}",
        describe(&gave_it_a_worktree)
    );

    let refused = instance.call("release_backlog_claim", &["1"]);
    assert!(
        !refused.status.success(),
        "a provisioned claim must not be releasable: {}",
        describe(&refused)
    );
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );

    // One that never got a worktree goes back to the backlog, which is what
    // the release is for.
    let released = instance.call("release_backlog_claim", &["2"]);
    assert!(released.status.success(), "{}", describe(&released));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 2"),
        "backlog"
    );
}

/// Deleting a task takes its watchers with it, in the same transaction. A watch
/// pointing at a task that no longer exists is not a row anybody can act on.
#[test]
fn deleting_a_task_takes_its_watchers_with_it() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "watched", "backlog", 1, ""),
            task_json(2, "watcher", "backlog", 1, ""),
        ])
        .to_string()],
    );
    let seeded = instance.call(
        "seed_task_watchers",
        &[&serde_json::json!([{
            "id": 1,
            "watcher_task_id": 2,
            "target_task_id": 1,
            "created_at": "2026-09-19 10:00:00",
        }])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let deleted = instance.call("delete_task", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(
        no_rows(&instance, "SELECT id FROM task_watchers"),
        "the watch must go with the task it pointed at"
    );
}

/// **Test 3 of Phase 10 (task #4914).** A task's delete detaches its
/// learnings (`source_task_id` set null, the learning survives as orphaned
/// provenance) and cascades their retrievals — reproduced as explicit
/// reducer logic rather than a SQLite `ON DELETE` clause
/// (`docs/specs/learnings.allium`'s Storage Backend section).
#[test]
fn deleting_a_task_detaches_its_learnings_and_cascades_their_retrievals() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "source", "backlog", 1, "")]).to_string()],
    );
    let seeded = instance.call(
        "seed_learnings",
        &[&serde_json::json!([seeded_learning_json(1, "A learning", Some(1))]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let seeded = instance.call(
        "seed_learning_retrievals",
        &[&serde_json::json!([{
            "id": 1,
            "task_id": 1,
            "learning_id": 1,
            "source": "prompt_injection",
            "retrieved_at": "2026-09-19 10:00:00",
        }])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    // SQL's `--format json` renders a SATS sum type positionally rather than
    // as the named `{"some": ..}`/`{"none": []}` shape reducer ARGUMENTS use
    // (see `seeded_learning_json` above) — captured here, before the delete,
    // so the "detached" assertion below compares against a shape this same
    // run observed rather than one hard-coded from a guess.
    let some_1 = column(
        &instance,
        "SELECT source_task_id FROM learnings WHERE id = 1",
    );

    let deleted = instance.call("delete_task", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    let after_delete = column(
        &instance,
        "SELECT source_task_id FROM learnings WHERE id = 1",
    );
    assert_ne!(
        after_delete, some_1,
        "source_task_id must change — the delete is a no-op if this still reads Some(1)"
    );
    assert_eq!(
        after_delete, "[1,[]]",
        "the learning survives, detached from its deleted source task (none's tag, empty payload)"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM learning_retrievals"),
        "a retrieval naming the deleted task must go with it"
    );
}

/// A learning's delete takes its own retrievals with it — the other half of
/// the cascade `deleting_a_task_detaches_its_learnings_and_cascades_their_retrievals`
/// covers. Also confirms `DeleteLearningViaMcp`'s refusal for a missing id,
/// unlike `delete_task`'s silent no-op.
#[test]
fn deleting_a_learning_cascades_its_retrievals_and_refuses_when_missing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "backlog", 1, "")]).to_string()],
    );
    let seeded = instance.call(
        "seed_learnings",
        &[&serde_json::json!([seeded_learning_json(1, "A learning", None)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let seeded = instance.call(
        "seed_learning_retrievals",
        &[&serde_json::json!([{
            "id": 1,
            "task_id": 1,
            "learning_id": 1,
            "source": "prompt_injection",
            "retrieved_at": "2026-09-19 10:00:00",
        }])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let deleted = instance.call("delete_learning", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(
        no_rows(&instance, "SELECT id FROM learnings"),
        "the learning itself must be gone"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM learning_retrievals"),
        "its retrievals must go with it"
    );

    let refused = instance.call("delete_learning", &["1"]);
    assert!(
        !refused.status.success(),
        "deleting an id that no longer exists must be refused, not a silent no-op"
    );
}

/// A `learnings` row, JSON-encoded for `seed_learnings`'s CLI call — a real
/// id and an explicit `source_task_id`, unlike `learning_json` above (which
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

/// **Test 1 of Phase 11 (task #4915), against a real server.** A usage event
/// recorded on one host's board is visible from another's.
///
/// Host A is the `record_usage_event` reducer call — a different process,
/// the same convention `a_row_written_elsewhere_arrives_through_the_subscription`
/// uses. Host B is this test's own `SharedRows`, subscribed with nothing
/// followed: `usage_events` needs no `epic_id` to see a row, because it is
/// unconditionally subscribed, the same as `learnings`.
#[test]
fn a_usage_event_recorded_elsewhere_is_visible_from_here() {
    if !spacetime_available_or_skip() {
        return;
    }
    use dispatch_tui::db::UsageQuery;

    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows = Arc::new(SharedRows::new());
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    runtime.block_on(async {
        let connector = SpacetimeSdkConnector::new(instance.database(), rows.clone());
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-b",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));

        assert!(
            rows.usage_summary(&UsageQuery::default()).is_empty(),
            "nothing has been written yet, so nothing may have arrived"
        );

        let mut woken = rows.changed();
        woken.mark_unchanged();

        // Host A records the event directly through the reducer.
        let recorded = instance.call(
            "record_usage_event",
            &[
                &usage_event_json("keybinding", "dispatch_task", Some("d"), "human").to_string(),
                "100",
            ],
        );
        assert!(recorded.status.success(), "{}", describe(&recorded));

        woken
            .changed()
            .await
            .expect("the subscription must deliver the usage event");

        let summary = rows.usage_summary(&UsageQuery::default());
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].action, "dispatch_task");
        assert_eq!(summary[0].count, 1);
    });
}

/// **Test 2 of Phase 11 (task #4915).** The row-count cap is enforced in the
/// reducer itself, not by local SQL — mirroring the original SQLite path's
/// `DELETE ... WHERE id <= MAX(id) - cap` in the same transaction as the
/// insert, but now as reducer logic every host's write goes through, the same
/// shape `deleting_a_task_detaches_its_learnings_and_cascades_their_retrievals`
/// proves for the learnings cascade above.
#[test]
fn recording_usage_events_prunes_beyond_the_cap() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    for i in 1..=5 {
        let recorded = instance.call(
            "record_usage_event",
            &[
                &usage_event_json("keybinding", &format!("action_{i}"), None, "human").to_string(),
                "3",
            ],
        );
        assert!(recorded.status.success(), "{}", describe(&recorded));
    }

    let count = column(&instance, "SELECT COUNT(*) AS c FROM usage_events");
    assert_eq!(count, "3", "the table must never exceed the cap");

    assert!(
        no_rows(
            &instance,
            "SELECT id FROM usage_events WHERE action = 'action_1'"
        ),
        "the oldest rows must be the ones pruned"
    );
    assert!(
        !no_rows(
            &instance,
            "SELECT id FROM usage_events WHERE action = 'action_5'"
        ),
        "the newest row must survive the prune"
    );
}

/// A `usage_events` row, JSON-encoded for `record_usage_event`'s CLI call.
fn usage_event_json(
    category: &str,
    action: &str,
    detail: Option<&str>,
    actor: &str,
) -> serde_json::Value {
    serde_json::json!({
        "id": 0,
        "recorded_at": "2026-09-26 10:00:00",
        "category": category,
        "action": action,
        "detail": match detail {
            Some(d) => serde_json::json!({"some": d}),
            None => serde_json::json!({"none": []}),
        },
        "actor": actor,
    })
}

/// A task created straight into Done carries a completion stamp.
///
/// `stamps_completion` covers every TRANSITION into done, and a create is not a
/// transition — so without this a Done card would sort to the bottom of the
/// column forever. Nothing creates a Done task today; the SQLite side stamps it
/// anyway, for the same reason.
#[test]
fn a_task_created_in_done_is_stamped_as_completed() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let made = instance.call(
        "create_task",
        &[&{
            let mut row = task_json(0, "born finished", "done", 0, "");
            row["owner"] = serde_json::json!("user-me");
            row
        }
        .to_string()],
    );
    assert!(made.status.success(), "{}", describe(&made));

    let stamped = column(
        &instance,
        "SELECT completed_at FROM tasks WHERE title = 'born finished'",
    );
    assert_ne!(stamped, "", "a task born in done must carry a completion");
    assert_eq!(stamped.len(), 23, "unexpected timestamp shape: {stamped}");
}

// -- Agent session state (Phase 6b) ------------------------------------------
//
// A handful of scenarios a unit test with a recording caller cannot check,
// the same reason this file exists at all: whether the REAL module reproduces
// `docs/specs/agent-health.allium`'s guarantees, not whether a mock assumed it
// would. `src/sync/tests/writes.rs` covers `ReducerWriter`'s decoding far more
// exhaustively; these are chosen to hit the module's own logic that decoding
// can't reach, especially the no-primary-key dedupe hazard an adversarial
// review of this task's plan flagged (`docs/plans/
// 2026-09-21-phase-6b-agent-session-state-reducers.md`, decision 7).

/// A running task with no live subagents or shells and no deferred Stop —
/// the ordinary "task exists" seed these tests build on.
fn running_task(instance: &Instance, id: i64) {
    let mut row = task_json(id, "t", "running", 0, "");
    row["owner"] = serde_json::json!("user-me");
    let seeded = instance.call("seed_tasks", &[&serde_json::json!([row]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
}

/// `live_subagents` counts real entries, and a repeated start for the SAME
/// agent does not double it. `task_subagents` has no primary key in the
/// module (unlike SQLite's `PRIMARY KEY (task_id, agent_id)`), so this is the
/// one property that hazard could actually break: `subagent_start` finding
/// and deleting the prior row by value before inserting the fresh one, not
/// merely fencing the session.
#[test]
fn a_repeated_subagent_start_for_the_same_agent_does_not_duplicate() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);

    let first = instance.call(
        "subagent_start",
        &["1", "agent-1", "session-1", "2026-09-19T10:00:00Z"],
    );
    assert!(first.status.success(), "{}", describe(&first));
    let second = instance.call(
        "subagent_start",
        &["1", "agent-1", "session-1", "2026-09-19T10:00:01Z"],
    );
    assert!(second.status.success(), "{}", describe(&second));

    assert_eq!(
        column(
            &instance,
            "SELECT COUNT(*) AS n FROM task_subagents WHERE task_id = 1"
        ),
        "1",
        "a repeated start for the same agent must replace, not duplicate, its row"
    );
    assert_eq!(
        column(&instance, "SELECT live_subagents FROM tasks WHERE id = 1"),
        "1"
    );
}

/// A second, distinct agent DOES raise the count — the dedupe above is keyed
/// on `agent_id`, not on the task.
#[test]
fn two_different_subagents_both_count() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);

    instance.call(
        "subagent_start",
        &["1", "agent-1", "session-1", "2026-09-19T10:00:00Z"],
    );
    instance.call(
        "subagent_start",
        &["1", "agent-2", "session-1", "2026-09-19T10:00:00Z"],
    );

    assert_eq!(
        column(&instance, "SELECT live_subagents FROM tasks WHERE id = 1"),
        "2"
    );

    let stopped = instance.call("subagent_stop", &["1", "agent-1", "session-1"]);
    assert!(stopped.status.success(), "{}", describe(&stopped));
    assert_eq!(
        column(&instance, "SELECT live_subagents FROM tasks WHERE id = 1"),
        "1"
    );
}

/// `HookStop`: nothing live, so the Stop flips the task straight to `review`.
#[test]
fn try_record_stop_flips_a_running_task_with_nothing_live() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);

    let stopped = instance.call("try_record_stop", &["1", "2026-09-19T10:00:00.000"]);
    assert!(stopped.status.success(), "{}", describe(&stopped));

    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "review"
    );
}

/// ...and a live subagent withholds the flip until it drains, at which point
/// the DRAIN — not a second Stop — applies it. `HookStop`/`HookSubagentStop`
/// in `docs/specs/agent-health.allium`.
#[test]
fn try_record_stop_defers_while_a_subagent_is_live_and_the_drain_applies_it() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);
    instance.call(
        "subagent_start",
        &["1", "agent-1", "session-1", "2026-09-19T10:00:00Z"],
    );

    let stopped = instance.call("try_record_stop", &["1", "2026-09-19T10:00:01.000"]);
    assert!(stopped.status.success(), "{}", describe(&stopped));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running",
        "a live subagent must withhold the flip"
    );
    assert_eq!(
        column(&instance, "SELECT stop_pending FROM tasks WHERE id = 1"),
        "true"
    );

    let drained = instance.call("subagent_stop", &["1", "agent-1", "session-1"]);
    assert!(drained.status.success(), "{}", describe(&drained));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "review",
        "draining the last subagent must apply the deferred Stop"
    );
    assert_eq!(
        column(&instance, "SELECT stop_pending FROM tasks WHERE id = 1"),
        "false"
    );
}

/// A Stop against a task that is not `Running` is refused outright — the
/// precondition failing is what lets `ReducerWriter` read `NoOp` back
/// unambiguously (this task's plan doc, decision 3) — rather than a silent
/// no-op the caller could not tell apart from a real flip.
#[test]
fn try_record_stop_refuses_a_task_that_is_not_running() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let mut row = task_json(1, "t", "backlog", 0, "");
    row["owner"] = serde_json::json!("user-me");
    let seeded = instance.call("seed_tasks", &[&serde_json::json!([row]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let refused = instance.call("try_record_stop", &["1", "2026-09-19T10:00:00.000"]);
    assert!(
        !refused.status.success(),
        "a Stop against a non-Running task must be refused, got {}",
        describe(&refused)
    );
}

/// The PR learnings gate fires EXACTLY once: the first call sets it and wins,
/// a second is refused. `ReducerWriter` reads that refusal as `false` via
/// `ReducerOutcome::won()` — the same shape as the dispatch claim.
#[test]
fn mark_pr_learnings_gate_shown_wins_exactly_once() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);

    let first = instance.call(
        "mark_pr_learnings_gate_shown",
        &["1", "2026-09-19T10:00:00.000"],
    );
    assert!(first.status.success(), "{}", describe(&first));

    let second = instance.call(
        "mark_pr_learnings_gate_shown",
        &["1", "2026-09-19T10:00:01.000"],
    );
    assert!(
        !second.status.success(),
        "a second call must be refused, got {}",
        describe(&second)
    );
}

// ---------------------------------------------------------------------------
// Phase 6c: feed ingestion, task watchers, and the stragglers
// ---------------------------------------------------------------------------

/// A feed item, already resolved the way the client resolves one before
/// sending — see `FeedTaskUpsertItem`'s doc comment in the module.
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

/// The two-part contract of a re-poll: fields the feed owns move, fields the
/// user or the store own do not. A re-poll changes `title`; a user-moved
/// `status`/`sub_status` and a store-set `worktree` must both survive it.
#[test]
fn feed_upsert_updates_feed_fields_and_preserves_user_and_store_fields() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );

    let first = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "original title", "backlog")]).to_string(),
            "test-creator",
        ],
    );
    assert!(first.status.success(), "{}", describe(&first));

    // The user moves it to Running and the store gives it a worktree —
    // neither of which a feed item carries or should be able to touch.
    let running = instance.call(
        "patch_task",
        &["1", &patch_setting("status", "running").to_string()],
    );
    assert!(running.status.success(), "{}", describe(&running));
    let mut worktree_patch = empty_task_patch();
    worktree_patch["worktree"] = serde_json::json!({"some": "wt-1"});
    let worktreed = instance.call("patch_task", &["1", &worktree_patch.to_string()]);
    assert!(worktreed.status.success(), "{}", describe(&worktreed));

    // Re-poll with a changed title and an item status the SQLite version's
    // `ON CONFLICT` would also ignore.
    let second = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "updated title", "done")]).to_string(),
            "test-creator",
        ],
    );
    assert!(second.status.success(), "{}", describe(&second));

    assert_eq!(
        column(&instance, "SELECT title FROM tasks WHERE id = 1"),
        "updated title",
        "the feed-owned field must update"
    );
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running",
        "a re-poll must not move a user-managed status"
    );
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        "wt-1",
        "a re-poll must not touch a store-owned field"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM tasks WHERE completed_at != ''"),
        "a task that was never done must not gain a completion stamp"
    );
}

/// An existing non-null url always wins over whatever the item carries; url
/// and url_type move together, never one without the other.
#[test]
fn feed_upsert_keeps_an_existing_url_over_a_re_polled_one() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    let mut first_item = feed_item_json("ext-1", "t", "backlog");
    first_item["url"] = serde_json::json!("https://example.com/1");
    first_item["url_type"] = serde_json::json!("pr");
    instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([first_item]).to_string(),
            "test-creator",
        ],
    );

    let mut second_item = feed_item_json("ext-1", "t", "backlog");
    second_item["url"] = serde_json::json!("https://example.com/2");
    second_item["url_type"] = serde_json::json!("issue");
    let second = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([second_item]).to_string(),
            "test-creator",
        ],
    );
    assert!(second.status.success(), "{}", describe(&second));

    assert_eq!(
        column(
            &instance,
            "SELECT url FROM tasks WHERE external_id = 'ext-1'"
        ),
        "https://example.com/1",
        "the FIRST url must win, not the re-polled one"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT url_type FROM tasks WHERE external_id = 'ext-1'"
        ),
        "pr"
    );
}

/// A task born already `done` is stamped complete; a re-poll never re-derives
/// that stamp, matching `write_task`'s own insert-only rule.
#[test]
fn feed_upsert_stamps_completion_only_on_a_genuine_insert() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    let made = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "t", "done")]).to_string(),
            "test-creator",
        ],
    );
    assert!(made.status.success(), "{}", describe(&made));
    assert_ne!(
        column(
            &instance,
            "SELECT completed_at FROM tasks WHERE external_id = 'ext-1'"
        ),
        "",
        "a task created directly into done must be stamped"
    );
}

/// A newly-inserted feed task stamps `created_by` from the caller — the
/// running host's own identity (`core.allium: Task.created_by`,
/// `feeds.allium: UpsertFeedTasks`) — and a later re-poll of the same item
/// leaves it untouched, matching `created_by`'s "stamped once" rule.
#[test]
fn feed_upsert_stamps_created_by_on_insert_only() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );

    let made = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "t", "backlog")]).to_string(),
            "host-a",
        ],
    );
    assert!(made.status.success(), "{}", describe(&made));
    assert_eq!(
        column(
            &instance,
            "SELECT created_by FROM tasks WHERE external_id = 'ext-1'"
        ),
        "host-a",
        "a genuine insert must stamp created_by from the caller"
    );

    let repolled = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "updated", "backlog")]).to_string(),
            "host-b",
        ],
    );
    assert!(repolled.status.success(), "{}", describe(&repolled));
    assert_eq!(
        column(
            &instance,
            "SELECT created_by FROM tasks WHERE external_id = 'ext-1'"
        ),
        "host-a",
        "a re-poll from a different host must not overwrite created_by"
    );
}

/// The reconciling variant deletes what the emission omits; the additive
/// variant never does, even given the identical inputs.
#[test]
fn feed_upsert_reconciles_but_additive_never_deletes() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([
                feed_item_json("ext-1", "one", "backlog"),
                feed_item_json("ext-2", "two", "backlog"),
            ])
            .to_string(),
            "test-creator",
        ],
    );

    // ext-2 is now absent from the emission.
    let additive = instance.call(
        "upsert_feed_tasks_additive",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "one", "backlog")]).to_string(),
            "test-creator",
        ],
    );
    assert!(additive.status.success(), "{}", describe(&additive));
    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM tasks WHERE epic_id = 1"
        ),
        "2",
        "additive must not delete an item merely absent from the emission"
    );

    let reconciling = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "one", "backlog")]).to_string(),
            "test-creator",
        ],
    );
    assert!(reconciling.status.success(), "{}", describe(&reconciling));
    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM tasks WHERE epic_id = 1"
        ),
        "1",
        "the reconciling variant must delete what the emission omits"
    );
}

/// The subtree-scoped delete reaches every direct child epic of `parent_id`,
/// preserves manual tasks (`external_id` empty) unconditionally, and leaves a
/// kept `external_id` alone.
#[test]
fn delete_stale_subtree_feed_tasks_scopes_to_children_and_keeps_manual_tasks() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            epic_json(1, "parent", "backlog", 0),
            epic_json(2, "child-a", "backlog", 1),
            epic_json(3, "child-b", "backlog", 1),
        ])
        .to_string()],
    );
    let mut stale = task_json(2, "stale", "backlog", 3, "");
    stale["external_id"] = serde_json::json!("stale");
    let mut kept = task_json(1, "keep", "backlog", 2, "");
    kept["external_id"] = serde_json::json!("keep");
    let mut manual = task_json(3, "manual", "backlog", 2, "");
    manual["external_id"] = serde_json::json!("");
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([kept, stale, manual]).to_string()],
    );

    let deleted = instance.call(
        "delete_stale_subtree_feed_tasks",
        &["1", &serde_json::json!(["keep"]).to_string()],
    );
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(
        !no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"),
        "the kept external_id must survive"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM tasks WHERE id = 2"),
        "a stale task in a CHILD epic must be removed"
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM tasks WHERE id = 3"),
        "a manual task (no external_id) must always survive"
    );
}

// ---------------------------------------------------------------------------
// Retired feed items — delete means "I am done with this item"
// ---------------------------------------------------------------------------
//
// `core.allium: RetiredFeedItem`, written by `tasks.allium: DeleteTask` and
// `epics.allium: DeleteEpic`, read by `feeds.allium:
// IngestSkipsRetiredFeedItems` and dropped by `DropClosedRetiredFeedItems`.
// Every retirement is keyed on the FEED EPIC — the nearest epic in the chain
// carrying a `feed_command` — never on the sub-epic the task sat in.

/// An epic carrying a `feed_command`, which is what makes it the key a
/// retirement is written under (`core/Epic.nearest_feed_epic`).
fn feed_epic_json(id: i64, title: &str, parent: i64) -> serde_json::Value {
    let mut epic = epic_json(id, title, "backlog", parent);
    epic["feed_command"] = serde_json::json!("echo []");
    epic
}

/// A feed task: a task row carrying `external_id`, in `epic`.
fn feed_task_json(id: i64, external_id: &str, status: &str, epic: i64) -> serde_json::Value {
    let mut task = task_json(id, external_id, status, epic, "");
    task["external_id"] = serde_json::json!(external_id);
    task
}

/// A `retired_feed_items` row as `seed_retired_feed_items` takes it. Seeded
/// ids are kept far above the auto_inc counter's start, because seeding an
/// explicit id leaves that counter unburned (#755) and a later generated
/// insert would otherwise collide with it.
fn retired_json(id: i64, feed_epic_id: i64, external_id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "feed_epic_id": feed_epic_id,
        "external_id": external_id,
        "retired_at": "2026-09-19 10:00:00",
    })
}

fn retired_count(instance: &Instance, where_clause: &str) -> String {
    column(
        instance,
        &format!("SELECT count(*) AS c FROM retired_feed_items WHERE {where_clause}"),
    )
}

/// `tasks.allium: DeleteTask`'s retirement clause: deleting a feed task
/// writes one record keyed on the NEAREST FEED EPIC in its chain — here the
/// grandparent, not the sub-epic the task sat in — in the same call that
/// removes the row.
#[test]
fn deleting_a_feed_task_retires_it_under_its_nearest_feed_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "repo group", "backlog", 1),
        ])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([feed_task_json(1, "ext-1", "done", 2)]).to_string()],
    );

    let deleted = instance.call("delete_task", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(
        no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"),
        "the task row must be gone"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-1'"),
        "1",
        "the retirement must be keyed on the feed epic, not the sub-epic"
    );
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM retired_feed_items WHERE feed_epic_id = 2"
        ),
        "nothing may be keyed on the sub-epic the task sat in"
    );
    assert_ne!(
        column(
            &instance,
            "SELECT retired_at FROM retired_feed_items WHERE external_id = 'ext-1'"
        ),
        "",
        "a retirement is stamped with the store's clock"
    );
}

/// A manual task (no `external_id`) and a feed-shaped task under no feed epic
/// both retire nothing: there is no cycle to suppress.
#[test]
fn deleting_a_manual_task_or_one_under_no_feed_epic_retires_nothing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "plain", "backlog", 0),
        ])
        .to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "manual", "done", 1, ""),
            feed_task_json(2, "ext-orphan", "done", 2),
        ])
        .to_string()],
    );

    for id in ["1", "2"] {
        let deleted = instance.call("delete_task", &[id]);
        assert!(deleted.status.success(), "{}", describe(&deleted));
    }

    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "both tasks must still be deleted"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM retired_feed_items"),
        "neither a manual task nor one under no feed epic may be retired"
    );
}

/// `core/RetiredFeedItem: UniqueRetiredFeedItemPerFeed`. Retiring the same
/// (feed epic, external_id) again — directly through
/// `create_retired_feed_item`, or by deleting a second task carrying the id —
/// is a no-op; the same id under a DIFFERENT feed epic is its own record.
#[test]
fn retiring_the_same_feed_item_twice_writes_one_record() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "sub-a", "backlog", 1),
            epic_json(3, "sub-b", "backlog", 1),
            feed_epic_json(4, "other feed", 0),
        ])
        .to_string()],
    );

    for _ in 0..2 {
        let retired = instance.call("create_retired_feed_item", &["1", "\"ext-1\""]);
        assert!(retired.status.success(), "{}", describe(&retired));
    }
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-1'"),
        "1",
        "a second direct retirement must be a no-op"
    );

    // Two tasks with the same id in two sub-epics of one feed: both deletes
    // resolve to the same key and the second must not write again.
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            feed_task_json(1, "ext-1", "done", 2),
            feed_task_json(2, "ext-1", "done", 3),
        ])
        .to_string()],
    );
    for id in ["1", "2"] {
        let deleted = instance.call("delete_task", &[id]);
        assert!(deleted.status.success(), "{}", describe(&deleted));
    }
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-1'"),
        "1",
        "a delete of an already-retired id must not write a second record"
    );

    let other = instance.call("create_retired_feed_item", &["4", "\"ext-1\""]);
    assert!(other.status.success(), "{}", describe(&other));
    assert_eq!(
        retired_count(&instance, "external_id = 'ext-1'"),
        "2",
        "the same id under a different feed epic is a separate record"
    );
}

/// `feeds.allium: IngestSkipsRetiredFeedItems`, reconcile and additive alike:
/// an item whose id is retired under the upsert epic's nearest feed epic, with
/// no surviving task, is never inserted — while a non-retired item in the same
/// batch is. The record is restored through `seed_retired_feed_items`, so this
/// also covers the snapshot seed path.
#[test]
fn feed_upsert_refuses_to_insert_a_retired_external_id() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "repo group", "backlog", 1),
        ])
        .to_string()],
    );
    let seeded = instance.call(
        "seed_retired_feed_items",
        &[&serde_json::json!([retired_json(100, 1, "ext-retired")]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    for (reducer, epic) in [
        ("upsert_feed_tasks", "1"),
        ("upsert_feed_tasks_additive", "2"),
    ] {
        let upserted = instance.call(
            reducer,
            &[
                epic,
                &serde_json::json!([
                    feed_item_json("ext-retired", "back again", "backlog"),
                    feed_item_json(&format!("ext-new-{epic}"), "fresh", "backlog"),
                ])
                .to_string(),
                "test-creator",
            ],
        );
        assert!(
            upserted.status.success(),
            "{reducer}: {}",
            describe(&upserted)
        );
        assert!(
            no_rows(
                &instance,
                "SELECT id FROM tasks WHERE external_id = 'ext-retired'"
            ),
            "{reducer} into epic {epic} must not insert a retired id"
        );
        assert_eq!(
            column(
                &instance,
                &format!("SELECT count(*) AS c FROM tasks WHERE external_id = 'ext-new-{epic}'")
            ),
            "1",
            "{reducer} must still insert the batch's non-retired item"
        );
    }
}

/// Suppression covers insertion only. A task still on the board with a
/// retired id is matched and refreshed like any other feed task, and the
/// record itself is untouched by the upsert.
#[test]
fn feed_upsert_refreshes_a_surviving_task_whose_id_is_retired() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([feed_epic_json(1, "feed", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([feed_task_json(1, "ext-1", "done", 1)]).to_string()],
    );
    instance.call("create_retired_feed_item", &["1", "\"ext-1\""]);

    let upserted = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "refreshed title", "backlog")]).to_string(),
            "test-creator",
        ],
    );
    assert!(upserted.status.success(), "{}", describe(&upserted));

    assert_eq!(
        column(&instance, "SELECT title FROM tasks WHERE id = 1"),
        "refreshed title",
        "a surviving task with a retired id must still be refreshed"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM tasks WHERE external_id = 'ext-1'"
        ),
        "1",
        "the refresh must not duplicate the row"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-1'"),
        "1",
        "an upsert must not touch the retirement record"
    );
}

/// `epics.allium: DeleteEpic`'s retirement clause, feed epic surviving: every
/// feed task anywhere in the deleted subtree — nested sub-epics included — is
/// retired under the surviving feed epic above it; a manual task is not.
#[test]
fn deleting_a_sub_epic_retires_its_whole_subtrees_feed_tasks() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "sub", "backlog", 1),
            epic_json(3, "sub-sub", "backlog", 2),
        ])
        .to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            feed_task_json(1, "ext-a", "done", 2),
            feed_task_json(2, "ext-b", "done", 3),
            task_json(3, "manual", "done", 2, ""),
        ])
        .to_string()],
    );

    let deleted = instance.call("delete_epic", &["2"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "the whole subtree's tasks must be deleted"
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM epics WHERE id = 1"),
        "the feed epic above the deleted sub-epic must survive"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-a'"),
        "1",
        "a feed task directly in the deleted epic must be retired"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-b'"),
        "1",
        "a feed task in a nested sub-epic must be retired too"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id > 0"),
        "2",
        "a manual task must not be retired, and nothing keyed elsewhere"
    );

    // And the retirement bites: the feed's next cycle does not re-insert.
    instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-a", "a", "backlog")]).to_string(),
            "test-creator",
        ],
    );
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM tasks WHERE external_id = 'ext-a'"
        ),
        "a cycle after the delete must not put the item straight back"
    );
}

/// Deleting a feed epic is a reset: its feed tasks are retired under nothing
/// (the feed epic is itself doomed), its existing records are dropped with it,
/// and records keyed on a feed epic outside the doomed subtree survive. A feed
/// epic nested under a deleted plain parent is doomed the same way.
#[test]
fn deleting_a_feed_epic_retires_nothing_and_drops_its_records() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "repo group", "backlog", 1),
            feed_epic_json(3, "other feed", 0),
            epic_json(4, "plain parent", "backlog", 0),
            feed_epic_json(5, "nested feed", 4),
        ])
        .to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            feed_task_json(1, "ext-a", "done", 2),
            feed_task_json(2, "ext-b", "done", 1),
            feed_task_json(3, "ext-c", "done", 5),
        ])
        .to_string()],
    );
    instance.call(
        "seed_retired_feed_items",
        &[&serde_json::json!([
            retired_json(100, 1, "ext-old"),
            retired_json(101, 3, "ext-kept"),
            retired_json(102, 5, "ext-nested-old"),
        ])
        .to_string()],
    );

    let deleted = instance.call("delete_epic", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM retired_feed_items WHERE feed_epic_id = 1"
        ),
        "a deleted feed epic's records go with it, and none are written for it"
    );
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM retired_feed_items WHERE external_id = 'ext-a' OR external_id = 'ext-b'"
        ),
        "a feed task under a doomed feed epic has nothing to be retired under"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 3 AND external_id = 'ext-kept'"),
        "1",
        "another feed's record must survive"
    );

    let deleted_parent = instance.call("delete_epic", &["4"]);
    assert!(
        deleted_parent.status.success(),
        "{}",
        describe(&deleted_parent)
    );
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM retired_feed_items WHERE feed_epic_id = 5"
        ),
        "a feed epic nested in a deleted subtree is reset too"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id > 0"),
        "1",
        "only the surviving feed's record may remain"
    );
}

/// `feeds.allium: DropClosedRetiredFeedItems`: every record under the feed
/// epic whose id the trusted emission no longer carries is dropped; one still
/// in the keep-set stays, and another feed epic's records are out of scope.
/// A dropped id then comes back as a fresh task on the next cycle — the
/// reopen case the rule exists for.
#[test]
fn drop_closed_retired_feed_items_drops_what_the_keep_set_omits() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            feed_epic_json(2, "other feed", 0),
        ])
        .to_string()],
    );
    for (epic, ext) in [
        ("1", "ext-1"),
        ("1", "ext-2"),
        ("1", "ext-3"),
        ("2", "ext-1"),
    ] {
        let retired = instance.call("create_retired_feed_item", &[epic, &format!("\"{ext}\"")]);
        assert!(retired.status.success(), "{}", describe(&retired));
    }

    let dropped = instance.call(
        "drop_closed_retired_feed_items",
        &["1", &serde_json::json!(["ext-2"]).to_string()],
    );
    assert!(dropped.status.success(), "{}", describe(&dropped));

    assert_eq!(
        column(
            &instance,
            "SELECT external_id FROM retired_feed_items WHERE feed_epic_id = 1"
        ),
        "ext-2",
        "only the id still in the keep-set may stay retired"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 2 AND external_id = 'ext-1'"),
        "1",
        "another feed epic's record with the same id must be untouched"
    );

    let reopened = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([
                feed_item_json("ext-1", "reopened", "backlog"),
                feed_item_json("ext-2", "still retired", "backlog"),
            ])
            .to_string(),
            "test-creator",
        ],
    );
    assert!(reopened.status.success(), "{}", describe(&reopened));
    assert_eq!(
        column(
            &instance,
            "SELECT title FROM tasks WHERE external_id = 'ext-1'"
        ),
        "reopened",
        "a dropped record must let a reopened item back in"
    );
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM tasks WHERE external_id = 'ext-2'"
        ),
        "a kept record must still suppress its id"
    );

    let emptied = instance.call(
        "drop_closed_retired_feed_items",
        &["1", &serde_json::json!([]).to_string()],
    );
    assert!(emptied.status.success(), "{}", describe(&emptied));
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM retired_feed_items WHERE feed_epic_id = 1"
        ),
        "an empty keep-set drops every record under the feed epic"
    );
}

/// Find-or-create: a second call for the same `(parent, title)` returns the
/// same epic rather than duplicating it. There is no archived state any more
/// for a found epic to be unarchived out of (`epics.allium`'s guidance on
/// `ProvisionManagedEpics`, which this reducer follows too) — a deleted
/// sub-epic simply is not found, and the None arm creates a fresh one.
#[test]
fn create_repo_group_sub_epic_is_idempotent() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    // Created, not seeded: seeding an explicit id leaves the auto_inc
    // counter unburned (#755), and this test goes on to auto-insert a SECOND
    // epic on this same table — a collision `create_epic` avoids by never
    // touching the counter with a manual id at all.
    instance.call(
        "create_epic",
        &[&epic_json(0, "parent", "backlog", 0).to_string()],
    );

    let first = instance.call("create_repo_group_sub_epic", &["1", "my-repo", "user-a"]);
    assert!(first.status.success(), "{}", describe(&first));
    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM epics WHERE parent_epic_id = 1"
        ),
        "1"
    );

    let second = instance.call("create_repo_group_sub_epic", &["1", "my-repo", "user-a"]);
    assert!(second.status.success(), "{}", describe(&second));
    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM epics WHERE parent_epic_id = 1"
        ),
        "1",
        "a second call must not create a duplicate"
    );
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 2"),
        "backlog",
        "the found epic is left exactly as it was"
    );
    assert_eq!(
        column(&instance, "SELECT created_by FROM epics WHERE id = 2"),
        "user-a"
    );
}

/// Find-or-create keyed on `(parent, feed_role)`, and `origin` stays
/// `"manual"` — the SQLite insert this mirrors never sets it either.
#[test]
fn create_managed_role_epic_is_idempotent_and_leaves_origin_manual() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    // Created, not seeded — see the identical note in
    // create_repo_group_sub_epic_is_idempotent.
    instance.call(
        "create_epic",
        &[&epic_json(0, "parent", "backlog", 0).to_string()],
    );

    for _ in 0..2 {
        let made = instance.call(
            "create_managed_role_epic",
            &["Reviews", "1", "reviews", "gh pr list", "300", "user-a"],
        );
        assert!(made.status.success(), "{}", describe(&made));
    }

    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM epics WHERE parent_epic_id = 1"
        ),
        "1",
        "a repeated call must not duplicate"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT origin FROM epics WHERE feed_role = 'reviews'"
        ),
        "manual"
    );
}

/// A watch is created once, is idempotent, and each delete method removes
/// exactly the rows on its own side.
#[test]
fn task_watchers_are_idempotent_and_each_delete_targets_its_own_side() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "target", "backlog", 1, ""),
            task_json(2, "watcher-a", "backlog", 1, ""),
            task_json(3, "watcher-b", "backlog", 1, ""),
        ])
        .to_string()],
    );

    for _ in 0..2 {
        let made = instance.call("create_task_watcher", &["2", "1"]);
        assert!(made.status.success(), "{}", describe(&made));
    }
    instance.call("create_task_watcher", &["3", "1"]);
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM task_watchers"),
        "2",
        "a repeated create must not duplicate the watch"
    );

    let removed = instance.call("delete_task_watcher", &["2", "1"]);
    assert!(removed.status.success(), "{}", describe(&removed));
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM task_watchers"),
        "1",
        "delete_task_watcher must remove only the named pair"
    );

    instance.call("create_task_watcher", &["2", "1"]);
    let by_target = instance.call("delete_watches_of_target", &["1"]);
    assert!(by_target.status.success(), "{}", describe(&by_target));
    assert!(
        no_rows(&instance, "SELECT id FROM task_watchers"),
        "delete_watches_of_target must remove every watch pointed at it"
    );

    instance.call("create_task_watcher", &["2", "1"]);
    instance.call("create_task_watcher", &["2", "3"]);
    let by_watcher = instance.call("delete_watches_by_watcher", &["2"]);
    assert!(by_watcher.status.success(), "{}", describe(&by_watcher));
    assert!(
        no_rows(&instance, "SELECT id FROM task_watchers"),
        "delete_watches_by_watcher must remove every watch this task holds"
    );
}

/// One call updates every task in the batch, and leaves a missing id as a
/// silent no-op rather than failing the whole call.
#[test]
fn batch_patch_sub_status_applies_the_whole_batch_in_one_call() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "a", "backlog", 1, ""),
            task_json(2, "b", "backlog", 1, ""),
        ])
        .to_string()],
    );

    let applied = instance.call(
        "batch_patch_sub_status",
        &[&serde_json::json!([
            {"task_id": 1, "sub_status": "active"},
            {"task_id": 2, "sub_status": "stale"},
            {"task_id": 999, "sub_status": "active"},
        ])
        .to_string()],
    );
    assert!(
        applied.status.success(),
        "a missing id must not fail the batch: {}",
        describe(&applied)
    );

    assert_eq!(
        column(&instance, "SELECT sub_status FROM tasks WHERE id = 1"),
        "active"
    );
    assert_eq!(
        column(&instance, "SELECT sub_status FROM tasks WHERE id = 2"),
        "stale"
    );
}

/// The successor is created and the predecessor's `phoenix` flag clears in
/// one call — `TheFlagIsTheReceipt`.
#[test]
fn respawn_phoenix_successor_creates_and_clears_the_flag_atomically() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    // Created, not seeded — see the identical note in
    // create_repo_group_sub_epic_is_idempotent: this test goes
    // on to auto-insert the successor on this same `tasks` table.
    let mut predecessor = task_json(0, "recurring", "done", 0, "");
    predecessor["owner"] = serde_json::json!("user-a");
    predecessor["phoenix"] = serde_json::json!(true);
    instance.call("create_task", &[&predecessor.to_string()]);

    let mut successor = task_json(0, "recurring", "backlog", 0, "");
    successor["owner"] = serde_json::json!("user-a");
    successor["phoenix"] = serde_json::json!(true);
    successor["created_at"] = serde_json::json!("2026-09-21 10:00:00");
    successor["updated_at"] = serde_json::json!("2026-09-21 10:00:00");
    let made = instance.call("respawn_phoenix_successor", &["1", &successor.to_string()]);
    assert!(made.status.success(), "{}", describe(&made));

    assert_eq!(
        column(&instance, "SELECT phoenix FROM tasks WHERE id = 1"),
        "false",
        "the predecessor's flag must clear"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM tasks WHERE title = 'recurring' AND phoenix = true"
        ),
        "1",
        "exactly one new successor must carry the flag onward"
    );

    let missing = instance.call(
        "respawn_phoenix_successor",
        &["999", &successor.to_string()],
    );
    assert!(
        !missing.status.success(),
        "a missing predecessor must refuse rather than orphan a successor"
    );
}

/// Plain upsert by id: a second call with a changed label overwrites rather
/// than duplicating.
#[test]
fn register_host_upserts_by_id() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let first = instance.call("register_host", &["host-1", "first-label", "user-a"]);
    assert!(first.status.success(), "{}", describe(&first));
    let second = instance.call("register_host", &["host-1", "second-label", "user-a"]);
    assert!(second.status.success(), "{}", describe(&second));

    assert_eq!(column(&instance, "SELECT count(*) AS c FROM hosts"), "1");
    assert_eq!(
        column(&instance, "SELECT label FROM hosts WHERE id = 'host-1'"),
        "second-label"
    );
}

// ---------------------------------------------------------------------------
// Phase 9: settings (`docs/specs/settings.allium`)
// ---------------------------------------------------------------------------

/// `save_setting` upserts by `(host, key)`, the same shape `register_host`
/// upserts by id.
#[test]
fn save_setting_upserts_by_host_and_key() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let first = instance.call("save_setting", &["host-a", "theme", "dark"]);
    assert!(first.status.success(), "{}", describe(&first));
    let second = instance.call("save_setting", &["host-a", "theme", "light"]);
    assert!(second.status.success(), "{}", describe(&second));

    assert_eq!(column(&instance, "SELECT count(*) AS c FROM settings"), "1");
    assert_eq!(
        column(
            &instance,
            "SELECT value FROM settings WHERE id = 'host-a/theme'"
        ),
        "light"
    );
}

/// **Test 1 of task #4913, exercised against the real store**: a setting
/// written by one host does not affect, and is not returned as, another
/// host's row.
#[test]
fn a_setting_written_by_one_host_is_a_separate_row_from_anothers() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    instance.call("save_setting", &["host-a", "theme", "dark"]);
    instance.call("save_setting", &["host-b", "theme", "light"]);

    assert_eq!(column(&instance, "SELECT count(*) AS c FROM settings"), "2");
    assert_eq!(
        column(
            &instance,
            "SELECT value FROM settings WHERE id = 'host-a/theme'"
        ),
        "dark",
        "host-a's own write must be unaffected by host-b's"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT value FROM settings WHERE id = 'host-b/theme'"
        ),
        "light"
    );
}

#[test]
fn clear_setting_removes_the_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call("save_setting", &["host-a", "theme", "dark"]);

    let cleared = instance.call("clear_setting", &["host-a", "theme"]);
    assert!(cleared.status.success(), "{}", describe(&cleared));

    assert!(no_rows(
        &instance,
        "SELECT id FROM settings WHERE id = 'host-a/theme'"
    ));
}

/// Clearing a key that was never set is a no-op, not a refusal
/// (`settings.allium: ClearSetting`).
#[test]
fn clearing_an_unset_setting_is_a_no_op() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let cleared = instance.call("clear_setting", &["host-a", "never-set"]);
    assert!(cleared.status.success(), "{}", describe(&cleared));
}

// ---------------------------------------------------------------------------
// Phase 7: poll ownership (`core.allium: PollOwner`)
// ---------------------------------------------------------------------------

/// `claim_poll_owner` fills an ABSENT row and is a no-op on an EXISTING
/// one, whoever it names — the only way an existing claim moves is
/// `override_poll_owner`. `pr-workflow.allium: PollPrStatus`.
#[test]
fn claim_poll_owner_fills_an_absent_row_but_not_an_existing_one() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);

    let first = instance.call("claim_poll_owner", &["task", "1", "host-a"]);
    assert!(first.status.success(), "{}", describe(&first));
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'task' AND scope_id = 1"
        ),
        "host-a"
    );

    // A second claim by a different host must not steal it.
    let second = instance.call("claim_poll_owner", &["task", "1", "host-b"]);
    assert!(second.status.success(), "{}", describe(&second));
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'task' AND scope_id = 1"
        ),
        "host-a",
        "claim must not steal an existing owner"
    );
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM poll_owners"),
        "1"
    );
}

/// `override_poll_owner` reassigns an EXISTING claim unconditionally —
/// the only way ownership ever moves. `pr-workflow.allium:
/// OverridePrPollOwner`.
#[test]
fn override_poll_owner_reassigns_unconditionally() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);

    let claimed = instance.call("claim_poll_owner", &["task", "1", "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));

    let overridden = instance.call("override_poll_owner", &["task", "1", "host-b"]);
    assert!(overridden.status.success(), "{}", describe(&overridden));
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'task' AND scope_id = 1"
        ),
        "host-b",
        "override must reassign even an existing owner"
    );
}

/// `override_poll_owner` on a scope with no existing claim behaves like
/// an ordinary claim — `feeds.allium: OverrideFeedOwner`'s "an override with
/// no existing owner behaves like a normal claim".
#[test]
fn override_poll_owner_with_no_existing_claim_behaves_like_a_claim() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "Feed Epic", "backlog", 0)]).to_string()],
    );

    let overridden = instance.call("override_poll_owner", &["epic", "1", "host-a"]);
    assert!(overridden.status.success(), "{}", describe(&overridden));
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'epic' AND scope_id = 1"
        ),
        "host-a"
    );

    // A later claim by another host must not steal it back.
    let claimed = instance.call("claim_poll_owner", &["epic", "1", "host-b"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'epic' AND scope_id = 1"
        ),
        "host-a"
    );
}

/// Task-scope and epic-scope claims are independent rows, even when they
/// happen to share the same numeric id — the `scope` column, not the id
/// alone, is what `core.allium: UniquePollOwnerPerScope` keys on.
#[test]
fn task_and_epic_scope_claims_do_not_collide_on_the_same_id() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "Feed Epic", "backlog", 0)]).to_string()],
    );

    let task_claim = instance.call("claim_poll_owner", &["task", "1", "host-a"]);
    assert!(task_claim.status.success(), "{}", describe(&task_claim));
    let epic_claim = instance.call("claim_poll_owner", &["epic", "1", "host-b"]);
    assert!(epic_claim.status.success(), "{}", describe(&epic_claim));

    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'task' AND scope_id = 1"
        ),
        "host-a"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'epic' AND scope_id = 1"
        ),
        "host-b"
    );
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM poll_owners"),
        "2"
    );
}

/// An invalid scope string is rejected rather than silently accepted — the
/// collapse from four scope-specific reducers to one scope-parameterised pair
/// traded compile-time scope safety (four distinct function names) for a
/// runtime check (`require_poll_scope`), so that check needs its own coverage.
#[test]
fn claim_poll_owner_rejects_an_unrecognised_scope() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let result = instance.call("claim_poll_owner", &["bogus", "1", "host-a"]);
    assert!(
        !result.status.success(),
        "an unrecognised scope must be rejected, got {}",
        describe(&result)
    );
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM poll_owners"),
        "0"
    );
}

/// **The seed, end to end** (task #4916; `spacetime-seed.allium`'s
/// `SeedSharedStore`). A board that has only ever run on SQLite is moved into
/// an empty store, and a store-backed board opened afterwards sees what the
/// SQLite board saw: its epic-less task on the user board, and its epic with
/// the task inside it. A second seed is refused.
#[test]
fn seeding_a_board_puts_its_rows_on_that_persons_store_backed_board() {
    use dispatch_tui::db::{
        CreateTaskRequest, Database, EpicCrud, EpicRead, HostStore, TaskCrud, TaskRead,
    };
    use dispatch_tui::models::TaskStatus;

    if !spacetime_available_or_skip() {
        return;
    }
    // The board connects to the fixed shared database name, so the module is
    // published under it — this instance is private to the test.
    let mut instance = Instance::start("module-test");
    instance.set_database(dispatch_tui::sync::SHARED_DATABASE_NAME);
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("board.db");
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let request = |title, epic_id| CreateTaskRequest {
            title,
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        };
        let (free, in_epic, epic) = {
            let db = Database::open(&db_path).await.unwrap();
            db.ensure_host_identity().await.unwrap();
            let epic = db.create_epic("An epic", "", None).await.unwrap();
            let free = db.create_task(request("Free", None)).await.unwrap();
            let in_epic = db
                .create_task(request("In the epic", Some(epic.id)))
                .await
                .unwrap();
            (free, in_epic, epic.id)
        };

        let store = dispatch_tui::spacetime::SpacetimeCliStore::new(
            Arc::new(RealProcessRunner::default()),
            instance.database(),
            Some(instance.host()),
        )
        .with_config_path(instance.config_path());
        let mut out = Vec::new();
        dispatch_tui::cli::commands::seed_store(&db_path, instance.host(), &store, &mut out)
            .await
            .unwrap_or_else(|e| panic!("seed: {e:#}"));

        let identity = Database::open(&db_path)
            .await
            .unwrap()
            .user_identity()
            .await
            .unwrap()
            .expect("the seed's connection minted an identity");
        assert_eq!(
            column(
                &instance,
                &format!("SELECT owner FROM tasks WHERE id = {}", free.0)
            ),
            identity
        );
        assert_eq!(
            column(
                &instance,
                &format!("SELECT created_by FROM epics WHERE id = {}", epic.0)
            ),
            identity
        );

        // What a board opened now would read, through the store.
        let board = dispatch_tui::runtime::open_cli_store(&db_path, Some(instance.host()))
            .await
            .unwrap();
        let tasks: Vec<_> = board
            .database
            .list_all()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(
            tasks.contains(&free) && tasks.contains(&in_epic),
            "{tasks:?}"
        );
        assert!(board
            .database
            .list_epics()
            .await
            .unwrap()
            .iter()
            .any(|e| e.id == epic));

        let again = dispatch_tui::cli::commands::seed_store(
            &db_path,
            instance.host(),
            &store,
            &mut Vec::new(),
        )
        .await
        .expect_err("a second seed must be refused");
        assert!(
            format!("{again:#}").contains("store not empty"),
            "{again:#}"
        );
    });
}
