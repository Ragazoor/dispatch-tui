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
    std::fs::create_dir_all(into.join("src")).expect("scratch module dir");
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
    tracked.push((
        "spacetime/module/Cargo.toml".to_string(),
        "Cargo.toml".to_string(),
    ));
    for (tracked, dest) in tracked {
        let bytes = git(&["show", &format!("HEAD:{tracked}")]);
        std::fs::write(into.join(dest), bytes).expect("write scratch module file");
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
    use dispatch_tui::embeddings::{rag_rank_learnings, serialize_embedding, RagRankParams};
    use dispatch_tui::store::LearningFilter;

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
        // subscription — this is `LearningStore::list_all_approved_non_task_learnings`'s
        // backing, `SharedRows::approved_non_task_learnings_with_embedding`.
        let candidates = rows.approved_non_task_learnings_with_embedding();
        assert_eq!(candidates.len(), 2, "both learnings must have arrived");

        let decoded: Vec<(dispatch_tui::models::Learning, Vec<f32>)> = candidates
            .into_iter()
            .map(|(l, bytes)| (l, dispatch_tui::embeddings::deserialize_embedding(&bytes)))
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
    // `done`, not `backlog`: this test is about learning detachment, not the
    // delete guard, and task #4971's reducer guard refuses to delete an
    // undone task.
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "source", "done", 1, "")]).to_string()],
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

/// `tasks.allium: DeleteTask`'s `requires: task.status = done`, enforced HERE,
/// at the reducer, not only by the TUI input layer's subscription-view check.
/// Calling the reducer directly, as if a stale view (a task reopened on one
/// board still reading as done on another) had let the caller through, must
/// still refuse and delete nothing: permanent delete makes the gap a
/// client-only guard leaves irrecoverable (task #4971). The single-row twin of
/// `deleting_an_epic_with_an_undone_subtree_task_is_refused_at_the_reducer`.
#[test]
fn deleting_an_undone_task_is_refused_at_the_reducer() {
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
        &[&serde_json::json!([task_json(1, "still running", "running", 1, "")]).to_string()],
    );

    let refused = instance.call("delete_task", &["1"]);
    assert!(
        !refused.status.success(),
        "a task that is not done must be refused: {}",
        describe(&refused)
    );
    assert!(
        describe(&refused).contains("not done"),
        "the refusal must say why: {}",
        describe(&refused)
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"),
        "a refused delete must leave the task row in place"
    );

    // The same task once finished is accepted, so the refusal is the rule
    // rather than the reducer being broken.
    let finished = instance.call(
        "patch_task",
        &["1", &patch_setting("status", "done").to_string()],
    );
    assert!(finished.status.success(), "{}", describe(&finished));
    let accepted = instance.call("delete_task", &["1"]);
    assert!(accepted.status.success(), "{}", describe(&accepted));
    assert!(
        no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"),
        "the accepted delete must remove the row"
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
    use dispatch_tui::store::UsageQuery;

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
// would. `src/sync/tests/writes.rs` covers the `Store`'s decoding far more
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
/// precondition failing is what lets the `Store` read `NoOp` back
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
/// a second is refused. The `Store` reads that refusal as `false` via
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
// Phase 6c: feed ingestion and the stragglers
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

// ---------------------------------------------------------------------------
// Retired feed items — delete means "I am done with this item"
// ---------------------------------------------------------------------------
//
// `core.allium: RetiredFeedItem`, written by `tasks.allium: DeleteTask` and
// `epics.allium: DeleteEpic`, read by `feeds.allium:
// IngestSkipsRetiredFeedItems` and dropped by `DropClosedRetiredFeedItems`
// (inside `ingest_feed`).
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

/// `core/RetiredFeedItem: UniqueRetiredFeedItemPerFeed`. Deleting a second
/// task that carries an already-retired (feed epic, external_id) is a no-op;
/// the same id under a DIFFERENT feed epic is its own record. Both properties
/// go entirely through `delete_task` — there is no standalone
/// direct-retirement reducer, since every real retirement writes inline from
/// a delete (or the migration).
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

    // Two tasks with the same id in two sub-epics of one feed: both deletes
    // resolve to the same (feed_epic, external_id) key and the second must
    // not write a second record.
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

    // The same external_id, deleted under a DIFFERENT feed epic, is its own
    // record.
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([feed_task_json(3, "ext-1", "done", 4)]).to_string()],
    );
    let other = instance.call("delete_task", &["3"]);
    assert!(other.status.success(), "{}", describe(&other));
    assert_eq!(
        retired_count(&instance, "external_id = 'ext-1'"),
        "2",
        "the same id under a different feed epic is a separate record"
    );
}

/// `feeds.allium: IngestSkipsRetiredFeedItems`, via the reconciling reducer:
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

    let upserted = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([
                feed_item_json("ext-retired", "back again", "backlog"),
                feed_item_json("ext-new-1", "fresh", "backlog"),
            ])
            .to_string(),
            "test-creator",
        ],
    );
    assert!(upserted.status.success(), "{}", describe(&upserted));
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM tasks WHERE external_id = 'ext-retired'"
        ),
        "upsert_feed_tasks must not insert a retired id"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM tasks WHERE external_id = 'ext-new-1'"
        ),
        "1",
        "upsert_feed_tasks must still insert the batch's non-retired item"
    );
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
    let seeded = instance.call(
        "seed_retired_feed_items",
        &[&serde_json::json!([retired_json(100, 1, "ext-1")]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

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

/// `epics.allium: DeleteEpic`'s guard — every task anywhere in the subtree is
/// `done` — enforced HERE, at the reducer, not only in
/// `EpicService::delete_epic`'s subscription-view check. Calling the reducer
/// directly, as if that client-side guard had been bypassed or fooled by an
/// incomplete view, must still refuse: permanent delete makes the gap a
/// client-only guard leaves irrecoverable (task #4971).
#[test]
fn deleting_an_epic_with_an_undone_subtree_task_is_refused_at_the_reducer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            epic_json(1, "root", "backlog", 0),
            epic_json(2, "sub", "backlog", 1),
        ])
        .to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "done one", "done", 1, ""),
            task_json(2, "still running", "running", 2, ""),
        ])
        .to_string()],
    );

    let refused = instance.call("delete_epic", &["1"]);
    assert!(
        !refused.status.success(),
        "a subtree holding an undone task anywhere in it (even in a nested \
         sub-epic) must be refused: {}",
        describe(&refused)
    );
    assert!(
        describe(&refused).contains("not done"),
        "the refusal must say why: {}",
        describe(&refused)
    );

    assert!(
        !no_rows(&instance, "SELECT id FROM epics"),
        "a refused delete must leave the whole subtree in place"
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM tasks"),
        "a refused delete must leave every task in place too"
    );

    // The same subtree with its task finished is accepted, so the refusal is
    // the rule rather than the reducer being broken.
    let finished = instance.call(
        "patch_task",
        &["2", &patch_setting("status", "done").to_string()],
    );
    assert!(finished.status.success(), "{}", describe(&finished));
    let accepted = instance.call("delete_epic", &["1"]);
    assert!(accepted.status.success(), "{}", describe(&accepted));
}

/// `tasks.allium: BatchDelete`'s atomic guarantee, over the reducer directly:
/// one task in the batch is done (would pass on its own, exactly like
/// `deleting_an_undone_task_is_refused_at_the_reducer`'s accepted case) and
/// one is not. Before `batch_delete` existed, the caller looped independent
/// `delete_task`/`delete_epic` calls, so the done one would have been deleted
/// while the other alone was refused — a partial batch. `batch_delete` must
/// instead refuse the WHOLE call and leave every row, including the done one,
/// untouched.
#[test]
fn batch_delete_refuses_the_whole_batch_when_one_task_is_not_done() {
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
            task_json(1, "finished", "done", 1, ""),
            task_json(2, "still running", "running", 1, ""),
        ])
        .to_string()],
    );

    let refused = instance.call("batch_delete", &["[1, 2]", "[]"]);
    assert!(
        !refused.status.success(),
        "one non-done task must refuse the whole batch: {}",
        describe(&refused)
    );
    assert!(
        describe(&refused).contains("not done"),
        "the refusal must say why: {}",
        describe(&refused)
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"),
        "the done task must survive too — a refused batch deletes nothing at all"
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM tasks WHERE id = 2"),
        "the not-done task must survive"
    );

    // The same batch once both are done is accepted and deletes everything
    // together, so the refusal above is the atomicity rule rather than the
    // reducer being broken.
    let finished = instance.call(
        "patch_task",
        &["2", &patch_setting("status", "done").to_string()],
    );
    assert!(finished.status.success(), "{}", describe(&finished));
    let accepted = instance.call("batch_delete", &["[1, 2]", "[]"]);
    assert!(accepted.status.success(), "{}", describe(&accepted));
    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "the accepted batch must remove every row"
    );
}

/// The success path over BOTH domains at once: a plain task and a
/// fully-done epic's subtree, deleted together in one `batch_delete` call —
/// the shape `handle_batch_delete` (src/tui/update/selection.rs) sends when
/// the selection mixes tasks and an epic.
#[test]
fn batch_delete_deletes_a_task_and_an_epic_subtree_together() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "done", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "plain", "done", 0, "user-1"),
            task_json(2, "in the epic", "done", 1, ""),
        ])
        .to_string()],
    );

    let accepted = instance.call("batch_delete", &["[1]", "[1]"]);
    assert!(accepted.status.success(), "{}", describe(&accepted));
    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "both the plain task and the epic's subtask must be gone"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM epics"),
        "the epic must be gone too"
    );
}

/// The same learning-detachment
/// `deleting_a_task_detaches_its_learnings_and_cascades_their_retrievals`
/// covers for `delete_task`, missing for the same reason `delete_task_side_effects`
/// exists: each path that removes task rows must reproduce it.
#[test]
fn deleting_an_epic_detaches_its_subtasks_learnings_and_cascades_their_retrievals() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    // `done`, not `backlog`: this test is about learning detachment, not the
    // delete guard, and task #4971's server-side guard now refuses to delete
    // a subtree holding an undone task.
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "source", "done", 1, "")]).to_string()],
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

    let some_1 = column(
        &instance,
        "SELECT source_task_id FROM learnings WHERE id = 1",
    );

    let deleted = instance.call("delete_epic", &["1"]);
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
        "a retrieval naming a task inside the deleted epic must go with it"
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
    let me = instance.identity();

    let first = instance.call("register_host", &["host-1", "first-label", &me]);
    assert!(first.status.success(), "{}", describe(&first));
    let second = instance.call("register_host", &["host-1", "second-label", &me]);
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

// -- Work packages (`docs/specs/work-packages.allium`) -----------------------

fn work_package_json(title: &str) -> serde_json::Value {
    serde_json::json!({
        "id": 0,
        "title": title,
        "description": "",
        "created_by": "user-a",
        "created_at": "2026-10-09 10:00:00",
        "updated_at": "2026-10-09 10:00:00",
    })
}

/// `work-packages.allium: CreateWorkPackage` — a blank title is refused and
/// nothing is created; a padded one is trimmed.
#[test]
fn creating_a_work_package_trims_its_title_and_refuses_a_blank_one() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let blank = instance.call(
        "create_work_package",
        &[&work_package_json("   ").to_string()],
    );
    assert!(!blank.status.success(), "{}", describe(&blank));
    assert!(no_rows(&instance, "SELECT id FROM work_packages"));

    let made = instance.call(
        "create_work_package",
        &[&work_package_json("  Window 1  ").to_string()],
    );
    assert!(made.status.success(), "{}", describe(&made));
    assert_eq!(
        column(&instance, "SELECT title FROM work_packages WHERE id = 1"),
        "Window 1"
    );
}

/// `work-packages.allium: UpdateWorkPackage` — a missing package is refused,
/// a blank title is refused, a description change leaves the title alone.
#[test]
fn patching_a_work_package_refuses_a_missing_package_and_a_blank_title() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_work_package",
        &[&work_package_json("Window 1").to_string()],
    );
    let missing = instance.call(
        "patch_work_package",
        &[
            "99",
            &serde_json::json!({"title": {"some": "x"}, "description": {"none": []}}).to_string(),
        ],
    );
    assert!(!missing.status.success(), "{}", describe(&missing));
    let blank = instance.call(
        "patch_work_package",
        &[
            "1",
            &serde_json::json!({"title": {"some": " "}, "description": {"none": []}}).to_string(),
        ],
    );
    assert!(!blank.status.success(), "{}", describe(&blank));
    let described = instance.call(
        "patch_work_package",
        &[
            "1",
            &serde_json::json!({"title": {"none": []}, "description": {"some": "the plan"}})
                .to_string(),
        ],
    );
    assert!(described.status.success(), "{}", describe(&described));
    assert_eq!(
        column(&instance, "SELECT title FROM work_packages WHERE id = 1"),
        "Window 1"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT description FROM work_packages WHERE id = 1"
        ),
        "the plan"
    );
}

