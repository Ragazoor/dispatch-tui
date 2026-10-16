use super::*;

// --- Retired feed items (feeds.allium: IngestSkipsRetiredFeedItems,
// DropClosedRetiredFeedItems; core.allium: RetiredFeedItem) ---
//
// A record is written by deleting a Done feed task (tasks.allium: DeleteTask)
// or its sub-epic (epics.allium: DeleteEpic), and observed through the only
// thing it changes: whether a later emission of the id inserts a task. A drop
// is observed the same way — once the record is gone, the id comes back.

/// Put `feed_command` on `epic`, making it the `nearest_feed_epic` its
/// subtree's deletions retire under.
async fn make_feed_epic(db: &Store, epic: EpicId) {
    db.patch_epic(epic, &EpicPatch::new().feed_command(Some("echo []")))
        .await
        .unwrap();
}

/// Every task anywhere under `root` (two levels deep covers every feed shape:
/// flat, grouped, role-routed, role-routed + group_by_repo) carrying `id`.
async fn subtree_tasks_with_id(db: &Store, root: EpicId, id: &str) -> Vec<crate::models::Task> {
    let mut epics = vec![root];
    for sub in db.list_sub_epics(root).await {
        epics.push(sub.id);
        for grand in db.list_sub_epics(sub.id).await {
            epics.push(grand.id);
        }
    }
    let mut out = vec![];
    for e in epics {
        out.extend(
            db.list_tasks_for_epic(e)
                .await
                .into_iter()
                .filter(|t| t.external_id.as_deref() == Some(id)),
        );
    }
    out
}

/// The DeleteTask gesture on the one task under `root` carrying `id`:
/// complete it, then delete it.
async fn retire_by_deleting(db: &Store, root: EpicId, id: &str) {
    let tasks = subtree_tasks_with_id(db, root, id).await;
    assert_eq!(tasks.len(), 1, "fixture: exactly one task for {id}");
    db.patch_task(tasks[0].id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    db.delete_task(tasks[0].id).await.unwrap();
}

async fn flat_feed_with_retired(db: &Store, title: &str, id: &str) -> EpicId {
    let epic = db.create_epic(title, "", None).await.unwrap();
    make_feed_epic(db, epic.id).await;
    let items = vec![make_item(id, "")];
    run_feed_sync(db, epic.id, false, entries(&items, &[""], &["main"]))
        .await
        .unwrap();
    retire_by_deleting(db, epic.id, id).await;
    epic.id
}

/// IngestSkipsRetiredFeedItems, flat reconcile path. The id is still in the
/// emission, so DropClosedRetiredFeedItems keeps the record and the insert is
/// refused.
#[tokio::test]
async fn flat_mirror_sync_inserts_no_task_for_a_retired_id() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic = flat_feed_with_retired(&db, "CVE", "cve-1").await;

    let items = vec![make_item("cve-1", ""), make_item("cve-2", "")];
    run_feed_sync(
        &*db,
        epic,
        false,
        entries(&items, &["", ""], &["main", "main"]),
    )
    .await
    .unwrap();

    assert!(subtree_tasks_with_id(&db, epic, "cve-1").await.is_empty());
    assert_eq!(subtree_tasks_with_id(&db, epic, "cve-2").await.len(), 1);
}

/// IngestSkipsRetiredFeedItems, flat additive path.
#[tokio::test]
async fn flat_additive_sync_inserts_no_task_for_a_retired_id() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic = flat_feed_with_retired(&db, "Log", "warn-1").await;

    let items = vec![make_item("warn-1", "")];
    run_feed_sync_with_mode(
        &*db,
        epic,
        false,
        entries(&items, &[""], &["main"]),
        SyncMode::Additive,
    )
    .await
    .unwrap();

    assert!(subtree_tasks_with_id(&db, epic, "warn-1").await.is_empty());
}

/// DropClosedRetiredFeedItems: a trusted mirror cycle whose emission no
/// longer carries a retired id drops the record — the upstream item closed —
/// so a later REOPEN of the same id shows up again as a fresh task.
#[tokio::test]
async fn mirror_sync_drops_the_record_of_an_id_no_longer_emitted_so_a_reopen_reappears() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic = flat_feed_with_retired(&db, "CVE", "cve-1").await;

    // Upstream closed it: a trusted, non-empty emission without the id.
    let closed = vec![make_item("cve-9", "")];
    run_feed_sync(&*db, epic, false, entries(&closed, &[""], &["main"]))
        .await
        .unwrap();

    // Upstream reopened it.
    let reopened = vec![make_item("cve-9", ""), make_item("cve-1", "")];
    run_feed_sync(
        &*db,
        epic,
        false,
        entries(&reopened, &["", ""], &["main", "main"]),
    )
    .await
    .unwrap();

    assert_eq!(
        subtree_tasks_with_id(&db, epic, "cve-1").await.len(),
        1,
        "the record was dropped when the id left a trusted emission, so the reopen is inserted"
    );
}

