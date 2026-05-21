mod exec;
mod ingest;
mod parse;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::db::TaskStore;
use crate::dispatch::resolve_feed_item_repo_paths;
use crate::mcp::McpEvent;
use crate::models::EpicId;
use crate::process::ProcessRunner;

pub(crate) use exec::resolve_base_branches;

const DEFAULT_FEED_INTERVAL: Duration = Duration::from_secs(30);

/// Poll interval for the background feed task.
/// Kept in `feed` (not reusing `TICK_INTERVAL` from `runtime`) so the two
/// concerns stay independent.
const FEED_POLL_INTERVAL: Duration = Duration::from_secs(2);

pub struct FeedRunner {
    db: Arc<dyn TaskStore>,
    notify: mpsc::UnboundedSender<McpEvent>,
    runner: Arc<dyn ProcessRunner>,
    last_run: HashMap<EpicId, Instant>,
}

impl FeedRunner {
    pub fn new(
        db: Arc<dyn TaskStore>,
        notify: mpsc::UnboundedSender<McpEvent>,
        runner: Arc<dyn ProcessRunner>,
    ) -> Self {
        Self {
            db,
            notify,
            runner,
            last_run: HashMap::new(),
        }
    }

    /// Spawns as an independent background task so slow feed commands can't freeze the UI.
    pub fn start(self) {
        tokio::spawn(async move {
            let mut runner = self;
            let mut interval = tokio::time::interval(FEED_POLL_INTERVAL);
            loop {
                interval.tick().await;
                runner.tick().await;
            }
        });
    }

    pub async fn tick(&mut self) {
        let epics = match self.db.list_epics().await {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("FeedRunner: failed to list epics: {err:#}");
                return;
            }
        };

        let active_ids: std::collections::HashSet<EpicId> = epics.iter().map(|e| e.id).collect();
        self.last_run.retain(|id, _| active_ids.contains(id));