/// `work-packages.allium: LinkItem` / `UnlinkItem` — both idempotent; a link
/// to an unknown item, an unknown package or an unknown kind is refused.
#[test]
fn linking_is_idempotent_and_refuses_unknown_targets() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_work_package",
        &[&work_package_json("Window 1").to_string()],
    );
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "backlog", 1, "")]).to_string()],
    );

    for _ in 0..2 {
        let linked = instance.call("link_work_package_item", &["1", "task", "1"]);
        assert!(linked.status.success(), "{}", describe(&linked));
    }
    let linked = instance.call("link_work_package_item", &["1", "epic", "1"]);
    assert!(linked.status.success(), "{}", describe(&linked));
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM work_package_links"),
        "2",
        "a repeated link must not duplicate"
    );

    for args in [
        ["1", "task", "99"],
        ["1", "epic", "99"],
        ["99", "task", "1"],
        ["1", "board", "1"],
    ] {
        let refused = instance.call("link_work_package_item", &args);
        assert!(
            !refused.status.success(),
            "{args:?}: {}",
            describe(&refused)
        );
    }

    for _ in 0..2 {
        let unlinked = instance.call("unlink_work_package_item", &["1", "task", "1"]);
        assert!(unlinked.status.success(), "{}", describe(&unlinked));
    }
    assert_eq!(
        column(&instance, "SELECT kind FROM work_package_links"),
        "epic"
    );
}

/// `work-packages.allium: DeleteWorkPackage` — the package and its links go;
/// the linked items stay.
#[test]
fn deleting_a_work_package_removes_its_links_and_keeps_the_items() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_work_package",
        &[&work_package_json("Window 1").to_string()],
    );
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "backlog", 1, "")]).to_string()],
    );
    let linked = instance.call("link_work_package_item", &["1", "task", "1"]);
    assert!(linked.status.success(), "{}", describe(&linked));

    let deleted = instance.call("delete_work_package", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert!(no_rows(&instance, "SELECT id FROM work_packages"));
    assert!(no_rows(&instance, "SELECT id FROM work_package_links"));
    assert_eq!(
        column(&instance, "SELECT title FROM tasks WHERE id = 1"),
        "t"
    );
}

/// `work-packages.allium: DeletingAnItemRemovesItsLinks` — deleting a task,
/// or an epic with its subtree, removes every link naming a deleted row, and
/// only those: a task and an epic sharing an id are different items.
#[test]
fn deleting_linked_items_removes_their_links_and_only_theirs() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_work_package",
        &[&work_package_json("Window 1").to_string()],
    );
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            epic_json(1, "E", "done", 0),
            epic_json(2, "F", "backlog", 0)
        ])
        .to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "in E", "done", 1, ""),
            task_json(2, "in F", "done", 2, ""),
            task_json(3, "kept", "backlog", 2, ""),
        ])
        .to_string()],
    );
    for (kind, id) in [
        ("epic", "1"),
        ("task", "1"),
        ("task", "2"),
        ("task", "3"),
        ("epic", "2"),
    ] {
        let linked = instance.call("link_work_package_item", &["1", kind, id]);
        assert!(linked.status.success(), "{}", describe(&linked));
    }

    let deleted = instance.call("delete_task", &["2"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    let deleted = instance.call("delete_epic", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    let kept = instance.sql("SELECT id FROM work_package_links");
    assert!(kept.contains("1/task/3"), "{kept}");
    assert!(kept.contains("1/epic/2"), "{kept}");
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM work_package_links"),
        "2",
        "{kept}"
    );
}

/// The batch path owes the same cascade (`tasks.allium: BatchDelete`).
#[test]
fn batch_delete_removes_the_links_of_everything_it_deletes() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_work_package",
        &[&work_package_json("Window 1").to_string()],
    );
    instance.call(
        "seed_epics",
        &[
            &serde_json::json!([epic_json(1, "E", "done", 0), epic_json(2, "F", "done", 0)])
                .to_string(),
        ],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(2, "in F", "done", 2, "")]).to_string()],
    );
    for (kind, id) in [("epic", "1"), ("task", "2")] {
        let linked = instance.call("link_work_package_item", &["1", kind, id]);
        assert!(linked.status.success(), "{}", describe(&linked));
    }

    let deleted = instance.call("batch_delete", &["[2]", "[1]"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert!(no_rows(&instance, "SELECT id FROM work_package_links"));
}

// -- store-split M2 (task #53255): data rules in the reducers and the gate ------

fn blank_epic_patch_json() -> serde_json::Value {
    let mut patch = serde_json::Map::new();
    for field in [
        "title",
        "description",
        "status",
        "plan_path",
        "sort_order",
        "auto_dispatch",
        "parent_epic_id",
        "feed_command",
        "feed_interval_secs",
        "group_by_repo",
        "feed_role",
        "origin",
        "feed_append_only",
        "completed_at",
    ] {
        patch.insert(field.into(), serde_json::json!({"none": []}));
    }
    serde_json::Value::Object(patch)
}

/// `mcp-task-tools.allium: UpdateTaskViaMcp`, enforced where the rows are.
#[test]
fn an_illegal_sub_status_is_refused_at_the_reducer_with_the_old_wording() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_epic",
        &[&epic_json(0, "epic", "backlog", 0).to_string()],
    );
    instance.call(
        "create_task",
        &[&task_json(0, "task", "backlog", 1, "").to_string()],
    );

    let refused = instance.call(
        "patch_task",
        &["1", &patch_setting("sub_status", "active").to_string()],
    );
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("sub_status 'active' is not valid for status 'backlog'"),
        "{}",
        describe(&refused)
    );

    // Judged against the status the patch moves to, not the one the row has.
    let mut both = patch_setting("sub_status", "active");
    both["status"] = serde_json::json!({"some": "running"});
    let accepted = instance.call("patch_task", &["1", &both.to_string()]);
    assert!(accepted.status.success(), "{}", describe(&accepted));
}

/// `epics.allium: UpdateEpicViaMcp` / `ReparentEpicFromTui`.
#[test]
fn a_parent_cycle_is_refused_at_the_reducer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_epic",
        &[&epic_json(0, "a", "backlog", 0).to_string()],
    );
    instance.call(
        "create_epic",
        &[&epic_json(0, "b", "backlog", 1).to_string()],
    );

    let mut patch = blank_epic_patch_json();
    patch["parent_epic_id"] = serde_json::json!({"some": 2});
    let refused = instance.call("patch_epic", &["1", &patch.to_string()]);
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("would create a cycle in the epic hierarchy"),
        "{}",
        describe(&refused)
    );
}

/// `epics.allium`'s routing note: the store places a task of a grouped epic.
#[test]
fn a_task_created_in_a_grouped_epic_lands_in_its_repo_sub_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let mut root = epic_json(0, "root", "backlog", 0);
    root["group_by_repo"] = serde_json::json!(true);
    instance.call("create_epic", &[&root.to_string()]);
    let mut task = task_json(0, "task", "backlog", 1, "");
    task["repo_path"] = serde_json::json!("/x/alpha");
    let created = instance.call("create_task", &[&task.to_string()]);
    assert!(created.status.success(), "{}", describe(&created));

    let epics = instance
        .sql("SELECT id, title, parent_epic_id, origin FROM epics WHERE origin = 'repo-group'");
    assert!(
        epics.contains("alpha") && epics.contains("repo-group"),
        "the sub-epic for the task's repo: {epics}"
    );
    let landed = instance.sql("SELECT epic_id FROM tasks");
    assert!(
        !landed.contains("[1]") && landed.contains("[2]"),
        "the task is in the sub-epic (epic 2), not the root: {landed}"
    );
}

/// A connection stamps the rules version, so a store upgraded by a plain
/// publish passes the gate: nothing in production calls `set_schema_version`.
#[test]
fn a_board_connecting_to_an_upgraded_store_passes_the_version_gate() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let scratch = tempfile::tempdir().expect("temp dir");
    let previous = committed_module(scratch.path());
    let first = instance.publish(&previous, Some(&scratch_target_dir()));
    assert!(first.status.success(), "{}", describe(&first));
    let migrated = instance.publish(&module_path(), None);
    assert!(migrated.status.success(), "{}", describe(&migrated));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let connector =
            SpacetimeSdkConnector::new(instance.database(), Arc::new(SharedRows::new()));
        connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("the gate must accept the module it was built with: {e}"));
    });
    let json = instance.sql("SELECT version FROM rules_version");
    assert!(
        json.contains(&format!(
            "[{}]",
            dispatch_tui::sync::rules_gate::BOARD_RULES_VERSION
        )),
        "the connection stamped the module's rules version: {json}"
    );
}

// -- store-split M4 (task #53257): dispatch as request rows ------------------
//
// `store-split.allium`: DispatchRequest and its rules. Each test drives the
// reducers a board and its peers call, and reads the rows back over SQL. What
// a unit test can pin without a store (the refusal wording, mode derivation,
// the backstop's body on a row whose deadline has passed) is pinned in the
// module's own `tests.rs` ("dispatch_requests"); here it is the transaction:
// that the claim and the row land together, that a refusal writes nothing, and
// that the scheduled timer row is created and cancelled with its request.
//
// THE BACKSTOP ITSELF IS NOT FIRED HERE. It is due 70 seconds after the
// request, and the suite does not wait on a wall clock
// (`scripts/check-no-test-sleep.sh`). Its schedule row is asserted to exist and
// to go away when the request finishes; what it does when it fires is the
// module unit test `the_backstop_fails_an_open_row_as_timed_out`.

/// Register `host` in the host registry with `owner`, which a new host must
/// name (`store-split.allium`, D11).
fn register_host(instance: &Instance, host: &str, owner: &str) {
    let registered = instance.call("register_host", &[host, "", owner]);
    assert!(registered.status.success(), "{}", describe(&registered));
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

/// `host-a` and `host-b`, both owned by the identity this instance calls as,
/// so the calls below may act for either (`store-split.allium`, D11).
fn register_own_hosts(instance: &Instance) {
    let me = instance.identity();
    register_host(instance, "host-a", &me);
    register_host(instance, "host-b", &me);
}

/// One backlog task per `(id, tag, plan_path, host)`, all in epic 1, with
/// `host-a` and `host-b` registered to this instance's own identity.
fn seed_dispatchable(instance: &Instance, tasks: &[(i64, &str, &str, &str)]) {
    register_own_hosts(instance);
    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let rows: Vec<serde_json::Value> = tasks
        .iter()
        .map(|(id, tag, plan, host)| {
            let mut row = task_json(*id, "t", "backlog", 1, host);
            row["tag"] = serde_json::json!(tag);
            row["plan_path"] = serde_json::json!(plan);
            row
        })
        .collect();
    let seeded = instance.call("seed_tasks", &[&serde_json::json!(rows).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
}

/// `request_dispatch(task_id, requester, origin, mode)`; `mode` "" derives.
fn request_dispatch(
    instance: &Instance,
    task: i64,
    requester: &str,
    origin: &str,
    mode: &str,
) -> std::process::Output {
    instance.call(
        "request_dispatch",
        &[&task.to_string(), requester, origin, mode],
    )
}

fn request_column(instance: &Instance, field: &str, task: i64) -> String {
    column(
        instance,
        &format!("SELECT {field} FROM dispatch_requests WHERE task_id = {task}"),
    )
}

fn request_count(instance: &Instance) -> String {
    column(instance, "SELECT COUNT(*) AS n FROM dispatch_requests")
}

fn timer_count(instance: &Instance, request: i64) -> String {
    column(
        instance,
        &format!(
            "SELECT COUNT(*) AS n FROM dispatch_request_timeouts WHERE request_id = {request}"
        ),
    )
}

/// A status move by hand, with the sub-status the move needs.
fn move_task(instance: &Instance, id: i64, status: &str, sub_status: &str) {
    let mut patch = patch_setting("status", status);
    patch["sub_status"] = serde_json::json!({"some": sub_status});
    let moved = instance.call("patch_task", &[&id.to_string(), &patch.to_string()]);
    assert!(moved.status.success(), "{}", describe(&moved));
}

/// `RequestDispatch`: the claim and the row land in ONE transaction, and the
/// request's backstop timer is scheduled with it.
#[test]
fn a_dispatch_request_claims_the_task_and_writes_a_requested_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);

    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );
    assert_eq!(
        column(&instance, "SELECT sub_status FROM tasks WHERE id = 1"),
        "active"
    );
    assert_ne!(
        column(
            &instance,
            "SELECT last_pre_tool_use_at FROM tasks WHERE id = 1"
        ),
        "",
        "the claim stamps last_pre_tool_use_at"
    );

    assert_eq!(request_count(&instance), "1");
    assert_eq!(request_column(&instance, "kind", 1), "dispatch");
    assert_eq!(request_column(&instance, "host", 1), "host-a");
    assert_eq!(request_column(&instance, "origin", 1), "user");
    assert_eq!(request_column(&instance, "mode", 1), "dispatch");
    assert_eq!(request_column(&instance, "state", 1), "requested");
    assert_eq!(
        request_column(&instance, "requested_at", 1).len(),
        23,
        "requested_at is the module clock, in the store's timestamp spelling"
    );
    assert_eq!(request_column(&instance, "claimed_at", 1), "");
    assert_eq!(request_column(&instance, "finished_at", 1), "");
    let id: i64 = request_column(&instance, "id", 1).parse().expect("id");
    assert_eq!(
        timer_count(&instance, id),
        "1",
        "one backstop timer per request"
    );
}

