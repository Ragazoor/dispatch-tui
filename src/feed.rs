use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::db::TaskStore;
use crate::dispatch::resolve_feed_item_repo_paths;
use crate::git::detect_default_branch;
use crate::mcp::McpEvent;
#[cfg(test)]
use crate::models::ProjectId;
use crate::models::{EpicId, FeedItem};
use crate::process::ProcessRunner;

const DEFAULT_FEED_INTERVAL: Duration = Duration::from_secs(30);

/// Resolve a base branch for each `repo_paths[i]`, caching by unique path so
/// `git symbolic-ref` is invoked at most once per distinct repo. Empty paths
/// (unresolved repos) get `"main"` without shelling out.
pub(crate) fn resolve_base_branches(
    repo_paths: &[String],
    runner: &dyn ProcessRunner,
) -> Vec<String> {
    let mut cache: HashMap<&str, String> = HashMap::new();
    repo_paths
        .iter()
        .map(|path| {
            cache
                .entry(path.as_str())
                .or_insert_with(|| {
                    if path.is_empty() {
                        "main".to_string()
                    } else {
                        detect_default_branch(path, runner)
                    }
                })
                .clone()
        })
        .collect()
}

/// Poll interval for the background feed task.
/// Kept in `feed.rs` (not reusing `TICK_INTERVAL` from `runtime`) so the two
/// concerns stay independent.
const FEED_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Group feed items by repo name and upsert each group into its own sub-epic.
/// Clears any flat feed tasks on the parent epic (migration + ongoing hygiene).
/// Returns `Some(sub_epic_ids)` on full success (one id per sub-epic written to),
/// or `None` on any DB error.
async fn sync_grouped_feed(
    db: &dyn crate::db::TaskStore,
    parent_id: crate::models::EpicId,
    project_id: crate::models::ProjectId,
    items: &[crate::models::FeedItem],
    repo_paths: &[String],
    base_branches: &[String],
) -> Option<Vec<crate::models::EpicId>> {
    use std::collections::HashMap;

    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        let name = crate::dispatch::repo_name_from_url(&item.url);
        groups.entry(name).or_default().push(i);
    }

    let existing_sub_epics = match db.list_sub_epics(parent_id).await {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!(
                epic_id = parent_id.0,
                "sync_grouped_feed: list_sub_epics failed: {err:#}"
            );
            return None;
        }
    };

    let mut all_ok = true;
    let mut sub_epic_ids: Vec<crate::models::EpicId> = Vec::new();

    for (repo_name, indices) in &groups {
        let group_items: Vec<crate::models::FeedItem> =
            indices.iter().map(|&i| items[i].clone()).collect();
        let group_repo_paths: Vec<String> = indices
            .iter()
            .map(|&i| repo_paths.get(i).cloned().unwrap_or_default())
            .collect();
        let group_base_branches: Vec<String> = indices
            .iter()
            .map(|&i| base_branches.get(i).cloned().unwrap_or_default())
            .collect();

        let sub_epic_id =
            if let Some(existing) = existing_sub_epics.iter().find(|e| e.title == *repo_name) {
                existing.id
            } else {
                match db
                    .create_epic(repo_name, "", "", Some(parent_id), project_id)
                    .await
                {
                    Ok(e) => e.id,
                    Err(err) => {
                        tracing::warn!(
                            epic_id = parent_id.0,
                            repo = %repo_name,
                            "sync_grouped_feed: create_epic failed: {err:#}"
                        );
                        all_ok = false;
                        continue;
                    }
                }
            };

        if let Err(err) = db
            .upsert_feed_tasks(
                sub_epic_id,
                &group_items,
                &group_repo_paths,
                &group_base_branches,
            )
            .await
        {
            tracing::warn!(
                epic_id = parent_id.0,
                sub_epic_id = sub_epic_id.0,
                "sync_grouped_feed: upsert_feed_tasks failed: {err:#}"
            );
            all_ok = false;
        } else {
            sub_epic_ids.push(sub_epic_id);
        }
    }

    // Always clear flat feed tasks from parent, regardless of per-group failures
    if let Err(err) = db.upsert_feed_tasks(parent_id, &[], &[], &[]).await {
        tracing::warn!(
            epic_id = parent_id.0,
            "sync_grouped_feed: failed to clear parent feed tasks: {err:#}"
        );
        all_ok = false;
    }

    if all_ok { Some(sub_epic_ids) } else { None }
}

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

            tokio::task::spawn(async move {
                let output = match tokio::process::Command::new("sh")
                    .args(["-c", &cmd])
                    .output()
                    .await
                {
                    Ok(o) => o,
                    Err(err) => {
                        tracing::warn!(
                            epic_id = epic_id.0,
                            epic_title = %epic_title,
                            "FeedRunner: failed to spawn command: {err:#}"
                        );
                        return;
                    }
                };

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::warn!(
                        epic_id = epic_id.0,
                        epic_title = %epic_title,
                        "FeedRunner: command exited non-zero: {stderr}"
                    );
                    return;
                }

                let items: Vec<FeedItem> =
                    match serde_json::from_slice::<Vec<FeedItem>>(&output.stdout) {
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

                let known_paths = match db.list_repo_paths().await {
                    Ok(p) => p,
                    Err(err) => {
                        tracing::warn!(
                            epic_id = epic_id.0,
                            "FeedRunner: failed to list repo_paths, using empty sentinel: {err:#}"
                        );
                        vec![]
                    }
                };
                let repo_paths = resolve_feed_item_repo_paths(&items, &known_paths);
                let base_branches = resolve_base_branches(&repo_paths, &*runner);

                if epic_group_by_repo {
                    if let Some(sub_ids) = sync_grouped_feed(
                        &*db,
                        epic_id,
                        epic_project_id,
                        &items,
                        &repo_paths,
                        &base_branches,
                    )
                    .await
                    {
                        let _ = notify.send(McpEvent::EpicChanged(epic_id)); // parent
                        for sub_id in sub_ids {
                            let _ = notify.send(McpEvent::EpicChanged(sub_id));
                        }
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
    use super::*;
    use crate::db::{Database, EpicCrud, EpicPatch, SettingsStore};
    use std::sync::Arc;

    // --- FeedRunner tests ---

    /// A `ProcessRunner` that always returns a non-zero exit. Used in feed
    /// tests that don't care about default-branch resolution — every
    /// `git symbolic-ref` call falls back to `"main"`.
    struct AlwaysFailRunner;

    impl ProcessRunner for AlwaysFailRunner {
        fn run(&self, _program: &str, _args: &[&str]) -> anyhow::Result<std::process::Output> {
            crate::process::MockProcessRunner::fail("not a git repo")
        }
    }

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
        use crate::models::TaskTag;
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
        assert!(names.contains(&"repo-a"), "expected repo-a sub-epic, got {names:?}");
        assert!(names.contains(&"repo-b"), "expected repo-b sub-epic, got {names:?}");

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
        assert_eq!(flat_tasks.len(), 1, "flat task should exist before migration");

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
