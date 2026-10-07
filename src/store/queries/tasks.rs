use anyhow::Result;

use crate::models::{
    EpicId, FeedItem, NotificationWrite, StopOutcome, SubStatus, SubagentDrain, TaskId,
    UserPromptOutcome,
};

use super::super::{CreateTaskRequest, Database, RemovedFeedTask, TaskPatch};

#[async_trait::async_trait]
impl super::super::TaskRead for Database {
    async fn get_task(&self, id: TaskId) -> Result<Option<crate::models::Task>> {
        let reader = self.shared_reader()?;
        reader.get_task(id).await
    }

    async fn task_exists(&self, id: TaskId) -> Result<bool> {
        let reader = self.shared_reader()?;
        reader.task_exists(id).await
    }

    async fn list_live_agent_tasks(&self) -> Result<Vec<crate::models::Task>> {
        let reader = self.shared_reader()?;
        reader.list_live_agent_tasks().await
    }

    async fn list_all(&self) -> Result<Vec<crate::models::Task>> {
        let reader = self.shared_reader()?;
        reader.list_all().await
    }

    async fn find_task_by_plan(&self, plan: &str) -> Result<Option<crate::models::Task>> {
        let reader = self.shared_reader()?;
        reader.find_task_by_plan(plan).await
    }
}

#[async_trait::async_trait]
impl super::super::TaskCrud for Database {
    async fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId> {
        // ROUTED. `sync.allium: BoardWritesThroughTheStore` — a board with a
        // store writes there and NOT here, so the return is the whole method
        // rather than a step before the local write.
        let writer = self.shared_writer()?;
        writer.create_task(req).await
    }

