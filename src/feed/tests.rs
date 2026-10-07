use std::sync::Arc;

use super::*;
use crate::models::{test_tmux_window, TaskStatus, TaskTag, MIN_FEED_INTERVAL_SECS};
use crate::store::{Database, EpicCrud, EpicPatch, EpicRead, RepoConfigStore, TaskCrud};

use super::exec::AlwaysFailRunner;

// --- FeedRunner tests ---

fn make_runner(db: Arc<Database>) -> (FeedRunner, mpsc::UnboundedReceiver<McpEvent>) {
    make_runner_with_runner(db, Arc::new(AlwaysFailRunner))
}

fn make_runner_with_runner(
    db: Arc<Database>,
    runner: Arc<dyn ProcessRunner>,
) -> (FeedRunner, mpsc::UnboundedReceiver<McpEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let board_reads = db
        .board_reads()
        .expect("a memory-attached handle serves its own board reads");
    (
        FeedRunner::new(db, tx, runner, board_reads, "test-host".into()),
        rx,
    )
}

/// `tick()` hands a slow feed command to a background task; it must not
/// await the command itself. The bound is an outer `timeout` rather than an
/// assertion on measured elapsed time — 5s is far above what a handful of
/// in-memory DB round-trips cost even on a loaded machine, and far below
/// the 30s a `tick()` that awaited the command inline would take.
#[tokio::test]
async fn tick_does_not_block_event_loop() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Slow Epic", "", None).await.unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("sleep 30")))
        .await
        .unwrap();

    let (mut runner, _rx) = make_runner(db.clone());

    tokio::time::timeout(Duration::from_secs(5), runner.tick())
        .await
        .expect("tick() must dispatch the feed command, not await it");
}

#[tokio::test]
async fn tick_background_task_upserts_tasks() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("BG Epic", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"bg1","title":"BG","description":"","status":"backlog","tag":"bug"}]'"#,
            )),
        ).await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent::Refresh")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "BG");
}

#[tokio::test]
async fn tick_done_epic_moves_to_backlog_when_new_feed_tasks_added() {
    // Regression test: a done epic should regress to backlog when the feed
    // adds new non-done tasks, because recalculate_epic_status must be
    // called after upsert_feed_tasks.
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Done Epic", "", None).await.unwrap();

    // Mark the epic as done before the feed runs.
    db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .status(TaskStatus::Done)
                .feed_command(Some(
                    r#"echo '[{"external_id":"new1","title":"New Task","description":"","status":"backlog","tag":"bug"}]'"#,
                )),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent")
        .expect("channel closed");

    // After the feed adds a new backlog task, the epic must regress to backlog.
    let refreshed = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(
        refreshed.status,
        TaskStatus::Backlog,
        "done epic with new backlog feed task should regress to backlog"
    );
}

#[tokio::test]
async fn tick_valid_json_upserts_tasks() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("My Epic", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"1","title":"T","description":"D","status":"backlog","tag":"bug"}]'"#,
            )),
        ).await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent::Refresh")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "T");
    assert_eq!(tasks[0].external_id.as_deref(), Some("1"));
}

// Regression coverage for feeds.allium: FeedCommandStderrOnSuccess on the
// auto-poll path — a command that writes to stderr while still exiting 0
// with a valid item array must sync exactly as if it had written nothing.
// Only the manual "r" path (src/runtime/tests.rs:3211) had this proven
// before; this closes the gap for the auto-poll path.
#[tokio::test]
async fn tick_stderr_on_zero_exit_does_not_suppress_sync() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Noisy Epic", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo 'Invalid search query' >&2; echo '[{"external_id":"1","title":"T","description":"D","status":"backlog","tag":"bug"}]'"#,
            )),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent::Refresh")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "stderr on a zero exit must not suppress the sync"
    );
    assert_eq!(tasks[0].title, "T");
    assert_eq!(tasks[0].external_id.as_deref(), Some("1"));
}

// Regression for #3989 (feeds.allium: DegradedEmptyEmission). A command that
// soft-fails to `[]` while reporting the reason on stderr must NOT reconcile —
// syncing it would delete every feed task already in the epic.
#[tokio::test]
async fn tick_degraded_empty_emission_does_not_delete_existing_tasks() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Degraded Epic", "", None).await.unwrap();

    // Seed one feed task, as a previous healthy poll would have.
    db.upsert_feed_tasks(
        epic.id,
        &[crate::models::FeedItem {
            external_id: "pr-1".to_string(),
            title: "Existing PR".to_string(),
            description: String::new(),
            url: String::new(),
            url_type: None,
            status: TaskStatus::Backlog,
            tag: TaskTag::PrReview,
            labels: Vec::new(),
            sort_order: None,
            signals: vec![],
            wrap_up_mode: None,
        }],
        &["".to_string()],
        &["main".to_string()],
    )
    .await
    .unwrap();
    assert_eq!(db.list_tasks_for_epic(epic.id).await.unwrap().len(), 1);

    db.patch_epic(
        epic.id,
        &EpicPatch::new().feed_command(Some("echo 'Invalid search query' >&2; echo '[]'")),
    )
    .await
    .unwrap();

    let (mut runner, _rx) = make_runner(db.clone());
    runner.tick().await;
    runner.join_spawned_jobs().await;

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "a degraded empty emission must not delete existing feed tasks"
    );
    assert_eq!(tasks[0].external_id.as_deref(), Some("pr-1"));
}

