use std::path::Path;

use super::super::exec::AlwaysFailRunner;
use super::*;
use crate::models::FeedRole;
use crate::store::{EpicCrud, EpicPatch, EpicRead, Store};

/// One PR, enough to be routed and therefore enough to be stranded.
const EMISSION: &str = r#"[{"external_id":"pr-1","title":"PR 1","description":"","status":"backlog","tag":"pr-review"}]"#;

/// A `reviews_parent` feed epic whose command records that it ran by
/// creating `sentinel`, then emits one item.
///
/// The role matters: `reviews_parent` is the flavour the bucket-5 failure
/// arms used to endanger. A cycle that fell back to `FeedRole::None` on a
/// failed epic read would route this emission through the FLAT upsert and
/// strand the task directly on the parent, violating
/// `NoFlatFeedTasksOnReviewsParent`.
async fn reviews_parent_with_sentinel_command(db: &Store, sentinel: &Path) -> EpicId {
    let epic = db.create_epic("Reviews", "", None).await.unwrap();
    let cmd = format!("touch {}; echo '{EMISSION}'", sentinel.display());
    db.patch_epic(
        epic.id,
        &EpicPatch::new()
            .feed_role(FeedRole::ReviewsParent)
            .feed_command(Some(cmd.as_str())),
    )
    .await
    .unwrap();
    epic.id
}

fn cycle(db: Arc<Store>, epic_id: EpicId) -> FeedCycle {
    FeedCycle {
        db,
        runner: Arc::new(AlwaysFailRunner),
        guard: Arc::new(FeedSyncGuard::default()),
        epic_id,
        epic_title: "Reviews".to_string(),
        known_paths: None,
        command_timeout: Duration::from_secs(5),
    }
}

fn failure(outcome: FeedCycleOutcome) -> String {
    match outcome {
        FeedCycleOutcome::Failed(err) => err,
        FeedCycleOutcome::Synced { count, .. } => {
            panic!("a failed epic read must not sync, but {count} item(s) were synced")
        }
        FeedCycleOutcome::Busy => panic!("nothing else claimed the epic"),
    }
}

/// feeds.allium `FeedCommandFailure` bucket 5: the epic is gone by the time
/// the cycle claims it. Nothing may be spawned and nothing may be synced —
/// in particular the cycle must not proceed with a defaulted `FeedRole`.
#[tokio::test]
async fn a_cycle_whose_epic_is_gone_fails_without_running_the_command() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let sentinel = dir.path().join("command-ran");
    let epic_id = reviews_parent_with_sentinel_command(&db, &sentinel).await;

    db.delete_epic(epic_id).await.unwrap();

    let err = failure(cycle(db, epic_id).run().await);

    assert!(
        err.contains("epic no longer exists"),
        "a missing epic must fail as such, got: {err}"
    );
    assert!(
        !sentinel.exists(),
        "the failure precedes the exec, so the feed command must never run"
    );
}

/// Bucket 5, mid-cycle: the epic is deleted while the command runs, so the
/// store refuses the sync. That is a race, logged below warn; a refusal
/// for an epic that still exists stays a warning.
#[tokio::test]
async fn a_sync_refused_because_the_epic_was_deleted_mid_cycle_is_not_a_warning() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let sentinel = dir.path().join("command-ran");
    let epic_id = reviews_parent_with_sentinel_command(&db, &sentinel).await;
    let refusal = || anyhow::anyhow!("the shared store refused: epic not found");

    let gone_log = crate::test_log::logged_during(|| async {
        db.delete_epic(epic_id).await.unwrap();
        let out = cycle(Arc::clone(&db), epic_id).fail_sync(refusal()).await;
        assert!(failure(out).contains("epic no longer exists"));
    })
    .await;
    assert!(!gone_log.contains("WARN"), "got: {gone_log}");

    let live = reviews_parent_with_sentinel_command(&db, &sentinel).await;
    let live_log = crate::test_log::logged_during(|| async {
        failure(cycle(Arc::clone(&db), live).fail_sync(refusal()).await);
    })
    .await;
    assert!(live_log.contains("WARN"), "got: {live_log}");
}

