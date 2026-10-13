//! The [`Store`] trait implementations, one file per domain: each read
//! answers from the subscription's rows, each write encodes its row and sends
//! it as a reducer call. The helpers here are the parts several writes share.

mod board;
mod dispatch_requests;
mod epics;
mod learnings;
mod pr_status;
mod session_requests;
mod settings;
mod tasks;
mod usage;
mod work_packages;

pub(crate) use settings::{HOST_ID_KEY, HOST_LABEL_KEY, USER_IDENTITY_KEY};

use anyhow::Result;

use crate::models::{EpicId, SubagentDrain, TaskId, TaskStatus};
use crate::spacetime::bindings;
use crate::sync::encode;
use crate::sync::writes::DrainReadBack;

use super::Store;

impl Store {
    /// This board's clock, in the store's timestamp format.
    ///
    /// The CREATE timestamps are the client's, deliberately, and they are the
    /// only ones that are. `created_at` records when the person asked, which is
    /// a fact about this machine; everything a reducer derives afterwards —
    /// `updated_at`, `completed_at`, the claim's seeded `last_pre_tool_use_at`
    /// — uses the store's clock, so that two boards' rows are ordered by one
    /// clock rather than by whose laptop is fast.
    fn now(&self) -> String {
        encode::stamp(self.clock.now())
    }

    /// This connection's own proven identity, or a refusal naming what could
    /// not happen without one.
    ///
    /// Every call site here needs the same thing — a name THIS connection has
    /// settled, to stamp on a row or send in a mutation — and differs only in
    /// what it was trying to do. `unable_to` is the tail of the refusal
    /// message, read as "...so {unable_to}".
    ///
    /// Deliberately `self.identity.user()`, the live per-connection cell
    /// (`sync.allium: SubscribeOnceIdentityIsSettled`), not a persisted
    /// setting read elsewhere. A persisted value can predate this connection's
    /// own handshake.
    async fn require_identity(&self, unable_to: &str) -> Result<String> {
        self.identity
            .user()
            .await?
            .ok_or_else(|| anyhow::anyhow!("this board has no user identity yet, so {unable_to}"))
    }

    /// Whether `id` was in status `want` just before an agent-session-state
    /// call, read from `rows` before the reducer runs.
    ///
    /// `subagent_stop`, `subagent_clear` and `record_user_prompt_submit`
    /// need it because their answer includes a bit (did this drain a deferred
    /// Stop; was this call a resume or a refresh) that is only decodable from
    /// a row already known to have been in that status before the call, and a
    /// reducer returns no value to say what it was. The read is advisory, not
    /// authoritative — the reducer decides and recalculates from its own view
    /// (docs/plans/2026-09-21-phase-6b-agent-session-state-reducers.md,
    /// decision 3).
    ///
    /// A task this board cannot see reads as `false`: the worst that does is
    /// under-report a drain/resume the reducer already applied and
    /// recalculated correctly on its own.
    fn prior_status_was(&self, id: TaskId, want: TaskStatus) -> bool {
        matches!(self.rows.task(id), Some(t) if t.status == want)
    }

    /// Combine a drain's pre-read (taken BEFORE the reducer call, via
    /// [`Self::prior_status_was`]) with the module's post-transaction
    /// read-back into the `SubagentDrain` the caller wants. `read.is_review`
    /// alone is never enough — see
    /// [`Self::prior_status_was`]'s doc comment.
    fn drain_outcome(prior_running: bool, read: DrainReadBack) -> SubagentDrain {
        SubagentDrain {
            live: read.live,
            applied_pending_stop: prior_running && read.is_review,
        }
    }