/// End-to-end wiring guard for the AUTO-POLL path: a real `FeedRunner::tick`
/// whose emission drops a task must actually shell out `git worktree remove`
/// for it.
///
/// This crosses the seam the rest of the suite leaves untested. The ingest
/// tests prove `FeedSyncOutcome::removed` is populated; the `cleanup_*` tests
/// call `cleanup_removed_feed_tasks` directly with a hand-built `Vec`.
/// Neither notices if the cycle stops passing one to the other — before
/// this test, deleting the fan-out call left the whole suite green. Removing
/// the helper's `#[allow(dead_code)]` was the compiler's only check on that
/// wiring, and it is gone now that a caller exists.
#[tokio::test]
async fn tick_removed_task_tears_down_its_worktree() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Reviews", "", None).await.unwrap();

    // Seed one feed task, as a previous healthy poll would have, and give it
    // the on-disk state a dispatched agent would own.
    db.upsert_feed_tasks(
        epic.id,
        &[crate::models::FeedItem {
            external_id: "pr-1".to_string(),
            title: "Merged PR".to_string(),
            description: String::new(),
            url: String::new(),
            url_type: None,
            status: TaskStatus::Backlog,
            tag: TaskTag::PrReview,
            labels: Vec::new(),
            sort_order: None,
            signals: vec![],
            wrap_up_mode: None,
        }],
        &["/repo/a".to_string()],
        &["main".to_string()],
    )
    .await
    .unwrap();
    let task = db.list_tasks_for_epic(epic.id).await.unwrap().remove(0);
    db.patch_task(
        task.id,
        &TaskPatch::new()
            .worktree(Some("/repo/a/.worktrees/7-pr-1"))
            .tmux_window(Some(&test_tmux_window("dispatch:pr-1"))),
    )
    .await
    .unwrap();

    // The PR merged, so this poll's emission no longer carries it. A clean
    // empty emission (no stderr) is a genuine reconcile, not a degraded run.
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("echo '[]'")))
        .await
        .unwrap();

    let proc_runner = Arc::new(MockProcessRunner::new(vec![
        // has_window: list-windows names the window, so the kill proceeds
        MockProcessRunner::ok_with_stdout(b"dispatch:pr-1\n"),
        MockProcessRunner::ok(), // tmux kill-window
        MockProcessRunner::ok(), // git worktree remove
        MockProcessRunner::ok(), // git branch -D (best effort)
    ]));
    let (mut runner, _rx) = make_runner_with_runner(db.clone(), proc_runner.clone());
    runner.tick().await;
    runner.join_spawned_jobs().await;

    assert!(
        db.list_tasks_for_epic(epic.id).await.unwrap().is_empty(),
        "the merged PR's row is gone"
    );

    let calls = proc_runner.flattened_calls();
    assert!(
        calls
            .iter()
            .any(|c| c.contains("worktree remove") && c.contains("/repo/a/.worktrees/7-pr-1")),
        "the auto-poll path must tear the removed task's worktree down, got: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("kill-window")),
        "and kill its tmux window, got: {calls:?}"
    );
}

/// Regression for #4095 (feeds.allium: DegradedNonEmptyEmission). The
/// counterpart to `tick_removed_task_tears_down_its_worktree`: the same
/// emission-drops-a-task shape, but the command reports an error on stderr
/// while still emitting an item — a PARTIALLY degraded run. The row must
/// survive AND no teardown may be shelled out for it.
///
/// The `MockProcessRunner` is scripted with no responses at all beyond the
/// base-branch probe, so any teardown attempt fails loudly rather than
/// passing silently: this test would be nearly worthless asserting only on
/// the DB, since the destroyed worktree is the part that cannot be undone.
#[tokio::test]
async fn tick_partially_degraded_emission_does_not_delete_or_tear_down() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Reviews", "", None).await.unwrap();

    // Two feed tasks from a previous healthy poll; `pr-1` carries a live
    // agent's worktree and tmux window.
    let seeded: Vec<crate::models::FeedItem> = ["pr-1", "pr-2"]
        .iter()
        .map(|ext| crate::models::FeedItem {
            external_id: ext.to_string(),
            title: "Seeded".to_string(),
            description: String::new(),
            url: String::new(),
            url_type: None,
            status: TaskStatus::Backlog,
            tag: TaskTag::PrReview,
            labels: Vec::new(),
            sort_order: None,
            signals: vec![],
            wrap_up_mode: None,
        })
        .collect();
    db.upsert_feed_tasks(
        epic.id,
        &seeded,
        &vec!["/repo/a".to_string(); 2],
        &vec!["main".to_string(); 2],
    )
    .await
    .unwrap();

    let live = db
        .list_tasks_for_epic(epic.id)
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.external_id.as_deref() == Some("pr-1"))
        .unwrap();
    db.patch_task(
        live.id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .sub_status(crate::models::SubStatus::Active)
            .worktree(Some("/repo/a/.worktrees/7-pr-1"))
            .tmux_window(Some(&test_tmux_window("dispatch:pr-1"))),
    )
    .await
    .unwrap();

    // One sub-query soft-failed: pr-1 is missing from an otherwise valid
    // emission, and the reason is on stderr.
    db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo 'fetch-reviews: gh search prs failed' >&2; echo '[{"external_id":"pr-2","title":"Other","description":"","status":"backlog","tag":"pr-review"}]'"#,
            )),
        )
        .await
        .unwrap();

    let proc_runner = Arc::new(MockProcessRunner::new(vec![]));
    let (mut runner, _rx) = make_runner_with_runner(db.clone(), proc_runner.clone());
    runner.tick().await;
    runner.join_spawned_jobs().await;

    let ids: Vec<String> = db
        .list_tasks_for_epic(epic.id)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|t| t.external_id)
        .collect();
    assert!(
        ids.contains(&"pr-1".to_string()),
        "a task omitted by a partially degraded emission must survive, got {ids:?}"
    );
    assert!(
        ids.contains(&"pr-2".to_string()),
        "the emitted item is still synced — additive, not suppressed"
    );

    let calls = proc_runner.flattened_calls();
    assert!(
        !calls.iter().any(|c| c.contains("worktree remove")),
        "a degraded emission must not force-remove a live agent's worktree, got: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("kill-window")),
        "nor kill its tmux window, got: {calls:?}"
    );
}

#[tokio::test]
async fn tick_persists_feed_tag() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Tagged Epic", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://github.com/o/r/pull/1","status":"backlog","tag":"pr-review"}]'"#,
            )),
        ).await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent::Refresh")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].tag, Some(TaskTag::PrReview));
}

#[tokio::test]
async fn tick_missing_tag_rejects_item() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Untagged Epic", "", None).await.unwrap();
    db.patch_epic(
        epic.id,
        &EpicPatch::new().feed_command(Some(
            r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog"}]'"#,
        )),
    )
    .await
    .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    // Parse must fail and no Refresh is sent.
    let result = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
    assert!(
        result.is_err(),
        "expected no notification when tag is missing"
    );

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert!(tasks.is_empty(), "no task should be inserted on parse fail");
}

