mod cycle;
mod exec;
mod guard;
mod ingest;
mod parse;
mod requests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::board_event::BoardEvent;
use crate::models::{decide_feed_poll_action, host_runs_feed_of, PollAction};
use crate::models::{Epic, EpicId, MIN_FEED_INTERVAL_SECS};
use crate::process::ProcessRunner;
use crate::store::{RemovedFeedTask, TaskStore};

pub(crate) use cycle::{FeedCycle, FeedCycleOutcome};
pub(crate) use exec::degraded_partial_emission;
pub(crate) use exec::{degraded_empty_emission, exec_feed_command, resolve_base_branches};
// `pub`: the `verify-feed` CLI (a separate bin crate) runs the same bounded
// command execution as the runtime.
pub use exec::{run_feed_command_bounded, FEED_COMMAND_TIMEOUT};
pub(crate) use guard::FeedSyncGuard;
pub(crate) use ingest::{ingest_emission, FeedItemWithTarget, SyncMode};
// `pub`, unlike the `pub(crate)` re-exports above: the `verify-feed` CLI in
// src/main.rs is a separate bin crate and is one of this function's three
// callers. See feeds.allium's FeedItemParse block.
pub use parse::parse_feed_items;

/// Split removed feed tasks into those this host may tear down and those
/// another host owns. A task's worktree and tmux window live on the host that
/// dispatched it (`Task.host`); only that host tears them down (the session
/// seam, `docs/invariants.md`). A row with no host was never dispatched, so
/// whichever host ran the cycle owes it the (best-effort) teardown as before.
pub(crate) fn split_removed_by_host(
    removed: Vec<RemovedFeedTask>,
    this_host: &str,
) -> (Vec<RemovedFeedTask>, Vec<RemovedFeedTask>) {
    removed
        .into_iter()
        .partition(|t| t.host.as_deref().is_none_or(|h| h == this_host))
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
/// [`crate::store::FeedIngestOutcome`] the sync returned — so both feed paths get the
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
            teardown_repo_queue(&*runner, tasks);
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

/// Tear down one repo's removed tasks one after another, so they do not
/// contend on that repo's index lock. A failure is logged and the queue goes on.
fn teardown_repo_queue(runner: &dyn ProcessRunner, tasks: Vec<RemovedFeedTask>) {
    for task in tasks {
        if let Err(failure) = crate::dispatch::teardown_task(
            &task.repo_path,
            task.worktree.as_deref(),
            task.tmux_window.as_ref(),
            runner,
        ) {
            tracing::warn!(
                task_id = task.id.0,
                "feed cleanup: teardown_task failed: {failure}"
            );
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

/// The failure text when a feed epic is gone, whichever path finds it
/// (feeds.allium: FeedCommandFailure bucket 5, ManualFeedTrigger,
/// ServeFeedRefreshRequest).
pub(crate) const EPIC_GONE: &str = "epic no longer exists";

/// Whether `epic` is due to run this tick, given `last_run` and `now`.
///
/// The feed-cadence floor is enforced on read (feeds.allium: FeedTick).
/// Write-time validation is what normally keeps a sub-floor row from
/// existing, so reaching the below-floor arm means the value arrived by a
/// path that bypassed the service — a hand-edited database, or a bug — and
/// the runner must not make it work anyway. This predicate stays pure: the
/// warning is [`FeedRunner::warn_below_floor_once`]'s, so it is not repeated
/// every tick. Skipped rather than clamped: a
/// clamped feed would run at a cadence nobody chose while looking healthy.
///
/// This also covers the negative case, which failed differently and worse.
/// The `as u64` this replaces wrapped a negative into an effectively
/// infinite cadence, so the feed went permanently silent with nothing
/// logged.
fn epic_due(epic: &Epic, last_run: &HashMap<EpicId, Instant>, now: Instant) -> bool {
    let interval = match epic.feed_interval_secs {
        Some(s) if s < MIN_FEED_INTERVAL_SECS => return false,
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
    /// The below-floor `feed_interval_secs` already warned about, per epic, so
    /// the warning is logged once per value rather than every tick. Cleared for
    /// an epic once its interval is valid again.
    warned_below_floor: HashMap<EpicId, i64>,
    /// Read seam for `core/PollOwner` — `FeedTick`'s host-scoping
    /// (feeds.allium: FeedTick) needs to know who currently owns a given
    /// epic's polling before spawning its cycle. Writes (claiming an
    /// unowned epic) go through `db` instead, since `TaskStore` already
    /// includes `PollOwnershipStore` and `FeedRunner` is a sanctioned
    /// direct-mutation consumer.
    board_reads: Arc<dyn crate::store::BoardReads>,
    /// This machine's own `Host.id`, compared against `core/PollOwner.host`.
    host_id: String,
    /// Feed request rows this process has already handed out, so a row is
    /// served once however many ticks see it open.
    served_requests: requests::ServedRequests,
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

/// Claim `epic_id`'s poll ownership for `host_id` and wait for the answer.
/// True only when this host owns the epic afterwards: a failed claim, or a
/// claim another host won first, stands this host down.
pub(crate) async fn claim_epic_poll(
    db: &dyn TaskStore,
    board_reads: &dyn crate::store::BoardReads,
    host_id: &str,
    epic_id: EpicId,
) -> bool {
    let scope = crate::models::PollScopeId::Epic(epic_id);
    if let Err(err) = db.claim_poll_owner(scope).await {
        tracing::debug!(
            epic_id = epic_id.0,
            "FeedRunner: failed to claim poll ownership, standing down: {err:#}"
        );
        return false;
    }
    // Read back from the subscription. If the claim's row has not landed yet
    // this reads `None` and stands down; the next tick sees the owner and
    // runs, so the worst case is one tick late, never two hosts running.
    let owner = board_reads.poll_owner(scope).await;
    let won = owner.as_deref() == Some(host_id);
    if !won {
        tracing::debug!(
            epic_id = epic_id.0,
            "FeedRunner: another host claimed this epic first, standing down"
        );
    }
    won
}

/// `feeds.allium: FeedCommandRunsOnSettersHost`: whether the host `host_id`
/// may run `epic`'s feed command, for the auto-poll and the manual refresh
/// alike. True only when the host's owner is the person who set the command.
pub(crate) async fn host_may_run_feed(
    board_reads: &dyn crate::store::BoardReads,
    host_id: &str,
    epic: &crate::models::Epic,
) -> bool {
    let owner = board_reads.host_owner(host_id).await;
    host_runs_feed_of(owner.as_deref(), epic.feed_setter.as_deref())
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
            warned_below_floor: HashMap::new(),
            board_reads,
            host_id,
            served_requests: requests::ServedRequests::default(),
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

    /// Whether `epic`'s stored interval is below the floor (so it is skipped),
    /// logging the warning only when this value has not been warned about yet.
    /// A valid interval forgets an earlier warning, so a later bad value warns
    /// afresh. feeds.allium: FeedTick.
    fn warn_below_floor_once(&mut self, epic: &Epic) -> bool {
        match epic.feed_interval_secs {
            Some(s) if s < MIN_FEED_INTERVAL_SECS => {
                if self.warned_below_floor.insert(epic.id, s) != Some(s) {
                    tracing::warn!(
                        epic_id = epic.id.0,
                        epic_title = %epic.title,
                        feed_interval_secs = s,
                        min_feed_interval_secs = MIN_FEED_INTERVAL_SECS,
                        "FeedRunner: feed_interval_secs is below the minimum; \
                         not polling this epic until it is corrected"
                    );
                }
                true
            }
            _ => {
                self.warned_below_floor.remove(&epic.id);
                false
            }
        }
    }

    pub async fn tick(&mut self) {
        // Before the feed-command early return: a teardown row outlives the
        // epic's feed command, and a refresh row asks for a cycle now,
        // whatever the interval (store-split.allium: FeedRequest).
        self.serve_feed_requests().await;

        let Some(epics) = self.list_epics_for_tick().await else {
            return;
        };

        // Fetch once per tick so N concurrent spawned tasks don't each hit the DB.
        let known_paths = Arc::new(self.db.list_repo_paths_saved_by_this_host().await);

        let now = Instant::now();
        for epic in epics {
            // Scheduling only reads feed_command to decide whether this epic is
            // pollable at all; the command the cycle actually runs is re-read
            // from the epic inside FeedCycle::run, after the claim.
            if epic.feed_command.is_none() {
                continue;
            }

            if self.warn_below_floor_once(&epic) {
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
            let Some(claim_first) = self.this_host_polls(&epic).await else {
                continue;
            };

            self.spawn_epic_cycle(epic.id, epic.title, Arc::clone(&known_paths), claim_first);
        }
    }

    /// Serve the feed requests other hosts wrote for this one: a refresh runs
    /// one cycle for the epic, a teardown releases a removed task's worktree
    /// and window. Rows are served once per process; a row left `running` by a
    /// board that died is served again by the next one.
    async fn serve_feed_requests(&mut self) {
        let rows = self.db.feed_requests().await;
        if rows.iter().all(|r| !r.state.is_open()) {
            return;
        }
        let Some(owner) = self.db.own_identity().await else {
            // Held back, not acted on: the next tick after the identity
            // settles hands them out.
            return;
        };
        let work = requests::rows_to_serve(rows, &self.host_id, &owner, &self.served_requests);
        let (refreshes, teardowns): (Vec<_>, Vec<_>) = work
            .into_iter()
            .partition(|r| r.kind == crate::models::FeedRequestKind::Refresh);
        if !teardowns.is_empty() {
            let db = Arc::clone(&self.db);
            let runner = Arc::clone(&self.runner);
            let _handle = tokio::task::spawn(requests::serve_teardowns(
                db,
                runner,
                teardowns,
                self.served_requests.clone(),
            ));
            #[cfg(test)]
            self.spawned.push(_handle);
        }
        for row in refreshes {
            self.spawn_refresh_request(row);
        }
    }

    /// Serve one refresh row: claim it, re-check that this host may run the
    /// epic's command, run the one cycle through the shared guard and report.
    /// A cycle already in flight satisfies the request (`Busy` is done).
    fn spawn_refresh_request(&mut self, row: crate::models::FeedRequest) {
        let Some(epic_id) = row.epic_id else {
            return;
        };
        let db = Arc::clone(&self.db);
        let runner = Arc::clone(&self.runner);
        let guard = Arc::clone(&self.guard);
        let board_reads = Arc::clone(&self.board_reads);
        let host_id = self.host_id.clone();
        let notify = self.notify.clone();
        let served = self.served_requests.clone();
        let _handle = tokio::task::spawn(async move {
            if !requests::claim_granted(&served, row.id, db.claim_feed_request(row.id).await) {
                return;
            }
            let Some(epic) = board_reads.get_epic(epic_id).await else {
                let reported = db.report_feed_request_failed(row.id, EPIC_GONE).await;
                report_outcome(&served, row.id, reported);
                return;
            };
            if !host_may_run_feed(&*board_reads, &host_id, &epic).await {
                let reported = db
                    .report_feed_request_failed(row.id, "this host's owner did not set the command")
                    .await;
                report_outcome(&served, row.id, reported);
                return;
            }
            // Only the poll owner reconciles: if the poll moved after the
            // request was written, this host stands down.
            let owner = board_reads
                .poll_owner(crate::models::PollScopeId::Epic(epic_id))
                .await;
            if owner.as_deref() != Some(host_id.as_str()) {
                let reported = db
                    .report_feed_request_failed(row.id, "this host no longer owns the poll")
                    .await;
                report_outcome(&served, row.id, reported);
                return;
            }
            let cycle = cycle::FeedCycle {
                db: Arc::clone(&db),
                board_reads: Arc::clone(&board_reads),
                runner,
                guard,
                epic_id,
                epic_title: epic.title,
                known_paths: None,
                command_timeout: FEED_COMMAND_TIMEOUT,
            };
            let reported = match cycle.run().await {
                FeedCycleOutcome::Synced { affected_epics, .. } => {
                    for id in affected_epics {
                        let _ = notify.send(BoardEvent::EpicChanged(id));
                    }
                    db.report_feed_request_done(row.id).await
                }
                FeedCycleOutcome::Busy => db.report_feed_request_done(row.id).await,
                FeedCycleOutcome::Failed(reason) => {
                    db.report_feed_request_failed(row.id, &reason).await
                }
            };
            report_outcome(&served, row.id, reported);
        });
        #[cfg(test)]
        self.spawned.push(_handle);
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

        let epics = self.db.list_epics().await;

        let active_ids: std::collections::HashSet<EpicId> = epics.iter().map(|e| e.id).collect();
        self.last_run.retain(|id, _| active_ids.contains(id));

        let has_feed_cmd = epics.iter().any(|e| e.feed_command.is_some());
        self.any_feed_cmds = Some(has_feed_cmd);

        if !has_feed_cmd {
            return None;
        }
        Some(epics)
    }

    /// Whether this host should run `epic`'s cycle this tick, per
    /// `core/PollOwner`: `None` to skip, `Some(claim_first)` to run, where
    /// `claim_first` says no host owns the epic yet, so the spawned job must
    /// win the claim before it runs anything
    /// (feeds.allium: `SerialisedFeedCycle`, scope across hosts).
    async fn this_host_polls(&mut self, epic: &crate::models::Epic) -> Option<bool> {
        let owner = self
            .board_reads
            .poll_owner(crate::models::PollScopeId::Epic(epic.id))
            .await;
        // `FeedCommandRunsOnSettersHost` comes first: a host whose owner did
        // not set the command neither claims nor runs it.
        let runs_here = host_may_run_feed(&*self.board_reads, &self.host_id, epic).await;
        match decide_feed_poll_action(owner.as_deref(), &self.host_id, runs_here) {
            PollAction::Skip => None,
            // The claim is awaited by the spawned job, not here: `tick` must
            // not block on a network round-trip for one epic while others
            // wait their turn, but the cycle must not start until this host
            // owns the epic, or two hosts could both run on a first claim.
            PollAction::ClaimAndProceed => Some(true),
            PollAction::Proceed => Some(false),
        }
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
        claim_first: bool,
    ) {
        let cycle = cycle::FeedCycle {
            db: self.db.clone(),
            board_reads: Arc::clone(&self.board_reads),
            runner: self.runner.clone(),
            guard: Arc::clone(&self.guard),
            epic_id,
            epic_title,
            known_paths: Some(known_paths),
            command_timeout: FEED_COMMAND_TIMEOUT,
        };
        let notify = self.notify.clone();
        let board_reads = Arc::clone(&self.board_reads);
        let host_id = self.host_id.clone();

        let _handle = tokio::task::spawn(async move {
            if claim_first && !claim_epic_poll(&*cycle.db, &*board_reads, &host_id, epic_id).await {
                return;
            }
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

/// Log a refresh report that never reached the store and hand the row back, so
/// the next tick sees it again (a `running` row is claimable once the module's
/// grace period has passed).
fn report_outcome(
    served: &requests::ServedRequests,
    id: crate::models::FeedRequestId,
    reported: anyhow::Result<bool>,
) {
    if let Err(err) = reported {
        tracing::warn!(request = id.0, "feed refresh: report not sent: {err:#}");
        served.forget(id);
    }
}

#[cfg(test)]
mod tests;
