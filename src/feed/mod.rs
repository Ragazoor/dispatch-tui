mod cycle;
mod exec;
mod guard;
mod ingest;
mod parse;
mod routing;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::board_event::BoardEvent;
use crate::models::{decide_poll_action, PollAction};
use crate::models::{Epic, EpicId, MIN_FEED_INTERVAL_SECS};
use crate::process::ProcessRunner;
use crate::store::{RemovedFeedTask, TaskStore};

pub(crate) use cycle::{FeedCycle, FeedCycleOutcome};
pub(crate) use exec::degraded_partial_emission;
pub(crate) use exec::{
    degraded_empty_emission, exec_feed_command, resolve_base_branches, FEED_COMMAND_TIMEOUT,
};
pub(crate) use guard::FeedSyncGuard;
pub(crate) use ingest::{run_feed_sync_by_role, FeedItemWithTarget, SyncMode};
// `pub`, unlike the `pub(crate)` re-exports above: the `verify-feed` CLI in
// src/main.rs is a separate bin crate and is one of this function's three
// callers. See feeds.allium's FeedItemParse block.
pub use parse::parse_feed_items;
pub use routing::{excluded_from_reviews, route};

/// Log-and-discard on `Err` for the feed writes that report what they deleted,
/// keeping the `Ok` payload so the removed rows reach
/// [`cleanup_removed_feed_tasks`]. An `Err` yields an empty vec — nothing was
/// reported, so there is nothing to tear down; the reconciliation pass
/// continues either way.
///
/// `context` labels the call site in the log line (e.g.
/// `"sync_grouped_feed: upsert_feed_tasks failed"`).
pub(crate) fn removed_or_warn(
    result: anyhow::Result<Vec<RemovedFeedTask>>,
    epic_id: EpicId,
    sub_epic_id: Option<EpicId>,
    context: &str,
) -> Vec<RemovedFeedTask> {
    match result {
        Ok(removed) => removed,
        Err(err) => {
            // `sub_epic_id` is attached when the write is scoped to a sub-epic
            // beneath `epic_id`, so a warning names the epic it actually wrote.
            match sub_epic_id {
                Some(sub_epic_id) => tracing::warn!(
                    epic_id = epic_id.0,
                    sub_epic_id = sub_epic_id.0,
                    "{context}: {err:#}"
                ),
                None => tracing::warn!(epic_id = epic_id.0, "{context}: {err:#}"),
            }
            Vec::new()
        }
    }
}

/// Recalculate an epic's status after feed tasks have been upserted, logging a
/// warning on failure. New non-done tasks can cause a done epic to regress to
/// backlog; the recalculation propagates upward to any parent epic.
///
/// `context` labels the call site in the log line (e.g. `"FeedRunner"`).
pub(crate) async fn recalculate_epic_status_after_feed(
    db: &dyn TaskStore,
    epic_id: EpicId,
    context: &str,
) {
    if let Err(err) = db.recalculate_epic_status(epic_id).await {
        tracing::warn!(
            epic_id = epic_id.0,
            "{context}: recalculate_epic_status failed: {err:#}"
        );
    }
}

