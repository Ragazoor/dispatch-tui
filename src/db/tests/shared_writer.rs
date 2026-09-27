//! Where a shared-table mutation goes — Phase 6 of the SpacetimeDB migration.
//!
//! Spec: `sync.allium`'s `BoardWritesThroughTheStore`,
//! `AWriteWithNoConnectionIsRefused` and `StoreRejectsAnInvalidMutation`.
//!
//! Three properties, and the second and third are the ones worth having:
//!
//!   1. With no writer attached — every board today — the SQLite write happens,
//!      exactly as it always has.
//!   2. With one attached, the SQLite write does NOT happen. Not "also
//!      happens": a shared table has exactly one copy, and a second one that
//!      nothing reads is a row the operator cannot see and cannot delete.
//!   3. A writer that refuses leaves SQLite untouched and the error reaches the
//!      caller. No queue, no fallback, no partial write.
//!
//! The fake writer here records rather than talks to a store. That is
//! deliberate: what these tests are about is the BRANCH, and a test that needed
//! a running SpacetimeDB to check which branch was taken would not run in CI.
//! The reducer implementation's own behaviour is `src/sync/tests/writes.rs`
//! and, against a live instance, `tests/spacetime_module.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::db::SharedWriter;
use std::sync::Mutex;

/// A writer that remembers what it was asked to do, and optionally refuses.
#[derive(Default)]
struct RecordingWriter {
    calls: Mutex<Vec<String>>,
    /// When set, every call fails with this message — the store being down.
    refuses_with: Option<String>,
}

impl RecordingWriter {
    fn refusing(why: &str) -> Self {
        Self {
            refuses_with: Some(why.to_string()),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, what: &str) -> Result<()> {
        if let Some(why) = &self.refuses_with {
            anyhow::bail!("{why}");
        }
        self.calls.lock().unwrap().push(what.to_string());
        Ok(())
    }
}

#[async_trait::async_trait]
impl SharedWriter for RecordingWriter {
    async fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId> {
        self.record(&format!("create_task {}", req.title))?;
        Ok(TaskId(1))
    }

    async fn patch_task(&self, id: TaskId, _patch: &TaskPatch<'_>) -> Result<()> {
        self.record(&format!("patch_task {id}"))
    }

    async fn delete_task(&self, id: TaskId) -> Result<()> {
        self.record(&format!("delete_task {id}"))
    }

    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()> {
        self.record(&format!("set_task_epic_id {task_id} {epic_id:?}"))
    }

    async fn try_claim_next_backlog_task(&self, epic_id: EpicId) -> Result<Option<TaskId>> {
        self.record(&format!("claim_next {epic_id}"))?;
        Ok(Some(TaskId(1)))
    }

    async fn try_claim_backlog_task(&self, id: TaskId) -> Result<bool> {
        self.record(&format!("claim {id}"))?;
        Ok(true)
    }

    async fn try_release_backlog_claim(&self, id: TaskId) -> Result<bool> {
        self.record(&format!("release {id}"))?;
        Ok(true)
    }

    async fn create_epic(
        &self,
        title: &str,
        _description: &str,
        _parent_epic_id: Option<EpicId>,
    ) -> Result<crate::models::Epic> {
        self.record(&format!("create_epic {title}"))?;
        Ok(crate::models::Epic {
            id: EpicId(1),
            title: title.to_string(),
            ..epic_fixture()
        })
    }

    async fn patch_epic(&self, id: EpicId, _patch: &EpicPatch<'_>) -> Result<()> {
        self.record(&format!("patch_epic {id}"))
    }

    async fn delete_epic(&self, id: EpicId) -> Result<()> {
        self.record(&format!("delete_epic {id}"))
    }

    async fn recalculate_epic_status(&self, id: EpicId) -> Result<()> {
        self.record(&format!("recalculate_epic_status {id}"))
    }

    async fn save_repo_path(&self, path: &str) -> Result<()> {
        self.record(&format!("save_repo_path {path}"))
    }

    async fn delete_repo_path(&self, path: &str) -> Result<()> {
        self.record(&format!("delete_repo_path {path}"))
    }

