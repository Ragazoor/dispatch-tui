use anyhow::Result;

use crate::models::{
    Agent, EpicId, FeedItem, NotificationWrite, StopOutcome, SubStatus, SubagentDrain, Task,
    TaskId, TaskStatus, UserPromptOutcome,
};
use crate::sync::{encode, ReducerOutcome};

use super::super::{CreateTaskRequest, FeedIngestOutcome, Store, TaskCrud, TaskPatch};

#[async_trait::async_trait]
impl super::super::TaskRead for Store {
    async fn get_task(&self, id: TaskId) -> Option<Task> {
        self.rows.task(id)
    }

    async fn task_exists(&self, id: TaskId) -> bool {
        self.rows.has_task(id)
    }

    async fn list_live_agent_tasks(&self) -> Vec<Task> {
        self.rows.live_agent_tasks()
    }

    async fn list_all(&self) -> Vec<Task> {
        self.rows.tasks()
    }

    async fn find_task_by_plan(&self, plan: &str) -> Option<Task> {
        self.rows.task_by_plan(plan)
    }
}

#[async_trait::async_trait]
impl super::super::AgentRead for Store {
    async fn list_agents(&self) -> Vec<Agent> {
        self.rows.agents()
    }

    async fn current_agent(&self, task: TaskId) -> Option<Agent> {
        self.rows.current_agent(task)
    }
}

#[async_trait::async_trait]
impl TaskCrud for Store {
    async fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId> {
        // EVERY CREATE NEEDS A SETTLED IDENTITY NOW, not only an epic-less
        // one — `sync.allium: CreatesRequireASettledIdentity`. An epic-less
        // task still needs it for `owner` (`core.allium:
        // OwnerTracksUserBoardTask`); a task landing in an epic needs it for
        // `created_by`, which survives epic membership and is how
        // `sync.allium`'s `own_creations` subscription finds this task
        // regardless of which epic it lands in.
        let identity = self
            .require_identity("there is no name to stamp on a new task; it was not created")
            .await?;
        let owner = if req.epic_id.is_none() {
            identity.as_str()
        } else {
            ""
        };
        let row = encode::create_task_row(&req, owner, &identity, &self.now());
        self.caller.create_task(row).await
    }

    async fn create_task_signalled(&self, req: CreateTaskRequest<'_>) -> Result<TaskId> {
        let identity = self
            .require_identity("there is no name to stamp on a new task; it was not created")
            .await?;
        let owner = if req.epic_id.is_none() {
            identity.as_str()
        } else {
            ""
        };
        let row = encode::create_task_row(&req, owner, &identity, &self.now());
        self.caller.create_task_signalled(row).await
    }