/// Tear down the worktree and tmux window of every feed task a sync removed.
///
/// A feed-driven removal is a deletion like any other and owes the same
/// teardown `DeleteTask` performs — `TaskTeardown` at the head of the delete
/// section of `docs/specs/tasks.allium`: kill the tmux window, remove the git
/// worktree, and delete the branch best-effort.
/// `crate::dispatch::teardown_task` performs all three, and which of them a given
/// row owes is *its* decision, not this function's — see
/// `TeardownIsOwedWheneverThereIsSomethingToRelease` in that spec. This wrapper
/// contributes exactly two policies: per-repo serialisation, and warn-on-failure.
///
/// # Why there is no shared-worktree check here
///
/// `TaskTeardown`'s worktree clause is unconditional, on this path and every
/// other — see `WorktreeIsNeverShared` in `docs/specs/tasks.allium` for why no
/// two tasks can name one worktree.
///
/// Worth stating here because this function is the tempting place to "restore" a
/// safety net: do **not**. Beyond buying nothing, it would cost a store handle
/// this function does not currently take, a round-trip per removed task, and an
/// error path with no correct answer. The tripwire against a reinstated guard now
/// lives on the one wrapper that *does* hold a store handle — see that spec
/// block's coverage note.
///
/// # Per-repo serialisation
///
/// Removals are grouped by `repo_path` and run sequentially within a repo.
/// `teardown_task` shells `git -C <repo> worktree remove --force` and
/// `git branch -D` against the *shared* checkout, and a reviews epic's tasks
/// overwhelmingly share one repo — running those concurrently would contend on
/// that repo's index lock and fail spuriously. Different repos have no shared
/// lock, so they still proceed in parallel.
///
/// The whole of `TaskTeardown` is best-effort: failures are logged at warn and
/// never surfaced, because feed reconciliation is background work, and one
/// task's failure must not abort the rest of its repo's queue.
/// Called once, from [`cycle::FeedCycle::run`], with the `removed` half of the
/// [`ingest::FeedSyncOutcome`] the sync returned — so both feed paths get the
/// teardown by sharing that cycle rather than by each remembering to call this.
pub(crate) async fn cleanup_removed_feed_tasks(
    runner: Arc<dyn ProcessRunner>,
    removed: Vec<RemovedFeedTask>,
) {
    if removed.is_empty() {
        return;
    }

    let mut by_repo: HashMap<String, Vec<RemovedFeedTask>> = HashMap::new();
    for task in removed {
        by_repo
            .entry(task.repo_path.clone())
            .or_default()
            .push(task);
    }

    let mut handles = Vec::new();
    for (_repo, tasks) in by_repo {
        let runner = runner.clone();
        handles.push(tokio::task::spawn_blocking(move || {
            for task in tasks {
                if let Err(failure) = crate::dispatch::teardown_task(
                    &task.repo_path,
                    task.worktree.as_deref(),
                    task.tmux_window.as_ref(),
                    &*runner,
                ) {
                    tracing::warn!(
                        task_id = task.id.0,
                        "feed cleanup: teardown_task failed: {failure}"
                    );
                }
            }
        }));
    }
    for handle in handles {
        // tokio does not log a `spawn_blocking` panic, and the default panic
        // hook writes to stderr — which belongs to the TUI, so that output is
        // lost or garbles the display. Log it ourselves, to the app log.
        if let Err(err) = handle.await {
            tracing::warn!("feed cleanup: teardown thread did not complete: {err}");
        }
    }
}

/// Cadence for a feed epic with no explicit `feed_interval_secs` —
/// `config.default_feed_interval` in `docs/specs/core.allium`.
///
/// Equal to [`MIN_FEED_INTERVAL_SECS`] today and MUST NOT fall below it: the
/// floor binds the *resolved* cadence, so an unset field that polled faster
/// than the fastest value a user may enter would make the floor a lie.
/// `the_default_interval_clears_the_floor` asserts the relation
/// (feeds.allium: `DefaultFeedIntervalClearsTheFloor`). Raising this is safe;
/// lowering it past the floor is not.
///
/// Kept a separate constant rather than derived from the floor: a future
/// default of 120s must not drag the floor up with it.
const DEFAULT_FEED_INTERVAL: Duration = Duration::from_secs(60);