    async fn respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        req: CreateTaskRequest<'_>,
        labels: &[String],
    ) -> Result<TaskId> {
        let writer = self.shared_writer()?;
        writer
            .respawn_phoenix_successor(predecessor, req, labels)
            .await
    }

    /// tasks.allium: `DeleteTask`. A feed task's delete retires its
    /// `external_id` under its `nearest_feed_epic` (core/Epic, computed here
    /// the same way `upsert_feed_tasks_inner` does) in the SAME transaction as
    /// the row delete — a cycle landing between the two would otherwise
    /// re-insert. `INSERT OR IGNORE` makes a second delete of an
    /// already-retired id (core/RetiredFeedItem:
    /// `UniqueRetiredFeedItemPerFeed`) a no-op on the record rather than a
    /// failed delete. A manual task (no `external_id`) or one under no feed
    /// epic in its chain retires nothing — there is no cycle to suppress.
    async fn delete_task(&self, id: TaskId) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.delete_task(id).await
    }

    /// tasks.allium: `BatchDelete`'s atomic counterpart to `delete_task`/
    /// `delete_epic` looped per item. On the shared store this is the ONLY
    /// path that actually needs the joint guard — see
    /// `spacetime/module/src/tasks_epics.rs::batch_delete`'s doc comment. The local
    /// SQLite fallback below re-validates nothing before deleting, matching
    /// `delete_task`'s and `delete_epic`'s own SQLite shape (neither
    /// re-checks status here either): the single serialized writer has no
    /// stale-subscription-view gap for a service-layer check to race against.
    async fn batch_delete(&self, task_ids: &[TaskId], epic_ids: &[EpicId]) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.batch_delete(task_ids, epic_ids).await
    }

    async fn patch_task(&self, id: TaskId, patch: &TaskPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        if matches!((patch.status, patch.sub_status), (Some(s), Some(ss)) if !ss.is_valid_for(s)) {
            anyhow::bail!(
                "invalid (status, sub_status) pair in patch: {:?}/{:?}",
                patch.status,
                patch.sub_status
            );
        }
        // AFTER the two guards above, deliberately. They are cheap, they are
        // about the ARGUMENTS rather than about the rows, and a store round
        // trip to be told a patch was empty is a round trip for nothing. Every
        // check that needs to see other rows is the store's — see
        // `sync.allium: StoreRejectsAnInvalidMutation`.
        let writer = self.shared_writer()?;
        writer.patch_task(id, patch).await
    }

    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<Vec<RemovedFeedTask>> {
        let writer = self.shared_writer()?;
        writer
            .upsert_feed_tasks(epic_id, items, repo_paths, base_branches)
            .await
    }

    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<Vec<RemovedFeedTask>> {
        let writer = self.shared_writer()?;
        writer
            .upsert_feed_tasks_additive(epic_id, items, repo_paths, base_branches)
            .await
    }

    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        keep_external_ids: &[String],
    ) -> Result<Vec<RemovedFeedTask>> {
        let writer = self.shared_writer()?;
        writer
            .delete_stale_subtree_feed_tasks(parent_id, keep_external_ids)
            .await
    }
    async fn mark_pr_learnings_gate_shown(&self, id: TaskId) -> Result<bool> {
        let writer = self.shared_writer()?;
        writer.mark_pr_learnings_gate_shown(id).await
    }

    async fn subagent_start(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64> {
        let writer = self.shared_writer()?;
        writer.subagent_start(id, agent_id, session_id, now).await
    }

    async fn subagent_stop(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
    ) -> Result<SubagentDrain> {
        let writer = self.shared_writer()?;
        writer.subagent_stop(id, agent_id, session_id).await
    }

    async fn subagent_clear(&self, id: TaskId) -> Result<SubagentDrain> {
        let writer = self.shared_writer()?;
        writer.subagent_clear(id).await
    }

    async fn subagent_clear_and_void_pending_stop(&self, id: TaskId) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.subagent_clear_and_void_pending_stop(id).await
    }

    async fn try_record_stop(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<StopOutcome> {
        let writer = self.shared_writer()?;
        writer.try_record_stop(id, now).await
    }

    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.record_pre_tool_use(id, sub_status, now).await
    }

    async fn record_notification(
        &self,
        id: TaskId,
        write: NotificationWrite,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.record_notification(id, write, now).await
    }

    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<UserPromptOutcome> {
        let writer = self.shared_writer()?;
        writer.record_user_prompt_submit(id, now).await
    }

    async fn try_claim_next_backlog_task(
        &self,
        epic_id: EpicId,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<TaskId>> {
        // ROUTED, and `now` is dropped: the store stamps its own clock, so two
        // hosts' claims are ordered by one clock rather than by whose laptop is
        // fast.
        let writer = self.shared_writer()?;
        writer.try_claim_next_backlog_task(epic_id).await
    }

    async fn try_claim_backlog_task(
        &self,
        id: TaskId,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        // ROUTED, and `now` is dropped on the way: the store stamps its own
        // clock, so two hosts' claims are ordered by one clock rather than by
        // whose laptop is fast.
        let writer = self.shared_writer()?;
        writer.try_claim_backlog_task(id).await
    }

    /// Moves Running -> Backlog without clearing `stop_pending`, and safely so:
    /// the `worktree IS NULL` guard restricts it to a claim that never finished
    /// provisioning, so no agent ever ran and no Stop hook can have fired for
    /// this claim. The claim itself also cleared the bit on the way in (see
    /// `DispatchTask` in `docs/specs/dispatch.allium`). Any relaxation of that
    /// guard has to clear it — see `PendingStopOnlyWhileRunning`
    /// (`docs/specs/core.allium`).
    async fn try_release_backlog_claim(&self, id: TaskId) -> Result<bool> {
        // ROUTED. `sync.allium: BoardWritesThroughTheStore`.
        let writer = self.shared_writer()?;
        writer.try_release_backlog_claim(id).await
    }

    async fn batch_patch_sub_status(&self, updates: &[(TaskId, SubStatus)]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }
        let writer = self.shared_writer()?;
        writer.batch_patch_sub_status(updates).await
    }

    async fn create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()> {
        let writer = self.shared_writer()?;
        writer
            .create_task_watcher(watcher_task_id, target_task_id)
            .await
    }

    async fn delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()> {
        let writer = self.shared_writer()?;
        writer
            .delete_task_watcher(watcher_task_id, target_task_id)
            .await
    }

    async fn list_watchers_of(&self, target_task_id: TaskId) -> Result<Vec<TaskId>> {
        let reader = self.shared_reader()?;
        reader.list_watchers_of(target_task_id).await
    }

    async fn delete_watches_of_target(&self, target_task_id: TaskId) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.delete_watches_of_target(target_task_id).await
    }

    async fn delete_watches_by_watcher(&self, watcher_task_id: TaskId) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.delete_watches_by_watcher(watcher_task_id).await
    }

    // Retired feed items.
    async fn retired_without_task(
        &self,
        feed_epic_id: EpicId,
        external_ids: &[String],
    ) -> Result<Vec<String>> {
        let reader = self.shared_retired_feed_item_reader()?;
        reader
            .retired_without_task(feed_epic_id, external_ids)
            .await
    }

    async fn drop_closed_retired_feed_items(
        &self,
        feed_epic_id: EpicId,
        keep_external_ids: &[String],
    ) -> Result<()> {
        let writer = self.shared_writer()?;
        writer
            .drop_closed_retired_feed_items(feed_epic_id, keep_external_ids)
            .await
    }
}

#[async_trait::async_trait]
impl super::super::PollOwnershipStore for Database {
    // `PollOwner` has no SQLite counterpart at all — a no-op with no writer
    // attached is the correct answer, not a missing branch: on a
    // single-machine install there is no other host to contend a claim with,
    // so nothing needs claiming (`core.allium: PollOwner`).
    async fn claim_poll_owner(&self, target: crate::models::PollScopeId) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.claim_poll_owner(target).await
    }

    async fn override_poll_owner(&self, target: crate::models::PollScopeId) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.override_poll_owner(target).await
    }
}
