use super::*;

// A `reviews_parent` epic's exclusion from flat-path reconciliation
// (docs/specs/feeds.allium: FlatFeedReconcile requires feed_role !=
// reviews_parent) is structural, not a runtime branch inside
// FlatFeedReconcile itself: `run_feed_sync_by_role`'s match arm (above)
// routes `FeedRole::ReviewsParent` to `run_role_routed_feed_sync`
// exclusively, so `run_feed_sync`'s flat branch is never reached for a
// reviews_parent epic. That dispatch is already covered by
// `route_routed_inserts_into_role_sub_epic` and the other
// `route_routed_*` / `role_routed_*` tests above, which exercise
// `run_role_routed_feed_sync` directly — no separate test needed here.

// --- ExcludeFromReviews (feeds.allium: "Non-review PRs are excluded before
// routing") ---

/// An emitted PR the user authored is dropped before routing: it lands in NO
/// role sub-epic, not even Bots, and not on the parent.
#[tokio::test]
async fn role_routed_drops_own_authored_pr_entirely() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let items = vec![
        // Own-authored, and carrying the signal that would otherwise put it in
        // My Reviews.
        make_signal_item(
            "pr-mine",
            "https://github.com/org/repo/pull/1",
            vec![Signal::Reviewed, Signal::AuthorMe],
        ),
        // A genuine review, to prove the filter is not just emptying the sync.
        make_signal_item(
            "pr-theirs",
            "https://github.com/org/repo/pull/2",
            vec![Signal::DirectRequest],
        ),
    ];

    run_role_routed_feed_sync(
        &*db,
        parent.id,
        entries(&items, &["", ""], &["main", "main"]),
    )
    .await
    .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    let bots = role_sub_epic(&db, parent.id, FeedRole::Bots).await;

    let my_ids: Vec<String> = db
        .list_tasks_for_epic(my)
        .await
        .into_iter()
        .filter_map(|t| t.external_id)
        .collect();
    assert_eq!(
        my_ids,
        vec!["pr-theirs".to_string()],
        "an own-authored PR must not reach My Reviews"
    );
    assert!(db.list_tasks_for_epic(team).await.is_empty());
    assert!(
        db.list_tasks_for_epic(bots).await.is_empty(),
        "an excluded PR is dropped, not rerouted to Bots"
    );
    assert!(
        db.list_tasks_for_epic(parent.id).await.is_empty(),
        "an excluded PR must not be stranded on the parent either"
    );
}

/// The exclusion is unconditional: an own-authored PR that ALSO carries
/// team_request and author_bot is still dropped, never routed to Team or Bots.
#[tokio::test]
async fn role_routed_drops_own_authored_pr_even_with_team_and_bot_signals() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let items = vec![make_signal_item(
        "pr-mine",
        "https://github.com/org/repo/pull/1",
        vec![Signal::AuthorMe, Signal::TeamRequest, Signal::AuthorBot],
    )];

    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    for role in [FeedRole::MyReviews, FeedRole::TeamReviews, FeedRole::Bots] {
        let sub = role_sub_epic(&db, parent.id, role).await;
        assert!(
            db.list_tasks_for_epic(sub).await.is_empty(),
            "{role:?} must be empty — the author_me exclusion is unconditional"
        );
    }
}

/// A PR the user has already approved, with no review requested from them, is
/// dropped before routing: it lands in NO role sub-epic and not on the parent.
/// Approving is done via reviewing, so the `Reviewed` signal that put it in My
/// Reviews is still attached and must not rescue it.
#[tokio::test]
async fn role_routed_drops_settled_approved_pr_entirely() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let items = vec![
        make_signal_item(
            "pr-approved",
            "https://github.com/org/repo/pull/1",
            vec![Signal::Reviewed, Signal::Approved],
        ),
        // A bot PR I approved: excluded too, not rerouted to Bots.
        make_signal_item(
            "pr-approved-bot",
            "https://github.com/org/repo/pull/2",
            vec![Signal::AuthorBot, Signal::Approved],
        ),
        // Reviewed but NOT approved — still review work.
        make_signal_item(
            "pr-open",
            "https://github.com/org/repo/pull/3",
            vec![Signal::Reviewed],
        ),
    ];

    run_role_routed_feed_sync(
        &*db,
        parent.id,
        entries(&items, &["", "", ""], &["main", "main", "main"]),
    )
    .await
    .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    let bots = role_sub_epic(&db, parent.id, FeedRole::Bots).await;

    let my_ids: Vec<String> = db
        .list_tasks_for_epic(my)
        .await
        .into_iter()
        .filter_map(|t| t.external_id)
        .collect();
    assert_eq!(
        my_ids,
        vec!["pr-open".to_string()],
        "an approved PR must not reach My Reviews"
    );
    assert!(db.list_tasks_for_epic(team).await.is_empty());
    assert!(
        db.list_tasks_for_epic(bots).await.is_empty(),
        "an approved bot PR is dropped, not rerouted to Bots"
    );
    assert!(
        db.list_tasks_for_epic(parent.id).await.is_empty(),
        "an excluded PR must not be stranded on the parent either"
    );
}