/// #4150: exec_feed_command has a deadline. A command that never exits
/// must fail within it AND release the epic's claim so the NEXT cycle
/// proceeds — proving only that one call returned is not enough (see
/// feeds.allium SerialisedFeedCycle's bounded-cost note). Reuses the
/// FIFO shape from
/// `src/runtime/tests/feeds.rs::manual_refresh_is_dropped_while_a_real_auto_poll_cycle_is_in_flight`:
/// `cat <fifo>` blocks forever because nothing ever opens the write end,
/// so the hang is genuine, not a timed sleep that would end on its own.
#[tokio::test]
async fn a_hung_command_times_out_and_the_epic_recovers_on_the_next_cycle() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic = db.create_epic("Feed", "", None).await.unwrap();

    let fifo = std::env::temp_dir().join(format!("dispatch_feed_timeout_{}", epic.id.0));
    let _ = std::fs::remove_file(&fifo);
    let mkfifo = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(mkfifo.success(), "mkfifo failed for {}", fifo.display());

    // Never written to: this command hangs forever until killed.
    db.patch_epic(
        epic.id,
        &EpicPatch::new().feed_command(Some(format!("cat {}", fifo.display()).as_str())),
    )
    .await
    .unwrap();

    let guard = Arc::new(FeedSyncGuard::default());
    let hung_cycle = FeedCycle {
        db: db.clone(),
        runner: Arc::new(AlwaysFailRunner),
        guard: guard.clone(),
        epic_id: epic.id,
        epic_title: "Feed".to_string(),
        known_paths: None,
        command_timeout: Duration::from_millis(100),
    };

    let err = match hung_cycle.run().await {
        FeedCycleOutcome::Failed(err) => err,
        FeedCycleOutcome::Synced { count, .. } => {
            panic!("a hung command must not sync, but {count} item(s) were synced")
        }
        FeedCycleOutcome::Busy => panic!("nothing else claimed the epic"),
    };
    assert!(
        err.contains("timed out"),
        "a hung command must fail as a timeout, got: {err}"
    );

    // The real proof: point the epic at a fast, successful command and
    // run a FRESH cycle. If the claim (or anything else) were still
    // wedged, this would come back Busy or Failed instead of Synced.
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("echo '[]'")))
        .await
        .unwrap();
    let recovered_cycle = FeedCycle {
        db,
        runner: Arc::new(AlwaysFailRunner),
        guard,
        epic_id: epic.id,
        epic_title: "Feed".to_string(),
        known_paths: None,
        command_timeout: Duration::from_secs(5),
    };
    let outcome = recovered_cycle.run().await;
    assert!(
        matches!(outcome, FeedCycleOutcome::Synced { .. }),
        "the epic must recover and sync on the next cycle after a timeout, got a non-Synced outcome"
    );

    let _ = std::fs::remove_file(&fifo);
}

// ---------------------------------------------------------------------
// AppendOnlyFeed (feeds.allium)
//
// The mode selection is the whole of this rule's implementation: the
// ingest layer below already implements "no removals" and is covered by
// the additive_* tests in src/feed/ingest/tests.rs. What these pin is
// that an append-only epic REACHES that layer in Additive mode, that a
// mirroring epic still does not, and that being append-only is not
// reported as a degradation.
// ---------------------------------------------------------------------

const TWO_RECORDS: &str = r#"[{"external_id":"log-1","title":"warn A","description":"","status":"backlog","tag":"bug"},{"external_id":"log-2","title":"warn B","description":"","status":"backlog","tag":"bug"}]"#;
const ONE_RECORD: &str =
    r#"[{"external_id":"log-2","title":"warn B","description":"","status":"backlog","tag":"bug"}]"#;