/// An empty trusted emission is a mirror cycle too (DegradedEmptyEmission is
/// the stderr/zero-exit case, which dispatches no FeedSync at all): every
/// record of the epic is absent from it and dropped.
#[tokio::test]
async fn mirror_sync_with_an_empty_emission_drops_the_record() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic = flat_feed_with_retired(&db, "CVE", "cve-1").await;

    run_feed_sync(&*db, epic, false, vec![]).await.unwrap();

    let items = vec![make_item("cve-1", "")];
    run_feed_sync(&*db, epic, false, entries(&items, &[""], &["main"]))
        .await
        .unwrap();
    assert_eq!(subtree_tasks_with_id(&db, epic, "cve-1").await.len(), 1);
}

/// "ADDITIVE CYCLES NEVER DROP, for either cause" (DropClosedRetiredFeedItems
/// requires `not additive`). The id's absence from an additive emission is
/// evidence of nothing, so the record survives and the next mirror cycle that
/// carries the id still refuses it.
#[tokio::test]
async fn additive_sync_never_drops_a_retired_record() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic = flat_feed_with_retired(&db, "Log", "warn-1").await;

    let other = vec![make_item("warn-2", "")];
    run_feed_sync_with_mode(
        &*db,
        epic,
        false,
        entries(&other, &[""], &["main"]),
        SyncMode::Additive,
    )
    .await
    .unwrap();

    let again = vec![make_item("warn-2", ""), make_item("warn-1", "")];
    run_feed_sync(
        &*db,
        epic,
        false,
        entries(&again, &["", ""], &["main", "main"]),
    )
    .await
    .unwrap();

    assert!(
        subtree_tasks_with_id(&db, epic, "warn-1").await.is_empty(),
        "an additive cycle must not have dropped the record"
    );
}

/// "The drop is per feed epic": a mirror cycle for one feed never drops
/// another feed's records, even for the same id.
#[tokio::test]
async fn mirror_sync_drops_only_the_records_of_its_own_feed_epic() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let a = flat_feed_with_retired(&db, "Feed A", "shared").await;
    let b = flat_feed_with_retired(&db, "Feed B", "shared").await;

    // A's upstream closed the item; B's did not emit anything this cycle.
    run_feed_sync(&*db, a, false, vec![]).await.unwrap();

    let items = vec![make_item("shared", "")];
    run_feed_sync(&*db, b, false, entries(&items, &[""], &["main"]))
        .await
        .unwrap();
    assert!(
        subtree_tasks_with_id(&db, b, "shared").await.is_empty(),
        "B's record must survive A's cycle"
    );
}

/// GroupedFeedUpsert drops a retired item with no task left in the subtree
/// BEFORE any sub-epic is found-or-created for it, so a repo whose only item
/// is retired gets no empty sub-epic.
#[tokio::test]
async fn grouped_sync_creates_no_sub_epic_for_a_retired_item_with_no_task() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let root = db.create_epic("Grouped", "", None).await.unwrap();
    make_feed_epic(&db, root.id).await;

    let items = vec![
        make_item("pr-a", "https://github.com/org/repo-a/pull/1"),
        make_item("pr-b", "https://github.com/org/repo-b/pull/1"),
    ];
    sync_grouped_feed(&*db, root.id, entries(&items, &["", ""], &["main", "main"])).await;

    // Delete the repo-a sub-epic with its (done) task: retired under the root.
    let repo_a = db
        .list_sub_epics(root.id)
        .await
        .into_iter()
        .find(|e| e.title == "repo-a")
        .unwrap();
    for t in db.list_tasks_for_epic(repo_a.id).await {
        db.patch_task(t.id, &TaskPatch::new().status(TaskStatus::Done))
            .await
            .unwrap();
    }
    db.delete_epic(repo_a.id).await.unwrap();

    sync_grouped_feed(&*db, root.id, entries(&items, &["", ""], &["main", "main"])).await;

    let titles: Vec<String> = db
        .list_sub_epics(root.id)
        .await
        .into_iter()
        .map(|e| e.title)
        .collect();
    assert_eq!(
        titles,
        vec!["repo-b".to_string()],
        "no repo-a sub-epic may be re-created just to hold nothing"
    );
    assert!(subtree_tasks_with_id(&db, root.id, "pr-a").await.is_empty());
}