/// `DispatchRequest.deadline` is stored on the row by the module: its own
/// `requested_at` plus `config.dispatch_request_timeout` (60 seconds), in the
/// store's timestamp spelling, so the host reads it straight off the row.
#[test]
fn a_dispatch_request_stores_its_deadline_sixty_seconds_after_it_was_requested() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    let format = "%Y-%m-%d %H:%M:%S%.3f";
    let parse = |s: &str| {
        chrono::NaiveDateTime::parse_from_str(s, format)
            .unwrap_or_else(|e| panic!("{s:?} is not a store timestamp: {e}"))
    };
    let requested_at = parse(&request_column(&instance, "requested_at", 1));
    let deadline = parse(&request_column(&instance, "deadline", 1));
    assert_eq!(deadline - requested_at, chrono::Duration::seconds(60));
}

/// `RequestDispatch` moves the dispatch-claim subagent clear point into the
/// same transaction: entries gone, count zero, no stop pending.
#[test]
fn a_dispatch_request_clears_the_tasks_subagent_state() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    let started = instance.call(
        "subagent_start",
        &["1", "agent-1", "session-1", "2026-10-10T10:00:00Z"],
    );
    assert!(started.status.success(), "{}", describe(&started));

    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    assert!(no_rows(
        &instance,
        "SELECT agent_id FROM task_subagents WHERE task_id = 1"
    ));
    assert_eq!(
        column(&instance, "SELECT live_subagents FROM tasks WHERE id = 1"),
        "0"
    );
    assert_eq!(
        column(&instance, "SELECT stop_pending FROM tasks WHERE id = 1"),
        "false"
    );
}

/// `DispatchClaimExclusive`, through the request reducer: the second request
/// is REFUSED, says why, and writes no second row.
#[test]
fn a_second_request_for_a_claimed_task_is_refused_and_writes_nothing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);

    let first = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(first.status.success(), "{}", describe(&first));
    let second = request_dispatch(&instance, 1, "host-b", "mcp", "");
    assert!(!second.status.success(), "{}", describe(&second));
    assert!(
        describe(&second).contains("already claimed"),
        "the refusal must say why: {}",
        describe(&second)
    );
    assert_eq!(request_count(&instance), "1");
}

/// `requires: task.host = null or task.host = requester`.
#[test]
fn a_request_for_a_task_on_another_host_is_refused_and_writes_nothing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "host-a")]);

    let refused = request_dispatch(&instance, 1, "host-b", "user", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );
    assert!(no_rows(&instance, "SELECT id FROM dispatch_requests"));
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM dispatch_request_timeouts"
    ));

    let won = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(won.status.success(), "{}", describe(&won));
}

/// `OneOpenDispatchRequestPerTask`: a task moved back to backlog by hand while
/// its request is still open is not requested a second time.
#[test]
fn a_backlog_task_with_an_open_request_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    let first = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(first.status.success(), "{}", describe(&first));
    move_task(&instance, 1, "backlog", "none");

    let refused = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("open dispatch request"),
        "{}",
        describe(&refused)
    );
    assert_eq!(request_count(&instance), "1");
}

/// D3: the caller's mode when named; otherwise plan -> dispatch, research tag
/// -> research, else dispatch. quick only when named.
#[test]
fn the_request_mode_is_derived_when_omitted_and_kept_when_named() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(
        &instance,
        &[
            (1, "", "", ""),
            (2, "research", "", ""),
            (3, "research", "/plans/p.md", ""),
            (4, "", "", ""),
            (5, "", "", ""),
        ],
    );
    for (task, origin, mode) in [
        (1, "user", ""),
        (2, "user", ""),
        (3, "user", ""),
        (4, "quick", "quick"),
        (5, "user", "research"),
    ] {
        let out = request_dispatch(&instance, task, "host-a", origin, mode);
        assert!(out.status.success(), "task {task}: {}", describe(&out));
    }
    assert_eq!(request_column(&instance, "mode", 1), "dispatch");
    assert_eq!(request_column(&instance, "mode", 2), "research");
    assert_eq!(request_column(&instance, "mode", 3), "dispatch");
    assert_eq!(request_column(&instance, "mode", 4), "quick");
    assert_eq!(request_column(&instance, "mode", 5), "research");
    assert_eq!(request_column(&instance, "origin", 4), "quick");
}

/// `HostClaimsDispatch`: only the row's host claims it, and only once.
#[test]
fn only_the_rows_host_claims_a_request_and_only_once() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);

    let foreign = instance.call("claim_dispatch_request", &[&id, "host-b"]);
    assert!(!foreign.status.success(), "{}", describe(&foreign));
    assert_eq!(request_column(&instance, "state", 1), "requested");

    let claimed = instance.call("claim_dispatch_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    assert_eq!(request_column(&instance, "state", 1), "provisioning");
    assert_eq!(request_column(&instance, "claimed_at", 1).len(), 23);

    let again = instance.call("claim_dispatch_request", &[&id, "host-a"]);
    assert!(
        !again.status.success(),
        "a second claim of the same row must be refused: {}",
        describe(&again)
    );
}

/// `HostReportsDispatched`: the row records the result, the task gets its
/// worktree, window and host, and the timer is cancelled.
#[test]
fn a_dispatched_report_records_the_result_on_the_row_and_the_task() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);

    let reported = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "true"],
    );
    assert!(reported.status.success(), "{}", describe(&reported));

    assert_eq!(request_column(&instance, "state", 1), "dispatched");
    assert_eq!(request_column(&instance, "finished_at", 1).len(), 23);
    assert_eq!(request_column(&instance, "worktree", 1), "/wt/1-t");
    assert_eq!(request_column(&instance, "tmux_window", 1), "task-1");
    assert_eq!(
        request_column(&instance, "reused_worktree", 1),
        "true",
        "the host's reused-worktree answer is kept on the row"
    );
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        "/wt/1-t"
    );
    assert_eq!(
        column(&instance, "SELECT tmux_window FROM tasks WHERE id = 1"),
        "task-1"
    );
    assert_eq!(
        column(&instance, "SELECT host FROM tasks WHERE id = 1"),
        "host-a"
    );
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );
    assert_eq!(timer_count(&instance, id.parse().unwrap()), "0");
}

/// `HostRepeatsDispatchedReport`: the same dispatched report sent again — a
/// host whose first report got no answer — is accepted and changes nothing;
/// a report naming another worktree is still refused.
#[test]
fn a_repeated_dispatched_report_is_accepted_and_changes_nothing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);
    let first = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(first.status.success(), "{}", describe(&first));
    let finished_at = request_column(&instance, "finished_at", 1);
    let updated_at = column(&instance, "SELECT updated_at FROM tasks WHERE id = 1");

    let again = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(again.status.success(), "{}", describe(&again));
    assert_eq!(request_column(&instance, "finished_at", 1), finished_at);
    assert_eq!(
        column(&instance, "SELECT updated_at FROM tasks WHERE id = 1"),
        updated_at
    );

    let other = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/other", "task-1", "false"],
    );
    assert!(!other.status.success(), "{}", describe(&other));
    assert_eq!(request_column(&instance, "worktree", 1), "/wt/1-t");
}

/// `epics.allium: PhoenixIsNeverChained`, enforced by `RequestDispatch`: the
/// epic chain's request for a phoenix task is refused and writes nothing;
/// a request from the TUI takes it.
#[test]
fn the_epic_chain_may_not_request_a_phoenix_task() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    let mut patch = empty_task_patch();
    patch["phoenix"] = serde_json::json!({"some": true});
    let marked = instance.call("patch_task", &["1", &patch.to_string()]);
    assert!(marked.status.success(), "{}", describe(&marked));

    let chained = request_dispatch(&instance, 1, "host-a", "epic_chain", "");
    assert!(!chained.status.success(), "{}", describe(&chained));
    assert!(
        describe(&chained).contains("phoenix"),
        "{}",
        describe(&chained)
    );
    assert_eq!(request_count(&instance), "0");
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );

    let by_hand = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(by_hand.status.success(), "{}", describe(&by_hand));
}

/// `requires: reporter = request.host` and `requires: request.state =
/// provisioning`, for both reports.
#[test]
fn a_report_from_another_host_or_on_an_unclaimed_row_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);

    let unclaimed = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(!unclaimed.status.success(), "{}", describe(&unclaimed));
    let unclaimed = instance.call("report_dispatch_failed", &[&id, "host-a", "boom"]);
    assert!(!unclaimed.status.success(), "{}", describe(&unclaimed));

    instance.call("claim_dispatch_request", &[&id, "host-a"]);
    let foreign = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-b", "/wt/1-t", "task-1", "false"],
    );
    assert!(!foreign.status.success(), "{}", describe(&foreign));
    let foreign = instance.call("report_dispatch_failed", &[&id, "host-b", "boom"]);
    assert!(!foreign.status.success(), "{}", describe(&foreign));

    assert_eq!(request_column(&instance, "state", 1), "provisioning");
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        ""
    );
}

/// `HostReportsDispatchFailed` + `ReleaseClaimOfFailedRequest`: the row fails
/// with the host's reason, the task goes back to backlog with its claim stamp
/// cleared, and the timer is cancelled.
#[test]
fn a_failed_report_fails_the_row_and_releases_the_claim() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);

    let failed = instance.call(
        "report_dispatch_failed",
        &[&id, "host-a", "deadline passed"],
    );
    assert!(failed.status.success(), "{}", describe(&failed));

    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(request_column(&instance, "reason", 1), "deadline passed");
    assert_eq!(request_column(&instance, "finished_at", 1).len(), 23);
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );
    assert_eq!(
        column(&instance, "SELECT sub_status FROM tasks WHERE id = 1"),
        "none"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT last_pre_tool_use_at FROM tasks WHERE id = 1"
        ),
        ""
    );
    assert_eq!(timer_count(&instance, id.parse().unwrap()), "0");

    // Released, so it can be requested afresh.
    let again = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(again.status.success(), "{}", describe(&again));
}

/// The release is CONDITIONAL: a task that got a worktree meanwhile stays
/// running even though its request failed.
#[test]
fn a_failed_report_leaves_a_task_that_got_a_worktree_alone() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);
    let patched = instance.call(
        "patch_task",
        &["1", &patch_setting("worktree", "/wt/1-t").to_string()],
    );
    assert!(patched.status.success(), "{}", describe(&patched));

    let failed = instance.call("report_dispatch_failed", &[&id, "host-a", "boom"]);
    assert!(failed.status.success(), "{}", describe(&failed));
    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );
}

/// A LATE REPORT IS REFUSED: once the row has failed, a dispatched report
/// writes neither the row nor the task.
#[test]
fn a_dispatched_report_after_the_row_failed_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);
    // A restart is the failure a test can trigger without a clock; the
    // backstop's "timed out" lands on the row the same way.
    let interrupted = instance.call("fail_interrupted_dispatch_requests", &["host-a"]);
    assert!(interrupted.status.success(), "{}", describe(&interrupted));

    let late = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(!late.status.success(), "{}", describe(&late));
    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(request_column(&instance, "worktree", 1), "");
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        ""
    );
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );
}