/// Poll interval for the background feed task.
/// Kept in `feed` (not reusing `TICK_INTERVAL` from `runtime`) so the two
/// concerns stay independent.
const FEED_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Whether `epic` is due to run this tick, given `last_run` and `now`.
///
/// The feed-cadence floor is enforced on read (feeds.allium: FeedTick).
/// Write-time validation is what normally keeps a sub-floor row from
/// existing, so reaching the below-floor arm means the value arrived by a
/// path that bypassed the service — a hand-edited database, or a bug — and
/// the runner must not make it work anyway. Skipped rather than clamped: a
/// clamped feed would run at a cadence nobody chose while looking healthy.
///
/// This also covers the negative case, which failed differently and worse.
/// The `as u64` this replaces wrapped a negative into an effectively
/// infinite cadence, so the feed went permanently silent with nothing
/// logged.
fn epic_due(epic: &Epic, last_run: &HashMap<EpicId, Instant>, now: Instant) -> bool {
    let interval = match epic.feed_interval_secs {
        Some(s) if s < MIN_FEED_INTERVAL_SECS => {
            tracing::warn!(
                epic_id = epic.id.0,
                epic_title = %epic.title,
                feed_interval_secs = s,
                min_feed_interval_secs = MIN_FEED_INTERVAL_SECS,
                "FeedRunner: feed_interval_secs is below the minimum; \
                 not polling this epic until it is corrected"
            );
            return false;
        }
        // The guard above proves `s >= MIN_FEED_INTERVAL_SECS`, which is
        // positive, so this conversion is lossless. `unsigned_abs`
        // rather than the `as u64` it replaces: `as` is what turned a
        // negative into a near-infinite cadence, and if a later edit
        // ever weakened the guard, `as` would silently do that again.
        Some(s) => Duration::from_secs(s.unsigned_abs()),
        None => DEFAULT_FEED_INTERVAL,
    };

    let elapsed = last_run
        .get(&epic.id)
        .map(|t| now.saturating_duration_since(*t))
        .unwrap_or(Duration::MAX);

    elapsed >= interval
}

pub struct FeedRunner {
    db: Arc<dyn TaskStore>,
    notify: mpsc::UnboundedSender<BoardEvent>,
    runner: Arc<dyn ProcessRunner>,
    last_run: HashMap<EpicId, Instant>,
    /// Cached result of "does any epic have a feed command?".
    /// `None` means uninitialised or invalidated; `Some(false)` lets `tick()` skip
    /// all DB work when no epic needs polling.
    any_feed_cmds: Option<bool>,
    /// Watch receiver: when the sender fires, `any_feed_cmds` is reset to `None`
    /// so the next `tick()` re-queries.
    epic_changed_rx: tokio::sync::watch::Receiver<()>,
    /// Counterpart of `epic_changed_rx`.  Clone this before calling `start()` to
    /// retain a handle for external invalidation (e.g. on `EpicChanged` events).
    epic_changed_tx: tokio::sync::watch::Sender<()>,
    /// Per-epic single-flight claims, shared with the manual "r" refresh so the
    /// two surfaces serialise against each other. Take a handle with
    /// [`FeedRunner::sync_guard`] — the manual path holding a DIFFERENT
    /// `FeedSyncGuard` type-checks and silently serialises nothing.
    guard: Arc<FeedSyncGuard>,
    /// Read seam for `core/PollOwner` — `FeedTick`'s host-scoping
    /// (feeds.allium: FeedTick) needs to know who currently owns a given
    /// epic's polling before spawning its cycle. Writes (claiming an
    /// unowned epic) go through `db` instead, since `TaskStore` already
    /// includes `PollOwnershipStore` and `FeedRunner` is a sanctioned
    /// direct-mutation consumer.
    board_reads: Arc<dyn crate::store::BoardReads>,
    /// This machine's own `Host.id`, compared against `core/PollOwner.host`.
    host_id: String,
    /// Test-only join handles for the jobs spawned by `tick`. Production keeps
    /// firing-and-forgetting: the field, and the push that fills it, exist only
    /// under `cfg(test)`. Tests need it because some feed-cycle outcomes
    /// deliberately send no `BoardEvent` — the degraded-empty-emission guard
    /// (feeds.allium: DegradedEmptyEmission) returns before any sync — so
    /// awaiting `rx` is not a usable completion signal there, and sleeping is
    /// banned by `./scripts/check-no-test-sleep.sh`.
    #[cfg(test)]
    spawned: Vec<tokio::task::JoinHandle<()>>,
}

