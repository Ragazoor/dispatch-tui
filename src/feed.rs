use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::db::TaskAndEpicStore;
use crate::mcp::McpEvent;
use crate::models::{
    AlertKind, AlertSeverity, EpicId, FeedItem, ReviewDecision, ReviewPr, SecurityAlert, TaskStatus,
};

/// Map a reviewer PR to a FeedItem for feed output.
pub fn review_pr_to_feed_item(pr: &ReviewPr) -> FeedItem {
    let external_id = format!("pr:{}#{}", pr.repo, pr.number);
    let title = format!("#{} {}", pr.number, pr.title);
    let description = pr.body.chars().take(500).collect();
    let status = review_decision_to_status(pr.review_decision);
    FeedItem {
        external_id,
        title,
        description,
        url: pr.url.clone(),
        status,
    }
}

/// Map a Dependabot PR to a FeedItem for feed output.
pub fn bot_pr_to_feed_item(pr: &ReviewPr) -> FeedItem {
    let external_id = format!("dep:{}#{}", pr.repo, pr.number);
    let title = format!("#{} {}", pr.number, pr.title);
    let description = pr.body.chars().take(500).collect();
    let status = review_decision_to_status(pr.review_decision);
    FeedItem {
        external_id,
        title,
        description,
        url: pr.url.clone(),
        status,
    }
}

/// Map a security alert to a FeedItem for feed output.
pub fn alert_to_feed_item(alert: &SecurityAlert) -> FeedItem {
    let kind_prefix = match alert.kind {
        AlertKind::Dependabot => "dependabot",
        AlertKind::CodeScanning => "code-scanning",
    };
    let external_id = format!("{}:{}#{}", kind_prefix, alert.repo, alert.number);
    let severity_label = match alert.severity {
        AlertSeverity::Critical => "CRIT",
        AlertSeverity::High => "HIGH",
        AlertSeverity::Medium => "MED",
        AlertSeverity::Low => "LOW",
    };
    let title = format!("[{}] {}", severity_label, alert.title);
    FeedItem {
        external_id,
        title,
        description: alert.description.clone(),
        url: alert.url.clone(),
        status: TaskStatus::Backlog,
    }
}

fn review_decision_to_status(decision: ReviewDecision) -> TaskStatus {
    match decision {
        ReviewDecision::ReviewRequired => TaskStatus::Backlog,
        ReviewDecision::WaitingForResponse => TaskStatus::Running,
        ReviewDecision::ChangesRequested => TaskStatus::Review,
        ReviewDecision::Approved => TaskStatus::Done,
    }
}

const DEFAULT_FEED_INTERVAL: Duration = Duration::from_secs(30);

pub struct FeedRunner {
    db: Arc<dyn TaskAndEpicStore>,
    notify: mpsc::UnboundedSender<McpEvent>,
    last_run: HashMap<EpicId, Instant>,
}

impl FeedRunner {
    pub fn new(db: Arc<dyn TaskAndEpicStore>, notify: mpsc::UnboundedSender<McpEvent>) -> Self {
        Self {
            db,
            notify,
            last_run: HashMap::new(),
        }
    }