/// A re-request after approval brings the card back: the approval still stands,
/// but a review is pending again, so the item is kept and routed as usual.
#[tokio::test]
async fn role_routed_keeps_approved_pr_with_a_pending_request() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let items = vec![
        make_signal_item(
            "pr-rerequested",
            "https://github.com/org/repo/pull/1",
            vec![Signal::Reviewed, Signal::Approved, Signal::DirectRequest],
        ),
        make_signal_item(
            "pr-team-rerequested",
            "https://github.com/org/repo/pull/2",
            vec![Signal::Approved, Signal::TeamRequest],
        ),
    ];

    run_role_routed_feed_sync(
        &*db,
        parent.id,
        entries(&items, &["", ""], &["main", "main"]),
    )
    .await
    .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;

    let my_ids: Vec<String> = db
        .list_tasks_for_epic(my)
        .await
        .into_iter()
        .filter_map(|t| t.external_id)
        .collect();
    assert_eq!(my_ids, vec!["pr-rerequested".to_string()]);

    let team_ids: Vec<String> = db
        .list_tasks_for_epic(team)
        .await
        .into_iter()
        .filter_map(|t| t.external_id)
        .collect();
    assert_eq!(team_ids, vec!["pr-team-rerequested".to_string()]);
}

/// Shared body for the two "an item that becomes excluded loses its task"
/// cases. Cycle 1 ingests the PR with `before` signals; cycle 2 re-emits the
/// SAME PR with `after` signals, which must exclude it. The feed script does
/// not filter the item — the runtime does — so the item still arrives on cycle
/// 2 and is dropped before routing. Being absent from the keep-set, the stale
/// delete reaches its task exactly as it reaches a merged PR's, while a manual
/// task (no `external_id`) in the same sub-epic survives.
async fn assert_newly_excluded_pr_loses_its_task(before: Vec<Signal>, after: Vec<Signal>) {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let url = "https://github.com/org/repo/pull/1";
    let cycle1 = vec![make_signal_item("pr-1", url, before)];
    run_role_routed_feed_sync(&*db, parent.id, entries(&cycle1, &[""], &["main"]))
        .await
        .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    assert_eq!(
        db.list_tasks_for_epic(my).await.len(),
        1,
        "precondition: the PR is in My Reviews before the exclusion applies"
    );

    let manual_id = create_manual_task(&db, "Manual", my).await;

    let cycle2 = vec![make_signal_item("pr-1", url, after)];
    run_role_routed_feed_sync(&*db, parent.id, entries(&cycle2, &[""], &["main"]))
        .await
        .unwrap();

    let remaining = db.list_tasks_for_epic(my).await;
    assert_eq!(
        remaining.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![manual_id],
        "the feed task is deleted and only the manual task remains"
    );
    assert_eq!(
        remaining[0].external_id, None,
        "the survivor is the manual (non-feed) task"
    );
}

/// Rule 1: the PR gains author_me. This is what cleans own-authored PRs that
/// were already sitting in the subtree when the exclusion landed.
#[tokio::test]
async fn role_routed_removes_existing_task_for_own_authored_pr() {
    assert_newly_excluded_pr_loses_its_task(
        vec![Signal::DirectRequest],
        vec![Signal::DirectRequest, Signal::AuthorMe],
    )
    .await;
}

/// Rule 2: the user approves a PR they had been reviewing. The card goes away
/// on the next poll rather than lingering until the PR merges.
#[tokio::test]
async fn role_routed_removes_existing_task_for_approved_pr() {
    assert_newly_excluded_pr_loses_its_task(
        vec![Signal::Reviewed],
        vec![Signal::Reviewed, Signal::Approved],
    )
    .await;
}

/// The exclusion applies on the additive (DegradedNonEmptyEmission) path too:
/// it shrinks what is inserted. The removal the exclusion would otherwise imply
/// is skipped with every other removal on that path, so a pre-existing task
/// survives a tainted cycle.
#[tokio::test]
async fn additive_role_routed_sync_does_not_insert_own_authored_pr() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let items = vec![make_signal_item(
        "pr-mine",
        "https://github.com/org/repo/pull/1",
        vec![Signal::DirectRequest, Signal::AuthorMe],
    )];

    run_role_routed_feed_sync_with_mode(
        &*db,
        parent.id,
        entries(&items, &[""], &["main"]),
        SyncMode::Additive,
    )
    .await
    .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    assert!(
        db.list_tasks_for_epic(my).await.is_empty(),
        "an excluded item is not inserted on the additive path either"
    );
}