    /// Shared body of `upsert_feed_tasks`/`upsert_feed_tasks_additive`.
    /// `delete_absent` selects the stale-delete pass.
    ///
    /// The predict-then-verify shape (this task's plan doc, decision 1): a
    /// reducer cannot report which rows it deleted, and a deleted row is gone
    /// from `ctx.db` by the time anything could match against it — unlike a
    /// CREATE's id, there is no later state to read. So this reads its own
    /// already-subscribed candidates BEFORE the call, using the identical
    /// predicate the reducer applies, then keeps only the ones CONFIRMED
    /// absent afterward. A candidate that survived (a race) is silently
    /// dropped rather than torn down — never a false positive that could
    /// destroy a worktree the reducer did not actually remove, only a
    /// possible missed teardown, which is the existing best-effort bargain
    /// `cleanup_removed_feed_tasks` already documents.
    async fn upsert_feed_tasks_inner(
        &self,
        epic_id: EpicId,
        items: &[crate::models::FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
        delete_absent: bool,
    ) -> Result<Vec<crate::store::RemovedFeedTask>> {
        if items.len() != repo_paths.len() || items.len() != base_branches.len() {
            anyhow::bail!(
                "upsert_feed_tasks slice length mismatch: items={}, repo_paths={}, base_branches={}",
                items.len(),
                repo_paths.len(),
                base_branches.len()
            );
        }
        let wire_items: Vec<bindings::FeedTaskUpsertItem> = items
            .iter()
            .zip(repo_paths)
            .zip(base_branches)
            .map(|((item, repo_path), base_branch)| {
                encode::feed_task_upsert_item(item, repo_path, base_branch)
            })
            .collect();

        let candidates = if delete_absent {
            let keep: std::collections::HashSet<&str> =
                items.iter().map(|i| i.external_id.as_str()).collect();
            self.feed_removal_candidates(epic_id, &keep)
        } else {
            Vec::new()
        };

        // Best-effort, unlike `require_identity`: a feed sync must not fail
        // just because this install has never connected before (no identity
        // to stamp yet — `core.allium: Task.created_by` is honestly empty in
        // that case, not a call this method refuses). `feeds.allium:
        // UpsertFeedTasks`.
        let created_by = self
            .identity
            .user()
            .await
            .ok()
            .flatten()
            .unwrap_or_default();

        if delete_absent {
            self.caller
                .upsert_feed_tasks(epic_id, wire_items, created_by)
                .await?
                .applied()?;
        } else {
            self.caller
                .upsert_feed_tasks_additive(epic_id, wire_items, created_by)
                .await?
                .applied()?;
        }

        Ok(self.confirm_removed(candidates))
    }

    /// `TaskCrud::ingest_feed`: one reducer call, then the board's own
    /// before-and-after read for the outcome.
    ///
    /// Predict-then-verify as [`Self::upsert_feed_tasks_inner`] documents, over
    /// the whole subtree because the module may delete in any of it. A task the
    /// module MOVED between sub-epics is still in the view afterwards, so it is
    /// never reported as removed. An additive call removes nothing and
    /// predicts nothing.
    async fn ingest_feed_inner(
        &self,
        epic_id: EpicId,
        items: &[crate::models::FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
        additive: bool,
    ) -> Result<crate::store::FeedIngestOutcome> {
        if items.len() != repo_paths.len() || items.len() != base_branches.len() {
            anyhow::bail!(
                "ingest_feed slice length mismatch: items={}, repo_paths={}, base_branches={}",
                items.len(),
                repo_paths.len(),
                base_branches.len()
            );
        }
        let wire_items: Vec<bindings::FeedIngestItem> = items
            .iter()
            .zip(repo_paths)
            .zip(base_branches)
            .map(|((item, repo_path), base_branch)| {
                encode::feed_ingest_item(item, repo_path, base_branch)
            })
            .collect();
        let candidates = if additive {
            Vec::new()
        } else {
            self.subtree_feed_tasks(epic_id)
        };
        let created_by = self
            .identity
            .user()
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        self.caller
            .ingest_feed(epic_id, wire_items, additive, created_by)
            .await?
            .applied()?;
        let mut below: Vec<EpicId> =
            crate::models::descendant_epic_ids(epic_id, &self.rows.epics())
                .into_iter()
                .filter(|id| *id != epic_id)
                .collect();
        below.sort_by_key(|id| id.0);
        let mut affected_epics = vec![epic_id];
        affected_epics.extend(below);
        Ok(crate::store::FeedIngestOutcome {
            affected_epics,
            removed: self.confirm_removed(candidates),
        })
    }

    /// Every feed-managed task (`external_id` set) this board can see in
    /// `epic_id`'s subtree, the epic itself included.
    fn subtree_feed_tasks(&self, epic_id: EpicId) -> Vec<crate::models::Task> {
        let subtree = crate::models::descendant_epic_ids(epic_id, &self.rows.epics());
        subtree
            .into_iter()
            .flat_map(|e| self.rows.tasks_for_epic(e))
            .filter(|t| t.external_id.is_some())
            .collect()
    }

    /// Tasks in `epic_id` this board can currently see whose `external_id` is
    /// set and not in `keep` — the same predicate the reducer's stale-delete
    /// pass applies, read from this connection's own subscription rather
    /// than predicted from nothing.
    fn feed_removal_candidates(
        &self,
        epic_id: EpicId,
        keep: &std::collections::HashSet<&str>,
    ) -> Vec<crate::models::Task> {
        self.rows
            .tasks_for_epic(epic_id)
            .into_iter()
            .filter(|t| matches!(&t.external_id, Some(e) if !keep.contains(e.as_str())))
            .collect()
    }

    /// The verify half of predict-then-verify: keep only candidates
    /// confirmed gone from this connection's view after the reducer call
    /// returned.
    fn confirm_removed(
        &self,
        candidates: Vec<crate::models::Task>,
    ) -> Vec<crate::store::RemovedFeedTask> {
        let mut removed = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            if !self.rows.has_task(candidate.id) {
                removed.push(crate::store::RemovedFeedTask {
                    id: candidate.id,
                    repo_path: candidate.repo_path,
                    worktree: candidate.worktree,
                    tmux_window: candidate.tmux_window,
                });
            }
        }
        removed
    }
}
