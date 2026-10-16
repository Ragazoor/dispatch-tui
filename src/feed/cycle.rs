//! The one feed cycle, shared by both surfaces that can start one.
//!
//! A FEED CYCLE is the whole sequence a feed request performs for one epic:
//! claim the epic, read its current feed configuration, exec the command, parse
//! stdout, apply the degraded-emission guard, dispatch the sync, and tear down
//! the worktrees of every task the sync removed. [`FeedCycle::run`] owns all of
//! it; its two callers — [`crate::feed::FeedRunner`]'s auto-poll job and the
//! manual "r" refresh in `src/runtime/epics.rs::exec_trigger_epic_feed` —
//! differ only in how they present the [`FeedCycleOutcome`].
//!
//! That single-function discipline is deliberate and load-bearing: the feed
//! pipeline has repeatedly grown a step that had to behave identically on both
//! paths (the shared exec, the shared parse, the role dispatcher, the teardown,
//! and now the serialisation claim), and every time it was added twice it drifted.
//! A new step belongs here, not in the two callers.
//!
//! See feeds.allium `SerialisedFeedCycle`.

use std::sync::Arc;
use std::time::Duration;

use super::guard::FeedSyncGuard;
use crate::dispatch::resolve_feed_item_repo_paths;
use crate::models::EpicId;
use crate::process::ProcessRunner;
use crate::store::TaskStore;

/// What a feed cycle did, for its caller to present.
pub(crate) enum FeedCycleOutcome {
    /// The sync ran. `count` is the number of items the command emitted (not
    /// the number of tasks written), and `affected_epics` is every epic whose
    /// contents may have changed, for notification.
    Synced {
        count: usize,
        affected_epics: Vec<EpicId>,
        /// `Some(reason)` when the sync ran ADDITIVELY because the command
        /// wrote to stderr — it removed nothing, so a caller that presents this
        /// outcome must say so rather than let it read as a full reconcile.
        /// See feeds.allium: `DegradedNonEmptyEmission`.
        ///
        /// `None` does NOT mean the cycle reconciled. An append-only epic is
        /// additive by configuration and reports no reason, deliberately: see
        /// feeds.allium `AppendOnlyFeed` for why that case is not surfaced.
        /// This field answers "was anything WRONG with this emission", not
        /// "did this cycle remove".
        degraded: Option<String>,
    },
    /// A cycle for this epic was already in flight, so this request did nothing
    /// at all — no exec, no sync, no teardown.
    Busy,
    /// The cycle stopped early. The string is already logged by the time it is
    /// returned; it is carried so the manual path can put it in the status bar.
    Failed(String),
}

/// One feed cycle for one epic.
///
/// `feed_command` and `feed_append_only` are deliberately NOT fields: they are
/// read from the epic inside [`run`](Self::run), after the claim, so neither
/// caller can act on a stale snapshot of them. `feed_role` and `group_by_repo`
/// are not read here at all: the module reads them from the epic's own row. Anything else the cycle ACTS on belongs on that list, not
/// in this struct.
pub(crate) struct FeedCycle {
    pub(crate) db: Arc<dyn TaskStore>,
    pub(crate) runner: Arc<dyn ProcessRunner>,
    pub(crate) guard: Arc<FeedSyncGuard>,
    pub(crate) epic_id: EpicId,
    /// Presentation only — status-bar strings and log fields. Never decides
    /// behaviour, so a stale title is harmless.
    pub(crate) epic_title: String,
    /// `Some` from the auto-poll path, which fetches the repo-path list once
    /// per tick and shares it across every epic's job. `None` from the manual
    /// path, resolved inside after the claim so a dropped request does no DB
    /// work.
    pub(crate) known_paths: Option<Arc<Vec<String>>>,
    /// Deadline passed to [`super::exec_feed_command`]. Both production
    /// callers pass `exec::FEED_COMMAND_TIMEOUT`; tests inject a short value
    /// so a deliberately-hung command times out fast instead of making the
    /// suite wait out the real production deadline.
    pub(crate) command_timeout: Duration,
}

impl FeedCycle {
    /// Run the cycle. Logs every failure itself — it holds the epic id and
    /// title, and logging here rather than in the callers is what stops a
    /// failure being logged twice or not at all.
    pub(crate) async fn run(self) -> FeedCycleOutcome {
        // Claim first: everything below, including the exec, is inside the
        // critical section. `_claim` releases on drop, so every `return` path
        // below — and a panic — frees the epic.
        let Some(_claim) = self.guard.try_claim(self.epic_id) else {
            tracing::debug!(
                epic_id = self.epic_id.0,
                epic_title = %self.epic_title,
                "feed: a cycle for this epic is already in flight; dropping this request"
            );
            return FeedCycleOutcome::Busy;
        };

        let Some(mut epic) = self.db.get_epic(self.epic_id).await else {
            return self.epic_gone();
        };
        let Some(feed_command) = epic.feed_command.take() else {
            return self.fail("epic has no feed command");
        };

        let output = match super::exec_feed_command(
            &feed_command,
            self.epic_id.0,
            &self.epic_title,
            self.command_timeout,
        )
        .await
        {
            Ok(output) => output,
            // exec_feed_command logs spawn failures, timeouts, non-zero exits
            // and stderr-on-success itself, so this is already in app.log.
            Err(err) => return FeedCycleOutcome::Failed(err),
        };

        let items = match super::parse_feed_items(&output.stdout) {
            Ok(items) => items,
            Err(err) => return self.fail(format!("failed to parse JSON output: {err:#}")),
        };

        let count = items.len();
        let (mode, degraded) =
            match self.classify_emission(count, &output.stderr, epic.feed_append_only) {
                Ok(v) => v,
                Err(outcome) => return outcome,
            };

        let outcome = match self.sync_emission(items, mode).await {
            Ok(outcome) => outcome,
            Err(err) => return self.fail_sync(err).await,
        };

        // Teardown before the caller notifies, on BOTH surfaces: a notification
        // means reconciled AND cleaned up, so the board never shows a row gone
        // while its worktree is still being removed. feeds.allium
        // RoleRoutedFeedSync, teardown-vs-notification.
        super::cleanup_removed_feed_tasks(self.runner.clone(), outcome.removed).await;

        FeedCycleOutcome::Synced {
            count,
            affected_epics: outcome.affected_epics,
            degraded,
        }
    }