    async fn set_verify_command(&self, path: &str, command: Option<&str>) -> Result<()> {
        self.record(&format!("set_verify_command {path} {command:?}"))
    }

    async fn record_base_branch(&self, repo_path: &str, branch: &str) -> Result<()> {
        self.record(&format!("record_base_branch {repo_path} {branch}"))
    }

    async fn subscribe_to_epic(&self, subscriber: &str, epic_id: i64) -> Result<()> {
        self.record(&format!("subscribe_to_epic {subscriber} {epic_id}"))
    }

    async fn unsubscribe_from_epic(&self, subscriber: &str, epic_id: i64) -> Result<bool> {
        self.record(&format!("unsubscribe_from_epic {subscriber} {epic_id}"))?;
        Ok(true)
    }

    async fn save_setting(&self, key: &str, value: &str) -> Result<()> {
        self.record(&format!("save_setting {key} {value}"))
    }

    async fn clear_setting(&self, key: &str) -> Result<()> {
        self.record(&format!("clear_setting {key}"))
    }

    async fn create_learning(&self, row: CreateLearningRow<'_>) -> Result<LearningId> {
        self.record(&format!("create_learning {}", row.summary))?;
        Ok(LearningId(1))
    }

    async fn patch_learning(&self, id: LearningId, _patch: &LearningPatch<'_>) -> Result<()> {
        self.record(&format!("patch_learning {id}"))
    }

    async fn delete_learning(&self, id: LearningId) -> Result<bool> {
        self.record(&format!("delete_learning {id}"))?;
        Ok(true)
    }

    async fn rescope_epic_learnings(&self, from: EpicId, to: EpicId) -> Result<()> {
        self.record(&format!("rescope_epic_learnings {from} {to}"))
    }

    async fn record_learning_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: crate::models::RetrievalSource,
    ) -> Result<()> {
        self.record(&format!(
            "record_learning_retrieval {task_id} {learning_id} {}",
            source.as_str()
        ))
    }

    async fn apply_learning_verdicts(
        &self,
        verdicts: &[(LearningId, crate::models::LearningVerdict)],
    ) -> Result<()> {
        self.record(&format!("apply_learning_verdicts {}", verdicts.len()))
    }

    async fn archive_stale_learnings(&self, _cutoff: chrono::DateTime<chrono::Utc>) -> Result<u64> {
        self.record("archive_stale_learnings")?;
        Ok(0)
    }

    async fn record_usage_event_with_cap(
        &self,
        event: &crate::models::UsageEvent,
        _cap: crate::db::UsageCap,
    ) -> Result<()> {
        self.record(&format!("record_usage_event {}", event.action))
    }

