use anyhow::Result;

use crate::models::{
    EpicId, FeedItem, NotificationWrite, StopOutcome, SubStatus, SubagentDrain, Task, TaskId,
    TaskStatus, UserPromptOutcome,
};
use crate::sync::{encode, ReducerOutcome};

use super::super::{CreateTaskRequest, RemovedFeedTask, Store, TaskCrud, TaskPatch};
use super::won;

#[async_trait::async_trait]
impl super::super::TaskRead for Store {
    async fn get_task(&self, id: TaskId) -> Result<Option<Task>> {
        Ok(self.rows.task(id))
    }

    async fn task_exists(&self, id: TaskId) -> Result<bool> {
        Ok(self.rows.has_task(id))
    }

    async fn list_live_agent_tasks(&self) -> Result<Vec<Task>> {
        Ok(self.rows.live_agent_tasks())
    }

    async fn list_all(&self) -> Result<Vec<Task>> {
        Ok(self.rows.tasks())
    }

    async fn find_task_by_plan(&self, plan: &str) -> Result<Option<Task>> {
        Ok(self.rows.task_by_plan(plan))
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
        self.caller
            .patch_task(id, encode::task_patch(patch))
            .await?
            .applied()
    }

    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<Vec<RemovedFeedTask>> {
        self.upsert_feed_tasks_inner(epic_id, items, repo_paths, base_branches, true)
            .await
    }

    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<Vec<RemovedFeedTask>> {
        self.upsert_feed_tasks_inner(epic_id, items, repo_paths, base_branches, false)
            .await
    }