/// `HostRestartFailsInterrupted`: this host's PROVISIONING rows fail with
/// "host restarted" and release their claims; its requested rows and every
/// other host's rows are untouched.
#[test]
fn a_restart_fails_only_this_hosts_provisioning_requests() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(
        &instance,
        &[(1, "", "", ""), (2, "", "", ""), (3, "", "", "")],
    );
    request_dispatch(&instance, 1, "host-a", "user", "");
    request_dispatch(&instance, 2, "host-a", "user", "");
    request_dispatch(&instance, 3, "host-b", "user", "");
    let first = request_column(&instance, "id", 1);
    let third = request_column(&instance, "id", 3);
    instance.call("claim_dispatch_request", &[&first, "host-a"]);
    instance.call("claim_dispatch_request", &[&third, "host-b"]);

    let interrupted = instance.call("fail_interrupted_dispatch_requests", &["host-a"]);
    assert!(interrupted.status.success(), "{}", describe(&interrupted));

    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(request_column(&instance, "reason", 1), "host restarted");
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );
    assert_eq!(timer_count(&instance, first.parse().unwrap()), "0");
    assert_eq!(request_column(&instance, "state", 2), "requested");
    assert_eq!(request_column(&instance, "state", 3), "provisioning");
}

/// `DeletingATaskFailsItsOpenRequest`, through `delete_task`: the open request
/// fails "task deleted" in the same reducer and its timer is cancelled.
#[test]
fn deleting_a_task_fails_its_open_request_and_cancels_its_timer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    // delete_task requires done; a manual move leaves the request open.
    move_task(&instance, 1, "done", "none");

    let deleted = instance.call("delete_task", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"));
    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(request_column(&instance, "reason", 1), "task deleted");
    assert_eq!(timer_count(&instance, id.parse().unwrap()), "0");
}

/// The same, through `batch_delete`.
#[test]
fn batch_delete_fails_the_open_requests_of_the_tasks_it_deletes() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", ""), (2, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    request_dispatch(&instance, 2, "host-a", "user", "");
    let second = request_column(&instance, "id", 2);
    instance.call("claim_dispatch_request", &[&second, "host-a"]);
    move_task(&instance, 1, "done", "none");
    move_task(&instance, 2, "done", "none");

    let deleted = instance.call("batch_delete", &["[1, 2]", "[]"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    for task in [1, 2] {
        assert_eq!(request_column(&instance, "state", task), "failed");
        assert_eq!(request_column(&instance, "reason", task), "task deleted");
    }
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM dispatch_request_timeouts"
    ));
}

/// The same, through `batch_delete`'s epic arm: a deleted epic's subtree tasks
/// fail their open requests too.
#[test]
fn deleting_an_epic_fails_the_open_requests_of_its_subtree_tasks() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    move_task(&instance, 1, "done", "none");

    let deleted = instance.call("batch_delete", &["[]", "[1]"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(request_column(&instance, "reason", 1), "task deleted");
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM dispatch_request_timeouts"
    ));
}

/// The same, through `ingest_feed`'s stale delete, which removes a feed task
/// whatever its status — so here the request is still open on a running task.
#[test]
fn a_feed_ingest_that_deletes_a_stale_task_fails_its_open_request() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    let mut epic = epic_json(1, "Feed", "backlog", 0);
    epic["feed_command"] = serde_json::json!("echo []");
    let seeded = instance.call("seed_epics", &[&serde_json::json!([epic]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let upserted = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "one", "backlog")]).to_string(),
            "test-creator",
        ],
    );
    assert!(upserted.status.success(), "{}", describe(&upserted));
    let task: i64 = column(
        &instance,
        "SELECT id FROM tasks WHERE external_id = 'ext-1'",
    )
    .parse()
    .expect("task id");
    let requested = request_dispatch(&instance, task, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    let ingested = instance.call("ingest_feed", &["1", "[]", "false", "test-creator"]);
    assert!(ingested.status.success(), "{}", describe(&ingested));

    assert!(no_rows(
        &instance,
        &format!("SELECT id FROM tasks WHERE id = {task}")
    ));
    assert_eq!(request_column(&instance, "state", task), "failed");
    assert_eq!(request_column(&instance, "reason", task), "task deleted");
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM dispatch_request_timeouts"
    ));
}

// -- The epic rollup follows every request outcome ------------------------------
//
// `store-split.allium`: `ReleaseClaimOfFailedRequest` and
// `HostReportsDispatched` run the epic status rollup for the task's epic
// chain in the same reducer, so an epic never waits on a board to recalculate
// it after the module moved its subtask. Observed through a STALE epic: one
// seeded `done` over an unfinished subtask, which any rollup returns to
// backlog. The backstop timer takes the same failure path
// (`fail_dispatch_request`) but is not fired here — see the section header
// above.

/// [`seed_dispatchable`]'s one task, under an epic left `done` by a rollup
/// that never ran.
fn seed_under_a_stale_done_epic(instance: &Instance) {
    register_own_hosts(instance);
    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "done", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let seeded = instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "backlog", 1, "")]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
}

/// A requested and claimed row for task 1 on `host-a`, under an epic made
/// stale AFTER the request — the request rolls the chain up itself, so the
/// stale epic is seeded once the row is provisioning. Returns the request id.
fn provisioning_under_a_stale_done_epic(instance: &Instance) -> String {
    register_own_hosts(instance);
    let seeded = instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "backlog", 1, "")]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let requested = request_dispatch(instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));
    let id = request_column(instance, "id", 1);
    let claimed = instance.call("claim_dispatch_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "done", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    assert_eq!(
        column(instance, "SELECT status FROM epics WHERE id = 1"),
        "done",
        "the fixture's epic must still be stale when the outcome lands"
    );
    id
}

/// `RequestDispatch` rolls up the claimed task's epic chain in the same
/// reducer.
#[test]
fn a_request_rolls_up_the_claimed_subtasks_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_under_a_stale_done_epic(&instance);

    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
}

#[test]
fn a_failed_report_rolls_up_the_released_subtasks_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let id = provisioning_under_a_stale_done_epic(&instance);

    let failed = instance.call("report_dispatch_failed", &[&id, "host-a", "boom"]);
    assert!(failed.status.success(), "{}", describe(&failed));

    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
}

#[test]
fn a_restart_sweep_rolls_up_the_released_subtasks_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    provisioning_under_a_stale_done_epic(&instance);

    let swept = instance.call("fail_interrupted_dispatch_requests", &["host-a"]);
    assert!(swept.status.success(), "{}", describe(&swept));

    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
}

#[test]
fn a_dispatched_report_rolls_up_the_subtasks_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let id = provisioning_under_a_stale_done_epic(&instance);

    let reported = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(reported.status.success(), "{}", describe(&reported));

    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
}

// -- D11: only a host's owner may act for it ------------------------------------
//
// `store-split.allium`, D11: request_dispatch, claim_dispatch_request, both
// reports and the restart sweep refuse unless the host they name is
// registered with an owner and that owner is the caller. Each refusal writes
// nothing.

/// A request records who made it, from the call.
#[test]
fn a_request_records_its_caller_as_requested_by() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);

    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    assert_eq!(
        request_column(&instance, "requested_by", 1),
        instance.identity()
    );
}

/// A request naming a host somebody else owns is refused, and writes nothing:
/// no claim, no row.
#[test]
fn a_request_naming_another_persons_host_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    register_teammates_host(&instance, "their-host");

    let refused = request_dispatch(&instance, 1, "their-host", "user", "");

    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("their-host"),
        "{}",
        describe(&refused)
    );
    assert_eq!(request_count(&instance), "0");
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );
}

/// A NEW host may not be registered ownerless (`store-split.allium`, D11):
/// host ids are public, so an ownerless row would go to whoever registered
/// the id next as their own. The refused registration writes no row, and a
/// host with no row cannot be named. (An ownerless LEGACY row is refused for
/// everyone too; no reducer can write one any more, so that branch is the
/// module unit test `nobody_may_act_for_an_ownerless_host`.)
#[test]
fn a_request_naming_an_ownerless_or_unregistered_host_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    let ownerless = instance.call("register_host", &["fresh-host", "", ""]);
    assert!(!ownerless.status.success(), "{}", describe(&ownerless));
    assert!(
        describe(&ownerless).contains("owner"),
        "{}",
        describe(&ownerless)
    );
    assert!(no_rows(
        &instance,
        "SELECT id FROM hosts WHERE id = 'fresh-host'"
    ));

    for host in ["fresh-host", "nowhere"] {
        let refused = request_dispatch(&instance, 1, host, "user", "");
        assert!(!refused.status.success(), "{}", describe(&refused));
        assert!(
            describe(&refused).contains("not registered"),
            "{}",
            describe(&refused)
        );
    }
    assert_eq!(request_count(&instance), "0");
}

/// The claim, both reports and the restart sweep name the host too, so a
/// teammate can do none of them to this host's rows.
#[test]
fn claims_reports_and_the_sweep_are_refused_for_another_persons_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", ""), (2, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    request_dispatch(&instance, 2, "host-a", "user", "");
    let requested = request_column(&instance, "id", 1);
    let provisioning = request_column(&instance, "id", 2);
    let claimed = instance.call("claim_dispatch_request", &[&provisioning, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));

    let teammate = |reducer: &str, args: &[&str]| instance.call_as("teammate", reducer, args);
    let claim = teammate("claim_dispatch_request", &[&requested, "host-a"]);
    assert!(!claim.status.success(), "{}", describe(&claim));
    let dispatched = teammate(
        "report_dispatch_provisioned",
        &[&provisioning, "host-a", "/wt/2-t", "task-2", "false"],
    );
    assert!(!dispatched.status.success(), "{}", describe(&dispatched));
    let failed = teammate("report_dispatch_failed", &[&provisioning, "host-a", "x"]);
    assert!(!failed.status.success(), "{}", describe(&failed));
    let swept = teammate("fail_interrupted_dispatch_requests", &["host-a"]);
    assert!(!swept.status.success(), "{}", describe(&swept));

    assert_eq!(request_column(&instance, "state", 1), "requested");
    assert_eq!(request_column(&instance, "state", 2), "provisioning");
}

/// ...and nobody may sweep or claim for a host that has no owned row — the
/// ownerless registration that would have made one is refused.
#[test]
fn the_sweep_and_a_claim_are_refused_for_an_ownerless_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    let ownerless = instance.call("register_host", &["fresh-host", "", ""]);
    assert!(!ownerless.status.success(), "{}", describe(&ownerless));

    let swept = instance.call("fail_interrupted_dispatch_requests", &["fresh-host"]);
    assert!(!swept.status.success(), "{}", describe(&swept));
    assert!(
        describe(&swept).contains("not registered"),
        "{}",
        describe(&swept)
    );
    let claim = instance.call("claim_dispatch_request", &[&id, "fresh-host"]);
    assert!(!claim.status.success(), "{}", describe(&claim));
    assert_eq!(request_column(&instance, "state", 1), "requested");
}

// -- LocalHostOwnerIsWrittenOnce, enforced by register_host ---------------------

/// A teammate can neither take over this host nor relabel it.
#[test]
fn a_teammate_cannot_overwrite_another_persons_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let me = instance.identity();
    register_host(&instance, "host-a", &me);
    instance.call_as("teammate", "register_host", &["scratch", "", ""]);
    let them = instance.identity_as("teammate");

    let takeover = instance.call_as("teammate", "register_host", &["host-a", "", &them]);
    assert!(!takeover.status.success(), "{}", describe(&takeover));
    let relabel = instance.call_as("teammate", "register_host", &["host-a", "mine now", &me]);
    assert!(!relabel.status.success(), "{}", describe(&relabel));

    assert_eq!(
        column(&instance, "SELECT owner FROM hosts WHERE id = 'host-a'"),
        me
    );
}

/// Nobody may name another identity as a host's owner, new host or not.
#[test]
fn a_host_cannot_be_registered_to_someone_elses_identity() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let them = register_teammates_host(&instance, "their-host");

    let gifted = instance.call("register_host", &["new-host", "", &them]);
    assert!(!gifted.status.success(), "{}", describe(&gifted));
    assert!(no_rows(
        &instance,
        "SELECT id FROM hosts WHERE id = 'new-host'"
    ));
}