#[tokio::test]
async fn tick_nonzero_exit_no_panic() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Err Epic", "", None).await.unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("exit 1")))
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await; // must not panic

    // No Refresh is sent on failure — expect timeout
    let result = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
    assert!(result.is_err(), "expected timeout but got a notification");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn tick_malformed_json_no_panic() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Bad JSON Epic", "", None).await.unwrap();
    db.patch_epic(
        epic.id,
        &EpicPatch::new().feed_command(Some("echo 'not-json'")),
    )
    .await
    .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await; // must not panic

    let result = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
    assert!(result.is_err(), "expected timeout but got a notification");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn tick_interval_not_elapsed_skips_command() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Interval Epic", "", None).await.unwrap();

    // Write a counter to a temp file so we can count how many times the command ran.
    let tmp = std::env::temp_dir().join(format!("feed_test_{}", epic.id.0));
    let cmd = format!(
        r#"echo 0 >> {path}; echo '[{{"external_id":"1","title":"T","description":"","status":"backlog","tag":"bug"}}]'"#,
        path = tmp.display()
    );
    db.patch_epic(
        epic.id,
        &EpicPatch::new()
            .feed_command(Some(&cmd))
            .feed_interval_secs(Some(10000)),
    )
    .await
    .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    // First tick: command runs, counter file gets one line.
    runner.tick().await;
    // Wait for the background task to finish before checking interval logic.
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for first tick refresh")
        .expect("channel closed");
    // Second tick immediately: interval (10000s) not elapsed, command must not run again.
    runner.tick().await;

    let content = std::fs::read_to_string(&tmp).unwrap_or_default();
    let lines: Vec<_> = content.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "command ran {count} times, expected 1",
        count = lines.len()
    );

    let _ = std::fs::remove_file(&tmp);
}

// --- the feed-cadence floor (feeds.allium: FeedTick) ---

/// Make an epic due on the next `tick()` without touching its interval and
/// without sleeping: dropping its `last_run` entry makes `elapsed` read as
/// `Duration::MAX`, which beats any interval.
///
/// This is the lever for "run the command again immediately". Setting the
/// interval to 0 used to serve that purpose, but a sub-floor interval is
/// now refused at read (`FeedTick`), so an interval of 0 tests the refusal
/// rather than the re-run.
fn force_due(runner: &mut FeedRunner, epic_id: EpicId) {
    runner.last_run.remove(&epic_id);
}

/// The default an unset interval inherits must itself clear the floor —
/// otherwise a blank field polls faster than the fastest value a user is
/// permitted to enter. (feeds.allium: DefaultFeedIntervalClearsTheFloor)
#[test]
fn the_default_interval_clears_the_floor() {
    assert!(
        DEFAULT_FEED_INTERVAL >= Duration::from_secs(MIN_FEED_INTERVAL_SECS as u64),
        "DEFAULT_FEED_INTERVAL ({DEFAULT_FEED_INTERVAL:?}) must not be below the floor \
             of {MIN_FEED_INTERVAL_SECS}s"
    );
}

// --- epic_due: the extracted per-epic predicate ---

fn cadence_test_epic(interval_secs: Option<i64>) -> crate::models::Epic {
    crate::models::Epic {
        id: EpicId(1),
        title: "Cadence Test".to_string(),
        description: String::new(),
        status: TaskStatus::Backlog,
        plan_path: None,
        sort_order: None,
        completed_at: None,
        auto_dispatch: false,
        parent_epic_id: None,
        feed_command: Some("echo hi".to_string()),
        feed_interval_secs: interval_secs,
        group_by_repo: false,
        feed_append_only: false,
        feed_role: crate::models::FeedRole::None,
        origin: crate::models::EpicOrigin::Manual,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn epic_due_is_true_when_never_run() {
    let epic = cadence_test_epic(Some(MIN_FEED_INTERVAL_SECS));
    let last_run = HashMap::new();
    assert!(epic_due(&epic, &last_run, Instant::now()));
}

#[test]
fn epic_due_is_false_before_the_interval_elapses() {
    let epic = cadence_test_epic(Some(3600));
    let mut last_run = HashMap::new();
    let now = Instant::now();
    last_run.insert(epic.id, now);
    assert!(!epic_due(&epic, &last_run, now));
}

#[test]
fn epic_due_is_true_once_the_interval_elapses() {
    let epic = cadence_test_epic(Some(MIN_FEED_INTERVAL_SECS));
    let mut last_run = HashMap::new();
    let started = Instant::now();
    last_run.insert(epic.id, started);
    let now = started + Duration::from_secs(MIN_FEED_INTERVAL_SECS as u64);
    assert!(epic_due(&epic, &last_run, now));
}

#[test]
fn epic_due_rejects_an_interval_below_the_floor() {
    let epic = cadence_test_epic(Some(MIN_FEED_INTERVAL_SECS - 1));
    let last_run = HashMap::new();
    assert!(!epic_due(&epic, &last_run, Instant::now()));
}

#[test]
fn epic_due_rejects_a_negative_interval_rather_than_wrapping() {
    let epic = cadence_test_epic(Some(-5));
    let last_run = HashMap::new();
    assert!(!epic_due(&epic, &last_run, Instant::now()));
}

/// Write-time validation is what normally keeps a sub-floor row from
/// existing, so this test writes one via `patch_epic`, bypassing the
/// service. The epic must not be polled at all: clamping would run it at a
/// cadence nobody chose while looking healthy. (An interval of 0 is not
/// tried: the store reads 0 as "no interval set", which is a different
/// state with its own default.)
#[tokio::test]
async fn tick_skips_an_epic_whose_stored_interval_is_below_the_floor() {
    for bad in [MIN_FEED_INTERVAL_SECS - 1] {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db.create_epic("Too Fast", "", None).await.unwrap();
        db.patch_epic(
                epic.id,
                &EpicPatch::new()
                    .feed_command(Some(
                        r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog","tag":"bug"}]'"#,
                    ))
                    .feed_interval_secs(Some(bad)),
            )
            .await
            .unwrap();

        let (mut runner, mut rx) = make_runner(db.clone());
        runner.tick().await;
        runner.join_spawned_jobs().await;

        assert!(
            db.list_tasks_for_epic(epic.id).await.unwrap().is_empty(),
            "interval {bad} is below the floor, so the command must not have run"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(200), rx.recv())
                .await
                .is_err(),
            "a skipped epic must not emit a refresh"
        );
    }
}

/// A negative interval failed differently and worse than a zero one: `as
/// u64` wrapped it into an effectively infinite cadence, so the feed went
/// permanently silent with nothing logged. It now takes the same visible
/// skip path as any other sub-floor value.
#[tokio::test]
async fn tick_skips_an_epic_with_a_negative_interval_rather_than_wrapping() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Negative", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .feed_command(Some(
                    r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog","tag":"bug"}]'"#,
                ))
                .feed_interval_secs(Some(-5)),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;
    runner.join_spawned_jobs().await;

    assert!(
        db.list_tasks_for_epic(epic.id).await.unwrap().is_empty(),
        "a negative interval must skip, not wrap into a huge one"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .is_err(),
        "a skipped epic must not emit a refresh"
    );
}