    async fn subagent_start(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64> {
        self.record(&format!("subagent_start {id} {agent_id} {session_id}"))?;
        Ok(1)
    }

    async fn subagent_stop(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
    ) -> Result<SubagentDrain> {
        self.record(&format!("subagent_stop {id} {agent_id} {session_id}"))?;
        Ok(SubagentDrain {
            live: 0,
            applied_pending_stop: false,
        })
    }

    async fn subagent_clear(&self, id: TaskId) -> Result<SubagentDrain> {
        self.record(&format!("subagent_clear {id}"))?;
        Ok(SubagentDrain {
            live: 0,
            applied_pending_stop: false,
        })
    }

    async fn subagent_clear_and_void_pending_stop(&self, id: TaskId) -> Result<()> {
        self.record(&format!("subagent_clear_and_void_pending_stop {id}"))
    }

    async fn try_record_stop(
        &self,
        id: TaskId,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<StopOutcome> {
        self.record(&format!("try_record_stop {id}"))?;
        Ok(StopOutcome::NoOp)
    }

    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        self.record(&format!("record_pre_tool_use {id} {}", sub_status.as_str()))
    }

    async fn record_notification(
        &self,
        id: TaskId,
        write: NotificationWrite,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        self.record(&format!("record_notification {id} {write:?}"))
    }

    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<UserPromptOutcome> {
        self.record(&format!("record_user_prompt_submit {id}"))?;
        Ok(UserPromptOutcome::NoOp)
    }

    async fn mark_pr_learnings_gate_shown(&self, id: TaskId) -> Result<bool> {
        self.record(&format!("mark_pr_learnings_gate_shown {id}"))?;
        Ok(true)
    }

    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: &[crate::models::FeedItem],
        _repo_paths: &[String],
        _base_branches: &[String],
    ) -> Result<Vec<crate::db::RemovedFeedTask>> {
        self.record(&format!("upsert_feed_tasks {epic_id} {}", items.len()))?;
        Ok(Vec::new())
    }

    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: &[crate::models::FeedItem],
        _repo_paths: &[String],
        _base_branches: &[String],
    ) -> Result<Vec<crate::db::RemovedFeedTask>> {
        self.record(&format!(
            "upsert_feed_tasks_additive {epic_id} {}",
            items.len()
        ))?;
        Ok(Vec::new())
    }

    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        _keep_external_ids: &[String],
    ) -> Result<Vec<crate::db::RemovedFeedTask>> {
        self.record(&format!("delete_stale_subtree_feed_tasks {parent_id}"))?;
        Ok(Vec::new())
    }

    async fn create_repo_group_sub_epic(&self, parent_id: EpicId, title: &str) -> Result<EpicId> {
        self.record(&format!("create_repo_group_sub_epic {parent_id} {title}"))?;
        Ok(EpicId(1))
    }

    async fn create_managed_role_epic(
        &self,
        title: &str,
        _parent_epic_id: Option<EpicId>,
        _role: crate::models::FeedRole,
        _feed_command: Option<&str>,
        _feed_interval_secs: Option<i64>,
    ) -> Result<EpicId> {
        self.record(&format!("create_managed_role_epic {title}"))?;
        Ok(EpicId(1))
    }

    async fn create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()> {
        self.record(&format!(
            "create_task_watcher {watcher_task_id} {target_task_id}"
        ))
    }

    async fn delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()> {
        self.record(&format!(
            "delete_task_watcher {watcher_task_id} {target_task_id}"
        ))
    }

    async fn delete_watches_of_target(&self, target_task_id: TaskId) -> Result<()> {
        self.record(&format!("delete_watches_of_target {target_task_id}"))
    }

    async fn delete_watches_by_watcher(&self, watcher_task_id: TaskId) -> Result<()> {
        self.record(&format!("delete_watches_by_watcher {watcher_task_id}"))
    }

    async fn claim_poll_owner(&self, target: crate::models::PollScopeId) -> Result<()> {
        self.record(&format!("claim_poll_owner {target:?}"))
    }

    async fn override_poll_owner(&self, target: crate::models::PollScopeId) -> Result<()> {
        self.record(&format!("override_poll_owner {target:?}"))
    }

    async fn batch_patch_sub_status(&self, updates: &[(TaskId, SubStatus)]) -> Result<()> {
        self.record(&format!("batch_patch_sub_status {}", updates.len()))
    }

    async fn respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        req: CreateTaskRequest<'_>,
        _labels: &[String],
    ) -> Result<TaskId> {
        self.record(&format!(
            "respawn_phoenix_successor {predecessor} {}",
            req.title
        ))?;
        Ok(TaskId(2))
    }
}

/// A blank epic, for the one method that has to return a whole row.
fn epic_fixture() -> crate::models::Epic {
    crate::models::Epic {
        id: EpicId(0),
        title: String::new(),
        description: String::new(),
        status: TaskStatus::Backlog,
        plan_path: None,
        sort_order: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        completed_at: None,
        auto_dispatch: false,
        parent_epic_id: None,
        feed_command: None,
        feed_interval_secs: None,
        group_by_repo: false,
        feed_role: crate::models::FeedRole::None,
        origin: crate::models::EpicOrigin::Manual,
        feed_append_only: false,
    }
}

fn a_request() -> CreateTaskRequest<'static> {
    CreateTaskRequest {
        title: "shared",
        description: "",
        repo_path: "/repo",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        wrap_up_mode: None,
        auto_run_plan: false,
        phoenix: false,
    }
}

fn a_feed_item() -> crate::models::FeedItem {
    crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "feed item".to_string(),
        description: String::new(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Bug,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }
}