/// The owner's own re-register — a rename — still works, and an owned host's
/// owner cannot be cleared.
#[test]
fn the_owner_may_relabel_but_not_disown_its_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let me = instance.identity();
    register_host(&instance, "host-a", &me);

    let renamed = instance.call("register_host", &["host-a", "laptop", &me]);
    assert!(renamed.status.success(), "{}", describe(&renamed));
    let disowned = instance.call("register_host", &["host-a", "laptop", ""]);
    assert!(!disowned.status.success(), "{}", describe(&disowned));

    assert_eq!(
        column(&instance, "SELECT label FROM hosts WHERE id = 'host-a'"),
        "laptop"
    );
    assert_eq!(
        column(&instance, "SELECT owner FROM hosts WHERE id = 'host-a'"),
        me
    );
}

// -- store-split M5 (task #53258): wrap-up and exit-session as request rows ---
//
// `store-split.allium`: SessionRequest and its rules (W1-W13). As for M4, what
// a unit test can pin without a store (refusal wording, the token rules, the
// close's writes, the backstop's body on a row whose deadline has passed) is
// pinned in the module's own `tests.rs` ("session_requests"); here it is the
// transaction: that the close, the token's consumption, the epic chain and
// the exit_session row land together, that a refusal writes nothing and leaves
// the token valid, and that the scheduled timer row is created and cancelled
// with its request. THE BACKSTOP ITSELF IS NOT FIRED HERE (no wall-clock waits,
// `scripts/check-no-test-sleep.sh`).

/// One task per `(id, status, tag)` in epic 1, each with a worktree, a window
/// and `host-a`, so it can be wrapped up. `host-a` and `host-b` are registered
/// to this instance's own identity. `auto_dispatch` is the epic's flag.
fn seed_wrappable(instance: &Instance, auto_dispatch: bool, tasks: &[(i64, &str, &str)]) {
    register_own_hosts(instance);
    let mut epic = epic_json(1, "E", "running", 0);
    epic["auto_dispatch"] = serde_json::json!(auto_dispatch);
    let seeded = instance.call("seed_epics", &[&serde_json::json!([epic]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let rows: Vec<serde_json::Value> = tasks
        .iter()
        .map(|(id, status, tag)| {
            let mut row = task_json(*id, "t", status, 1, "host-a");
            row["tag"] = serde_json::json!(tag);
            row["worktree"] = serde_json::json!(format!("/wt/{id}-t"));
            row["tmux_window"] = serde_json::json!(format!("task-{id}"));
            row["sub_status"] = serde_json::json!(if *status == "review" {
                "awaiting_review"
            } else {
                "active"
            });
            row
        })
        .collect();
    let seeded = instance.call("seed_tasks", &[&serde_json::json!(rows).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
}

/// A backlog subtask of epic 1 with `sort_order` and `phoenix`.
fn seed_backlog_subtask(instance: &Instance, id: i64, sort_order: i64, phoenix: bool) {
    let mut row = task_json(id, "next", "backlog", 1, "");
    row["sort_order"] = serde_json::json!({"some": sort_order});
    row["phoenix"] = serde_json::json!(phoenix);
    let seeded = instance.call("seed_tasks", &[&serde_json::json!([row]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
}

fn request_wrap_up(
    instance: &Instance,
    task: i64,
    host: &str,
    action: &str,
) -> std::process::Output {
    instance.call("request_wrap_up", &[&task.to_string(), host, action])
}

fn request_exit_session(
    instance: &Instance,
    task: i64,
    token: &str,
    action: &str,
    pr_url: &str,
) -> std::process::Output {
    instance.call(
        "request_exit_session",
        &[&task.to_string(), token, action, pr_url],
    )
}

/// A session row's column, by the task and the kind (one such row per test).
fn session_column(instance: &Instance, field: &str, task: i64, kind: &str) -> String {
    column(
        instance,
        &format!("SELECT {field} FROM session_requests WHERE task_id = {task} AND kind = '{kind}'"),
    )
}

fn session_count(instance: &Instance) -> String {
    column(instance, "SELECT COUNT(*) AS n FROM session_requests")
}

fn session_timer_count(instance: &Instance, request: &str) -> String {
    column(
        instance,
        &format!("SELECT COUNT(*) AS n FROM session_request_timeouts WHERE request_id = {request}"),
    )
}

fn task_column(instance: &Instance, field: &str, task: i64) -> String {
    column(
        instance,
        &format!("SELECT {field} FROM tasks WHERE id = {task}"),
    )
}

/// wrap_up for `task` through to a done row, as a board does it: written done
/// for done and pr; claimed and reported done for rebase. Returns the token.
fn wrap_up_to_token(instance: &Instance, task: i64, action: &str) -> String {
    let requested = request_wrap_up(instance, task, "host-a", action);
    assert!(requested.status.success(), "{}", describe(&requested));
    let id = session_column(instance, "id", task, "wrap_up");
    if action == "rebase" {
        let claimed = instance.call("claim_session_request", &[&id, "host-a"]);
        assert!(claimed.status.success(), "{}", describe(&claimed));
        let done = instance.call("report_session_request_done", &[&id, "host-a"]);
        assert!(done.status.success(), "{}", describe(&done));
    }
    id
}

/// `RequestWrapUp`, rebase: a requested row on the task's host with its
/// deadline and one backstop timer, and no write to the task.
#[test]
fn a_rebase_wrap_up_writes_a_requested_row_with_a_timer_and_leaves_the_task() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let before = task_column(&instance, "updated_at", 1);

    let requested = request_wrap_up(&instance, 1, "host-a", "rebase");
    assert!(requested.status.success(), "{}", describe(&requested));

    assert_eq!(session_count(&instance), "1");
    assert_eq!(session_column(&instance, "kind", 1, "wrap_up"), "wrap_up");
    assert_eq!(session_column(&instance, "action", 1, "wrap_up"), "rebase");
    assert_eq!(session_column(&instance, "host", 1, "wrap_up"), "host-a");
    assert_eq!(
        session_column(&instance, "state", 1, "wrap_up"),
        "requested"
    );
    assert_eq!(
        session_column(&instance, "requested_by", 1, "wrap_up"),
        instance.identity()
    );
    assert_eq!(
        session_column(&instance, "requested_at", 1, "wrap_up").len(),
        23
    );
    assert_eq!(
        session_column(&instance, "deadline", 1, "wrap_up").len(),
        23
    );
    assert_eq!(session_column(&instance, "claimed_at", 1, "wrap_up"), "");
    assert_eq!(session_column(&instance, "finished_at", 1, "wrap_up"), "");
    let id = session_column(&instance, "id", 1, "wrap_up");
    assert_eq!(session_timer_count(&instance, &id), "1");
    assert_eq!(task_column(&instance, "updated_at", 1), before);
    assert_eq!(task_column(&instance, "status", 1), "running");
}

/// done and pr: the row is written already done in the same call, is the
/// token at once, and has no timer.
#[test]
fn a_done_or_pr_wrap_up_is_written_already_done_without_a_timer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);

    for (task, action) in [(1, "done"), (2, "pr")] {
        let requested = request_wrap_up(&instance, task, "host-a", action);
        assert!(requested.status.success(), "{}", describe(&requested));
        assert_eq!(session_column(&instance, "state", task, "wrap_up"), "done");
        assert_eq!(session_column(&instance, "action", task, "wrap_up"), action);
        assert_eq!(
            session_column(&instance, "finished_at", task, "wrap_up").len(),
            23
        );
        assert_eq!(session_column(&instance, "claimed_at", task, "wrap_up"), "");
        let id = session_column(&instance, "id", task, "wrap_up");
        assert_eq!(session_timer_count(&instance, &id), "0");
    }
}

/// Each refusal names why and writes nothing.
#[test]
fn a_refused_wrap_up_request_writes_nothing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(
        &instance,
        false,
        &[
            (1, "backlog", ""),
            (2, "running", "pr-review"),
            (3, "running", "dependabot"),
            (4, "running", ""),
            (5, "done", ""),
        ],
    );

    for (task, host, why) in [
        (1, "host-a", "cannot be wrapped up"),
        (2, "host-a", "retag"),
        (3, "host-a", "retag"),
        (4, "host-b", "host-a"),
        (5, "host-a", "cannot be wrapped up"),
        (99, "host-a", "99"),
    ] {
        let refused = request_wrap_up(&instance, task, host, "rebase");
        assert!(
            !refused.status.success(),
            "task {task}: {}",
            describe(&refused)
        );
        assert!(
            describe(&refused).contains(why),
            "task {task} must say {why:?}: {}",
            describe(&refused)
        );
    }
    let unknown = request_wrap_up(&instance, 4, "host-a", "merge");
    assert!(!unknown.status.success(), "{}", describe(&unknown));
    assert_eq!(session_count(&instance), "0");
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM session_request_timeouts"
    ));
}

/// `OneOpenSessionRequestPerTask`: a second wrap_up while the rebase row is
/// open is refused; once it finishes a new one is accepted.
#[test]
fn a_second_wrap_up_while_one_is_open_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());

    let second = request_wrap_up(&instance, 1, "host-a", "done");
    assert!(!second.status.success(), "{}", describe(&second));
    assert!(
        describe(&second).contains("open session request"),
        "{}",
        describe(&second)
    );
    assert_eq!(session_count(&instance), "1");

    let id = session_column(&instance, "id", 1, "wrap_up");
    instance.call("claim_session_request", &[&id, "host-a"]);
    instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(request_wrap_up(&instance, 1, "host-a", "done")
        .status
        .success());
}

/// `HostClaimsSessionRequest`: only the row's host, only once, and the claim
/// clears a Conflict sub_status in the same transaction.
#[test]
fn the_claim_is_the_rows_hosts_alone_once_and_clears_a_conflict() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    move_task(&instance, 1, "running", "conflict");
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");
    assert_eq!(
        task_column(&instance, "sub_status", 1),
        "conflict",
        "the request alone does not clear it"
    );

    let foreign = instance.call("claim_session_request", &[&id, "host-b"]);
    assert!(!foreign.status.success(), "{}", describe(&foreign));
    assert_eq!(
        session_column(&instance, "state", 1, "wrap_up"),
        "requested"
    );
    assert_eq!(task_column(&instance, "sub_status", 1), "conflict");

    let claimed = instance.call("claim_session_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "running");
    assert_eq!(
        session_column(&instance, "claimed_at", 1, "wrap_up").len(),
        23
    );
    assert_eq!(task_column(&instance, "sub_status", 1), "active");

    let again = instance.call("claim_session_request", &[&id, "host-a"]);
    assert!(!again.status.success(), "{}", describe(&again));
}

/// `HostReportsSessionRequestDone`: the row is done (a valid token from now
/// on), the timer is cancelled; a repeat is accepted and changes nothing; a
/// report from another host, or on an unclaimed row, is refused.
#[test]
fn a_done_report_finishes_the_row_cancels_the_timer_and_may_repeat() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");

    let unclaimed = instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(!unclaimed.status.success(), "{}", describe(&unclaimed));
    instance.call("claim_session_request", &[&id, "host-a"]);
    let foreign = instance.call("report_session_request_done", &[&id, "host-b"]);
    assert!(!foreign.status.success(), "{}", describe(&foreign));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "running");

    let done = instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(done.status.success(), "{}", describe(&done));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "done");
    let finished = session_column(&instance, "finished_at", 1, "wrap_up");
    assert_eq!(finished.len(), 23);
    assert_eq!(session_timer_count(&instance, &id), "0");

    let repeat = instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(repeat.status.success(), "{}", describe(&repeat));
    assert_eq!(
        session_column(&instance, "finished_at", 1, "wrap_up"),
        finished
    );
}

/// `HostReportsSessionRequestFailed`: a conflict report makes the MODULE set
/// the Conflict sub_status; a plain failure leaves the task alone; both
/// record the reason and cancel the timer, and issue no token.
#[test]
fn a_failed_report_records_the_reason_and_a_conflict_sets_the_sub_status() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    for task in [1, 2] {
        assert!(request_wrap_up(&instance, task, "host-a", "rebase")
            .status
            .success());
        let id = session_column(&instance, "id", task, "wrap_up");
        instance.call("claim_session_request", &[&id, "host-a"]);
    }
    let first = session_column(&instance, "id", 1, "wrap_up");
    let second = session_column(&instance, "id", 2, "wrap_up");

    let conflict = instance.call(
        "report_session_request_failed",
        &[&first, "host-a", "conflict in src/a.rs", "true"],
    );
    assert!(conflict.status.success(), "{}", describe(&conflict));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 1, "wrap_up"),
        "conflict in src/a.rs"
    );
    assert_eq!(task_column(&instance, "sub_status", 1), "conflict");
    assert_eq!(session_timer_count(&instance, &first), "0");

    let plain = instance.call(
        "report_session_request_failed",
        &[&second, "host-a", "dirty primary worktree", "false"],
    );
    assert!(plain.status.success(), "{}", describe(&plain));
    assert_eq!(task_column(&instance, "sub_status", 2), "active");

    // A late report on a finished row is refused.
    let late = instance.call("report_session_request_done", &[&first, "host-a"]);
    assert!(!late.status.success(), "{}", describe(&late));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
}

