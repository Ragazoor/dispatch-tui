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
        let epics = match self.db.list_epics() {
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

                let known_paths = match db.list_repo_paths() {
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

                if let Err(err) = db.upsert_feed_tasks(epic_id, &items, &repo_paths, &base_branches)
                {
                    tracing::warn!(
                        epic_id = epic_id.0,
                        "FeedRunner: upsert_feed_tasks failed: {err:#}"
                    );
                } else {
                    let _ = notify.send(McpEvent::Refresh);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
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
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db
            .create_epic("Slow Epic", "", "/repo", None, ProjectId(1))
            .unwrap();
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("sleep 5")))
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
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db
            .create_epic("BG Epic", "", "/repo", None, ProjectId(1))
            .unwrap();
        db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"bg1","title":"BG","description":"","status":"backlog"}]'"#,
            )),
        )
        .unwrap();

        let (mut runner, mut rx) = make_runner(db.clone());
        runner.tick().await;

        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for McpEvent::Refresh")
            .expect("channel closed");

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "BG");
    }

    #[tokio::test]
    async fn tick_valid_json_upserts_tasks() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db
            .create_epic("My Epic", "", "/repo", None, ProjectId(1))
            .unwrap();
        db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"1","title":"T","description":"D","status":"backlog"}]'"#,
            )),
        )
        .unwrap();

        let (mut runner, mut rx) = make_runner(db.clone());
        runner.tick().await;

        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for McpEvent::Refresh")
            .expect("channel closed");

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "T");
        assert_eq!(tasks[0].external_id.as_deref(), Some("1"));
    }

    #[tokio::test]
    async fn tick_nonzero_exit_no_panic() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db
            .create_epic("Err Epic", "", "/repo", None, ProjectId(1))
            .unwrap();
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("exit 1")))
            .unwrap();

        let (mut runner, mut rx) = make_runner(db.clone());
        runner.tick().await; // must not panic

        // No Refresh is sent on failure — expect timeout
        let result = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        assert!(result.is_err(), "expected timeout but got a notification");

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn tick_malformed_json_no_panic() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db
            .create_epic("Bad JSON Epic", "", "/repo", None, ProjectId(1))
            .unwrap();
        db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some("echo 'not-json'")),
        )
        .unwrap();

        let (mut runner, mut rx) = make_runner(db.clone());
        runner.tick().await; // must not panic

        let result = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        assert!(result.is_err(), "expected timeout but got a notification");

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn tick_interval_not_elapsed_skips_command() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db
            .create_epic("Interval Epic", "", "/repo", None, ProjectId(1))
            .unwrap();

        // Write a counter to a temp file so we can count how many times the command ran.
        let tmp = std::env::temp_dir().join(format!("feed_test_{}", epic.id.0));
        let cmd = format!(
            r#"echo 0 >> {path}; echo '[{{"external_id":"1","title":"T","description":"","status":"backlog"}}]'"#,
            path = tmp.display()
        );
        db.patch_epic(
            epic.id,
            &EpicPatch::new()
                .feed_command(Some(&cmd))
                .feed_interval_secs(Some(10000)),
        )
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
        let db = Arc::new(Database::open_in_memory().unwrap());
        // Epic with no feed_command (default)
        let epic = db
            .create_epic("Plain Epic", "", "/repo", None, ProjectId(1))
            .unwrap();

        let (mut runner, mut rx) = make_runner(db.clone());
        runner.tick().await;

        // No background task spawned — channel stays empty
        let result = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        assert!(
            result.is_err(),
            "expected empty channel but got notification"
        );

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn start_returns_immediately_without_blocking() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db
            .create_epic("Slow Epic", "", "/repo", None, ProjectId(1))
            .unwrap();
        // A command that would block for 5 seconds if awaited inline.
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("sleep 5")))
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
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db
            .create_epic("BG Feed Epic", "", "/repo", None, ProjectId(1))
            .unwrap();
        db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"bg1","title":"BG Task","description":"","status":"backlog"}]'"#,
            )),
        )
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

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
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
        let db = Arc::new(Database::open_in_memory().unwrap());
        // Register a known repo path matching "myrepo"
        db.save_repo_path("/home/user/code/myrepo").unwrap();
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .unwrap();
        let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://github.com/org/myrepo/pull/42","status":"backlog"}]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await;

        // Wait briefly for the spawned task to complete
        tokio::time::sleep(Duration::from_millis(200)).await;

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].repo_path, "/home/user/code/myrepo",
            "repo_path should be resolved from GitHub URL"
        );
    }

    #[tokio::test]
    async fn tick_no_matching_repo_stores_empty_sentinel() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        // Known repo is "other-repo", not matching "myrepo"
        db.save_repo_path("/home/user/code/other-repo").unwrap();
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .unwrap();
        let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://github.com/org/myrepo/pull/42","status":"backlog"}]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await;

        tokio::time::sleep(Duration::from_millis(200)).await;

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].repo_path, "",
            "unresolved URL should store empty sentinel"
        );
    }

    #[tokio::test]
    async fn tick_empty_url_stores_empty_sentinel() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.save_repo_path("/home/user/code/myrepo").unwrap();
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .unwrap();
        let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","status":"backlog"}]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await;

        tokio::time::sleep(Duration::from_millis(200)).await;

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
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
            self.calls.lock().unwrap().get(path).copied().unwrap_or(0)
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
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.save_repo_path("/home/user/code/repo-a").unwrap();
        db.save_repo_path("/home/user/code/repo-b").unwrap();
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .unwrap();
        // Three items: two for repo-a (master), one for repo-b (develop).
        let cmd = r#"echo '[
            {"external_id":"1","title":"A1","description":"","url":"https://github.com/org/repo-a/pull/1","status":"backlog"},
            {"external_id":"2","title":"A2","description":"","url":"https://github.com/org/repo-a/pull/2","status":"backlog"},
            {"external_id":"3","title":"B1","description":"","url":"https://github.com/org/repo-b/pull/1","status":"backlog"}
        ]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .unwrap();

        let proc_runner = Arc::new(PerRepoBranchRunner::new(&[
            ("/home/user/code/repo-a", "master"),
            ("/home/user/code/repo-b", "develop"),
        ]));
        let (mut runner, _rx) = make_runner_with_runner(db.clone(), proc_runner.clone());
        runner.tick().await;

        // Wait for the spawned task to finish writing tasks.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
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
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.save_repo_path("/home/user/code/repo-a").unwrap();
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .unwrap();
        let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://github.com/org/repo-a/pull/1","status":"backlog"}]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .unwrap();

        // AlwaysFailRunner → detect_default_branch returns "main".
        let (mut runner, _rx) = make_runner_with_runner(db.clone(), Arc::new(AlwaysFailRunner));
        runner.tick().await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].base_branch, "main");
    }

    #[tokio::test]
    async fn tick_non_github_url_stores_empty_sentinel() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.save_repo_path("/home/user/code/myrepo").unwrap();
        let epic = db
            .create_epic("Feed Epic", "", "/fallback", None, ProjectId(1))
            .unwrap();
        let cmd = r#"echo '[{"external_id":"1","title":"T","description":"","url":"https://jira.company.com/PROJ-123","status":"backlog"}]'"#;
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd)))
            .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await;

        tokio::time::sleep(Duration::from_millis(200)).await;

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].repo_path, "",
            "non-github url should store empty sentinel"
        );
    }
}