/// The boundary from the other side: the floor itself is a legal cadence,
/// so an epic set exactly there polls normally.
#[tokio::test]
async fn tick_polls_an_epic_whose_interval_is_exactly_the_floor() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("At Floor", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .feed_command(Some(
                    r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog","tag":"bug"}]'"#,
                ))
                .feed_interval_secs(Some(MIN_FEED_INTERVAL_SECS)),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for the refresh")
        .expect("channel closed");
    runner.join_spawned_jobs().await;

    assert_eq!(
        db.list_tasks_for_epic(epic.id).await.unwrap().len(),
        1,
        "an interval at the floor is legal and must poll"
    );
}

#[tokio::test]
async fn tick_null_feed_command_skipped() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    // Epic with no feed_command (default)
    let epic = db.create_epic("Plain Epic", "", None).await.unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    // No background task spawned — channel stays empty
    let result = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
    assert!(
        result.is_err(),
        "expected empty channel but got notification"
    );

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert!(tasks.is_empty());
}

// --- group_by_repo feed grouping tests ---

#[tokio::test]
async fn tick_grouped_creates_sub_epics_per_repo() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Dependabot", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .feed_command(Some(
                    r#"echo '[
                        {"external_id":"1","title":"A","description":"","url":"https://github.com/org/repo-a/pull/1","status":"backlog","tag":"pr-review"},
                        {"external_id":"2","title":"B","description":"","url":"https://github.com/org/repo-b/pull/1","status":"backlog","tag":"pr-review"}
                    ]'"#,
                ))
                .group_by_repo(true),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    let sub_epics = db.list_sub_epics(epic.id).await.unwrap();
    assert_eq!(sub_epics.len(), 2);
    let names: Vec<&str> = sub_epics.iter().map(|e| e.title.as_str()).collect();
    assert!(
        names.contains(&"repo-a"),
        "expected repo-a sub-epic, got {names:?}"
    );
    assert!(
        names.contains(&"repo-b"),
        "expected repo-b sub-epic, got {names:?}"
    );

    for sub in &sub_epics {
        let tasks = db.list_tasks_for_epic(sub.id).await.unwrap();
        assert_eq!(tasks.len(), 1, "sub-epic {} should have 1 task", sub.title);
    }

    let parent_tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(parent_tasks.len(), 0, "parent should have no direct tasks");
}

#[tokio::test]
async fn tick_done_epic_grouped_moves_to_backlog_when_new_feed_tasks_added() {
    // Grouped feed variant: a done parent epic should regress to backlog when
    // the feed adds new backlog tasks into a sub-epic.
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Done Grouped Epic", "", None).await.unwrap();

    // Mark the parent epic as done before the feed runs.
    db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .status(TaskStatus::Done)
                .feed_command(Some(
                    r#"echo '[{"external_id":"g1","title":"G Task","description":"","url":"https://github.com/org/repo-a/pull/1","status":"backlog","tag":"pr-review"}]'"#,
                ))
                .group_by_repo(true),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent")
        .expect("channel closed");

    // After the feed adds a new backlog task into a sub-epic, the parent
    // epic must regress to backlog.
    let refreshed = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(
        refreshed.status,
        TaskStatus::Backlog,
        "done parent epic with new grouped feed task should regress to backlog"
    );
}

#[tokio::test]
async fn tick_grouped_migrates_existing_flat_tasks() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Dependabot", "", None).await.unwrap();
    // First run: flat (group_by_repo = false by default)
    db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"1","title":"A","description":"","url":"https://github.com/org/repo-a/pull/1","status":"backlog","tag":"pr-review"}]'"#,
            )),
        )
        .await
        .unwrap();
    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out")
        .expect("closed");

    let flat_tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(
        flat_tasks.len(),
        1,
        "flat task should exist before migration"
    );

    // Enable group_by_repo and run again
    db.patch_epic(epic.id, &EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    let (mut runner2, mut rx2) = make_runner(db.clone());
    runner2.tick().await;
    tokio::time::timeout(Duration::from_secs(5), rx2.recv())
        .await
        .expect("timed out")
        .expect("closed");

    let parent_tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(parent_tasks.len(), 0, "flat task should have migrated");

    let sub_epics = db.list_sub_epics(epic.id).await.unwrap();
    assert_eq!(sub_epics.len(), 1);
    assert_eq!(sub_epics[0].title, "repo-a");
    let sub_tasks = db.list_tasks_for_epic(sub_epics[0].id).await.unwrap();
    assert_eq!(sub_tasks.len(), 1);
}

#[tokio::test]
async fn tick_grouped_uses_other_for_no_url() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Feed", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .feed_command(Some(
                    r#"echo '[{"external_id":"1","title":"X","description":"","status":"backlog","tag":"bug"}]'"#,
                ))
                .group_by_repo(true),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out")
        .expect("closed");

    let sub_epics = db.list_sub_epics(epic.id).await.unwrap();
    assert_eq!(sub_epics.len(), 1);
    assert_eq!(sub_epics[0].title, "other");
}

// --- reviews_parent role routing (WP3) ---

/// Drain all pending `EpicChanged` events, returning once the channel has
/// been quiet for the timeout window. Used by routing tests to wait for the
/// spawned reconcile(s) to finish without `tokio::time::sleep`.
async fn drain_events(rx: &mut mpsc::UnboundedReceiver<McpEvent>) {
    while tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .is_ok_and(|m| m.is_some())
    {}
}

fn role_sub(subs: &[crate::models::Epic], role: crate::models::FeedRole) -> &crate::models::Epic {
    subs.iter()
        .find(|e| e.feed_role == role)
        .unwrap_or_else(|| panic!("missing {role:?} sub-epic in {subs:?}"))
}