/// A conflict reported for a task that moved on leaves it alone.
#[test]
fn a_conflict_report_leaves_a_task_that_moved_on_alone() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");
    instance.call("claim_session_request", &[&id, "host-a"]);
    move_task(&instance, 1, "done", "none");

    let reported = instance.call(
        "report_session_request_failed",
        &[&id, "host-a", "conflict", "true"],
    );
    assert!(reported.status.success(), "{}", describe(&reported));
    assert_eq!(task_column(&instance, "sub_status", 1), "none");
}

/// `RequestExitSession`, rebase: the close lands whole with the exit_session
/// row, the epic rollup and the token's consumption.
#[test]
fn an_exit_session_applies_the_close_and_writes_the_teardown_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "rebase");

    let closed = request_exit_session(&instance, 1, &token, "rebase", "");
    assert!(closed.status.success(), "{}", describe(&closed));

    assert_eq!(task_column(&instance, "status", 1), "done");
    assert_eq!(task_column(&instance, "sub_status", 1), "none");
    assert_eq!(task_column(&instance, "tmux_window", 1), "");
    assert_eq!(task_column(&instance, "worktree", 1), "/wt/1-t");
    assert_eq!(task_column(&instance, "completed_at", 1).len(), 23);
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "done",
        "the epic rolls up in the same transaction"
    );

    assert_eq!(
        session_column(&instance, "state", 1, "exit_session"),
        "requested"
    );
    assert_eq!(
        session_column(&instance, "host", 1, "exit_session"),
        "host-a"
    );
    assert_eq!(
        session_column(&instance, "action", 1, "exit_session"),
        "rebase"
    );
    assert_eq!(
        session_column(&instance, "tmux_window", 1, "exit_session"),
        "task-1"
    );
    assert_eq!(
        session_column(&instance, "wrap_up_request", 1, "exit_session"),
        token
    );
    assert_eq!(
        session_column(&instance, "chained_task", 1, "exit_session"),
        "0"
    );
    assert_eq!(
        session_column(&instance, "requested_by", 1, "exit_session"),
        instance.identity()
    );
    assert_eq!(
        session_column(&instance, "deadline", 1, "exit_session").len(),
        23
    );
    let id = session_column(&instance, "id", 1, "exit_session");
    assert_eq!(session_timer_count(&instance, &id), "1");
}

/// pr: review with the pr-typed url, and the url recorded on the row.
#[test]
fn a_pr_exit_session_takes_the_task_to_review_with_the_url() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "pr");
    let url = "https://github.com/acme/repo/pull/7";

    let closed = request_exit_session(&instance, 1, &token, "pr", url);
    assert!(closed.status.success(), "{}", describe(&closed));

    assert_eq!(task_column(&instance, "status", 1), "review");
    assert_eq!(task_column(&instance, "sub_status", 1), "awaiting_review");
    assert_eq!(task_column(&instance, "url", 1), url);
    assert_eq!(task_column(&instance, "url_type", 1), "pr");
    assert_eq!(task_column(&instance, "tmux_window", 1), "");
    assert_eq!(task_column(&instance, "completed_at", 1), "");
    assert_eq!(session_column(&instance, "pr_url", 1, "exit_session"), url);
}

/// The token is consumed once: a second call is told to wrap up again.
#[test]
fn an_exit_token_is_consumed_by_the_close() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "done");
    assert!(request_exit_session(&instance, 1, &token, "done", "")
        .status
        .success());
    // Finish the teardown so the open-request check is not what refuses.
    let exit = session_column(&instance, "id", 1, "exit_session");
    instance.call("claim_session_request", &[&exit, "host-a"]);
    instance.call("report_session_request_done", &[&exit, "host-a"]);

    let again = request_exit_session(&instance, 1, &token, "done", "");
    assert!(!again.status.success(), "{}", describe(&again));
    assert!(
        describe(&again).contains("no exit token — call wrap_up first"),
        "{}",
        describe(&again)
    );
    assert_eq!(session_count(&instance), "2");
}

/// Every refusal writes nothing, leaves the task and the token as they were,
/// and says why; the same token then closes once the cause is fixed.
#[test]
fn a_refused_exit_session_leaves_the_task_and_the_token_untouched() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "pr");
    let rows_before = session_count(&instance);

    for (action, token_arg, url, why) in [
        ("pr", "999", "https://x/pull/1", "invalid exit token"),
        (
            "rebase",
            token.as_str(),
            "",
            "exit token was issued for wrap_up(action=\"pr\"), but exit_session was called with action=\"rebase\"",
        ),
        (
            "pr",
            token.as_str(),
            "",
            "pr_url is required for action 'pr'",
        ),
    ] {
        let refused = request_exit_session(&instance, 1, token_arg, action, url);
        assert!(!refused.status.success(), "{why}: {}", describe(&refused));
        assert!(describe(&refused).contains(why), "{why}: {}", describe(&refused));
    }
    assert_eq!(session_count(&instance), rows_before);
    assert_eq!(task_column(&instance, "status", 1), "running");
    assert_eq!(task_column(&instance, "tmux_window", 1), "task-1");

    let retried = request_exit_session(&instance, 1, &token, "pr", "https://x/pull/1");
    assert!(retried.status.success(), "{}", describe(&retried));
    assert_eq!(task_column(&instance, "status", 1), "review");
}

/// No token at all, and a task with no window.
#[test]
fn an_exit_session_without_a_token_or_a_window_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);

    let none = request_exit_session(&instance, 1, "5", "done", "");
    assert!(!none.status.success(), "{}", describe(&none));
    assert!(
        describe(&none).contains("no exit token — call wrap_up first"),
        "{}",
        describe(&none)
    );

    let token = wrap_up_to_token(&instance, 2, "done");
    // The window went away by some other path (the TUI, crash detection).
    let patched = instance.call(
        "patch_task",
        &["2", &patch_setting("tmux_window", "").to_string()],
    );
    assert!(patched.status.success(), "{}", describe(&patched));
    let closed = request_exit_session(&instance, 2, &token, "done", "");
    assert!(!closed.status.success(), "{}", describe(&closed));
    assert!(
        describe(&closed).contains("task #2 has no active session"),
        "{}",
        describe(&closed)
    );
    assert_eq!(task_column(&instance, "status", 2), "running");
}

/// A newer wrap_up supersedes the older token.
#[test]
fn a_newer_wrap_up_supersedes_the_older_token() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let old = wrap_up_to_token(&instance, 1, "done");
    let newer = request_wrap_up(&instance, 1, "host-a", "pr");
    assert!(newer.status.success(), "{}", describe(&newer));
    let new = column(
        &instance,
        "SELECT id FROM session_requests WHERE task_id = 1 AND action = 'pr'",
    );

    let refused = request_exit_session(&instance, 1, &old, "done", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("invalid exit token"),
        "{}",
        describe(&refused)
    );
    let closed = request_exit_session(&instance, 1, &new, "pr", "https://x/pull/1");
    assert!(closed.status.success(), "{}", describe(&closed));
}

/// A LATER DISPATCH VOIDS THE TOKEN: the task went back to backlog and was
/// dispatched again after the wrap-up, so the old token cannot close the new
/// session.
#[test]
fn a_later_dispatch_voids_the_exit_token() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "done");
    move_task(&instance, 1, "backlog", "none");
    assert!(request_dispatch(&instance, 1, "host-a", "user", "")
        .status
        .success());
    let dispatch = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&dispatch, "host-a"]);
    let reported = instance.call(
        "report_dispatch_provisioned",
        &[&dispatch, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(reported.status.success(), "{}", describe(&reported));

    let refused = request_exit_session(&instance, 1, &token, "done", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("invalid exit token"),
        "{}",
        describe(&refused)
    );
    assert_eq!(task_column(&instance, "status", 1), "running");
}

/// An open session request (a second wrap_up's rebase still running) holds
/// the task: the close is refused and the token stays valid.
#[test]
fn an_open_session_request_refuses_the_close() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "done");
    // A second wrap_up (rebase) that is still open is not a done row, so it
    // does not supersede the token; it holds the task instead.
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let refused = request_exit_session(&instance, 1, &token, "done", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert_eq!(task_column(&instance, "status", 1), "running");
}

/// D11: the task's host is the row's host, and only its owner may ask.
#[test]
fn a_teammate_may_not_act_for_my_hosts_session_requests() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    register_teammates_host(&instance, "their-host");
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");
    let token = wrap_up_to_token(&instance, 2, "done");

    let teammate = |reducer: &str, args: &[&str]| instance.call_as("teammate", reducer, args);
    for out in [
        teammate("request_wrap_up", &["2", "host-a", "done"]),
        teammate("request_wrap_up", &["2", "their-host", "done"]),
        teammate("claim_session_request", &[&id, "host-a"]),
        teammate("report_session_request_done", &[&id, "host-a"]),
        teammate(
            "report_session_request_failed",
            &[&id, "host-a", "x", "false"],
        ),
        teammate("request_exit_session", &["2", &token, "done", ""]),
        teammate("fail_interrupted_session_requests", &["host-a"]),
    ] {
        assert!(!out.status.success(), "{}", describe(&out));
    }
    assert_eq!(
        session_column(&instance, "state", 1, "wrap_up"),
        "requested"
    );
    assert_eq!(task_column(&instance, "status", 2), "running");

    let ownerless = request_wrap_up(&instance, 2, "nowhere", "done");
    assert!(!ownerless.status.success(), "{}", describe(&ownerless));
    assert!(
        describe(&ownerless).contains("not registered"),
        "{}",
        describe(&ownerless)
    );
}

/// The teardown: the host claims the exit_session row and reports it done, or
/// failed; a failed teardown does NOT revert the close.
#[test]
fn a_failed_teardown_does_not_revert_the_close() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "done");
    assert!(request_exit_session(&instance, 1, &token, "done", "")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "exit_session");

    let claimed = instance.call("claim_session_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    assert_eq!(task_column(&instance, "sub_status", 1), "none");
    let failed = instance.call(
        "report_session_request_failed",
        &[&id, "host-a", "kill failed", "false"],
    );
    assert!(failed.status.success(), "{}", describe(&failed));

    assert_eq!(
        session_column(&instance, "state", 1, "exit_session"),
        "failed"
    );
    assert_eq!(
        session_column(&instance, "reason", 1, "exit_session"),
        "kill failed"
    );
    assert_eq!(task_column(&instance, "status", 1), "done");
    assert_eq!(session_timer_count(&instance, &id), "0");
    let conflict = instance.call(
        "report_session_request_failed",
        &[&id, "host-a", "again", "true"],
    );
    assert!(!conflict.status.success(), "{}", describe(&conflict));
}

/// A teardown reported done after its deadline still records what happened;
/// the close stands either way. (The deadline itself is a unit test.)
#[test]
fn a_teardown_reported_done_finishes_the_row_and_cancels_the_timer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "done");
    request_exit_session(&instance, 1, &token, "done", "");
    let id = session_column(&instance, "id", 1, "exit_session");
    instance.call("claim_session_request", &[&id, "host-a"]);

    let done = instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(done.status.success(), "{}", describe(&done));
    assert_eq!(
        session_column(&instance, "state", 1, "exit_session"),
        "done"
    );
    assert_eq!(session_timer_count(&instance, &id), "0");
}

/// W6, the epic chain: a close in an auto-dispatch epic requests the first
/// backlog subtask in board order whose epic_chain request is accepted,
/// passing over a phoenix one, on the closed task's host, in the same
/// transaction; the exit_session row records it.
#[test]
fn an_exit_session_chains_the_next_subtask_passing_over_a_phoenix_one() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, true, &[(1, "running", "")]);
    seed_backlog_subtask(&instance, 10, 1, true);
    seed_backlog_subtask(&instance, 11, 2, false);
    seed_backlog_subtask(&instance, 12, 3, false);
    let token = wrap_up_to_token(&instance, 1, "rebase");

    let closed = request_exit_session(&instance, 1, &token, "rebase", "");
    assert!(closed.status.success(), "{}", describe(&closed));

    assert_eq!(
        session_column(&instance, "chained_task", 1, "exit_session"),
        "11"
    );
    assert_eq!(request_column(&instance, "origin", 11), "epic_chain");
    assert_eq!(request_column(&instance, "host", 11), "host-a");
    assert_eq!(request_column(&instance, "state", 11), "requested");
    assert_eq!(
        request_column(&instance, "requested_by", 11),
        instance.identity()
    );
    assert_eq!(task_column(&instance, "status", 11), "running");
    assert_eq!(task_column(&instance, "status", 10), "backlog");
    assert_eq!(task_column(&instance, "status", 12), "backlog");
    assert_eq!(request_count(&instance), "1");
    let dispatch = request_column(&instance, "id", 11);
    assert_eq!(timer_count(&instance, dispatch.parse().unwrap()), "1");
}