        // Fetch once per tick so N concurrent spawned tasks don't each hit the DB.
        let known_paths = match self.db.list_repo_paths().await {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!("FeedRunner: failed to list repo_paths, using empty sentinel: {err:#}");
                vec![]
            }
        };

        for epic in epics {
            let Some(ref cmd) = epic.feed_command else {
                continue;
            };

            let interval = epic
                .feed_interval_secs
                .map(|s| Duration::from_secs(s as u64))
                .unwrap_or(DEFAULT_FEED_INTERVAL);

            let elapsed = self
                .last_run
                .get(&epic.id)
                .map(|t| t.elapsed())
                .unwrap_or(Duration::MAX);

            if elapsed < interval {
                continue;
            }

            self.last_run.insert(epic.id, Instant::now());

            let db = self.db.clone();
            let notify = self.notify.clone();
            let runner = self.runner.clone();
            let cmd = cmd.clone();
            let epic_id = epic.id;
            let epic_title = epic.title.clone();
            let epic_group_by_repo = epic.group_by_repo;
            let epic_project_id = epic.project_id;
            let known_paths = known_paths.clone();

            tokio::task::spawn(async move {
                let Some(stdout) =
                    exec::exec_feed_command(&cmd, epic_id.0, &epic_title).await
                else {
                    return;
                };

                let items = match parse::parse_feed_items(&stdout) {
                    Ok(i) => i,
                    Err(err) => {
                        tracing::warn!(
                            epic_id = epic_id.0,
                            epic_title = %epic_title,
                            "FeedRunner: failed to parse JSON output: {err:#}"
                        );
                        return;
                    }
                };

                let repo_paths = resolve_feed_item_repo_paths(&items, &known_paths);
                let base_branches = resolve_base_branches(&repo_paths, &*runner);

                if epic_group_by_repo {
                    let sub_ids = ingest::sync_grouped_feed(
                        &*db,
                        epic_id,
                        epic_project_id,
                        &items,
                        &repo_paths,
                        &base_branches,
                    )
                    .await;
                    // Parent's flat task list was cleared — notify so the TUI
                    // reflects the empty list even when no sub-epics changed.
                    let _ = notify.send(McpEvent::EpicChanged(epic_id));
                    for sub_id in sub_ids {
                        let _ = notify.send(McpEvent::EpicChanged(sub_id));
                    }
                } else {
                    match db
                        .upsert_feed_tasks(epic_id, &items, &repo_paths, &base_branches)
                        .await
                    {
                        Ok(()) => {
                            // One targeted event per sync batch — the runtime reloads
                            // the epic and its tasks in a single splice.
                            let _ = notify.send(McpEvent::EpicChanged(epic_id));
                        }
                        Err(err) => {
                            tracing::warn!(
                                epic_id = epic_id.0,
                                "FeedRunner: upsert_feed_tasks failed: {err:#}"
                            );
                        }
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::Arc;

    use super::*;
    use crate::db::{Database, EpicCrud, EpicPatch, SettingsStore};
    use crate::models::{ProjectId, TaskTag};

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
        (FeedRunner::new(db, tx, runner), rx)
    }

    #[tokio::test]
    async fn tick_does_not_block_event_loop() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db
            .create_epic("Slow Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("sleep 5")))
            .await
            .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());

        let start = std::time::Instant::now();
        runner.tick().await;
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(500),
            "tick() blocked for {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn tick_background_task_upserts_tasks() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db
            .create_epic("BG Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
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
    async fn tick_valid_json_upserts_tasks() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db
            .create_epic("My Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
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

    #[tokio::test]
    async fn tick_persists_feed_tag() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db
            .create_epic("Tagged Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
        db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog","tag":"pr-review"}]'"#,
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
        let epic = db
            .create_epic("Untagged Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
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
        let epic = db
            .create_epic("Err Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
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
        let epic = db
            .create_epic("Bad JSON Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
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
        let epic = db
            .create_epic("Interval Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();

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

    #[tokio::test]
    async fn tick_null_feed_command_skipped() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        // Epic with no feed_command (default)
        let epic = db
            .create_epic("Plain Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();

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
        let epic = db
            .create_epic("Dependabot", "", "", None, ProjectId(1))
            .await
            .unwrap();
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
    async fn tick_grouped_migrates_existing_flat_tasks() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db
            .create_epic("Dependabot", "", "", None, ProjectId(1))
            .await
            .unwrap();
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
        let epic = db
            .create_epic("Feed", "", "", None, ProjectId(1))
            .await
            .unwrap();
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

    #[tokio::test]
    async fn tick_grouped_creates_fresh_sub_epic_when_existing_one_is_archived() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let feed_cmd = r#"echo '[{"external_id":"1","title":"A","description":"","url":"https://github.com/org/repo-a/pull/1","status":"backlog","tag":"pr-review"}]'"#;

        let epic = db
            .create_epic("Reviews", "", "", None, ProjectId(1))
            .await
            .unwrap();
        db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .feed_command(Some(feed_cmd))
                .group_by_repo(true),
        )
        .await
        .unwrap();

        // First run: creates sub-epic for repo-a
        let (mut runner, mut rx) = make_runner(db.clone());
        runner.tick().await;
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        let sub_epics = db.list_sub_epics(epic.id).await.unwrap();
        assert_eq!(sub_epics.len(), 1);
        let archived_id = sub_epics[0].id;

        // User archives the sub-epic
        db.patch_epic(
            archived_id,
            &EpicPatch::new().status(crate::models::TaskStatus::Archived),
        )
        .await
        .unwrap();

        // Second run: must create a NEW active sub-epic, not reuse the archived one
        let (mut runner2, mut rx2) = make_runner(db.clone());
        runner2.tick().await;
        tokio::time::timeout(Duration::from_secs(5), rx2.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        let all_sub_epics = db.list_sub_epics(epic.id).await.unwrap();
        let active: Vec<_> = all_sub_epics
            .iter()
            .filter(|e| e.status != crate::models::TaskStatus::Archived)
            .collect();
        assert_eq!(
            active.len(),
            1,
            "expected a fresh active sub-epic after archiving; got sub-epics: {:?}",
            all_sub_epics
                .iter()
                .map(|e| (&e.title, &e.status))
                .collect::<Vec<_>>()
        );
        assert_eq!(active[0].title, "repo-a");
        assert_ne!(
            active[0].id, archived_id,
            "must be a new sub-epic, not the archived one"
        );
        let tasks = db.list_tasks_for_epic(active[0].id).await.unwrap();
        assert_eq!(tasks.len(), 1, "new sub-epic should have the feed task");
    }

    #[tokio::test]
    async fn start_returns_immediately_without_blocking() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db
            .create_epic("Slow Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
        // A command that would block for 5 seconds if awaited inline.
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("sleep 5")))
            .await
            .unwrap();

        let (tx, _rx) = mpsc::unbounded_channel();
        let proc_runner: Arc<dyn ProcessRunner> =
            Arc::new(crate::process::MockProcessRunner::new(vec![]));
        let runner = FeedRunner::new(db as Arc<dyn crate::db::TaskStore>, tx, proc_runner);

        let before = std::time::Instant::now();
        runner.start(); // must return immediately — it just spawns a task
        let elapsed = before.elapsed();

        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "start() took {elapsed:?}, expected <50ms — it must not block"
        );
    }

    #[tokio::test]
    async fn start_background_task_eventually_runs_feed_command() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db
            .create_epic("BG Feed Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
        db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"bg1","title":"BG Task","description":"","status":"backlog","tag":"bug"}]'"#,
            )),
        ).await
        .unwrap();

        let (tx, _rx) = mpsc::unbounded_channel();
        let proc_runner: Arc<dyn ProcessRunner> =
            Arc::new(crate::process::MockProcessRunner::new(vec![]));
        let runner = FeedRunner::new(
            Arc::clone(&db) as Arc<dyn crate::db::TaskStore>,
            tx,
            proc_runner,
        );
        runner.start();

        // The tokio interval fires on the first tick almost immediately.
        // Give the background task 500 ms to complete.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

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
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .await
            .unwrap();
        let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://github.com/org/myrepo/pull/42","status":"backlog","tag":"bug"}]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .await
            .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await;

        // Wait briefly for the spawned task to complete
        tokio::time::sleep(Duration::from_millis(200)).await;

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
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .await
            .unwrap();
        let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://github.com/org/myrepo/pull/42","status":"backlog","tag":"bug"}]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .await
            .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await;

        tokio::time::sleep(Duration::from_millis(200)).await;

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
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .await
            .unwrap();
        let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog","tag":"bug"}]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .await
            .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await;

        tokio::time::sleep(Duration::from_millis(200)).await;

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
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .await
            .unwrap();
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
        let (mut runner, _rx) = make_runner_with_runner(db.clone(), proc_runner.clone());
        runner.tick().await;

        // Wait for the spawned task to finish writing tasks.
        tokio::time::sleep(Duration::from_millis(300)).await;

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
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .await
            .unwrap();
        let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://github.com/org/repo-a/pull/1","status":"backlog","tag":"bug"}]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .await
            .unwrap();

        // AlwaysFailRunner → detect_default_branch returns "main".
        let (mut runner, _rx) = make_runner_with_runner(db.clone(), Arc::new(AlwaysFailRunner));
        runner.tick().await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].base_branch, "main");
    }

    #[tokio::test]
    async fn tick_twice_is_idempotent() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db
            .create_epic("Idem Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
        db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .feed_command(Some(
                    r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog","tag":"bug"}]'"#,
                ))
                // 0-second interval so the second tick re-runs the command.
                .feed_interval_secs(Some(0)),
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
        let epic = db
            .create_epic("Empty Epic", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("echo '[]'")))
            .await
            .unwrap();

        let (mut runner, mut rx) = make_runner(db.clone());
        runner.tick().await;

        let _ = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;

        let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
        assert!(tasks.is_empty(), "empty feed array must not create tasks");
    }

    #[tokio::test]
    async fn tick_non_github_url_stores_empty_sentinel() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        db.save_repo_path("/home/user/code/myrepo").await.unwrap();
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .await
            .unwrap();
        let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://jira.company.com/PROJ-123","status":"backlog","tag":"bug"}]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .await
            .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await;

        tokio::time::sleep(Duration::from_millis(200)).await;

        let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].repo_path, "",
            "non-github url should store empty sentinel"
        );
    }
}