impl FeedRunner {
    pub fn new(
        db: Arc<dyn TaskStore>,
        notify: mpsc::UnboundedSender<BoardEvent>,
        runner: Arc<dyn ProcessRunner>,
        board_reads: Arc<dyn crate::store::BoardReads>,
        host_id: String,
    ) -> Self {
        let (epic_changed_tx, epic_changed_rx) = tokio::sync::watch::channel(());
        Self {
            db,
            notify,
            runner,
            last_run: HashMap::new(),
            any_feed_cmds: None,
            epic_changed_rx,
            epic_changed_tx,
            guard: Arc::new(FeedSyncGuard::default()),
            board_reads,
            host_id,
            #[cfg(test)]
            spawned: Vec::new(),
        }
    }

    /// Handle to the per-epic feed-cycle claims, for the manual "r" refresh to
    /// share. Both surfaces MUST hold this same `Arc`: the serialisation is
    /// per-registry, so a second registry silently disables it. Wire it at
    /// construction — see `TuiRuntime`'s `feed_sync_guard`.
    pub(crate) fn sync_guard(&self) -> Arc<FeedSyncGuard> {
        Arc::clone(&self.guard)
    }

    /// Await every job spawned by the ticks run so far, draining the handle
    /// list. Deterministic replacement for "wait for an `BoardEvent`" in tests
    /// covering paths that emit no event.
    #[cfg(test)]
    pub(crate) async fn join_spawned_jobs(&mut self) {
        for handle in std::mem::take(&mut self.spawned) {
            let _ = handle.await;
        }
    }

    /// Returns a sender that can be used to invalidate the feed-command cache.
    /// Clone and retain this handle before calling `start()`.
    pub fn epic_invalidate_tx(&self) -> tokio::sync::watch::Sender<()> {
        self.epic_changed_tx.clone()
    }

    /// Inspection accessor for the cached "does any epic have a feed command?"
    /// flag. `Some(false)` means the next `tick()` short-circuits without DB
    /// work; `None` means it will re-query. Used by tests asserting that a
    /// freshly-enabled feed becomes pollable after the cache is invalidated.
    #[cfg(test)]
    pub(crate) fn any_feed_cmds_cache(&self) -> Option<bool> {
        self.any_feed_cmds
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
        let Some(epics) = self.list_epics_for_tick().await else {
            return;
        };

        // Fetch once per tick so N concurrent spawned tasks don't each hit the DB.
        let known_paths = Arc::new(match self.db.list_repo_paths().await {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(
                    "FeedRunner: failed to list repo_paths, using empty sentinel: {err:#}"
                );
                vec![]
            }
        });