async fn db_with(writer: RecordingWriter) -> (Database, Arc<RecordingWriter>) {
    let writer = Arc::new(writer);
    let db = in_memory_db().await.with_shared_writer(writer.clone());
    (db, writer)
}

/// The unchanged board. No writer, so the row lands in SQLite and can be read
/// back — this is the single-machine install and not a degraded one.
#[tokio::test]
async fn with_no_writer_a_shared_write_goes_to_sqlite() {
    let db = in_memory_db().await;
    db.save_repo_path("/repo").await.unwrap();
    assert_eq!(db.list_repo_paths().await.unwrap(), vec!["/repo"]);
}

/// The configured board. The writer is called and SQLite is NOT written.
///
/// The second assertion is the one that matters. A board that wrote both would
/// have a local copy nothing reads, diverging from the store from the first
/// mutation onward, and an operator with no way to tell which they were
/// looking at.
#[tokio::test]
async fn with_a_writer_a_shared_write_bypasses_sqlite() {
    let (db, writer) = db_with(RecordingWriter::default()).await;

    db.save_repo_path("/repo").await.unwrap();

    assert_eq!(writer.calls(), vec!["save_repo_path /repo"]);
    assert!(
        db.list_repo_paths().await.unwrap().is_empty(),
        "the local table must stay empty; the store holds the only copy"
    );
}

/// `sync.allium: AWriteWithNoConnectionIsRefused`. The error reaches the
/// caller and nothing was written anywhere.
#[tokio::test]
async fn a_write_with_the_store_down_fails_and_changes_nothing() {
    let (db, _) = db_with(RecordingWriter::refusing("store unreachable")).await;

    let refused = db.save_repo_path("/repo").await;

    let why = refused.expect_err("a write with the store down must fail");
    assert!(
        why.to_string().contains("store unreachable"),
        "the refusal must name the reason, got {why}"
    );
    assert!(
        db.list_repo_paths().await.unwrap().is_empty(),
        "a refused write must not fall back to the local copy"
    );
}

/// A refusal is never retried or buffered. `sync.allium: NoWriteIsEverQueued`.
///
/// Asserted by making the refusal one-shot at the call site: the caller gets
/// exactly one failure per attempt, and a second attempt is a second call
/// rather than a replay of the first.
#[tokio::test]
async fn a_refused_write_is_not_replayed_on_the_next_one() {
    let (db, writer) = db_with(RecordingWriter::default()).await;

    // One successful write, so there is a call history to compare against.
    db.save_repo_path("/a").await.unwrap();
    assert_eq!(writer.calls(), vec!["save_repo_path /a"]);

    // A second write adds exactly one call. Nothing from before reappears.
    db.save_repo_path("/b").await.unwrap();
    assert_eq!(
        writer.calls(),
        vec!["save_repo_path /a", "save_repo_path /b"]
    );
}

/// Task creation routes too, and the id the store generated comes back.
#[tokio::test]
async fn a_task_create_routes_to_the_writer() {
    let (db, writer) = db_with(RecordingWriter::default()).await;

    let id = db.create_task(a_request()).await.unwrap();

    assert_eq!(id, TaskId(1));
    assert_eq!(writer.calls(), vec!["create_task shared"]);
    assert!(db.list_all().await.unwrap().is_empty());
}

