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
/// subscription. The board belongs to the epic's workspace and follows it, so
/// `covered_epics` serves it (workspaces.allium, "THE PER-CALLER VIEWS").
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
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        admit(&instance, &[&accepted.identity]);
        follow(&instance, &accepted.identity, 1);

        assert!(
            rows.epics().is_empty(),
            "nothing has been written yet, so nothing may have arrived"
        );

        // A different process writes the row. Nothing below asks for it.
        call_ok(
            &instance,
            "seed_epics",
            serde_json::json!([shared_epic(1, "Written by somebody else", 0)]),
        );

        wait_for(&rows, |rows| !rows.epics().is_empty())
            .await
            .unwrap_or_else(|_| panic!("the row never arrived"));

        let epics = rows.epics();
        assert_eq!(epics.len(), 1);
        assert_eq!(epics[0].title, "Written by somebody else");
    });
}

/// **Task #4927**: following an epic brings its whole sub-epic tree, at any
/// depth, and keeps bringing it as the tree grows.
///
/// The store walks the tree (`covered_epics`, workspaces.allium "THE
/// PER-CALLER VIEWS"); the board only follows 1. Two sub-epic levels exist
/// before the board subscribes, which proves the walk on a cold start; a third
/// level and a task inside it are written by another process once the board is
/// up, which proves it live.
#[test]
fn a_followed_epics_whole_sub_epic_tree_arrives_live() {
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
        admit(&instance, &[&accepted.identity]);
        follow(&instance, &accepted.identity, 1);
        call_ok(
            &instance,
            "seed_epics",
            serde_json::json!([
                shared_epic(1, "Followed", 0),
                shared_epic(2, "Child", 1),
                shared_epic(3, "Grandchild", 2),
                shared_epic(9, "Unfollowed", 0),
            ]),
        );

        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));

        wait_for(&rows, |rows| rows.epics().len() == 3)
            .await
            .unwrap_or_else(|_| {
                panic!("cold start: the tree never arrived: {:?}", epic_ids(&rows))
            });
        assert_eq!(epic_ids(&rows), vec![1, 2, 3], "epic 9 is not followed");

        call_ok(
            &instance,
            "seed_epics",
            serde_json::json!([shared_epic(4, "Great-grandchild", 3)]),
        );
        call_ok(
            &instance,
            "seed_tasks",
            serde_json::json!([task_json(1, "deep inside", "backlog", 4, "host-b")]),
        );

        wait_for(&rows, |rows| {
            rows.epics().len() == 4 && rows.tasks().len() == 1
        })
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

/// Unfollowing one epic takes its tree off the board and leaves another
/// followed epic's deeper rows where they are, with no resubscribe: the
/// `subscriptions` row the unfollow deletes is what `covered_epics` reads.
/// Epic 1 holds a grandchild (3) with a task; epic 9 and its task leave.
///
/// Task 3 in epic 9 is this board's own, so it is in `own_tasks` and
/// `covered_tasks` both: leaving one view must not take it off the board
/// (`sync.allium: AViewUpdateNeverErasesTheRow`).
#[test]
fn unfollowing_one_epic_keeps_the_deep_rows_of_another() {
    if !spacetime_available_or_skip() {
        return;
    }
    use dispatch_tui::models::EpicId;
    use dispatch_tui::sync::{ReducerCaller, SdkReducerCaller, SettledIdentity};

    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows = Arc::new(SharedRows::new());
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    runtime.block_on(async {
        let connector = Arc::new(SpacetimeSdkConnector::new(
            instance.database(),
            rows.clone(),
        ));
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        let me = accepted.identity.clone();
        admit(&instance, &[&me]);
        follow(&instance, &me, 1);
        follow(&instance, &me, 9);
        call_ok(
            &instance,
            "seed_epics",
            serde_json::json!([
                shared_epic(1, "Followed", 0),
                shared_epic(2, "Child", 1),
                shared_epic(3, "Grandchild", 2),
                shared_epic(9, "Unfollowed", 0),
            ]),
        );
        let mut mine = task_json(3, "mine in the unfollowed", "backlog", 9, "host-a");
        mine["created_by"] = serde_json::json!(me);
        call_ok(
            &instance,
            "seed_tasks",
            serde_json::json!([
                task_json(1, "deep inside", "backlog", 3, "host-b"),
                task_json(2, "in the unfollowed", "backlog", 9, "host-b"),
                mine,
            ]),
        );

        connector
            .subscribe(&SubscriptionRequest::new(me.clone(), "host-a"))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        wait_for(&rows, |rows| {
            rows.epics().len() == 4 && rows.tasks().len() == 3
        })
        .await
        .expect("cold start: both trees should arrive");

        let caller = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        caller
            .unsubscribe_from_epic(me.clone(), EpicId(9))
            .await
            .unwrap_or_else(|e| panic!("unsubscribe: {e}"))
            .applied()
            .unwrap_or_else(|why| panic!("unsubscribe refused: {why}"));

        wait_for(&rows, |rows| {
            rows.epics().iter().all(|e| e.id.0 != 9)
                && rows.epics().len() == 3
                && rows.tasks().len() == 2
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "after the unfollow: epics {:?}, tasks {:?}",
                epic_ids(&rows),
                rows.tasks()
                    .iter()
                    .map(|t| t.title.clone())
                    .collect::<Vec<_>>()
            )
        });
        let mut titles: Vec<String> = rows.tasks().iter().map(|t| t.title.clone()).collect();
        titles.sort();
        assert_eq!(
            titles,
            vec![
                "deep inside".to_string(),
                "mine in the unfollowed".to_string()
            ],
            "a still-followed epic's depth-2 task, and my own task, must survive an unfollow"
        );
        assert_eq!(epic_ids(&rows), vec![1, 2, 3]);
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
/// this test's own `SharedRows`, subscribed with nothing followed. Both are
/// members of workspace 1, and each learning comes from a task in it, so it
/// lands in that workspace and `visible_learnings` serves it to every member
/// (`workspaces.allium: LearningVisibility`).
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
        admit(&instance, &[&instance.identity(), &accepted.identity]);
        call_ok(
            &instance,
            "seed_epics",
            serde_json::json!([shared_epic(1, "Shared", 0)]),
        );
        call_ok(
            &instance,
            "seed_tasks",
            serde_json::json!([task_json(1, "source", "backlog", 1, "host-a")]),
        );
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                "host-b",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));

        assert!(
            rows.learnings_matching(&LearningFilter::default())
                .is_empty(),
            "nothing has been written yet, so nothing may have arrived"
        );

        // Host A records both learnings directly through the reducer.
        for (summary, embedding) in [
            ("High similarity", &high_sim_bytes),
            ("Low similarity", &low_sim_bytes),
        ] {
            call_ok(
                &instance,
                "create_learning",
                learning_json(summary, embedding),
            );
        }

        wait_for(&rows, |rows| {
            rows.approved_non_task_learnings_with_embedding().len() == 2
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "both learnings must arrive, got {}",
                rows.approved_non_task_learnings_with_embedding().len()
            )
        });

        // Host B's read: the RAG candidate pool, sourced entirely from the
        // subscription — this is `LearningStore::list_all_approved_non_task_learnings`'s
        // backing, `SharedRows::approved_non_task_learnings_with_embedding`.
        let candidates = rows.approved_non_task_learnings_with_embedding();

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