/// The chain stops, normally and without failing the close: no auto_dispatch,
/// or nothing but phoenix subtasks left.
#[test]
fn the_chain_stops_quietly_without_auto_dispatch_or_candidates() {
    if !spacetime_available_or_skip() {
        return;
    }
    let off = published_instance();
    seed_wrappable(&off, false, &[(1, "running", "")]);
    seed_backlog_subtask(&off, 11, 2, false);
    let token = wrap_up_to_token(&off, 1, "done");
    assert!(request_exit_session(&off, 1, &token, "done", "")
        .status
        .success());
    assert_eq!(task_column(&off, "status", 11), "backlog");
    assert_eq!(request_count(&off), "0");
    assert_eq!(session_column(&off, "chained_task", 1, "exit_session"), "0");

    let only_phoenix = published_instance();
    seed_wrappable(&only_phoenix, true, &[(1, "running", "")]);
    seed_backlog_subtask(&only_phoenix, 10, 1, true);
    let token = wrap_up_to_token(&only_phoenix, 1, "done");
    let closed = request_exit_session(&only_phoenix, 1, &token, "done", "");
    assert!(closed.status.success(), "{}", describe(&closed));
    assert_eq!(task_column(&only_phoenix, "status", 10), "backlog");
    assert_eq!(request_count(&only_phoenix), "0");
    assert_eq!(task_column(&only_phoenix, "status", 1), "done");
}

/// A candidate whose worktree is on another host is refused by the request
/// and the chain moves on to the next.
#[test]
fn the_chain_passes_over_a_subtask_on_another_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, true, &[(1, "running", "")]);
    let mut elsewhere = task_json(10, "next", "backlog", 1, "host-b");
    elsewhere["sort_order"] = serde_json::json!({"some": 1});
    let seeded = instance.call("seed_tasks", &[&serde_json::json!([elsewhere]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
    seed_backlog_subtask(&instance, 11, 2, false);
    let token = wrap_up_to_token(&instance, 1, "done");

    assert!(request_exit_session(&instance, 1, &token, "done", "")
        .status
        .success());
    assert_eq!(
        session_column(&instance, "chained_task", 1, "exit_session"),
        "11"
    );
    assert_eq!(task_column(&instance, "status", 10), "backlog");
}

/// A phoenix task closing into Done respawns its successor in the same
/// transaction, and clears its own flag (`TheFlagIsTheReceipt`); a pr close
/// does not respawn.
#[test]
fn a_phoenix_task_respawns_with_a_done_close_but_not_a_pr_close() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    // The successor's id is generated, so the sequence is advanced past the
    // seeded ids first, before any row is loaded (`burn_id_sequence`).
    let burned = instance.call("burn_id_sequence", &["tasks", "10"]);
    assert!(burned.status.success(), "{}", describe(&burned));
    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "running", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let rows: Vec<serde_json::Value> = [1, 2]
        .iter()
        .map(|id| {
            let mut row = task_json(*id, "weekly", "running", 1, "host-a");
            row["worktree"] = serde_json::json!(format!("/wt/{id}-t"));
            row["tmux_window"] = serde_json::json!(format!("task-{id}"));
            row["sub_status"] = serde_json::json!("active");
            row["phoenix"] = serde_json::json!(true);
            row["tag"] = serde_json::json!("chore");
            row
        })
        .collect();
    let seeded = instance.call("seed_tasks", &[&serde_json::json!(rows).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let token = wrap_up_to_token(&instance, 2, "pr");
    assert!(
        request_exit_session(&instance, 2, &token, "pr", "https://x/pull/1")
            .status
            .success()
    );
    assert_eq!(
        column(&instance, "SELECT COUNT(*) AS n FROM tasks"),
        "2",
        "a pr close lands in review and does not respawn"
    );
    assert_eq!(task_column(&instance, "phoenix", 2), "true");

    let token = wrap_up_to_token(&instance, 1, "done");
    let closed = request_exit_session(&instance, 1, &token, "done", "");
    assert!(closed.status.success(), "{}", describe(&closed));
    assert_eq!(column(&instance, "SELECT COUNT(*) AS n FROM tasks"), "3");
    assert_eq!(task_column(&instance, "phoenix", 1), "false");
    assert_eq!(
        column(
            &instance,
            "SELECT status FROM tasks WHERE id <> 1 AND id <> 2"
        ),
        "backlog"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT phoenix FROM tasks WHERE id <> 1 AND id <> 2"
        ),
        "true"
    );
}

/// `HostRestartFailsInterruptedSessionRequests`: this host's RUNNING rows
/// fail with "host restarted" and cancel their timers; its requested rows and
/// every other host's rows are untouched.
#[test]
fn a_restart_fails_only_this_hosts_running_session_requests() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(
        &instance,
        false,
        &[(1, "running", ""), (2, "running", ""), (3, "running", "")],
    );
    for task in [1, 2] {
        assert!(request_wrap_up(&instance, task, "host-a", "rebase")
            .status
            .success());
    }
    let first = session_column(&instance, "id", 1, "wrap_up");
    instance.call("claim_session_request", &[&first, "host-a"]);

    let swept = instance.call("fail_interrupted_session_requests", &["host-a"]);
    assert!(swept.status.success(), "{}", describe(&swept));

    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 1, "wrap_up"),
        "host restarted"
    );
    assert_eq!(session_timer_count(&instance, &first), "0");
    assert_eq!(
        session_column(&instance, "state", 2, "wrap_up"),
        "requested"
    );
    let swept_other = instance.call("fail_interrupted_session_requests", &["host-b"]);
    assert!(swept_other.status.success(), "{}", describe(&swept_other));
    assert_eq!(
        session_column(&instance, "state", 2, "wrap_up"),
        "requested"
    );
}

/// The scheduled reducer refuses every client: nobody may time a request out
/// early.
#[test]
fn a_client_may_not_run_the_session_backstop() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");

    let forged = instance.call(
        "expire_session_request",
        &[&format!(
            "{{\"scheduled_id\": 1, \"scheduled_at\": {{\"Interval\": 0}}, \"request_id\": {id}}}"
        )],
    );
    assert!(!forged.status.success(), "{}", describe(&forged));
    assert_eq!(
        session_column(&instance, "state", 1, "wrap_up"),
        "requested"
    );
}

/// W9, `DeletingATaskFailsItsOpenSessionRequest`, through `delete_task`: the
/// open row fails "task deleted" and its timer is cancelled.
#[test]
fn deleting_a_task_fails_its_open_session_request_and_cancels_its_timer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");
    move_task(&instance, 1, "done", "none");

    let deleted = instance.call("delete_task", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 1, "wrap_up"),
        "task deleted"
    );
    assert_eq!(session_timer_count(&instance, &id), "0");
}

/// The same, through `batch_delete`'s task arm and its epic arm.
#[test]
fn batch_delete_fails_the_open_session_requests_of_the_tasks_it_deletes() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    for task in [1, 2] {
        assert!(request_wrap_up(&instance, task, "host-a", "rebase")
            .status
            .success());
        move_task(&instance, task, "done", "none");
    }

    let deleted = instance.call("batch_delete", &["[1]", "[]"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 1, "wrap_up"),
        "task deleted"
    );
    assert_eq!(
        session_column(&instance, "state", 2, "wrap_up"),
        "requested"
    );

    let deleted = instance.call("batch_delete", &["[]", "[1]"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert_eq!(session_column(&instance, "state", 2, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 2, "wrap_up"),
        "task deleted"
    );
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM session_request_timeouts"
    ));
}

/// The same, through `ingest_feed`'s stale delete, which removes a feed task
/// whatever its status.
#[test]
fn a_feed_ingest_that_deletes_a_stale_task_fails_its_open_session_request() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    let mut epic = epic_json(1, "Feed", "backlog", 0);
    epic["feed_command"] = serde_json::json!("echo []");
    let seeded = instance.call("seed_epics", &[&serde_json::json!([epic]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let upserted = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "one", "running")]).to_string(),
            "test-creator",
        ],
    );
    assert!(upserted.status.success(), "{}", describe(&upserted));
    let task: i64 = column(
        &instance,
        "SELECT id FROM tasks WHERE external_id = 'ext-1'",
    )
    .parse()
    .expect("task id");
    let patched = instance.call(
        "patch_task",
        &[
            &task.to_string(),
            &{
                let mut patch = patch_setting("worktree", "/wt/feed");
                patch["host"] = serde_json::json!({"some": "host-a"});
                patch
            }
            .to_string(),
        ],
    );
    assert!(patched.status.success(), "{}", describe(&patched));
    let requested = request_wrap_up(&instance, task, "host-a", "rebase");
    assert!(requested.status.success(), "{}", describe(&requested));

    let ingested = instance.call("ingest_feed", &["1", "[]", "false", "test-creator"]);
    assert!(ingested.status.success(), "{}", describe(&ingested));

    assert!(no_rows(
        &instance,
        &format!("SELECT id FROM tasks WHERE id = {task}")
    ));
    assert_eq!(
        session_column(&instance, "state", task, "wrap_up"),
        "failed"
    );
    assert_eq!(
        session_column(&instance, "reason", task, "wrap_up"),
        "task deleted"
    );
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM session_request_timeouts"
    ));
}

// -- store-split M7 (task #53260): close_task, patch_task's transition rules and
// ensure_managed_epics ----------------------------------------------------------
//
// `store-split.allium`: RequestCloseTask (X1), X2 and X4. As for M5, the unit
// tests in `tests.rs` pin the pure rules; here it is the transaction.

fn close_task(instance: &Instance, task: i64, host: &str) -> std::process::Output {
    instance.call("close_task", &[&task.to_string(), host])
}

fn set_task_field(instance: &Instance, task: i64, field: &str, value: serde_json::Value) {
    let mut patch = empty_task_patch();
    patch[field] = serde_json::json!({ "some": value });
    let out = instance.call("patch_task", &[&task.to_string(), &patch.to_string()]);
    assert!(out.status.success(), "{}", describe(&out));
}

/// A running task with a window closes whole: Done, window cleared, completed,
/// the epic rolled up, and ONE close row with its timer; no token is touched.
#[test]
fn close_task_applies_the_done_close_and_writes_a_close_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    set_task_field(&instance, 1, "stop_pending", serde_json::json!(true));

    let closed = close_task(&instance, 1, "host-b");
    assert!(closed.status.success(), "{}", describe(&closed));

    assert_eq!(task_column(&instance, "status", 1), "done");
    assert_eq!(task_column(&instance, "sub_status", 1), "none");
    assert_eq!(task_column(&instance, "tmux_window", 1), "");
    assert_eq!(task_column(&instance, "completed_at", 1).len(), 23);
    assert_eq!(task_column(&instance, "stop_pending", 1), "false");
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "done"
    );

    assert_eq!(session_count(&instance), "1");
    assert_eq!(session_column(&instance, "state", 1, "close"), "requested");
    assert_eq!(session_column(&instance, "action", 1, "close"), "done");
    assert_eq!(
        session_column(&instance, "host", 1, "close"),
        "host-a",
        "the task's own host, not the argument"
    );
    assert_eq!(
        session_column(&instance, "tmux_window", 1, "close"),
        "task-1"
    );
    assert_eq!(
        session_column(&instance, "wrap_up_request", 1, "close"),
        "0"
    );
    assert_eq!(session_column(&instance, "chained_task", 1, "close"), "0");
    assert_eq!(session_column(&instance, "pr_url", 1, "close"), "");
    assert_eq!(
        session_column(&instance, "requested_by", 1, "close"),
        instance.identity()
    );
    assert_eq!(session_column(&instance, "deadline", 1, "close").len(), 23);
    let id = session_column(&instance, "id", 1, "close");
    assert_eq!(session_timer_count(&instance, &id), "1");
}