    async fn respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        req: CreateTaskRequest<'_>,
        labels: &[String],
    ) -> Result<TaskId> {
        let identity = self
            .require_identity(
                "there is no name to stamp on a phoenix successor; it was not created",
            )
            .await?;
        let owner = if req.epic_id.is_none() {
            identity.as_str()
        } else {
            ""
        };
        let mut row = encode::create_task_row(&req, owner, &identity, &self.now());
        row.labels = serde_json::to_string(labels).unwrap_or_else(|_| "[]".to_string());
        self.caller
            .respawn_phoenix_successor(predecessor, row)
            .await
    }

    /// tasks.allium: `DeleteTask`. A feed task's delete retires its
    /// `external_id` under its `nearest_feed_epic` (core/Epic) in the same
    /// reducer transaction as the row delete — a cycle landing between the two
    /// would otherwise re-insert. A second delete of an already-retired id
    /// (core/RetiredFeedItem: `UniqueRetiredFeedItemPerFeed`) is a no-op on
    /// the record rather than a failed delete. A manual task (no
    /// `external_id`) or one under no feed epic in its chain retires nothing —
    /// there is no cycle to suppress.
    async fn delete_task(&self, id: TaskId) -> Result<()> {
        self.caller.delete_task(id).await?.applied()
    }

    /// tasks.allium: `BatchDelete`'s atomic counterpart to `delete_task`/
    /// `delete_epic` looped per item — one reducer call over both domains,
    /// which is where the joint guard runs. See
    /// `spacetime/module/src/tasks_epics.rs::batch_delete`'s doc comment.
    async fn batch_delete(&self, task_ids: &[TaskId], epic_ids: &[EpicId]) -> Result<()> {
        let task_ids = task_ids.to_vec();
        let epic_ids = epic_ids.to_vec();
        self.caller
            .batch_delete(task_ids, epic_ids)
            .await?
            .applied()
    }

    async fn patch_task(&self, id: TaskId, patch: &TaskPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        // Whether the sub-status is legal for the status is the store's to say
        // (`patch_task`'s check), not this client's: a second copy here was a
        // validator free to disagree with the real one. An empty patch is
        // answered above because it is about the argument, and a round trip to
        // be told so is a round trip for nothing. Every check that needs to see
        // other rows is the store's — see
        // `sync.allium: StoreRejectsAnInvalidMutation`.
        self.caller
            .patch_task(id, encode::task_patch(patch))
            .await?
            .applied()
    }

    async fn update_task(
        &self,
        id: TaskId,
        epic: Option<Option<EpicId>>,
        patch: &TaskPatch<'_>,
    ) -> Result<()> {
        let owner = match epic {
            Some(target) => self.owner_for_epic_move(target).await?,
            None => String::new(),
        };
        self.caller
            .update_task(id, epic, owner, encode::task_patch(patch))
            .await?
            .applied()
    }

    async fn ingest_feed(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
        additive: bool,
    ) -> Result<FeedIngestOutcome> {
        self.ingest_feed_inner(epic_id, items, repo_paths, base_branches, additive)
            .await
    }

    /// `Applied`/`Refused` map straight onto `true`/`false` via
    /// [`ReducerOutcome::won`] — no read-back needed.
    async fn mark_pr_learnings_gate_shown(&self, id: TaskId) -> Result<bool> {
        Ok(self
            .caller
            .mark_pr_learnings_gate_shown(id, self.clock.now())
            .await?
            .won())
    }

    // -- Agent session state ---------------------------------------------------
    //
    // `docs/specs/agent-health.allium` is the reference. `started_at`/
    // `last_pre_tool_use_at`/`last_notification_at`/`stop_pending_at` are all
    // EVENT times — this connection's `now`, the instant the hook fired — not
    // the store's `updated_at`.

    async fn subagent_start(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64> {
        self.caller
            .subagent_start(id, agent_id.to_string(), session_id.to_string(), now)
            .await
    }

    async fn subagent_stop(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
    ) -> Result<SubagentDrain> {
        let prior_running = self.prior_status_was(id, TaskStatus::Running);
        let read = self
            .caller
            .subagent_stop(id, agent_id.to_string(), session_id.to_string())
            .await?;
        Ok(Self::drain_outcome(prior_running, read))
    }

    async fn subagent_clear(&self, id: TaskId) -> Result<SubagentDrain> {
        let prior_running = self.prior_status_was(id, TaskStatus::Running);
        let read = self.caller.subagent_clear(id).await?;
        Ok(Self::drain_outcome(prior_running, read))
    }

    async fn subagent_clear_and_void_pending_stop(&self, id: TaskId) -> Result<()> {
        self.caller
            .subagent_clear_and_void_pending_stop(id)
            .await?
            .applied()
    }

    /// `Refused` reads as `NoOp` (the task was not `Running`); once accepted,
    /// `Flipped` and `Deferred` are unambiguous from the row the module reads
    /// back — see `try_record_stop`'s own doc comment in the module for why.
    async fn try_record_stop(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<StopOutcome> {
        Ok(match self.caller.try_record_stop(id, now).await? {
            None => StopOutcome::NoOp,
            Some(true) => StopOutcome::Flipped,
            Some(false) => StopOutcome::Deferred,
        })
    }

    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        self.caller
            .record_pre_tool_use(id, sub_status, now)
            .await?
            .applied()
    }

    async fn record_notification(
        &self,
        id: TaskId,
        write: NotificationWrite,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        // Ignore never reaches the store, which saves a round trip for the
        // commonest kind (`auth_success`).
        if write == NotificationWrite::Ignore {
            return Ok(());
        }
        self.caller
            .record_notification(id, write, now)
            .await?
            .applied()
    }

    /// Resumed vs. Refreshed is NOT decodable from the row the module writes
    /// — both write the identical field set (`status = running`, `sub_status
    /// = active`, `last_pre_tool_use_at`). Classified instead from a PRE-read
    /// (was the task in `Review` just before this call — the same kind of
    /// read [`Store::prior_status_was`] does for the drain methods, just
    /// testing the other status) taken before the call; advisory only,
    /// because the reducer decides `resumed` from its own unambiguous view
    /// and recalculates the epic itself when it is true.
    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<UserPromptOutcome> {
        let prior_review = self.prior_status_was(id, TaskStatus::Review);
        // Both timestamps are the SAME event time: the store side uses the
        // millis format for both, since parsing tolerates the extra precision
        // either way.
        match self.caller.record_user_prompt_submit(id, now, now).await? {
            ReducerOutcome::Refused(_) => Ok(UserPromptOutcome::NoOp),
            ReducerOutcome::Applied(_) => Ok(if prior_review {
                UserPromptOutcome::Resumed
            } else {
                UserPromptOutcome::Refreshed
            }),
        }
    }

    async fn batch_patch_sub_status(&self, updates: &[(TaskId, SubStatus)]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }
        self.caller
            .batch_patch_sub_status(updates.to_vec())
            .await?
            .applied()
    }

    // Retired feed items.
    async fn retired_without_task(
        &self,
        feed_epic_id: EpicId,
        external_ids: &[String],
    ) -> Result<Vec<String>> {
        Ok(self.rows.retired_without_task(feed_epic_id, external_ids))
    }
}

#[async_trait::async_trait]
impl super::super::PollOwnershipStore for Store {
    /// `core.allium: PollOwner`. Fills an absent row; claims as this host.
    async fn claim_poll_owner(&self, target: crate::models::PollScopeId) -> Result<()> {
        self.caller
            .claim_poll_owner(target, self.host.clone())
            .await?
            .applied()
    }

    /// Unconditionally reassigns an existing row to this host.
    async fn override_poll_owner(&self, target: crate::models::PollScopeId) -> Result<()> {
        self.caller
            .override_poll_owner(target, self.host.clone())
            .await?
            .applied()
    }
}