/// Workspace 1, with `members` in it. A board sees an epic only in a workspace
/// it belongs to (`workspaces.allium: Epic.visible_to`).
fn admit(instance: &Instance, members: &[&str]) {
    call_ok(
        instance,
        "seed_workspaces",
        serde_json::json!([{
            "id": 1, "name": "Shared", "created_by": "", "created_at": "2026-10-30 10:00:00",
            "listed": true,
        }]),
    );
    let memberships: Vec<serde_json::Value> = members
        .iter()
        .enumerate()
        .map(|(i, user)| {
            serde_json::json!({
                "id": i as i64 + 1, "workspace_id": 1, "user": user, "role": "member",
                "created_at": "2026-10-30 10:00:00",
            })
        })
        .collect();
    call_ok(
        instance,
        "seed_memberships",
        serde_json::Value::from(memberships),
    );
}

/// `who` follows `epic`. Seeded, because `subscribe_to_epic` refuses a
/// subscriber that is not the caller and the CLI is not this board.
fn follow(instance: &Instance, who: &str, epic: i64) {
    call_ok(
        instance,
        "seed_subscriptions",
        serde_json::json!([{
            "id": format!("{who}/{epic}"), "epic_id": epic, "subscriber": who,
        }]),
    );
}

/// A public epic in workspace 1.
fn shared_epic(id: i64, title: &str, parent: i64) -> serde_json::Value {
    let mut epic = epic_json(id, title, "backlog", parent);
    epic["workspace_id"] = serde_json::json!(1);
    epic
}

fn epic_ids(rows: &SharedRows) -> Vec<i64> {
    let mut ids: Vec<i64> = rows.epics().iter().map(|e| e.id.0).collect();
    ids.sort_unstable();
    ids
}

/// A repo-scoped `learnings` row from task 1, JSON-encoded for
/// `create_learning`'s CLI call.
///
/// `embedding` is passed pre-serialized bytes rather than a float vector: the
/// module treats it as opaque, exactly as `serialize_embedding` produces it.
fn learning_json(summary: &str, embedding: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "id": 0,
        "kind": "convention",
        "summary": summary,
        "detail": {"none": []},
        "scope": "repo",
        "scope_ref": {"some": "/repo/a"},
        "tags": "[]",
        "status": "approved",
        "source_task_id": {"some": 1},
        "upvote_count": 0,
        "last_upvoted_at": {"none": []},
        "created_at": "2026-09-26 10:00:00",
        "updated_at": "2026-09-26 10:00:00",
        "embedding": {"some": embedding},
        "workspace_id": 0,
        "author": "",
    })
}