/// A reviews_parent root carrying a feed_command, with `pr-1` already routed
/// into a role sub-epic and then deleted from it — retired under the ROOT,
/// since role sub-epics carry no command of their own.
async fn reviews_feed_with_retired_pr(db: &Store) -> EpicId {
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();
    make_feed_epic(db, parent.id).await;
    let items = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::DirectRequest],
    )];
    run_role_routed_feed_sync(db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();
    retire_by_deleting(db, parent.id, "pr-1").await;
    parent.id
}

/// IngestSkipsRetiredFeedItems, role-routed path — including under a
/// different role than the one the task was deleted from: the record is keyed
/// on the feed epic, not the role sub-epic.
#[tokio::test]
async fn role_routed_sync_inserts_no_task_for_a_retired_id_under_any_role() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = reviews_feed_with_retired_pr(&db).await;

    let rerouted = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::TeamRequest],
    )];
    run_role_routed_feed_sync(&*db, parent, entries(&rerouted, &[""], &["main"]))
        .await
        .unwrap();

    assert!(subtree_tasks_with_id(&db, parent, "pr-1").await.is_empty());
}

/// "`items` is the FULL PARSED emission" (DropClosedRetiredFeedItems): a PR
/// the source still emits but ExcludeFromReviews filters out is still open
/// upstream, so its record must survive — otherwise it would come back the
/// moment it stopped being excluded.
#[tokio::test]
async fn role_routed_mirror_keeps_the_record_of_an_excluded_but_emitted_pr() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = reviews_feed_with_retired_pr(&db).await;

    // Still emitted, but now excluded (own-authored) — dropped before routing.
    let excluded = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::Reviewed, Signal::AuthorMe],
    )];
    run_role_routed_feed_sync(&*db, parent, entries(&excluded, &[""], &["main"]))
        .await
        .unwrap();

    // No longer excluded.
    let included = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::DirectRequest],
    )];
    run_role_routed_feed_sync(&*db, parent, entries(&included, &[""], &["main"]))
        .await
        .unwrap();

    assert!(
        subtree_tasks_with_id(&db, parent, "pr-1").await.is_empty(),
        "an excluded-but-emitted PR is still open upstream: its record must not be dropped"
    );
}

/// DropClosedRetiredFeedItems on the role-routed path: a PR that genuinely
/// left the emission (merged/closed) loses its record, so a reopen reappears.
#[tokio::test]
async fn role_routed_mirror_drops_the_record_of_a_pr_no_longer_emitted() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = reviews_feed_with_retired_pr(&db).await;

    run_role_routed_feed_sync(&*db, parent, vec![])
        .await
        .unwrap();

    let reopened = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::DirectRequest],
    )];
    run_role_routed_feed_sync(&*db, parent, entries(&reopened, &[""], &["main"]))
        .await
        .unwrap();

    assert_eq!(subtree_tasks_with_id(&db, parent, "pr-1").await.len(), 1);
}

/// Additive role-routed cycles never drop either.
#[tokio::test]
async fn role_routed_additive_sync_never_drops_a_retired_record() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = reviews_feed_with_retired_pr(&db).await;

    run_role_routed_feed_sync_with_mode(&*db, parent, vec![], SyncMode::Additive)
        .await
        .unwrap();

    let again = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::DirectRequest],
    )];
    run_role_routed_feed_sync(&*db, parent, entries(&again, &[""], &["main"]))
        .await
        .unwrap();
    assert!(subtree_tasks_with_id(&db, parent, "pr-1").await.is_empty());
}

/// RoleRoutedFeedSync with group_by_repo on a role sub-epic: a retired item
/// with no task left is skipped before routing, so no repo-group sub-epic is
/// found-or-created for it.
#[tokio::test]
async fn role_routed_group_by_repo_creates_no_repo_sub_epic_for_a_retired_item() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();
    make_feed_epic(&db, parent.id).await;

    // First cycle creates the role sub-epics; retire pr-1 from Team Reviews.
    let items = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/myrepo/pull/1",
        vec![Signal::TeamRequest],
    )];
    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();
    retire_by_deleting(&db, parent.id, "pr-1").await;

    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();

    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    assert!(
        db.list_sub_epics(team).await.is_empty(),
        "no repo-group sub-epic may be created for a retired item"
    );
    assert!(subtree_tasks_with_id(&db, parent.id, "pr-1")
        .await
        .is_empty());
}
