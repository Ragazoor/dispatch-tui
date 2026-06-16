#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration test (WP5): provisioning the managed feed-epic tree from the
//! reviews/CVE config, then driving one `FeedRunner::tick()` and asserting the
//! reviews emission routes into the correct role sub-epics (WP3 router) while
//! the CVE feed populates its own epic.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use dispatch_tui::db::{Database, EpicCrud, SettingsStore};
use dispatch_tui::feed::FeedRunner;
use dispatch_tui::mcp::McpEvent;
use dispatch_tui::models::{Epic, FeedRole};
use dispatch_tui::process::{MockProcessRunner, ProcessRunner};
use dispatch_tui::service::provision_managed_feeds_from_settings;

/// Always-failing runner: each `git symbolic-ref` call falls back to "main".
struct AlwaysFailRunner;

impl ProcessRunner for AlwaysFailRunner {
    fn run(&self, _program: &str, _args: &[&str]) -> anyhow::Result<std::process::Output> {
        MockProcessRunner::fail("not a git repo")
    }
}

const REVIEWS_CMD: &str = r#"echo '[
    {"external_id":"pr-1","title":"Direct","description":"","url":"https://github.com/org/repo/pull/1","status":"backlog","tag":"pr-review","signals":["direct-request"]},
    {"external_id":"pr-2","title":"Team","description":"","url":"https://github.com/org/repo/pull/2","status":"backlog","tag":"pr-review","signals":["team-request"]}
]'"#;

const CVE_CMD: &str =
    r#"echo '[{"external_id":"cve-1","title":"CVE-2024-0001","description":"","status":"backlog","tag":"bug"}]'"#;

fn role_epic(epics: &[Epic], role: FeedRole) -> Epic {
    epics
        .iter()
        .find(|e| e.feed_role == role)
        .cloned()
        .unwrap_or_else(|| panic!("no managed epic for role {role}"))
}

#[tokio::test]
async fn provisioned_reviews_tick_routes_into_role_sub_epics() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());

    // Configure both managed feeds, then provision (the startup path).
    db.set_reviews_feed_command(Some(REVIEWS_CMD)).await.unwrap();
    db.set_cve_feed_command(Some(CVE_CMD)).await.unwrap();
    provision_managed_feeds_from_settings(&*db).await.unwrap();

    // The tree exists with the expected shape.
    let epics = db.list_epics().await.unwrap();
    assert_eq!(epics.len(), 5, "managed tree provisioned");
    let parent = role_epic(&epics, FeedRole::ReviewsParent);
    let my = role_epic(&epics, FeedRole::MyReviews);
    let team = role_epic(&epics, FeedRole::TeamReviews);
    let cve = role_epic(&epics, FeedRole::Cve);
    assert_eq!(parent.feed_command.as_deref(), Some(REVIEWS_CMD));
    for role in [FeedRole::MyReviews, FeedRole::TeamReviews, FeedRole::Bots] {
        assert_eq!(
            role_epic(&epics, role).feed_command,
            None,
            "role sub-epic {role} carries no feed_command (B3 guard)"
        );
    }

    // Drive a tick: only the parent + cve are polled (sub-epics have no command).
    let (tx, mut rx) = mpsc::unbounded_channel::<McpEvent>();
    let mut runner = FeedRunner::new(db.clone(), tx, Arc::new(AlwaysFailRunner));
    runner.tick().await;

    // Wait until both syncs have landed (background tasks emit McpEvents).
    loop {
        let my_tasks = db.list_tasks_for_epic(my.id).await.unwrap();
        let team_tasks = db.list_tasks_for_epic(team.id).await.unwrap();
        let cve_tasks = db.list_tasks_for_epic(cve.id).await.unwrap();
        if !my_tasks.is_empty() && !team_tasks.is_empty() && !cve_tasks.is_empty() {
            break;
        }
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(_)) => continue,
            _ => break,
        }
    }

    let my_tasks = db.list_tasks_for_epic(my.id).await.unwrap();
    assert_eq!(my_tasks.len(), 1, "direct-request PR routes to My Reviews");
    assert_eq!(my_tasks[0].external_id.as_deref(), Some("pr-1"));

    let team_tasks = db.list_tasks_for_epic(team.id).await.unwrap();
    assert_eq!(team_tasks.len(), 1, "team-request PR routes to Team Reviews");
    assert_eq!(team_tasks[0].external_id.as_deref(), Some("pr-2"));

    let cve_tasks = db.list_tasks_for_epic(cve.id).await.unwrap();
    assert_eq!(cve_tasks.len(), 1, "CVE feed populates its own epic");
    assert_eq!(cve_tasks[0].external_id.as_deref(), Some("cve-1"));
}