#[tokio::test]
async fn tick_routes_reviews_parent_into_role_sub_epics() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
            parent.id,
            &EpicPatch::new()
                .feed_role(crate::models::FeedRole::ReviewsParent)
                .feed_command(Some(
                    r#"echo '[
                        {"external_id":"pr-1","title":"Direct","description":"","url":"https://github.com/org/repo/pull/1","status":"backlog","tag":"pr-review","signals":["direct-request"]},
                        {"external_id":"pr-2","title":"Team","description":"","url":"https://github.com/org/repo/pull/2","status":"backlog","tag":"pr-review","signals":["team-request"]}
                    ]'"#,
                )),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;
    drain_events(&mut rx).await;

    let subs = db.list_sub_epics(parent.id).await.unwrap();
    let my = role_sub(&subs, crate::models::FeedRole::MyReviews);
    let team = role_sub(&subs, crate::models::FeedRole::TeamReviews);
    let bots = role_sub(&subs, crate::models::FeedRole::Bots);

    let my_tasks = db.list_tasks_for_epic(my.id).await.unwrap();
    assert_eq!(my_tasks.len(), 1, "direct-request PR routes to My Reviews");
    assert_eq!(my_tasks[0].external_id.as_deref(), Some("pr-1"));

    let team_tasks = db.list_tasks_for_epic(team.id).await.unwrap();
    assert_eq!(
        team_tasks.len(),
        1,
        "team-request PR routes to Team Reviews"
    );
    assert_eq!(team_tasks[0].external_id.as_deref(), Some("pr-2"));

    assert!(db.list_tasks_for_epic(bots.id).await.unwrap().is_empty());
    assert!(
        db.list_tasks_for_epic(parent.id).await.unwrap().is_empty(),
        "parent holds no direct feed tasks"
    );
}

/// feeds.allium SerialisedFeedCycle: a tick for an epic whose cycle is
/// already in flight is DROPPED — it must not exec, sync, or notify.
///
/// The claim is taken here in the test, which is what makes this
/// deterministic: no second cycle has to be raced into existence.
#[tokio::test]
async fn tick_skips_an_epic_whose_cycle_is_already_in_flight() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Busy Epic", "", None).await.unwrap();
    // The command would insert a task if it ever ran. It must not run.
    db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog","tag":"bug"}]'"#,
            )),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());

    // Stand in for a cycle already running for this epic.
    let _claim = runner
        .sync_guard()
        .try_claim(epic.id)
        .expect("the epic starts unclaimed");

    runner.tick().await;
    runner.join_spawned_jobs().await;

    assert!(
        tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .is_err(),
        "a dropped tick must send no notification"
    );
    assert!(
        db.list_tasks_for_epic(epic.id).await.unwrap().is_empty(),
        "a dropped tick must not run the feed command or write anything"
    );
}

/// The other half of the claim's contract: once the in-flight cycle ends,
/// the epic polls normally again. Without this, a guard that never released
/// would pass the test above and silently kill the feed.
#[tokio::test]
async fn tick_resumes_after_the_in_flight_cycle_releases() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Busy Epic", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .feed_command(Some(
                    r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog","tag":"bug"}]'"#,
                )),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());

    let claim = runner
        .sync_guard()
        .try_claim(epic.id)
        .expect("the epic starts unclaimed");
    runner.tick().await;
    runner.join_spawned_jobs().await;
    assert!(
        db.list_tasks_for_epic(epic.id).await.unwrap().is_empty(),
        "precondition: the first tick was dropped"
    );

    drop(claim);

    // The dropped tick still bumped last_run, so clear it to make the epic
    // eligible again immediately.
    force_due(&mut runner, epic.id);
    runner.tick().await;
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for the resumed poll")
        .expect("channel closed");
    assert_eq!(
        db.list_tasks_for_epic(epic.id).await.unwrap().len(),
        1,
        "releasing the claim must let the epic poll again"
    );
}

/// B3 concurrency: two back-to-back ticks must not drop the task to a
/// move/delete interleave.
///
/// Still race-free under SerialisedFeedCycle, and worth stating why: the
/// claim is taken INSIDE the spawned job, not synchronously in `tick()`, so
/// `tick` never blocks and never observes contention itself. Whichever of
/// the two spawned jobs reaches `try_claim` first wins and the loser is
/// dropped. The outcome is order-dependent; this assertion is not, which is
/// why it must not be rewritten to expect one specific arm.
#[tokio::test]
async fn tick_two_ticks_lose_nothing() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
            parent.id,
            &EpicPatch::new()
                .feed_role(crate::models::FeedRole::ReviewsParent)
                .feed_command(Some(
                    r#"echo '[{"external_id":"pr-1","title":"Team","description":"","url":"https://github.com/org/repo/pull/1","status":"backlog","tag":"pr-review","signals":["team-request"]}]'"#,
                )),
        )
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    // Both ticks must run the feed and spawn a reconcile, so clear last_run
    // between them to make the epic eligible again immediately.
    runner.tick().await;
    force_due(&mut runner, parent.id);
    runner.tick().await;
    drain_events(&mut rx).await;

    let subs = db.list_sub_epics(parent.id).await.unwrap();
    let team = role_sub(&subs, crate::models::FeedRole::TeamReviews);
    let team_tasks = db.list_tasks_for_epic(team.id).await.unwrap();
    assert_eq!(
        team_tasks.len(),
        1,
        "the PR must survive two reconciles, exactly once"
    );
    assert_eq!(team_tasks[0].external_id.as_deref(), Some("pr-1"));

    // No duplicate or orphaned feed task anywhere in the subtree.
    let total_feed: usize = {
        let mut n = 0;
        for s in &subs {
            n += db
                .list_tasks_for_epic(s.id)
                .await
                .unwrap()
                .iter()
                .filter(|t| t.external_id.is_some())
                .count();
        }
        n
    };
    assert_eq!(total_feed, 1, "exactly one feed task across the subtree");
}

/// The loop `start()` spawns really does poll and run the feed command.
///
/// This is deliberately the whole of `start()`'s coverage: `start()` is a
/// synchronous `fn` whose body is a bare `tokio::spawn`, so "does not await
/// the poll loop" is a type-level property with no runtime signal to assert
/// on. See "No `tokio::time::sleep` in tests" in docs/conventions.md.
#[tokio::test]
async fn start_background_task_eventually_runs_feed_command() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("BG Feed Epic", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"bg1","title":"BG Task","description":"","status":"backlog","tag":"bug"}]'"#,
            )),
        ).await
        .unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let proc_runner: Arc<dyn ProcessRunner> =
        Arc::new(crate::process::MockProcessRunner::new(vec![]));
    let board_reads = db.board_reads().expect("memory handle has board reads");
    let runner = FeedRunner::new(
        Arc::clone(&db) as Arc<dyn crate::store::TaskStore>,
        tx,
        proc_runner,
        board_reads,
        "test-host".into(),
    );
    runner.start();

    // The tokio interval fires on the first tick almost immediately; await
    // the EpicChanged event the background task emits after upserting.
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "background task should have upserted one feed task"
    );
    assert_eq!(tasks[0].title, "BG Task");
    assert_eq!(tasks[0].external_id.as_deref(), Some("bg1"));
}