/// The teardown of a close row is served like an exit_session's: claim, done,
/// and the timer goes.
#[test]
fn a_close_row_is_claimed_and_reported_like_a_teardown() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(close_task(&instance, 1, "host-a").status.success());
    let id = session_column(&instance, "id", 1, "close");

    let claimed = instance.call("claim_session_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    let done = instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(done.status.success(), "{}", describe(&done));
    assert_eq!(session_column(&instance, "state", 1, "close"), "done");
    assert_eq!(session_timer_count(&instance, &id), "0");
    assert_eq!(task_column(&instance, "status", 1), "done");
}

/// No window and no pr url: nothing for a host to do, so no row.
#[test]
fn close_task_without_host_work_writes_no_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "review", "")]);
    set_task_field(&instance, 1, "tmux_window", serde_json::json!(""));

    let closed = close_task(&instance, 1, "host-a");
    assert!(closed.status.success(), "{}", describe(&closed));
    assert_eq!(task_column(&instance, "status", 1), "done");
    assert_eq!(session_count(&instance), "0");
}

/// A pr-typed url alone is host work, on the caller's host when the task has none.
#[test]
fn close_task_with_a_pr_url_and_no_host_writes_a_row_on_the_argument_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "review", "")]);
    set_task_field(&instance, 1, "tmux_window", serde_json::json!(""));
    set_task_field(&instance, 1, "host", serde_json::json!(""));
    set_task_field(
        &instance,
        1,
        "url",
        serde_json::json!("https://github.com/a/b/pull/1"),
    );
    set_task_field(&instance, 1, "url_type", serde_json::json!("pr"));

    let closed = close_task(&instance, 1, "host-b");
    assert!(closed.status.success(), "{}", describe(&closed));
    assert_eq!(session_column(&instance, "host", 1, "close"), "host-b");
    assert_eq!(session_column(&instance, "tmux_window", 1, "close"), "");
}

/// D11: a teammate's host is not the caller's to tear down. The task still closes.
#[test]
fn close_task_on_a_teammates_host_closes_without_a_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    register_teammates_host(&instance, "their-host");
    set_task_field(&instance, 1, "host", serde_json::json!("their-host"));

    let closed = close_task(&instance, 1, "host-a");
    assert!(closed.status.success(), "{}", describe(&closed));
    assert_eq!(task_column(&instance, "status", 1), "done");
    assert_eq!(task_column(&instance, "tmux_window", 1), "");
    assert_eq!(session_count(&instance), "0");
}

/// A stuck wrap_up does not block closing: it is failed "task closed", its
/// timer cancelled, and the close row written after.
#[test]
fn close_task_fails_the_open_session_request() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let wrap = session_column(&instance, "id", 1, "wrap_up");
    assert_eq!(session_timer_count(&instance, &wrap), "1");

    let closed = close_task(&instance, 1, "host-a");
    assert!(closed.status.success(), "{}", describe(&closed));

    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 1, "wrap_up"),
        "task closed"
    );
    assert_eq!(session_timer_count(&instance, &wrap), "0");
    assert_eq!(session_column(&instance, "state", 1, "close"), "requested");
    // A late report from the failed rebase is refused.
    let late = instance.call("report_session_request_done", &[&wrap, "host-a"]);
    assert!(!late.status.success(), "{}", describe(&late));
}

/// An earlier close still open is failed by a re-close, so at most one is open.
#[test]
fn a_reclose_fails_the_earlier_open_close_row_and_keeps_completed_at() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(close_task(&instance, 1, "host-a").status.success());
    let first = session_column(&instance, "id", 1, "close");
    let completed = task_column(&instance, "completed_at", 1);
    // Give the task a window again, as a hand-edit would, so the re-close has work.
    set_task_field(&instance, 1, "tmux_window", serde_json::json!("task-1"));

    assert!(close_task(&instance, 1, "host-a").status.success());

    assert_eq!(task_column(&instance, "completed_at", 1), completed);
    assert_eq!(
        column(
            &instance,
            &format!("SELECT state FROM session_requests WHERE id = {first}")
        ),
        "failed"
    );
    assert_eq!(session_timer_count(&instance, &first), "0");
    assert_eq!(
        column(
            &instance,
            "SELECT COUNT(*) AS n FROM session_requests WHERE state = 'requested'"
        ),
        "1"
    );
}

/// No token and no chain: an auto-dispatch epic's backlog subtask is not requested.
#[test]
fn close_task_runs_no_epic_chain() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, true, &[(1, "running", "")]);
    seed_backlog_subtask(&instance, 2, 1, false);

    assert!(close_task(&instance, 1, "host-a").status.success());

    assert_eq!(
        column(&instance, "SELECT COUNT(*) AS n FROM dispatch_requests"),
        "0"
    );
    assert_eq!(task_column(&instance, "status", 2), "backlog");
}

#[test]
fn close_task_of_a_missing_task_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let refused = close_task(&instance, 99, "host-a");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert_eq!(session_count(&instance), "0");
}

/// PhoenixRespawn on a close that enters Done; the flag is the receipt, and a
/// done -> done re-close does not respawn again.
#[test]
fn close_task_respawns_a_phoenix_task_once() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    // A respawn inserts a task, so the id counter must be past the seeded ids.
    let burned = instance.call("burn_id_sequence", &["tasks", "10"]);
    assert!(burned.status.success(), "{}", describe(&burned));
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    set_task_field(&instance, 1, "phoenix", serde_json::json!(true));
    let count = || column(&instance, "SELECT COUNT(*) AS n FROM tasks");
    assert_eq!(count(), "1");

    assert!(close_task(&instance, 1, "host-a").status.success());
    assert_eq!(count(), "2");
    assert_eq!(task_column(&instance, "phoenix", 1), "false");

    assert!(close_task(&instance, 1, "host-a").status.success());
    assert_eq!(count(), "2");
}

/// X2: a patch out of running clears stop_pending, unless it names the field.
#[test]
fn patch_task_leaving_running_clears_stop_pending() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    set_task_field(&instance, 1, "stop_pending", serde_json::json!(true));
    set_task_field(&instance, 2, "stop_pending", serde_json::json!(true));

    set_task_field(&instance, 1, "status", serde_json::json!("review"));
    assert_eq!(task_column(&instance, "stop_pending", 1), "false");

    let mut patch = empty_task_patch();
    patch["status"] = serde_json::json!({"some": "review"});
    patch["stop_pending"] = serde_json::json!({"some": true});
    let out = instance.call("patch_task", &["2", &patch.to_string()]);
    assert!(out.status.success(), "{}", describe(&out));
    assert_eq!(task_column(&instance, "stop_pending", 2), "true");
}

/// X2: a patch into Done from another status respawns a phoenix task, in the
/// same transaction; a feed-owned task and a patch that stays done do not.
#[test]
fn patch_task_into_done_respawns_a_phoenix_task() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    // A respawn inserts a task, so the id counter must be past the seeded ids.
    let burned = instance.call("burn_id_sequence", &["tasks", "10"]);
    assert!(burned.status.success(), "{}", describe(&burned));
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    set_task_field(&instance, 1, "phoenix", serde_json::json!(true));
    set_task_field(&instance, 2, "phoenix", serde_json::json!(true));
    set_task_field(&instance, 2, "external_id", serde_json::json!("ext-2"));
    let count = || column(&instance, "SELECT COUNT(*) AS n FROM tasks");

    set_task_field(&instance, 1, "status", serde_json::json!("done"));
    assert_eq!(count(), "3");
    assert_eq!(task_column(&instance, "phoenix", 1), "false");
    assert_eq!(
        column(
            &instance,
            "SELECT status FROM tasks WHERE title = 't' AND id > 10"
        ),
        "backlog"
    );

    set_task_field(&instance, 1, "title", serde_json::json!("renamed"));
    assert_eq!(count(), "3", "no transition, no respawn");

    set_task_field(&instance, 2, "status", serde_json::json!("done"));
    assert_eq!(count(), "3", "a feed-owned task is exempt");
}

fn ensure_managed_epics(
    instance: &Instance,
    reviews: &str,
    reviews_interval: i64,
    cve: &str,
    cve_interval: i64,
) -> std::process::Output {
    instance.call(
        "ensure_managed_epics",
        &[
            reviews,
            &reviews_interval.to_string(),
            cve,
            &cve_interval.to_string(),
        ],
    )
}

fn epic_count_by_role(instance: &Instance, role: &str) -> String {
    column(
        instance,
        &format!("SELECT COUNT(*) AS n FROM epics WHERE feed_role = '{role}'"),
    )
}

/// X4: the whole tree in one call, sub-epics under the reviews root carrying no
/// command; a second call changes nothing.
#[test]
fn ensure_managed_epics_creates_the_tree_and_is_idempotent() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let made = ensure_managed_epics(&instance, "reviews.sh", 600, "cve.sh", 900);
    assert!(made.status.success(), "{}", describe(&made));
    let again = ensure_managed_epics(&instance, "reviews.sh", 600, "cve.sh", 900);
    assert!(again.status.success(), "{}", describe(&again));

    assert_eq!(column(&instance, "SELECT COUNT(*) AS n FROM epics"), "5");
    let parent = column(
        &instance,
        "SELECT id FROM epics WHERE feed_role = 'reviews-parent'",
    );
    assert_eq!(
        column(
            &instance,
            "SELECT title FROM epics WHERE feed_role = 'reviews-parent'"
        ),
        "PR Reviews"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT feed_command FROM epics WHERE feed_role = 'reviews-parent'"
        ),
        "reviews.sh"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT feed_interval_secs FROM epics WHERE feed_role = 'reviews-parent'"
        ),
        "600"
    );
    for (role, title) in [
        ("my-reviews", "My Reviews"),
        ("team-reviews", "Team Reviews"),
        ("bots", "Bots"),
    ] {
        assert_eq!(epic_count_by_role(&instance, role), "1", "{role}");
        assert_eq!(
            column(
                &instance,
                &format!("SELECT title FROM epics WHERE feed_role = '{role}'")
            ),
            title
        );
        assert_eq!(
            column(
                &instance,
                &format!("SELECT parent_epic_id FROM epics WHERE feed_role = '{role}'")
            ),
            parent
        );
        assert_eq!(
            column(
                &instance,
                &format!("SELECT feed_command FROM epics WHERE feed_role = '{role}'")
            ),
            ""
        );
    }
    assert_eq!(
        column(&instance, "SELECT title FROM epics WHERE feed_role = 'cve'"),
        "CVE"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT feed_interval_secs FROM epics WHERE feed_role = 'cve'"
        ),
        "900"
    );
}

/// An empty command leaves that subtree alone; a changed command is patched
/// onto the existing epic with its renamed title untouched.
#[test]
fn ensure_managed_epics_skips_disabled_feeds_and_reconciles_commands() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let only_cve = ensure_managed_epics(&instance, "", 0, "cve.sh", 0);
    assert!(only_cve.status.success(), "{}", describe(&only_cve));
    assert_eq!(column(&instance, "SELECT COUNT(*) AS n FROM epics"), "1");
    assert_eq!(epic_count_by_role(&instance, "reviews-parent"), "0");

    let id = column(&instance, "SELECT id FROM epics WHERE feed_role = 'cve'");
    let mut patch = blank_epic_patch_json();
    patch["title"] = serde_json::json!({"some": "My CVEs"});
    let renamed = instance.call("patch_epic", &[&id, &patch.to_string()]);
    assert!(renamed.status.success(), "{}", describe(&renamed));

    let changed = ensure_managed_epics(&instance, "", 0, "cve2.sh", 1200);
    assert!(changed.status.success(), "{}", describe(&changed));
    assert_eq!(epic_count_by_role(&instance, "cve"), "1");
    assert_eq!(
        column(
            &instance,
            "SELECT feed_command FROM epics WHERE feed_role = 'cve'"
        ),
        "cve2.sh"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT feed_interval_secs FROM epics WHERE feed_role = 'cve'"
        ),
        "1200"
    );
    assert_eq!(
        column(&instance, "SELECT title FROM epics WHERE feed_role = 'cve'"),
        "My CVEs"
    );

    // Deleting a managed epic is how it goes away; the next ensure re-creates it.
    let deleted = instance.call("delete_epic", &[&id]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert_eq!(epic_count_by_role(&instance, "cve"), "0");
    assert!(ensure_managed_epics(&instance, "", 0, "cve2.sh", 1200)
        .status
        .success());
    assert_eq!(epic_count_by_role(&instance, "cve"), "1");
}