/// Learnings route to the writer as of Phase 10 (task #4914) — the knowledge
/// base was never actually per-machine data, only filed that way, so it moved
/// onto the shared half alongside settings (Phase 9). Usage
/// telemetry followed in Phase 11 (task #4915) — see
/// `a_usage_event_write_routes_to_the_writer` below.
#[tokio::test]
async fn a_learning_write_routes_to_the_writer() {
    use crate::models::{LearningKind, LearningScope};

    let (db, writer) = db_with(RecordingWriter::default()).await;

    let id = db
        .create_learning(crate::db::CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "a learning",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    assert_eq!(id, LearningId(1));
    assert_eq!(writer.calls(), vec!["create_learning a learning"]);
    assert!(
        db.list_learnings(crate::db::LearningFilter::default())
            .await
            .unwrap()
            .is_empty(),
        "the local table must stay empty; the store holds the only copy"
    );
}

/// Usage events route to the writer as of Phase 11 (task #4915).
#[tokio::test]
async fn a_usage_event_write_routes_to_the_writer() {
    use crate::models::{UsageActor, UsageCategory, UsageEvent};

    let (db, writer) = db_with(RecordingWriter::default()).await;

    db.record_usage_event(&UsageEvent {
        category: UsageCategory::Keybinding,
        action: "dispatch_task".to_string(),
        detail: Some("d".to_string()),
        actor: UsageActor::Human,
    })
    .await
    .unwrap();

    assert_eq!(writer.calls(), vec!["record_usage_event dispatch_task"]);
    assert!(
        db.query_usage(&crate::db::UsageQuery::default())
            .await
            .unwrap()
            .is_empty(),
        "the local table must stay empty; the store holds the only copy"
    );
}

/// A generic setting routes to the writer, and the identity/credential keys —
/// which share the same physical table but stay local unconditionally
/// (task #4907) — are unaffected: this is what
/// `SettingsStore::get_setting_string`/`HostStore::ensure_host_identity`
/// staying different call paths is *for*.
#[tokio::test]
async fn a_generic_setting_routes_but_the_host_identity_does_not() {
    let (db, writer) = db_with(RecordingWriter::default()).await;

    db.set_setting_string("theme", "dark").await.unwrap();
    assert_eq!(writer.calls(), vec!["save_setting theme dark"]);

    let (host_id, _label) = db.ensure_host_identity().await.unwrap();
    assert!(!host_id.is_empty());
    db.rename_host("my-laptop").await.unwrap();
    assert_eq!(
        writer.calls(),
        vec!["save_setting theme dark"],
        "minting/renaming the host must never reach the shared writer"
    );
}

/// EVERY ROUTED METHOD IS ROUTED, checked one call at a time.
///
/// The value here is not any single assertion — it is that a method added to
/// [`SharedWriter`] and then NOT guarded in `src/db/queries/` compiles, passes
/// every other test, and silently writes to the wrong store. Nothing but a call
/// through `Database` catches that, so this calls all of them.
///
/// It asserts the WRITER saw it rather than that SQLite did not, because a
/// couple of these (a patch with no changes, an epic recalculation) have no
/// observable local row to be absent.
#[tokio::test]
async fn every_routed_mutation_reaches_the_writer() {
    let (db, writer) = db_with(RecordingWriter::default()).await;

    db.create_task(a_request()).await.unwrap();
    db.patch_task(TaskId(1), &TaskPatch::new().title("t"))
        .await
        .unwrap();
    db.set_task_epic_id(TaskId(1), Some(EpicId(2)))
        .await
        .unwrap();
    db.delete_task(TaskId(1)).await.unwrap();

    db.try_claim_next_backlog_task(EpicId(1), chrono::Utc::now())
        .await
        .unwrap();
    db.try_claim_backlog_task(TaskId(1), chrono::Utc::now())
        .await
        .unwrap();
    db.try_release_backlog_claim(TaskId(1)).await.unwrap();

    db.create_epic("E", "", None).await.unwrap();
    db.patch_epic(EpicId(1), &EpicPatch::new().title("e"))
        .await
        .unwrap();
    db.recalculate_epic_status(EpicId(1)).await.unwrap();
    db.delete_epic(EpicId(1)).await.unwrap();

    db.save_repo_path("/repo").await.unwrap();
    db.set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();
    db.record_base_branch("/repo", "main").await.unwrap();
    db.delete_repo_path("/repo").await.unwrap();

    db.subscribe_to_epic("user-me", 1).await.unwrap();
    db.unsubscribe_from_epic("user-me", 1).await.unwrap();

    db.set_setting_bool("notifications", true).await.unwrap();
    db.set_setting_string("theme", "dark").await.unwrap();
    db.set_reviews_feed_command(None).await.unwrap();

    let now = chrono::Utc::now();
    db.subagent_start(TaskId(1), "agent-1", "session-1", now)
        .await
        .unwrap();
    db.subagent_stop(TaskId(1), "agent-1", "session-1")
        .await
        .unwrap();
    db.subagent_clear(TaskId(1)).await.unwrap();
    db.subagent_clear_and_void_pending_stop(TaskId(1))
        .await
        .unwrap();
    db.try_record_stop(TaskId(1), now).await.unwrap();
    db.record_pre_tool_use(TaskId(1), SubStatus::Active, now)
        .await
        .unwrap();
    db.record_notification(TaskId(1), NotificationWrite::Raise, now)
        .await
        .unwrap();
    db.record_user_prompt_submit(TaskId(1), now).await.unwrap();
    db.mark_pr_learnings_gate_shown(TaskId(1)).await.unwrap();

    let items = vec![a_feed_item()];
    let repo_paths = vec!["/repo".to_string()];
    let base_branches = vec!["main".to_string()];
    db.upsert_feed_tasks(EpicId(1), &items, &repo_paths, &base_branches)
        .await
        .unwrap();
    db.upsert_feed_tasks_additive(EpicId(1), &items, &repo_paths, &base_branches)
        .await
        .unwrap();
    db.delete_stale_subtree_feed_tasks(EpicId(1), &["ext-1".to_string()])
        .await
        .unwrap();
    db.create_repo_group_sub_epic(EpicId(1), "repo")
        .await
        .unwrap();
    db.create_managed_role_epic(
        "Reviews",
        Some(EpicId(1)),
        crate::models::FeedRole::None,
        None,
        None,
    )
    .await
    .unwrap();

    db.create_task_watcher(TaskId(1), TaskId(2)).await.unwrap();
    db.delete_task_watcher(TaskId(1), TaskId(2)).await.unwrap();
    db.delete_watches_of_target(TaskId(2)).await.unwrap();
    db.delete_watches_by_watcher(TaskId(1)).await.unwrap();

    db.batch_patch_sub_status(&[(TaskId(1), SubStatus::Active)])
        .await
        .unwrap();
    db.respawn_phoenix_successor(TaskId(1), a_request(), &[])
        .await
        .unwrap();

    db.record_usage_event(&crate::models::UsageEvent {
        category: crate::models::UsageCategory::Keybinding,
        action: "dispatch_task".to_string(),
        detail: None,
        actor: crate::models::UsageActor::Human,
    })
    .await
    .unwrap();

    let names: Vec<String> = writer
        .calls()
        .into_iter()
        .map(|c| c.split_whitespace().next().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "create_task",
            "patch_task",
            "set_task_epic_id",
            "delete_task",
            "claim_next",
            "claim",
            "release",
            "create_epic",
            "patch_epic",
            "recalculate_epic_status",
            "delete_epic",
            "save_repo_path",
            "set_verify_command",
            "record_base_branch",
            "delete_repo_path",
            "subscribe_to_epic",
            "unsubscribe_from_epic",
            "save_setting",
            "save_setting",
            "clear_setting",
            "subagent_start",
            "subagent_stop",
            "subagent_clear",
            "subagent_clear_and_void_pending_stop",
            "try_record_stop",
            "record_pre_tool_use",
            "record_notification",
            "record_user_prompt_submit",
            "mark_pr_learnings_gate_shown",
            "upsert_feed_tasks",
            "upsert_feed_tasks_additive",
            "delete_stale_subtree_feed_tasks",
            "create_repo_group_sub_epic",
            "create_managed_role_epic",
            "create_task_watcher",
            "delete_task_watcher",
            "delete_watches_of_target",
            "delete_watches_by_watcher",
            "batch_patch_sub_status",
            "respawn_phoenix_successor",
            "record_usage_event",
        ]
    );
}