        let now = Instant::now();
        for epic in epics {
            // Scheduling only reads feed_command to decide whether this epic is
            // pollable at all; the command the cycle actually runs is re-read
            // from the epic inside FeedCycle::run, after the claim.
            if epic.feed_command.is_none() {
                continue;
            }

            if !epic_due(&epic, &self.last_run, now) {
                continue;
            }

            // Host scoping (feeds.allium: FeedTick, "Host scoping"): `Epic`
            // carries no `host` field the way a dispatched `Task` does, so
            // `core/PollOwner` is the only answer to "who runs this epic's
            // feed?" — not a narrower case of it. `last_run` is bumped
            // either way, matching this rule's existing "bumped even when
            // the request is then dropped" behaviour for
            // `SerialisedFeedCycle` contention: a non-owning host must not
            // retry every tick just because it lost the ownership check.
            self.last_run.insert(epic.id, now);
            if !self.this_host_polls(epic.id).await {
                continue;
            }

            self.spawn_epic_cycle(epic.id, epic.title, Arc::clone(&known_paths));
        }
    }

    /// The epics to consider this tick, or `None` when there is nothing to poll
    /// (no epic has a feed command, or the epics could not be listed). Keeps the
    /// "any feed commands" cache and `last_run` in step with the board.
    async fn list_epics_for_tick(&mut self) -> Option<Vec<crate::models::Epic>> {
        // Invalidate the cache if an EpicChanged signal arrived since last tick.
        if self.epic_changed_rx.has_changed().unwrap_or(true) {
            self.epic_changed_rx.borrow_and_update();
            self.any_feed_cmds = None;
        }

        // Skip all DB work when we know no epic has a feed command.
        if self.any_feed_cmds == Some(false) {
            return None;
        }

        let epics = match self.db.list_epics().await {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("FeedRunner: failed to list epics: {err:#}");
                return None;
            }
        };

        let active_ids: std::collections::HashSet<EpicId> = epics.iter().map(|e| e.id).collect();
        self.last_run.retain(|id, _| active_ids.contains(id));

        let has_feed_cmd = epics.iter().any(|e| e.feed_command.is_some());
        self.any_feed_cmds = Some(has_feed_cmd);

        if !has_feed_cmd {
            return None;
        }
        Some(epics)
    }

    /// Whether this host should run `epic_id`'s cycle this tick, per
    /// `core/PollOwner`. Claims ownership (without waiting for the answer) when
    /// no host has it yet.
    async fn this_host_polls(&mut self, epic_id: EpicId) -> bool {
        let owner = match self
            .board_reads
            .poll_owner(crate::models::PollScopeId::Epic(epic_id))
            .await
        {
            Ok(owner) => owner,
            Err(err) => {
                tracing::debug!(
                    epic_id = epic_id.0,
                    "FeedRunner: failed to read poll ownership, skipping this tick: {err:#}"
                );
                return false;
            }
        };
        match decide_poll_action(owner.as_deref(), &self.host_id) {
            PollAction::Skip => return false,
            // Fired without awaiting it: `tick` must not block on a
            // network round-trip for one epic while others are still
            // waiting their turn in this loop. This is the same
            // "proceed optimistically, let the loser's next tick stand
            // down" tradeoff `exec_check_status_if_owned` makes for the
            // PR-poll side of the same mechanism (`src/runtime/pr.rs`).
            PollAction::ClaimAndProceed => {
                let db = self.db.clone();
                let _claim_handle = tokio::task::spawn(async move {
                    if let Err(err) = db
                        .claim_poll_owner(crate::models::PollScopeId::Epic(epic_id))
                        .await
                    {
                        tracing::debug!(
                            epic_id = epic_id.0,
                            "FeedRunner: failed to claim poll ownership, running anyway: {err:#}"
                        );
                    }
                });
                #[cfg(test)]
                self.spawned.push(_claim_handle);
            }
            PollAction::Proceed => {}
        }
        true
    }

    /// Spawn one epic's feed cycle, so a slow feed command cannot stall the
    /// poll loop. The claim is taken INSIDE the cycle rather than here:
    /// `tick` must not block, so contention is resolved by whichever spawned
    /// cycle reaches try_claim first, and the loser returns Busy.
    fn spawn_epic_cycle(
        &mut self,
        epic_id: EpicId,
        epic_title: String,
        known_paths: Arc<Vec<String>>,
    ) {
        let cycle = cycle::FeedCycle {
            db: self.db.clone(),
            runner: self.runner.clone(),
            guard: Arc::clone(&self.guard),
            epic_id,
            epic_title,
            known_paths: Some(known_paths),
            command_timeout: FEED_COMMAND_TIMEOUT,
        };
        let notify = self.notify.clone();

        let _handle = tokio::task::spawn(async move {
            match cycle.run().await {
                // The cycle has already torn down every removed task's
                // worktree by the time it returns, so these notifications
                // mean "reconciled and cleaned up" (feeds.allium
                // RoleRoutedFeedSync).
                FeedCycleOutcome::Synced { affected_epics, .. } => {
                    for id in affected_epics {
                        let _ = notify.send(BoardEvent::EpicChanged(id));
                    }
                }
                // Both already logged by the cycle. The auto-poll path adds
                // no TUI surface, per feeds.allium FeedCommandFailure ("the
                // TUI is NOT notified"); a dropped request is not a failure.
                FeedCycleOutcome::Busy | FeedCycleOutcome::Failed(_) => {}
            }
        });
        #[cfg(test)]
        self.spawned.push(_handle);
    }
}

#[cfg(test)]
mod tests;