/// A plain flat feed epic (no role, no grouping) whose command emits
/// `emission`, optionally declaring itself append-only.
async fn flat_feed_epic(db: &Store, emission: &str, append_only: bool) -> EpicId {
    let epic = db.create_epic("Log warnings", "", None).await.unwrap();
    let cmd = format!("echo '{emission}'");
    db.patch_epic(
        epic.id,
        &EpicPatch::new()
            .feed_command(Some(cmd.as_str()))
            .feed_append_only(append_only),
    )
    .await
    .unwrap();
    epic.id
}

/// Repoint an existing feed epic at a new emission and run another cycle.
async fn recycle(db: &Arc<Store>, epic_id: EpicId, emission: &str) -> FeedCycleOutcome {
    let cmd = format!("echo '{emission}'");
    db.patch_epic(epic_id, &EpicPatch::new().feed_command(Some(cmd.as_str())))
        .await
        .unwrap();
    cycle(Arc::clone(db), epic_id).run().await
}

async fn external_ids(db: &Store, epic_id: EpicId) -> Vec<String> {
    let mut ids: Vec<String> = db
        .list_tasks_for_epic(epic_id)
        .await
        .into_iter()
        .filter_map(|t| t.external_id)
        .collect();
    ids.sort();
    ids
}

/// The rule itself. A log record is an event: it is emitted once and never
/// again, and its absence from the next emission says nothing. An
/// append-only epic must therefore keep the task.
#[tokio::test]
async fn an_append_only_epic_keeps_a_task_absent_from_the_emission() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic_id = flat_feed_epic(&db, TWO_RECORDS, true).await;

    cycle(Arc::clone(&db), epic_id).run().await;
    assert_eq!(external_ids(&db, epic_id).await, ["log-1", "log-2"]);

    recycle(&db, epic_id, ONE_RECORD).await;

    assert_eq!(
        external_ids(&db, epic_id).await,
        ["log-1", "log-2"],
        "an append-only feed never removes: log-1 is absent from the emission, not retracted by it"
    );
}

/// The boundary the rule must not cross. Identical setup, flag off: the
/// ordinary feed-as-source-of-truth removal still applies.
#[tokio::test]
async fn a_mirroring_epic_still_removes_a_task_absent_from_the_emission() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic_id = flat_feed_epic(&db, TWO_RECORDS, false).await;

    cycle(Arc::clone(&db), epic_id).run().await;
    assert_eq!(external_ids(&db, epic_id).await, ["log-1", "log-2"]);

    recycle(&db, epic_id, ONE_RECORD).await;

    assert_eq!(
        external_ids(&db, epic_id).await,
        ["log-2"],
        "feed_append_only defaults off, so absence from a trusted emission still removes"
    );
}

/// An append-only cycle is healthy, not degraded. Reporting it as degraded
/// every poll would train the user to ignore the line that matters.
#[tokio::test]
async fn an_append_only_cycle_is_not_reported_as_degraded() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic_id = flat_feed_epic(&db, TWO_RECORDS, true).await;

    match cycle(db, epic_id).run().await {
        FeedCycleOutcome::Synced { degraded, .. } => assert!(
            degraded.is_none(),
            "withholding removals is this epic's configured behaviour, not a symptom, got: {degraded:?}"
        ),
        _ => panic!("a clean append-only emission must sync"),
    }
}

/// The two causes of additivity are independent, and neither swallows the
/// other: an append-only epic whose command ALSO wrote to stderr is still
/// a degraded run, and must still say so.
#[tokio::test]
async fn an_append_only_epic_still_reports_a_degraded_emission() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic_id = flat_feed_epic(&db, TWO_RECORDS, true).await;
    // Same epic, same emission — only the stderr differs, so this isolates
    // the second cause rather than re-standing-up the first.
    let cmd = format!("echo '{TWO_RECORDS}'; echo 'scan window truncated' >&2");
    db.patch_epic(epic_id, &EpicPatch::new().feed_command(Some(cmd.as_str())))
        .await
        .unwrap();

    match cycle(db, epic_id).run().await {
        FeedCycleOutcome::Synced { degraded, .. } => assert!(
            degraded.is_some(),
            "append-only does not suppress the degradation report; the command still failed part way"
        ),
        _ => panic!("a non-empty emission must sync even with stderr"),
    }
}