/// ...and none of them touched SQLite. The other half of the claim above,
/// checked over the tables that do have observable rows.
#[tokio::test]
async fn no_routed_mutation_leaves_a_local_row() {
    let (db, _) = db_with(RecordingWriter::default()).await;

    db.create_task(a_request()).await.unwrap();
    db.create_epic("E", "", None).await.unwrap();
    db.save_repo_path("/repo").await.unwrap();
    db.record_base_branch("/repo", "main").await.unwrap();
    db.subscribe_to_epic("user-me", 1).await.unwrap();
    db.set_setting_string("theme", "dark").await.unwrap();

    let now = chrono::Utc::now();
    db.subagent_start(TaskId(1), "agent-1", "session-1", now)
        .await
        .unwrap();

    let items = vec![a_feed_item()];
    let repo_paths = vec!["/repo".to_string()];
    let base_branches = vec!["main".to_string()];
    db.upsert_feed_tasks(EpicId(1), &items, &repo_paths, &base_branches)
        .await
        .unwrap();
    db.create_repo_group_sub_epic(EpicId(1), "repo")
        .await
        .unwrap();
    db.create_task_watcher(TaskId(1), TaskId(2)).await.unwrap();
    db.batch_patch_sub_status(&[(TaskId(1), SubStatus::Active)])
        .await
        .unwrap();
    db.respawn_phoenix_successor(TaskId(1), a_request(), &[])
        .await
        .unwrap();
    db.record_usage_event(&crate::models::UsageEvent {
        category: crate::models::UsageCategory::Keybinding,
        action: "dispatch_task".to_string(),
        detail: None,
        actor: crate::models::UsageActor::Human,
    })
    .await
    .unwrap();

    assert!(db.list_all().await.unwrap().is_empty(), "tasks");
    assert!(db.list_epics().await.unwrap().is_empty(), "epics");
    assert!(db.list_repo_paths().await.unwrap().is_empty(), "repo_paths");
    assert!(
        db.list_all_base_branches().await.unwrap().is_empty(),
        "repo_base_branches"
    );
    assert!(
        db.subscribed_epics("user-me").await.unwrap().is_empty(),
        "subscriptions"
    );
    assert!(
        db.get_setting_string("theme").await.unwrap().is_none(),
        "settings"
    );
    assert_eq!(
        db.db_call(|conn| Ok(conn
            .query_row("SELECT COUNT(*) FROM task_subagents", [], |r| r
                .get::<_, i64>(0))?))
            .await
            .unwrap(),
        0,
        "task_subagents"
    );
    assert_eq!(
        db.db_call(|conn| Ok(conn
            .query_row("SELECT COUNT(*) FROM task_watchers", [], |r| r
                .get::<_, i64>(0))?))
            .await
            .unwrap(),
        0,
        "task_watchers"
    );
    assert!(
        db.query_usage(&crate::db::UsageQuery::default())
            .await
            .unwrap()
            .is_empty(),
        "usage_events"
    );
}