// --- repo_path resolution via URL ---

#[tokio::test]
async fn tick_github_url_resolves_to_known_repo_path() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    // Register a known repo path matching "myrepo"
    db.save_repo_path("/home/user/code/myrepo").await.unwrap();
    let epic = db.create_epic("Feed Epic", "", None).await.unwrap();
    let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://github.com/org/myrepo/pull/42","status":"backlog","tag":"bug"}]'"#;
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    // Await the background upsert deterministically.
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].repo_path, "/home/user/code/myrepo",
        "repo_path should be resolved from GitHub URL"
    );
}

#[tokio::test]
async fn tick_no_matching_repo_stores_empty_sentinel() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    // Known repo is "other-repo", not matching "myrepo"
    db.save_repo_path("/home/user/code/other-repo")
        .await
        .unwrap();
    let epic = db.create_epic("Feed Epic", "", None).await.unwrap();
    let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://github.com/org/myrepo/pull/42","status":"backlog","tag":"bug"}]'"#;
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].repo_path, "",
        "unresolved URL should store empty sentinel"
    );
}

#[tokio::test]
async fn tick_empty_url_stores_empty_sentinel() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    db.save_repo_path("/home/user/code/myrepo").await.unwrap();
    let epic = db.create_epic("Feed Epic", "", None).await.unwrap();
    let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog","tag":"bug"}]'"#;
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].repo_path, "",
        "empty url should store empty sentinel"
    );
}

/// A `ProcessRunner` that returns a fixed `origin/HEAD` per repo path,
/// counting how many times each path was queried.
struct PerRepoBranchRunner {
    branches: HashMap<String, String>,
    calls: std::sync::Mutex<HashMap<String, usize>>,
}

impl PerRepoBranchRunner {
    fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            branches: pairs
                .iter()
                .map(|(p, b)| (p.to_string(), b.to_string()))
                .collect(),
            calls: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn calls_for(&self, path: &str) -> usize {
        self.calls
            .lock()
            .expect("feed lock poisoned")
            .get(path)
            .copied()
            .unwrap_or(0)
    }
}

impl ProcessRunner for PerRepoBranchRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<std::process::Output> {
        assert_eq!(program, "git");
        // args = ["-C", <path>, "symbolic-ref", "refs/remotes/origin/HEAD"]
        let path = args.get(1).copied().unwrap_or("");
        *self
            .calls
            .lock()
            .unwrap()
            .entry(path.to_string())
            .or_insert(0) += 1;
        match self.branches.get(path) {
            Some(branch) => crate::process::MockProcessRunner::ok_with_stdout(
                format!("refs/remotes/origin/{branch}\n").as_bytes(),
            ),
            None => crate::process::MockProcessRunner::fail("unknown repo"),
        }
    }
}

#[tokio::test]
async fn tick_resolves_default_branch_per_unique_repo() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    db.save_repo_path("/home/user/code/repo-a").await.unwrap();
    db.save_repo_path("/home/user/code/repo-b").await.unwrap();
    let epic = db.create_epic("Feed Epic", "", None).await.unwrap();
    // Three items: two for repo-a (master), one for repo-b (develop).
    let cmd = r#"echo '[
            {"external_id":"1","title":"A1","description":"","url":"https://github.com/org/repo-a/pull/1","status":"backlog","tag":"bug"},
            {"external_id":"2","title":"A2","description":"","url":"https://github.com/org/repo-a/pull/2","status":"backlog","tag":"bug"},
            {"external_id":"3","title":"B1","description":"","url":"https://github.com/org/repo-b/pull/1","status":"backlog","tag":"bug"}
        ]'"#;
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
        .await
        .unwrap();

    let proc_runner = Arc::new(PerRepoBranchRunner::new(&[
        ("/home/user/code/repo-a", "master"),
        ("/home/user/code/repo-b", "develop"),
    ]));
    let (mut runner, mut rx) = make_runner_with_runner(db.clone(), proc_runner.clone());
    runner.tick().await;

    // Await the spawned task finishing its writes deterministically.
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 3);

    let by_ext = |ext: &str| {
        tasks
            .iter()
            .find(|t| t.external_id.as_deref() == Some(ext))
            .unwrap()
    };
    assert_eq!(by_ext("1").base_branch, "master");
    assert_eq!(by_ext("2").base_branch, "master");
    assert_eq!(by_ext("3").base_branch, "develop");

    // Cache check: each unique repo should have been queried exactly once.
    assert_eq!(
        proc_runner.calls_for("/home/user/code/repo-a"),
        1,
        "repo-a default branch should be resolved once, not per-item"
    );
    assert_eq!(proc_runner.calls_for("/home/user/code/repo-b"), 1);
}

#[tokio::test]
async fn tick_falls_back_to_main_when_origin_head_missing() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    db.save_repo_path("/home/user/code/repo-a").await.unwrap();
    let epic = db.create_epic("Feed Epic", "", None).await.unwrap();
    let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://github.com/org/repo-a/pull/1","status":"backlog","tag":"bug"}]'"#;
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
        .await
        .unwrap();

    // AlwaysFailRunner → detect_default_branch returns "main".
    let (mut runner, mut rx) = make_runner_with_runner(db.clone(), Arc::new(AlwaysFailRunner));
    runner.tick().await;
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].base_branch, "main");
}

#[tokio::test]
async fn tick_twice_is_idempotent() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Idem Epic", "", None).await.unwrap();
    db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .feed_command(Some(
                    r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog","tag":"bug"}]'"#,
                )),
        ).await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());

    runner.tick().await;
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("first tick: timed out waiting for refresh")
        .expect("channel closed");

    let first = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(first.len(), 1);
    let first_id = first[0].id;

    // Clear last_run so the second tick re-runs the command.
    force_due(&mut runner, epic.id);
    runner.tick().await;
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("second tick: timed out waiting for refresh")
        .expect("channel closed");

    let second = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(
        second.len(),
        1,
        "running the same feed twice must not duplicate tasks"
    );
    assert_eq!(
        second[0].id, first_id,
        "task id must be stable across upserts"
    );
    assert_eq!(second[0].external_id.as_deref(), Some("1"));
}

#[tokio::test]
async fn tick_empty_array_creates_no_tasks() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Empty Epic", "", None).await.unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("echo '[]'")))
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    let _ = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert!(tasks.is_empty(), "empty feed array must not create tasks");
}

