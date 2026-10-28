use super::*;

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
                "workspace_id": 0,
                "visibility": "public",
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

/// Unfollowing one epic must not take another followed epic's deeper rows with
/// it.
///
/// `sync.allium: AnUnfollowReassertsTheWholeAsk`. The replacing subscribe
/// resets every widening handle, and the deeper levels come back only from
/// insert callbacks; rows already in the SDK cache fire none. Epic 1 holds a
/// grandchild (3) with a task, epic 9 is unfollowed, and the task in 3 must
/// survive.
#[test]
fn unfollowing_one_epic_keeps_the_deep_rows_of_another() {
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
    let seeded = instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "deep inside", "backlog", 3, "host-b"),
            task_json(2, "in the unfollowed", "backlog", 9, "host-b"),
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
        let ask =
            |epics: Vec<i64>| SubscriptionRequest::new(accepted.identity.clone(), epics, "host-a");
        connector
            .subscribe(&ask(vec![1, 9]))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));

        let woken = rows.changed();
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

        wait_for(|rows| rows.epics().len() == 4 && rows.tasks().len() == 2)
            .await
            .expect("cold start: both trees should arrive");

        connector
            .subscribe(&ask(vec![1]))
            .await
            .unwrap_or_else(|e| panic!("resubscribe: {e}"));

        // Epic 9 and its task leave the view; the grandchild's task must be
        // (or come back to be) the only one left. A widening is a round trip,
        // so this waits on the rows rather than asserting at once.
        wait_for(|rows| {
            rows.epics().iter().all(|e| e.id.0 != 9)
                && rows.epics().len() == 3
                && rows.tasks().len() == 1
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "after the unfollow: epics {:?}, tasks {:?}",
                rows.epics().iter().map(|e| e.id.0).collect::<Vec<_>>(),
                rows.tasks()
                    .iter()
                    .map(|t| t.title.clone())
                    .collect::<Vec<_>>()
            )
        });
        let titles: Vec<String> = rows.tasks().iter().map(|t| t.title.clone()).collect();
        assert_eq!(
            titles,
            vec!["deep inside".to_string()],
            "a still-followed epic's depth-2 task must survive an unfollow"
        );
        let mut ids: Vec<i64> = rows.epics().iter().map(|e| e.id.0).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2, 3]);
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
        "workspace_id": 0,
    })
}