/// A refusal on ANY of them changes nothing locally. The no-fallback rule is
/// per-method, not a property of the one method it was first written for.
#[tokio::test]
async fn a_refusal_never_falls_back_to_the_local_store() {
    let (db, _) = db_with(RecordingWriter::refusing("store unreachable")).await;

    assert!(db.create_task(a_request()).await.is_err());
    assert!(db.create_epic("E", "", None).await.is_err());
    assert!(db.save_repo_path("/repo").await.is_err());
    assert!(db.set_setting_string("theme", "dark").await.is_err());
    assert!(db
        .subagent_start(TaskId(1), "agent-1", "session-1", chrono::Utc::now())
        .await
        .is_err());
    assert!(db
        .try_record_stop(TaskId(1), chrono::Utc::now())
        .await
        .is_err());
    assert!(db.mark_pr_learnings_gate_shown(TaskId(1)).await.is_err());
    assert!(db
        .upsert_feed_tasks(
            EpicId(1),
            &[a_feed_item()],
            &["/repo".to_string()],
            &["main".to_string()]
        )
        .await
        .is_err());
    assert!(db
        .create_repo_group_sub_epic(EpicId(1), "repo")
        .await
        .is_err());
    assert!(db.create_task_watcher(TaskId(1), TaskId(2)).await.is_err());
    assert!(db
        .batch_patch_sub_status(&[(TaskId(1), SubStatus::Active)])
        .await
        .is_err());
    assert!(db
        .respawn_phoenix_successor(TaskId(1), a_request(), &[])
        .await
        .is_err());

    assert!(db.list_all().await.unwrap().is_empty());
    assert!(db.list_epics().await.unwrap().is_empty());
    assert!(db.list_repo_paths().await.unwrap().is_empty());
    assert!(db.get_setting_string("theme").await.unwrap().is_none());
}