// --- cache / EpicChanged invalidation tests ---

#[tokio::test]
async fn tick_sets_cache_to_false_when_no_feed_commands() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    db.create_epic("Plain Epic", "", None).await.unwrap();

    let (mut runner, _rx) = make_runner(db.clone());
    runner.tick().await;

    assert_eq!(
        runner.any_feed_cmds,
        Some(false),
        "cache should be Some(false) after tick with no feed commands"
    );
}

#[tokio::test]
async fn tick_sets_cache_to_true_when_feed_command_exists() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Feed Epic", "", None).await.unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("echo '[]'")))
        .await
        .unwrap();

    let (mut runner, _rx) = make_runner(db.clone());
    runner.tick().await;

    assert_eq!(
        runner.any_feed_cmds,
        Some(true),
        "cache should be Some(true) when at least one epic has a feed command"
    );
}

#[tokio::test]
async fn tick_skips_db_queries_when_cache_is_false_and_no_invalidation() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    db.create_epic("Plain Epic", "", None).await.unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());

    // First tick: no feed commands → cache = Some(false)
    runner.tick().await;
    assert_eq!(runner.any_feed_cmds, Some(false));

    // Add a feed command directly to the DB (simulates MCP update, no EpicChanged signal)
    let epic2 = db.create_epic("Feed Epic", "", None).await.unwrap();
    let cmd = r#"echo '[{"external_id":"c1","title":"C","description":"","status":"backlog","tag":"bug"}]'"#;
    db.patch_epic(epic2.id, &EpicPatch::new().feed_command(Some(cmd)))
        .await
        .unwrap();

    // Second tick: cache is Some(false) → body skipped → task not created
    runner.tick().await;

    let result = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
    assert!(
        result.is_err(),
        "tick should skip body when cache is Some(false)"
    );
    let tasks = db.list_tasks_for_epic(epic2.id).await.unwrap();
    assert!(
        tasks.is_empty(),
        "no task should be created while cache prevents DB query"
    );
}

#[tokio::test]
async fn tick_re_queries_after_epic_changed_invalidation() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    db.create_epic("Plain Epic", "", None).await.unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());

    // First tick: no feed commands → cache = Some(false)
    runner.tick().await;
    assert_eq!(runner.any_feed_cmds, Some(false));

    // Add a feed command and then invalidate the cache via the watch sender
    let epic2 = db.create_epic("Feed Epic", "", None).await.unwrap();
    let cmd = r#"echo '[{"external_id":"r1","title":"R","description":"","status":"backlog","tag":"bug"}]'"#;
    db.patch_epic(epic2.id, &EpicPatch::new().feed_command(Some(cmd)))
        .await
        .unwrap();
    runner.epic_invalidate_tx().send(()).ok();

    // Third tick: cache invalidated → re-queries → processes feed command
    runner.tick().await;
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent after cache invalidation")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic2.id).await.unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "task should be created after cache invalidation"
    );
}

#[tokio::test]
async fn tick_non_github_url_stores_empty_sentinel() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    db.save_repo_path("/home/user/code/myrepo").await.unwrap();
    let epic = db.create_epic("Feed Epic", "", None).await.unwrap();
    let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://jira.company.com/PROJ-123","status":"backlog","tag":"bug"}]'"#;
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
        .await
        .unwrap();

    let (mut runner, mut rx) = make_runner(db.clone());
    runner.tick().await;

    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent")
        .expect("channel closed");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].repo_path, "",
        "non-github url should store empty sentinel"
    );
}

// --- cleanup_removed_feed_tasks ---

use std::collections::HashSet;
use std::sync::{Condvar, Mutex};

use crate::models::TaskId;
use crate::process::MockProcessRunner;
use crate::store::{RemovedFeedTask, TaskPatch};

fn removed_task(
    id: i64,
    repo: &str,
    worktree: Option<&str>,
    window: Option<&str>,
) -> RemovedFeedTask {
    RemovedFeedTask {
        id: TaskId(id),
        repo_path: repo.to_string(),
        worktree: worktree.map(str::to_string),
        tmux_window: window.map(test_tmux_window),
    }
}

#[tokio::test]
async fn cleanup_removes_worktree_and_kills_window() {
    let runner = Arc::new(MockProcessRunner::new(vec![
        // has_window: list-windows names the window, so the kill proceeds
        MockProcessRunner::ok_with_stdout(b"dispatch:pr-1\n"),
        MockProcessRunner::ok(), // tmux kill-window
        MockProcessRunner::ok(), // git worktree remove
        MockProcessRunner::ok(), // git branch -D (best effort)
    ]));

    cleanup_removed_feed_tasks(
        runner.clone(),
        vec![removed_task(
            1,
            "/repo/a",
            Some("/repo/a/.worktrees/pr-1"),
            Some("dispatch:pr-1"),
        )],
    )
    .await;

    let calls = runner.flattened_calls();
    assert!(
        calls
            .iter()
            .any(|c| c.contains("worktree remove") && c.contains("/repo/a/.worktrees/pr-1")),
        "must remove the worktree, got: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("kill-window")),
        "must kill the tmux window, got: {calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("branch -D")),
        "must best-effort delete the branch, got: {calls:?}"
    );
}

// This module's copy of the shared-worktree tripwire
// (cleanup_removes_the_worktree_even_if_another_row_names_it) went with #4096's
// unification; the survivor is
// `src/runtime/tests.rs::exec_cleanup_tears_down_even_if_another_row_names_the_worktree`.

#[tokio::test]
async fn cleanup_kills_window_only_when_there_is_no_worktree() {
    let runner = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"dispatch:pr-1\n"),
        MockProcessRunner::ok(), // tmux kill-window
    ]));

    cleanup_removed_feed_tasks(
        runner.clone(),
        vec![removed_task(7, "/repo/a", None, Some("dispatch:pr-1"))],
    )
    .await;

    let calls = runner.flattened_calls();
    assert!(
        calls.iter().any(|c| c.contains("kill-window")),
        "the window must be killed, got: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.starts_with("git ")),
        "with no worktree there is nothing for git to do, got: {calls:?}"
    );
}