    /// Predicts nothing: unlike the two upserts, this delete has no items to
    /// compute a keep-set from — `keep_external_ids` IS the keep-set.
    ///
    /// One scan of every task, not one `tasks_for_epic` call per child: the
    /// rows have no per-epic index, so asking once per child epic would scan
    /// every task in the board's view once per child — O(children × tasks)
    /// instead of O(tasks). A single scan filtered by the child-epic id set
    /// stays O(tasks) regardless of subtree fan-out.
    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        keep_external_ids: &[String],
    ) -> Result<Vec<RemovedFeedTask>> {
        let children: std::collections::HashSet<EpicId> = self
            .rows
            .epics_with_parent(Some(parent_id))
            .into_iter()
            .map(|e| e.id)
            .collect();
        let keep: std::collections::HashSet<&str> =
            keep_external_ids.iter().map(String::as_str).collect();
        let candidates: Vec<Task> = self
            .rows
            .tasks()
            .into_iter()
            .filter(|t| t.epic_id.is_some_and(|e| children.contains(&e)))
            .filter(|t| matches!(&t.external_id, Some(e) if !keep.contains(e.as_str())))
            .collect();

        self.caller
            .delete_stale_subtree_feed_tasks(parent_id, keep_external_ids.to_vec())
            .await?
            .applied()?;

        Ok(self.confirm_removed(candidates))
    }

    /// `Applied`/`Refused` map straight onto `true`/`false` via
    /// [`ReducerOutcome::won`] — the exact `try_claim_backlog_task` shape, no
    /// read-back needed.
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

    /// Offer the epic's backlog subtasks to the store, in order, until one is
    /// won. `now` is dropped: the store stamps its own clock, so two hosts'
    /// claims are ordered by one clock rather than by whose laptop is fast.
    ///
    /// # Why the candidate is chosen HERE and the claim is arbitrated THERE
    ///
    /// A reducer returns no value, so a store-side "pick the next one and claim
    /// it" could not tell this caller WHICH task it got — and the caller is
    /// about to provision that task's worktree. So the choosing happens here.
    ///
    /// That is safe, and it is worth being precise about why, because the
    /// neighbouring decision went the other way. An epic's STATUS has to be
    /// derived at the store because its children can be anywhere and no board
    /// sees all of them. Its BACKLOG SUBTASKS are different: a subscription to
    /// an epic is `WHERE epic_id = N`, which returns every subtask of it
    /// regardless of owner, so a board that is chaining an epic can see the
    /// whole candidate list. The ordering decision is made on complete
    /// information — read from the same rows the board draws, so the chain
    /// takes the task the column shows as next.
    ///
    /// Exclusivity is still the store's. Each offer is a real claim that the
    /// store refuses if another host took it first, and the loop simply moves
    /// on — so two boards racing down the same list end up on different tasks
    /// rather than the same one (`dispatch.allium: DispatchClaimExclusive`).
    ///
    /// `phoenix` rows are passed over: `epics.allium: PhoenixIsNeverChained`. A
    /// recurring subtask respawns on completion, so chaining its successor
    /// would launch an agent at it immediately, forever.
    async fn try_claim_next_backlog_task(
        &self,
        epic_id: EpicId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<TaskId>> {
        let candidates: Vec<TaskId> = self
            .rows
            .tasks_for_epic(epic_id)
            .into_iter()
            .filter(|t| {
                t.status == TaskStatus::Backlog
                    && !t.phoenix
                    && t.is_locally_owned(Some(&self.host))
            })
            .map(|t| t.id)
            .collect();
        // `tasks_for_epic` already sorts by `sort_key`, which is
        // `COALESCE(sort_order, id)` — the order the board draws them in, and
        // the order the chain must take them in.

        for id in candidates {
            if self.try_claim_backlog_task(id, now).await? {
                return Ok(Some(id));
            }
        }
        // Nothing left is the ordinary end of a chain, not a failure. A store
        // that was DOWN produced an `Err` from the first offer above.
        Ok(None)
    }

    /// A REFUSAL HERE IS AN ANSWER, NOT A FAULT.
    ///
    /// `Ok(false)` means another host claimed it first, which is the ordinary
    /// outcome of a race and the one the caller handles by provisioning
    /// nothing. A store that is DOWN still produces `Err`, because the
    /// transport failed rather than the reducer — see [`ReducerOutcome`] for
    /// why the two are kept apart.
    ///
    /// `now` is dropped: the store stamps its own clock, so two hosts' claims
    /// are ordered by one clock rather than by whose laptop is fast.
    async fn try_claim_backlog_task(
        &self,
        id: TaskId,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        Ok(won(
            self.caller
                .claim_backlog_task(id, self.host.clone())
                .await?,
            "claim",
            id,
        ))
    }

    /// Moves Running -> Backlog without clearing `stop_pending`, and safely so:
    /// the `worktree IS NULL` guard restricts it to a claim that never finished
    /// provisioning, so no agent ever ran and no Stop hook can have fired for
    /// this claim. The claim itself also cleared the bit on the way in (see
    /// `DispatchTask` in `docs/specs/dispatch.allium`). Any relaxation of that
    /// guard has to clear it — see `PendingStopOnlyWhileRunning`
    /// (`docs/specs/core.allium`).
    async fn try_release_backlog_claim(&self, id: TaskId) -> Result<bool> {
        Ok(won(
            self.caller.release_backlog_claim(id).await?,
            "release",
            id,
        ))
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

    async fn create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()> {
        self.caller
            .create_task_watcher(watcher_task_id, target_task_id)
            .await?
            .applied()
    }

    async fn delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()> {
        self.caller
            .delete_task_watcher(watcher_task_id, target_task_id)
            .await?
            .applied()
    }

    async fn list_watchers_of(&self, target_task_id: TaskId) -> Result<Vec<TaskId>> {
        Ok(self.rows.watchers_of(target_task_id))
    }

    async fn delete_watches_of_target(&self, target_task_id: TaskId) -> Result<()> {
        self.caller
            .delete_watches_of_target(target_task_id)
            .await?
            .applied()
    }

    async fn delete_watches_by_watcher(&self, watcher_task_id: TaskId) -> Result<()> {
        self.caller
            .delete_watches_by_watcher(watcher_task_id)
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

    async fn drop_closed_retired_feed_items(
        &self,
        feed_epic_id: EpicId,
        keep_external_ids: &[String],
    ) -> Result<()> {
        self.caller
            .drop_closed_retired_feed_items(feed_epic_id, keep_external_ids.to_vec())
            .await?
            .applied()
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