    pub async fn tick(&mut self) {
        let epics = match self.db.list_epics() {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("FeedRunner: failed to list epics: {err:#}");
                return;
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

            let output = match tokio::process::Command::new("sh")
                .args(["-c", cmd])
                .output()
                .await
            {
                Ok(o) => o,
                Err(err) => {
                    tracing::warn!(
                        epic_id = epic.id.0,
                        epic_title = %epic.title,
                        "FeedRunner: failed to spawn command: {err:#}"
                    );
                    self.last_run.insert(epic.id, Instant::now());
                    continue;
                }
            };

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!(
                    epic_id = epic.id.0,
                    epic_title = %epic.title,
                    "FeedRunner: command exited non-zero: {stderr}"
                );
                self.last_run.insert(epic.id, Instant::now());
                continue;
            }

            let items: Vec<FeedItem> = match serde_json::from_slice::<Vec<FeedItem>>(&output.stdout)
            {
                Ok(i) => i,
                Err(err) => {
                    tracing::warn!(
                        epic_id = epic.id.0,
                        epic_title = %epic.title,
                        "FeedRunner: failed to parse JSON output: {err:#}"
                    );
                    self.last_run.insert(epic.id, Instant::now());
                    continue;
                }
            };

            if let Err(err) = self.db.upsert_feed_tasks(epic.id, &items) {
                tracing::warn!(
                    epic_id = epic.id.0,
                    "FeedRunner: upsert_feed_tasks failed: {err:#}"
                );
            } else {
                let _ = self.notify.send(McpEvent::Refresh);
            }

            self.last_run.insert(epic.id, Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, EpicCrud, EpicPatch};
    use crate::models::CiStatus;
    use std::sync::Arc;

    fn make_pr(
        number: i64,
        repo: &str,
        title: &str,
        body: &str,
        url: &str,
        decision: ReviewDecision,
    ) -> ReviewPr {
        ReviewPr {
            number,
            title: title.to_string(),
            author: "author".to_string(),
            repo: repo.to_string(),
            url: url.to_string(),
            is_draft: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            additions: 0,
            deletions: 0,
            review_decision: decision,
            labels: vec![],
            body: body.to_string(),
            head_ref: "main".to_string(),
            ci_status: CiStatus::Success,
            reviewers: vec![],
        }
    }

    fn make_alert(
        number: i64,
        repo: &str,
        kind: AlertKind,
        severity: AlertSeverity,
        title: &str,
        description: &str,
        url: &str,
    ) -> SecurityAlert {
        SecurityAlert {
            number,
            repo: repo.to_string(),
            severity,
            kind,
            title: title.to_string(),
            package: None,
            vulnerable_range: None,
            fixed_version: None,
            cvss_score: None,
            url: url.to_string(),
            created_at: chrono::Utc::now(),
            state: "open".to_string(),
            description: description.to_string(),
        }
    }

    // --- review_pr_to_feed_item ---

    #[test]
    fn review_pr_external_id_format() {
        let pr = make_pr(
            42,
            "acme/app",
            "Fix bug",
            "",
            "https://gh/pr/42",
            ReviewDecision::ReviewRequired,
        );
        let item = review_pr_to_feed_item(&pr);
        assert_eq!(item.external_id, "pr:acme/app#42");
    }

    #[test]
    fn review_pr_external_id_is_stable() {
        let pr1 = make_pr(
            7,
            "org/repo",
            "T",
            "",
            "https://gh",
            ReviewDecision::Approved,
        );
        let pr2 = make_pr(
            7,
            "org/repo",
            "T",
            "",
            "https://gh",
            ReviewDecision::Approved,
        );
        assert_eq!(
            review_pr_to_feed_item(&pr1).external_id,
            review_pr_to_feed_item(&pr2).external_id
        );
    }

    #[test]
    fn review_pr_title_format() {
        let pr = make_pr(
            10,
            "a/b",
            "My PR",
            "",
            "https://gh",
            ReviewDecision::ReviewRequired,
        );
        let item = review_pr_to_feed_item(&pr);
        assert_eq!(item.title, "#10 My PR");
    }

    #[test]
    fn review_pr_description_truncated_to_500() {
        let long_body: String = "x".repeat(600);
        let pr = make_pr(
            1,
            "a/b",
            "T",
            &long_body,
            "https://gh",
            ReviewDecision::ReviewRequired,
        );
        let item = review_pr_to_feed_item(&pr);
        assert_eq!(item.description.chars().count(), 500);
    }

    #[test]
    fn review_pr_url_preserved() {
        let pr = make_pr(
            1,
            "a/b",
            "T",
            "",
            "https://github.com/a/b/pull/1",
            ReviewDecision::ReviewRequired,
        );
        let item = review_pr_to_feed_item(&pr);
        assert_eq!(item.url, "https://github.com/a/b/pull/1");
    }

    #[test]
    fn review_pr_status_mapping() {
        let cases = [
            (ReviewDecision::ReviewRequired, TaskStatus::Backlog),
            (ReviewDecision::WaitingForResponse, TaskStatus::Running),
            (ReviewDecision::ChangesRequested, TaskStatus::Review),
            (ReviewDecision::Approved, TaskStatus::Done),
        ];
        for (decision, expected_status) in cases {
            let pr = make_pr(1, "a/b", "T", "", "https://gh", decision);
            assert_eq!(
                review_pr_to_feed_item(&pr).status,
                expected_status,
                "decision: {decision:?}"
            );
        }
    }

    // --- bot_pr_to_feed_item ---

    #[test]
    fn bot_pr_external_id_uses_dep_prefix() {
        let pr = make_pr(
            5,
            "acme/lib",
            "Bump lodash",
            "",
            "https://gh",
            ReviewDecision::ReviewRequired,
        );
        let item = bot_pr_to_feed_item(&pr);
        assert_eq!(item.external_id, "dep:acme/lib#5");
    }

    // --- alert_to_feed_item ---

    #[test]
    fn alert_dependabot_external_id() {
        let alert = make_alert(
            3,
            "acme/app",
            AlertKind::Dependabot,
            AlertSeverity::High,
            "vuln",
            "",
            "https://gh",
        );
        let item = alert_to_feed_item(&alert);
        assert_eq!(item.external_id, "dependabot:acme/app#3");
    }

    #[test]
    fn alert_code_scanning_external_id() {
        let alert = make_alert(
            9,
            "acme/app",
            AlertKind::CodeScanning,
            AlertSeverity::Low,
            "issue",
            "",
            "https://gh",
        );
        let item = alert_to_feed_item(&alert);
        assert_eq!(item.external_id, "code-scanning:acme/app#9");
    }

    #[test]
    fn alert_title_includes_severity_badge() {
        let cases = [
            (AlertSeverity::Critical, "[CRIT]"),
            (AlertSeverity::High, "[HIGH]"),
            (AlertSeverity::Medium, "[MED]"),
            (AlertSeverity::Low, "[LOW]"),
        ];
        for (severity, badge) in cases {
            let alert = make_alert(
                1,
                "a/b",
                AlertKind::Dependabot,
                severity,
                "some vuln",
                "",
                "https://gh",
            );
            let item = alert_to_feed_item(&alert);
            assert!(
                item.title.starts_with(badge),
                "expected {badge} prefix, got: {}",
                item.title
            );
        }
    }

    #[test]
    fn alert_description_and_url_preserved() {
        let alert = make_alert(
            1,
            "a/b",
            AlertKind::Dependabot,
            AlertSeverity::High,
            "t",
            "detailed desc",
            "https://alerts/1",
        );
        let item = alert_to_feed_item(&alert);
        assert_eq!(item.description, "detailed desc");
        assert_eq!(item.url, "https://alerts/1");
    }

    #[test]
    fn alert_status_is_always_backlog() {
        let alert = make_alert(
            1,
            "a/b",
            AlertKind::CodeScanning,
            AlertSeverity::Critical,
            "t",
            "",
            "https://gh",
        );
        let item = alert_to_feed_item(&alert);
        assert_eq!(item.status, TaskStatus::Backlog);
    }

    // --- FeedRunner tests ---

    fn make_runner(db: Arc<Database>) -> (FeedRunner, mpsc::UnboundedReceiver<McpEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (FeedRunner::new(db, tx), rx)
    }

    #[tokio::test]
    async fn tick_valid_json_upserts_tasks() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db.create_epic("My Epic", "", "/repo", None).unwrap();
        db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some(
                r#"echo '[{"external_id":"1","title":"T","description":"D","status":"backlog"}]'"#,
            )),
        )
        .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await;

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "T");
        assert_eq!(tasks[0].external_id.as_deref(), Some("1"));
    }

    #[tokio::test]
    async fn tick_nonzero_exit_no_panic() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db.create_epic("Err Epic", "", "/repo", None).unwrap();
        db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("exit 1")))
            .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await; // must not panic

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn tick_malformed_json_no_panic() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db.create_epic("Bad JSON Epic", "", "/repo", None).unwrap();
        db.patch_epic(
            epic.id,
            &EpicPatch::new().feed_command(Some("echo 'not-json'")),
        )
        .unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await; // must not panic

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn tick_interval_not_elapsed_skips_command() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let epic = db.create_epic("Interval Epic", "", "/repo", None).unwrap();

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

        let (mut runner, _rx) = make_runner(db.clone());
        // First tick: command runs, counter file gets one line.
        runner.tick().await;
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
        let epic = db.create_epic("Plain Epic", "", "/repo", None).unwrap();

        let (mut runner, _rx) = make_runner(db.clone());
        runner.tick().await;

        let tasks = db.list_tasks_for_epic(epic.id).unwrap();
        assert!(tasks.is_empty());
    }
}