#[tokio::test]
async fn cleanup_of_a_stateless_row_runs_no_commands() {
    // An empty response queue panics on the first shell-out — but that panic
    // happens inside `spawn_blocking` and `cleanup_removed_feed_tasks` only
    // *logs* the resulting `JoinError`, so queue exhaustion alone cannot fail
    // this test. The explicit negative below is what carries the assertion.
    let runner = Arc::new(MockProcessRunner::new(vec![]));

    cleanup_removed_feed_tasks(runner.clone(), vec![removed_task(8, "/repo/a", None, None)]).await;

    let calls = runner.flattened_calls();
    assert!(
        calls.is_empty(),
        "a row with neither worktree nor window must shell out to nothing, got: {calls:?}"
    );
}

/// One task's failure must not abort the rest of its repo's queue.
#[tokio::test]
async fn cleanup_continues_after_a_failure() {
    let runner = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: could not lock index"), // pr-1 remove
        MockProcessRunner::ok(),                                // pr-2 remove
        MockProcessRunner::ok(),                                // pr-2 branch -D
    ]));

    cleanup_removed_feed_tasks(
        runner.clone(),
        vec![
            removed_task(1, "/repo/a", Some("/repo/a/.worktrees/pr-1"), None),
            removed_task(2, "/repo/a", Some("/repo/a/.worktrees/pr-2"), None),
        ],
    )
    .await;

    let calls = runner.flattened_calls();
    assert!(
        calls.iter().any(|c| c.contains("/repo/a/.worktrees/pr-2")),
        "pr-2 must still be torn down after pr-1 failed, got: {calls:?}"
    );
}

/// How long [`OverlapRunner`] holds the first call of each repo open, giving
/// a concurrently-issued sibling call the chance to be seen. Generous: the
/// correct implementation cannot end the wait early, so this is the test's
/// floor, while a wrong implementation ends it in microseconds.
const GATE_WINDOW: Duration = Duration::from_millis(500);

/// A `ProcessRunner` decorator that observes call *overlap* rather than call
/// order — order alone cannot distinguish "serialised" from "concurrent but
/// happened to finish in order".
///
/// For every `git -C <repo>` call it records whether another call for the
/// same repo — or for a different repo — was in flight at that moment. To
/// make an overlap observable at all it holds the FIRST call of each repo
/// open for [`GATE_WINDOW`], releasing early the moment a same-repo overlap
/// is seen. This is a bounded wait on a *signal*, not a wall-clock sleep:
/// what the test asserts on is whether the signal arrived.
///
/// The two flags it collects are the two halves of the design requirement,
/// and neither can be produced by scheduling luck:
///
/// * a per-repo-sequential implementation can NEVER show a same-repo
///   overlap — the held call occupies the one thread that would issue the
///   sibling call, so no amount of scheduling produces one;
/// * with both repos held open concurrently for the same window, an
///   all-serialised implementation can NEVER show a cross-repo overlap —
///   the second repo cannot reach the runner while the first is held.
struct OverlapRunner {
    inner: MockProcessRunner,
    state: Mutex<OverlapState>,
    wake: Condvar,
}

#[derive(Default)]
struct OverlapState {
    in_flight: HashMap<String, usize>,
    gated: HashSet<String>,
    same_repo_overlap: bool,
    cross_repo_overlap: bool,
}

impl OverlapRunner {
    fn new(responses: Vec<anyhow::Result<std::process::Output>>) -> Self {
        Self {
            inner: MockProcessRunner::new(responses),
            state: Mutex::new(OverlapState::default()),
            wake: Condvar::new(),
        }
    }

    /// The repo a `git -C <repo> …` call targets. `None` for anything else.
    fn repo_of(program: &str, args: &[&str]) -> Option<String> {
        if program != "git" {
            return None;
        }
        let i = args.iter().position(|a| *a == "-C")?;
        args.get(i + 1).map(|r| (*r).to_string())
    }
}

impl ProcessRunner for OverlapRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<std::process::Output> {
        let Some(repo) = Self::repo_of(program, args) else {
            return self.inner.run(program, args);
        };

        {
            let mut state = self.state.lock().unwrap();
            let others: usize = state
                .in_flight
                .iter()
                .filter(|(other, _)| *other != &repo)
                .map(|(_, n)| *n)
                .sum();
            if state.in_flight.get(&repo).copied().unwrap_or(0) > 0 {
                state.same_repo_overlap = true;
            }
            if others > 0 {
                state.cross_repo_overlap = true;
            }
            *state.in_flight.entry(repo.clone()).or_default() += 1;
            let gate = state.gated.insert(repo.clone());
            self.wake.notify_all();
            if gate {
                let _unused = self
                    .wake
                    .wait_timeout_while(state, GATE_WINDOW, |state| !state.same_repo_overlap)
                    .unwrap();
            }
        }

        let out = self.inner.run(program, args);
        *self
            .state
            .lock()
            .unwrap()
            .in_flight
            .entry(repo)
            .or_default() -= 1;
        out
    }
}

// Two removals in the same repo must not run git concurrently — git locks
// the repo's worktree metadata and index. Removals in *different* repos
// still may.
#[tokio::test]
async fn cleanup_serialises_same_repo_removals() {
    // Every response is an identical success: with two repos in flight the
    // pop order is not deterministic, so nothing may depend on it.
    let runner = Arc::new(OverlapRunner::new(
        (0..6).map(|_| MockProcessRunner::ok()).collect(),
    ));

    cleanup_removed_feed_tasks(
        runner.clone(),
        vec![
            removed_task(1, "/repo/a", Some("/repo/a/.worktrees/pr-1"), None),
            removed_task(2, "/repo/a", Some("/repo/a/.worktrees/pr-2"), None),
            removed_task(3, "/repo/b", Some("/repo/b/.worktrees/pr-3"), None),
        ],
    )
    .await;

    let calls = runner.inner.flattened_calls();
    for wanted in ["pr-1", "pr-2", "pr-3"] {
        assert!(
            calls.iter().any(|c| c.contains(wanted)),
            "{wanted} must have been torn down, got: {calls:?}"
        );
    }

    let state = runner.state.lock().unwrap();
    assert!(
        !state.same_repo_overlap,
        "two removals in one repo must never have git calls in flight together"
    );
    assert!(
        state.cross_repo_overlap,
        "removals in different repos must proceed in parallel"
    );

    // Belt and braces on ordering: within /repo/a, pr-1 is fully handled
    // before pr-2 starts.
    let pr1 = calls.iter().rposition(|c| c.contains("pr-1")).unwrap();
    let pr2 = calls.iter().position(|c| c.contains("pr-2")).unwrap();
    assert!(
        pr1 < pr2,
        "pr-1's git calls must all precede pr-2's, got: {calls:?}"
    );
}