    /// Decide how this emission syncs: `Err` is a degraded empty emission
    /// (already logged), `Ok` carries the mode and, for a partially degraded
    /// emission, the reason to report.
    fn classify_emission(
        &self,
        item_count: usize,
        stderr: &str,
        feed_append_only: bool,
    ) -> Result<(super::SyncMode, Option<String>), FeedCycleOutcome> {
        // A zero-item emission that also wrote to stderr is a degraded run, not
        // an empty one: syncing it would delete every feed task in this epic's
        // subtree. feeds.allium: DegradedEmptyEmission.
        if let Some(reason) = super::degraded_empty_emission(item_count, stderr) {
            return Err(self.fail(reason));
        }

        // A NON-empty emission that also wrote to stderr is partially degraded:
        // trustworthy about what it contains, not about what it omits. It syncs,
        // but additively — no stale delete, and so no teardown of a live review
        // agent whose PR one soft-failed sub-query happened to drop.
        // feeds.allium: DegradedNonEmptyEmission.
        let degraded = super::degraded_partial_emission(item_count, stderr);
        if let Some(reason) = &degraded {
            tracing::warn!(
                epic_id = self.epic_id.0,
                epic_title = %self.epic_title,
                "feed: syncing additively, no removals this cycle: {reason}"
            );
        }
        // The two causes of additivity are independent and neither overrides
        // the other — additive is their union. `degraded` says this EMISSION is
        // not trusted; `feed_append_only` says this EPIC never mirrors at all,
        // because its source emits events that are never retracted. Only the
        // former is reported: withholding removals is a symptom there and the
        // configured behaviour here. feeds.allium: DegradedNonEmptyEmission,
        // AppendOnlyFeed.
        let mode = if degraded.is_some() || feed_append_only {
            super::SyncMode::Additive
        } else {
            super::SyncMode::Reconcile
        };
        Ok((mode, degraded))
    }

    /// Resolve each item's repo and base branch from local clones, then post
    /// the lot to the store, which decides what it does.
    async fn sync_emission(
        &self,
        items: Vec<crate::models::FeedItem>,
        mode: super::SyncMode,
    ) -> anyhow::Result<crate::store::FeedIngestOutcome> {
        let known_paths = match &self.known_paths {
            Some(paths) => Arc::clone(paths),
            None => Arc::new(self.db.list_repo_paths().await),
        };
        let repo_paths = resolve_feed_item_repo_paths(&items, &known_paths);
        let base_branches = super::resolve_base_branches(&repo_paths, &*self.runner);
        let entries = super::FeedItemWithTarget::zip(items, repo_paths, base_branches);

        super::ingest_emission(&*self.db, self.epic_id, entries, mode).await
    }

    /// The sync failed. If the epic was deleted while the command ran (it can
    /// run for minutes), the store's refusal is that race, not a fault.
    async fn fail_sync(&self, err: anyhow::Error) -> FeedCycleOutcome {
        if self.db.get_epic(self.epic_id).await.is_none() {
            return self.epic_gone();
        }
        self.fail(format!("{err:#}"))
    }

    /// The epic was deleted under the cycle. Expected, so debug not warn; the
    /// next tick no longer lists the epic. feeds.allium `FeedCommandFailure`
    /// bucket 5.
    fn epic_gone(&self) -> FeedCycleOutcome {
        tracing::debug!(
            epic_id = self.epic_id.0,
            epic_title = %self.epic_title,
            "feed cycle dropped: epic no longer exists"
        );
        FeedCycleOutcome::Failed("epic no longer exists".to_string())
    }

    /// Log a failure against this cycle's epic and return it for the caller to
    /// present. Every `Failed` outcome except the exec's (which
    /// `exec_feed_command` has already logged) goes through here.
    fn fail(&self, reason: impl Into<String>) -> FeedCycleOutcome {
        let reason = reason.into();
        tracing::warn!(
            epic_id = self.epic_id.0,
            epic_title = %self.epic_title,
            "feed cycle failed: {reason}"
        );
        FeedCycleOutcome::Failed(reason)
    }
}

#[cfg(test)]
mod tests;
