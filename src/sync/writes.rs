//! Sending a shared mutation to the store.
//!
//! Spec: `docs/specs/sync.allium`'s `BoardWritesThroughTheStore`,
//! `AWriteWithNoConnectionIsRefused` and `StoreRejectsAnInvalidMutation`.
//!
//! This is the implementation of [`crate::db::SharedWriter`] that a configured
//! board runs. [`ReducerWriter`] holds the two things a mutation needs that the
//! store cannot supply — who is writing, and when — encodes the row, and hands
//! it to a [`ReducerCaller`].
//!
//! # Why the transport is a second trait
//!
//! Everything interesting here is the encoding and the refusal path, and
//! neither needs a server. Splitting the transport out means the tests for both
//! run in CI, where no SpacetimeDB exists — the same reason
//! [`super::StoreConnector`] is a trait. [`SdkReducerCaller`] is the one
//! implementation that talks to a real store.
//!
//! # Nothing here retries
//!
//! A refusal is returned, once, to the caller that asked. There is no buffer to
//! put it in and no loop to put it through: `sync.allium: NoWriteIsEverQueued`,
//! and the reasoning is in its guidance — a replayed write carries an intent
//! formed against a board state that no longer holds, and the operator was told
//! it had already worked.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::db::{CreateTaskRequest, CreateTodoRow, EpicPatch, SharedWriter, TaskPatch, TodoPatch};
use crate::models::{
    Epic, EpicId, NotificationWrite, ShellDrain, StopOutcome, SubStatus, SubagentDrain, TaskId,
    TaskStatus, TodoId, UserPromptOutcome,
};
use crate::spacetime::bindings;

use super::encode;

/// One reducer call, and the store's verdict on it.
///
/// A method per reducer rather than one `call(name, args)`, so the argument
/// types are the generated ones and a signature that drifts from the module is
/// a compile error rather than a runtime decode failure.
/// What one reducer call did.
///
/// `Applied` and `Refused` are both successful ROUND TRIPS: the store was
/// reached and answered. A transport failure is the `Err` of the enclosing
/// `Result` instead, and keeping the two apart is what lets a caller treat
/// "somebody else won the claim" as an ordinary answer while still failing
/// loudly when the store is down (`sync.allium: AWriteWithNoConnectionIsRefused`
/// against `StoreRejectsAnInvalidMutation`).
#[derive(Debug)]
pub enum ReducerOutcome {
    /// The store applied it. Carries the ids of any rows the caller asked to
    /// have read back, which is the only way a reducer answers. A method with
    /// a different fact to read back (a live count, whether a row is now in
    /// `review`) is typed for it instead — see [`DrainReadBack`] — rather than
    /// squeezing it into this `Vec<i64>` by position.
    Applied(Vec<i64>),
    /// The store reached the call and declined it, with a reason.
    Refused(String),
}

impl ReducerOutcome {
    /// Turn a refusal into an error. The right reading for every mutation whose
    /// refusal means something went wrong, which is all of them except a claim.
    pub fn into_result(self) -> Result<Vec<i64>> {
        match self {
            Self::Applied(ids) => Ok(ids),
            Self::Refused(why) => Err(anyhow::anyhow!("the shared store refused: {why}")),
        }
    }

    /// [`Self::into_result`] for a caller with no ids to collect, which is
    /// every mutation but the three creates.
    pub fn applied(self) -> Result<()> {
        self.into_result().map(|_| ())
    }

    /// Whether it applied. The right reading for a claim, where a refusal is
    /// "somebody else got there first" rather than a fault.
    pub fn won(&self) -> bool {
        matches!(self, Self::Applied(_))
    }
}

/// The post-transaction read-back for a drain: the live count of the counter
/// this call touched, and whether the row is now in `review`. Two named
/// facts rather than positional `Vec<i64>` slots — shared by `subagent_stop`,
/// `subagent_clear` and `shell_stop`, which all answer off the same
/// predicate (`apply_pending_stop_if_drained` in the module).
#[derive(Debug, Clone, Copy, Default)]
pub struct DrainReadBack {
    pub live: i64,
    pub is_review: bool,
}

/// One reducer call, and the store's verdict on it.
///
/// A method per reducer rather than one `call(name, args)`, so the argument
/// types are the generated ones and a signature that drifts from the module is
/// a compile error rather than a runtime decode failure.
#[async_trait]
pub trait ReducerCaller: Send + Sync {
    /// Insert a task and answer with the id the store generated.
    ///
    /// The one call here that has to read a row back. A reducer cannot answer,
    /// so the id comes off the transaction the callback runs in — see
    /// [`super::SdkReducerCaller::create_task`] for how, and for what that
    /// costs.
    async fn create_task(&self, row: bindings::Task) -> Result<TaskId>;
    async fn patch_task(&self, id: TaskId, patch: bindings::TaskPatch) -> Result<ReducerOutcome>;
    async fn delete_task(&self, id: TaskId) -> Result<ReducerOutcome>;
    async fn set_task_epic(
        &self,
        id: TaskId,
        epic_id: i64,
        owner: String,
    ) -> Result<ReducerOutcome>;

    async fn claim_backlog_task(&self, id: TaskId, host: String) -> Result<ReducerOutcome>;
    async fn release_backlog_claim(&self, id: TaskId) -> Result<ReducerOutcome>;

    /// Insert an epic and answer with the id the store generated.
    async fn create_epic(&self, row: bindings::Epic) -> Result<i64>;
    async fn patch_epic(&self, id: i64, patch: bindings::EpicPatch) -> Result<ReducerOutcome>;
    async fn delete_epic(&self, id: i64) -> Result<ReducerOutcome>;
    async fn recalculate_epic_status(&self, id: i64) -> Result<ReducerOutcome>;

    /// Insert a todo and answer with the id the store generated.
    async fn create_todo(&self, row: bindings::Todo) -> Result<i64>;
    async fn patch_todo(&self, id: i64, patch: bindings::TodoPatch) -> Result<ReducerOutcome>;
    async fn delete_todo(&self, id: i64) -> Result<ReducerOutcome>;
    async fn delete_done_todos(&self, owner: String) -> Result<ReducerOutcome>;

    async fn save_repo_path(&self, path: String, last_used: String) -> Result<ReducerOutcome>;
    async fn delete_repo_path(&self, path: String) -> Result<ReducerOutcome>;
    async fn set_verify_command(&self, path: String, command: String) -> Result<ReducerOutcome>;
    async fn record_base_branch(
        &self,
        repo_path: String,
        branch: String,
        last_used: String,
    ) -> Result<ReducerOutcome>;

    async fn subscribe_to_epic(&self, subscriber: String, epic_id: i64) -> Result<ReducerOutcome>;
    async fn unsubscribe_from_epic(
        &self,
        subscriber: String,
        epic_id: i64,
    ) -> Result<ReducerOutcome>;

    // Agent session state (Phase 6b). Every one of these acts on a row whose
    // id the caller already has, so what needs reading back is a FACT off
    // that row rather than an id to match by content — but unlike the three
    // creates, that fact isn't a bare id, so it gets its own type
    // ([`DrainReadBack`], a plain `i64`, `Option<bool>`) instead of being
    // squeezed into `ReducerOutcome`'s `Vec<i64>`: a positional slot the
    // caller has to remember the layout of is exactly the kind of thing a
    // typo compiles cleanly through. The four with no read-back ambiguity at
    // all (they're a plain applied-or-refused, nothing to decode) stay on
    // `ReducerOutcome` below, same as every earlier reducer.
    async fn subagent_start(
        &self,
        task_id: i64,
        agent_id: String,
        session_id: String,
        started_at: String,
    ) -> Result<i64>;
    async fn subagent_stop(
        &self,
        task_id: i64,
        agent_id: String,
        session_id: String,
    ) -> Result<DrainReadBack>;
    async fn subagent_clear(&self, task_id: i64) -> Result<DrainReadBack>;
    async fn subagent_clear_and_void_pending_stop(&self, task_id: i64) -> Result<ReducerOutcome>;
    async fn shell_start(
        &self,
        task_id: i64,
        shell_id: String,
        session_id: String,
        started_at: String,
    ) -> Result<i64>;
    async fn shell_stop(
        &self,
        task_id: i64,
        shell_id: String,
        session_id: String,
    ) -> Result<DrainReadBack>;
    async fn shell_clear_no_drain(&self, task_id: i64) -> Result<ReducerOutcome>;
    /// `None` for a refusal (the task was not `Running`) — this task's
    /// `StopOutcome::NoOp`. `Some(true)`/`Some(false)` is `Flipped`/`Deferred`,
    /// unambiguous once accepted (see `try_record_stop`'s doc comment in the
    /// module).
    async fn try_record_stop(&self, id: i64, stop_pending_at: String) -> Result<Option<bool>>;
    async fn record_pre_tool_use(
        &self,
        id: i64,
        sub_status: String,
        at: String,
    ) -> Result<ReducerOutcome>;
    async fn record_notification(
        &self,
        id: i64,
        mode: String,
        at: String,
    ) -> Result<ReducerOutcome>;
    async fn record_user_prompt_submit(
        &self,
        id: i64,
        activity_at: String,
        prompt_at: String,
    ) -> Result<ReducerOutcome>;
    async fn mark_pr_learnings_gate_shown(&self, id: i64, at: String) -> Result<ReducerOutcome>;

    // -- Feed ingestion (Phase 6c) --------------------------------------------
    //
    // No id read-back and no reported removals here — that decoding lives in
    // `ReducerWriter`, which predicts candidates from its own already-
    // subscribed view before the call and confirms them absent afterward
    // (see this task's plan doc, decision 1). These three are plain
    // applied-or-refused calls.
    async fn upsert_feed_tasks(
        &self,
        epic_id: i64,
        items: Vec<bindings::FeedTaskUpsertItem>,
    ) -> Result<ReducerOutcome>;
    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: i64,
        items: Vec<bindings::FeedTaskUpsertItem>,
    ) -> Result<ReducerOutcome>;
    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: i64,
        keep_external_ids: Vec<String>,
    ) -> Result<ReducerOutcome>;

    /// Find-or-create; answers with the epic's id either way. Matched by the
    /// domain key `(parent_id, title)` — exact, not a content/timestamp tie-
    /// break, because that pair is genuinely unique in the domain (decision 2
    /// of this task's plan doc).
    async fn create_repo_group_sub_epic(
        &self,
        parent_id: i64,
        title: String,
        created_by: String,
    ) -> Result<i64>;
    /// Find-or-create keyed on `(parent_epic_id, role)`, on the same terms as
    /// [`Self::create_repo_group_sub_epic`].
    async fn create_managed_role_epic(
        &self,
        title: String,
        parent_epic_id: i64,
        role: String,
        feed_command: String,
        feed_interval_secs: i64,
        created_by: String,
    ) -> Result<i64>;

    // -- Task watchers ---------------------------------------------------------
    async fn create_task_watcher(
        &self,
        watcher_task_id: i64,
        target_task_id: i64,
    ) -> Result<ReducerOutcome>;
    async fn delete_task_watcher(
        &self,
        watcher_task_id: i64,
        target_task_id: i64,
    ) -> Result<ReducerOutcome>;
    async fn delete_watches_of_target(&self, target_task_id: i64) -> Result<ReducerOutcome>;
    async fn delete_watches_by_watcher(&self, watcher_task_id: i64) -> Result<ReducerOutcome>;

    // -- Stragglers ------------------------------------------------------------
    async fn batch_patch_sub_status(
        &self,
        updates: Vec<bindings::SubStatusUpdate>,
    ) -> Result<ReducerOutcome>;
    /// Insert the successor and read its generated id back — the same
    /// content-matched read-back [`Self::create_task`] uses.
    async fn respawn_phoenix_successor(
        &self,
        predecessor: i64,
        successor: bindings::Task,
    ) -> Result<TaskId>;

    // -- Host registry (Phase 6c) -----------------------------------------------
    //
    // NOT part of `SharedWriter` — see `db::SharedWriter`'s doc comment and
    // `sync.allium: RegisterHostOnConnect`/`RegisterHostOnRename`. Still an
    // ordinary reducer call, which is why it lives on this transport trait
    // rather than being bolted on separately; [`push_host_registration`]
    // below is the best-effort wrapper both call sites share.
    async fn register_host(
        &self,
        id: String,
        label: String,
        owner: String,
    ) -> Result<ReducerOutcome>;
}

/// Push this machine's current host row to the store, best-effort.
///
/// The shared wrapper `sync.allium: RegisterHostOnConnect` and
/// `RegisterHostOnRename` both call: a failed or delayed push must not block
/// the connection or the rename it followed, so this logs and returns rather
/// than propagating. Not a `SharedWriter` method — see `db::SharedWriter`'s
/// doc comment for why the host identity write itself stays local and
/// unconditional; this is the separate mirror on top of it.
pub async fn push_host_registration(
    caller: &dyn ReducerCaller,
    id: String,
    label: String,
    owner: String,
) {
    let host_id = id.clone();
    match caller.register_host(id, label, owner).await {
        Ok(ReducerOutcome::Applied(_)) => {}
        Ok(ReducerOutcome::Refused(why)) => {
            tracing::warn!(
                host_id,
                "the shared store refused to register this host: {why}"
            );
        }
        Err(e) => {
            tracing::warn!(
                host_id,
                "failed to register this host with the shared store: {e:#}"
            );
        }
    }
}

/// Who the board is writing as.
///
/// # Why this is read per write rather than resolved at bootstrap
///
/// The obvious design is to resolve it once and hold the string. It does not
/// work: the writer is built at bootstrap, the identity is settled by the
/// CONNECTION, and the connection has not happened yet. A string captured then
/// would be whatever the last run stored, or nothing at all on a first run.
///
/// So this is a reader, consulted by the one mutation that needs an answer.
/// Creates are rare enough that a settings lookup on that path costs nothing
/// worth designing around, and everything else — every patch, every delete —
/// never asks.
#[async_trait]
pub trait WriterIdentity: Send + Sync {
    /// The user this board writes as, or `None` before it has ever connected.
    async fn user(&self) -> Result<Option<String>>;
}

/// The identity this board's connection settled on, once it has.
///
/// # Why a cell rather than a read of the local store
///
/// The local store does hold it — the handshake writes it there
/// (`host.allium: AdoptUserIdentity`) — but reading it from there at bootstrap
/// is not possible: the writer is built before the `Database` it would read,
/// and the two would refer to each other. Filling a cell afterwards breaks that
/// and is the truer statement besides. What a write may stamp is the identity
/// THIS CONNECTION settled, not whatever a previous run left on disk: a board
/// that has not connected must not create user-board tasks under a name it
/// cannot currently prove.
///
/// Empty until the first successful connection, and never emptied again. A
/// dropped connection does not clear it, because the person did not change —
/// and a write during the outage is refused by the transport anyway.
#[derive(Default)]
pub struct SettledIdentity {
    user: std::sync::Mutex<Option<String>>,
    /// Why the connection is currently down, as the session recorded it.
    ///
    /// Published here for one reader: the refusal a write produces while the
    /// store is unreachable. `sync.allium: AWriteWithNoConnectionIsRefused`
    /// says that refusal carries `connection.last_error` — "the store is
    /// unreachable: connection refused" is actionable and "could not save" is
    /// not — and the transport cannot reach the session that owns it. This is
    /// the one-way channel that makes the spec true.
    ///
    /// Cleared on a successful connection, so a refusal never quotes an outage
    /// that is over.
    last_error: std::sync::Mutex<Option<String>>,
}

impl SettledIdentity {
    /// Record who the store said we are. Called once per successful connection.
    pub fn settle(&self, user: impl Into<String>) {
        *self.user.lock().unwrap_or_else(|e| e.into_inner()) = Some(user.into());
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Record why the connection is down, or clear it with `None`.
    pub fn set_last_error(&self, reason: Option<String>) {
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = reason;
    }

    /// Why the connection is down, if it is and the session said so.
    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[async_trait]
impl WriterIdentity for SettledIdentity {
    async fn user(&self) -> Result<Option<String>> {
        Ok(self.user.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

/// Whether the store applied it — and if it did not, WHY, in the log.
///
/// The signature the claim needs is a bool: the caller's next move is the same
/// whichever reason it lost for. But the reasons are not the same, and
/// `sync.allium: StoreRejectsAnInvalidMutation` says a rejection carries one.
/// `claim_backlog_task` refuses for three (the task is gone, it is no longer in
/// backlog, its worktree is on another machine) and only the middle one is an
/// ordinary lost race. Collapsing all three to `false` silently turned a
/// misconfigured host into "somebody else was quicker".
///
/// So the bool is still the answer and the reason still goes somewhere a person
/// can find it. Logged rather than surfaced because there is nothing for an
/// operator to DO about a lost race, and the two cases that are worth acting on
/// are rare enough to be worth reading a log for.
fn won(outcome: ReducerOutcome, what: &str, id: TaskId) -> bool {
    match outcome {
        ReducerOutcome::Applied(_) => true,
        ReducerOutcome::Refused(why) => {
            tracing::info!("the shared store refused the {what} of task {id}: {why}");
            false
        }
    }
}

/// Shared mutations, as reducer calls.
pub struct ReducerWriter {
    caller: Arc<dyn ReducerCaller>,
    identity: Arc<dyn WriterIdentity>,
    clock: Arc<dyn crate::service::Clock>,
    /// This machine's `Host` id, resolved once at bootstrap.
    ///
    /// Unlike the user identity this is known before any connection — it is
    /// minted locally on first run and immutable afterwards
    /// (`host.allium: MintHostIdentity`) — so it is a value rather than a cell.
    /// The claim needs it: a task whose worktree is on another machine is one
    /// this board must not take.
    host: String,
    /// What this board can see, which for the chain — and, since Phase 6b, for
    /// classifying an agent-session-state answer the store cannot otherwise
    /// distinguish — is enough.
    ///
    /// The by-epic claim needs it to choose a candidate, because a reducer
    /// cannot choose one for it — see
    /// [`ReducerWriter::try_claim_next_backlog_task`]. `subagent_stop`,
    /// `subagent_clear`, `shell_stop` and `record_user_prompt_submit` read it
    /// too, for a DIFFERENT reason: their answer includes a bit (did this
    /// drain a deferred Stop; was this call a resume or a refresh) that is
    /// only decodable from a row already known to have been `Running` before
    /// the call, and a reducer returns no value to say what it was. See
    /// [`ReducerWriter::prior_running`] and this task's plan doc
    /// (docs/plans/2026-09-21-phase-6b-agent-session-state-reducers.md,
    /// decision 3) for why that pre-read is advisory rather than
    /// authoritative — the reducer itself decides and recalculates from its
    /// own unambiguous view, regardless of what this read comes back with.
    ///
    /// Typed as the read SEAM rather than as the subscription behind it, even
    /// though only a store-backed board ever builds a `ReducerWriter`. The
    /// repo's rule is that a read which decides what the board does goes
    /// through `BoardReads`, and a writer reaching past it for one query is how
    /// a second read path starts.
    reads: Arc<dyn super::BoardReads>,
}

impl ReducerWriter {
    pub fn new(
        caller: Arc<dyn ReducerCaller>,
        identity: Arc<dyn WriterIdentity>,
        clock: Arc<dyn crate::service::Clock>,
        host: String,
        reads: Arc<dyn super::BoardReads>,
    ) -> Self {
        Self {
            caller,
            identity,
            clock,
            host,
            reads,
        }
    }

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
    /// own handshake — see `insert_todo`, the one call site that used to trust
    /// such a value instead of asking here.
    async fn require_identity(&self, unable_to: &str) -> Result<String> {
        self.identity
            .user()
            .await?
            .ok_or_else(|| anyhow::anyhow!("this board has no user identity yet, so {unable_to}"))
    }

    /// Whether `id` was in status `want` just before an agent-session-state
    /// call — the pre-read this struct's `reads` field doc comment explains.
    /// A read failure or a task this board cannot see reads as `false`: the
    /// worst that does is under-report a drain/resume the reducer already
    /// applied and recalculated correctly on its own.
    async fn prior_status_was(&self, id: TaskId, want: TaskStatus) -> bool {
        matches!(self.reads.get_task(id).await, Ok(Some(t)) if t.status == want)
    }

    /// Combine a drain's pre-read (taken BEFORE the reducer call, via
    /// [`Self::prior_status_was`]) with the module's post-transaction
    /// read-back into the `SubagentDrain`/`ShellDrain` (one type, two names)
    /// the caller wants. `read.is_review` alone is never enough — see
    /// [`Self::prior_status_was`]'s doc comment.
    fn drain_outcome(prior_running: bool, read: DrainReadBack) -> SubagentDrain {
        SubagentDrain {
            live: read.live,
            applied_pending_stop: prior_running && read.is_review,
        }
    }

    /// Shared body of `upsert_feed_tasks`/`upsert_feed_tasks_additive`.
    /// `delete_absent` selects the stale-delete pass, the same switch
    /// `src/db/queries/tasks.rs::upsert_feed_tasks_inner` uses.
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
    ) -> Result<Vec<crate::db::RemovedFeedTask>> {
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
            self.feed_removal_candidates(epic_id, &keep).await
        } else {
            Vec::new()
        };

        if delete_absent {
            self.caller
                .upsert_feed_tasks(epic_id.0, wire_items)
                .await?
                .applied()?;
        } else {
            self.caller
                .upsert_feed_tasks_additive(epic_id.0, wire_items)
                .await?
                .applied()?;
        }

        Ok(self.confirm_removed(candidates).await)
    }

    /// Tasks in `epic_id` this board can currently see whose `external_id` is
    /// set and not in `keep` — the same predicate the reducer's stale-delete
    /// pass applies, read from this connection's own subscription rather
    /// than predicted from nothing.
    async fn feed_removal_candidates(
        &self,
        epic_id: EpicId,
        keep: &std::collections::HashSet<&str>,
    ) -> Vec<crate::models::Task> {
        self.reads
            .list_tasks_for_epic(epic_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|t| matches!(&t.external_id, Some(e) if !keep.contains(e.as_str())))
            .collect()
    }

    /// The verify half of predict-then-verify: keep only candidates
    /// confirmed gone from this connection's view after the reducer call
    /// returned.
    async fn confirm_removed(
        &self,
        candidates: Vec<crate::models::Task>,
    ) -> Vec<crate::db::RemovedFeedTask> {
        let mut removed = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            if matches!(self.reads.get_task(candidate.id).await, Ok(None)) {
                removed.push(crate::db::RemovedFeedTask {
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

#[async_trait]
impl SharedWriter for ReducerWriter {
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

    async fn patch_task(&self, id: TaskId, patch: &TaskPatch<'_>) -> Result<()> {
        self.caller
            .patch_task(id, encode::task_patch(patch))
            .await?
            .applied()
    }

    async fn delete_task(&self, id: TaskId) -> Result<()> {
        self.caller.delete_task(id).await?.applied()
    }

    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()> {
        // A task leaving its epic lands on a user board and needs an owner;
        // one joining an epic gives its owner up. The store enforces both arms
        // (`core.allium: OwnerTracksUserBoardTask`), so the only job here is to
        // supply the name it may need.
        let owner = match epic_id {
            Some(_) => String::new(),
            None => {
                self.require_identity(
                    "a task cannot be moved out of its epic onto a user board; nothing was changed",
                )
                .await?
            }
        };
        self.caller
            .set_task_epic(task_id, epic_id.map(|e| e.0).unwrap_or(0), owner)
            .await?
            .applied()
    }

    /// Offer the epic's backlog subtasks to the store, in order, until one is
    /// won.
    ///
    /// # Why the candidate is chosen HERE and the claim is arbitrated THERE
    ///
    /// A reducer returns no value, so a store-side "pick the next one and claim
    /// it" could not tell this caller WHICH task it got — and the caller is
    /// about to provision that task's worktree. So the choosing moved here.
    ///
    /// That is safe, and it is worth being precise about why, because the
    /// neighbouring decision went the other way. An epic's STATUS has to be
    /// derived at the store because its children can be anywhere and no board
    /// sees all of them. Its BACKLOG SUBTASKS are different: a subscription to
    /// an epic is `WHERE epic_id = N`, which returns every subtask of it
    /// regardless of owner, so a board that is chaining an epic can see the
    /// whole candidate list. The ordering decision is made on complete
    /// information.
    ///
    /// Exclusivity is still the store's. Each offer is a real claim that the
    /// store refuses if another host took it first, and the loop simply moves
    /// on — so two boards racing down the same list end up on different tasks
    /// rather than the same one (`dispatch.allium: DispatchClaimExclusive`).
    ///
    /// `phoenix` rows are passed over: `epics.allium: PhoenixIsNeverChained`. A
    /// recurring subtask respawns on completion, so chaining its successor
    /// would launch an agent at it immediately, forever.
    async fn try_claim_next_backlog_task(&self, epic_id: EpicId) -> Result<Option<TaskId>> {
        let candidates: Vec<TaskId> = self
            .reads
            .list_tasks_for_epic(epic_id)
            .await?
            .into_iter()
            .filter(|t| {
                t.status == crate::models::TaskStatus::Backlog
                    && !t.phoenix
                    && t.is_locally_owned(Some(&self.host))
            })
            .map(|t| t.id)
            .collect();
        // `tasks_for_epic` already sorts by `sort_key`, which is
        // `COALESCE(sort_order, id)` — the order the board draws them in, and
        // the order the chain must take them in.

        for id in candidates {
            if self.try_claim_backlog_task(id).await? {
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
    async fn try_claim_backlog_task(&self, id: TaskId) -> Result<bool> {
        Ok(won(
            self.caller
                .claim_backlog_task(id, self.host.clone())
                .await?,
            "claim",
            id,
        ))
    }

    async fn try_release_backlog_claim(&self, id: TaskId) -> Result<bool> {
        Ok(won(
            self.caller.release_backlog_claim(id).await?,
            "release",
            id,
        ))
    }

    async fn create_epic(
        &self,
        title: &str,
        description: &str,
        parent_epic_id: Option<EpicId>,
    ) -> Result<Epic> {
        // `sync.allium: CreatesRequireASettledIdentity` — an epic has no
        // `owner` at all, so `created_by` is the only way `own_creations` can
        // find it before anyone follows it, and there is no name to stamp
        // without a settled identity.
        let identity = self
            .require_identity("there is no name to stamp on a new epic; it was not created")
            .await?;
        let now = self.now();
        let row = encode::create_epic_row(title, description, parent_epic_id, &identity, &now);
        let id = self.caller.create_epic(row.clone()).await?;
        // THE ROW AS SENT, WITH THE ID FILLED IN, rather than a read-back.
        //
        // Elsewhere this codebase insists the row is the truth and re-reads it
        // — `claim_next_backlog_task` says so explicitly. A create is the one
        // case where there is nothing to drift from: every field was chosen
        // here a moment ago, including both timestamps, and the store added
        // exactly one thing. Re-reading would also mean waiting for the
        // subscription to deliver a row this board may not even be subscribed
        // to.
        crate::sync::decode::epic(&bindings::Epic { id, ..row })
            .map_err(|e| anyhow::anyhow!("the created epic could not be read back: {e}"))
    }

    async fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()> {
        self.caller
            .patch_epic(id.0, encode::epic_patch(patch))
            .await?
            .applied()
    }

    async fn delete_epic(&self, id: EpicId) -> Result<()> {
        self.caller.delete_epic(id.0).await?.applied()
    }

    async fn recalculate_epic_status(&self, id: EpicId) -> Result<()> {
        self.caller.recalculate_epic_status(id.0).await?.applied()
    }

    async fn insert_todo(&self, row: CreateTodoRow<'_>) -> Result<TodoId> {
        // `sync.allium: CreatesRequireASettledIdentity`. Stamped from THIS
        // connection's own proven identity rather than trusting `row.owner` —
        // `TodoService`'s read of the PERSISTED setting (`todo.allium:
        // CreateTodo`), which can predate this connection's own handshake on
        // an install that has connected before. Asking here instead closes
        // that window: a todo cannot be created under an identity this
        // session has not itself settled.
        let owner = self
            .require_identity("there is no name to stamp on a new todo; it was not created")
            .await?;
        self.caller
            .create_todo(encode::create_todo_row(&row, &owner, &self.now()))
            .await
            .map(TodoId)
    }

    async fn patch_todo(&self, id: TodoId, patch: &TodoPatch<'_>) -> Result<()> {
        self.caller
            .patch_todo(id.0, encode::todo_patch(patch))
            .await?
            .applied()
    }

    async fn delete_todo(&self, id: TodoId) -> Result<()> {
        self.caller.delete_todo(id.0).await?.applied()
    }

    /// Clears THIS PERSON's finished todos, and the scoping is the whole
    /// difference from the local version.
    ///
    /// On one machine "every done todo" was every done todo there was. On a
    /// shared store it would be every colleague's completed checklist, cleared
    /// from whichever board pressed the key.
    async fn delete_done_todos(&self) -> Result<()> {
        let owner = self
            .require_identity("it cannot tell which checklist is yours; nothing was cleared")
            .await?;
        self.caller.delete_done_todos(owner).await?.applied()
    }

    async fn save_repo_path(&self, path: &str) -> Result<()> {
        self.caller
            .save_repo_path(path.to_string(), self.now())
            .await?
            .applied()
    }

    async fn delete_repo_path(&self, path: &str) -> Result<()> {
        self.caller
            .delete_repo_path(path.to_string())
            .await?
            .applied()
    }

    async fn set_verify_command(&self, path: &str, command: Option<&str>) -> Result<()> {
        // `""` clears it. The module's absent sentinel, not a command that
        // happens to be empty — see its header.
        self.caller
            .set_verify_command(path.to_string(), command.unwrap_or_default().to_string())
            .await?
            .applied()
    }

    async fn record_base_branch(&self, repo_path: &str, branch: &str) -> Result<()> {
        self.caller
            .record_base_branch(repo_path.to_string(), branch.to_string(), self.now())
            .await?
            .applied()
    }

    async fn subscribe_to_epic(&self, subscriber: &str, epic_id: i64) -> Result<()> {
        self.caller
            .subscribe_to_epic(subscriber.to_string(), epic_id)
            .await?
            .applied()
    }

    /// `Ok(false)` for an epic that was not followed.
    ///
    /// `sync.allium: UnsubscribeFromEpic` makes that a refusal rather than a
    /// no-op, and the store says so; the local signature reports it as `false`
    /// rather than as an error, exactly as the SQLite version does.
    async fn unsubscribe_from_epic(&self, subscriber: &str, epic_id: i64) -> Result<bool> {
        Ok(self
            .caller
            .unsubscribe_from_epic(subscriber.to_string(), epic_id)
            .await?
            .won())
    }

    // -- Agent session state (Phase 6b) --------------------------------------
    //
    // `docs/specs/agent-health.allium` is the reference; nothing here changes
    // a guarantee, only where the counting happens. `started_at`/
    // `last_pre_tool_use_at`/`last_notification_at`/`stop_pending_at` are all
    // EVENT times — this connection's `now`, the instant the hook fired — not
    // the store's `updated_at`. See the module's "Agent session state"
    // section header for why that distinction survives the move to a store
    // rather than becoming moot.

    async fn subagent_start(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64> {
        // RFC 3339, matching `subagents.rs::subagent_start`'s
        // `now.to_rfc3339()` — this column is never compared across rows, so
        // it carries no format requirement `encode::stamp`'s callers rely on.
        self.caller
            .subagent_start(
                id.0,
                agent_id.to_string(),
                session_id.to_string(),
                now.to_rfc3339(),
            )
            .await
    }

    async fn subagent_stop(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
    ) -> Result<SubagentDrain> {
        let prior_running = self.prior_status_was(id, TaskStatus::Running).await;
        let read = self
            .caller
            .subagent_stop(id.0, agent_id.to_string(), session_id.to_string())
            .await?;
        Ok(Self::drain_outcome(prior_running, read))
    }

    async fn subagent_clear(&self, id: TaskId) -> Result<SubagentDrain> {
        let prior_running = self.prior_status_was(id, TaskStatus::Running).await;
        let read = self.caller.subagent_clear(id.0).await?;
        Ok(Self::drain_outcome(prior_running, read))
    }

    async fn subagent_clear_and_void_pending_stop(&self, id: TaskId) -> Result<()> {
        self.caller
            .subagent_clear_and_void_pending_stop(id.0)
            .await?
            .applied()
    }

    async fn shell_start(
        &self,
        id: TaskId,
        shell_id: &str,
        session_id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64> {
        // The module's fixed-width TEXT format, matching
        // `shells.rs::shell_start`'s `format_datetime_millis` — THIS column IS
        // compared, via the module's lexicographic `MIN`, unlike the subagent
        // twin above.
        self.caller
            .shell_start(
                id.0,
                shell_id.to_string(),
                session_id.to_string(),
                encode::stamp(now),
            )
            .await
    }

    async fn shell_stop(&self, id: TaskId, shell_id: &str, session_id: &str) -> Result<ShellDrain> {
        let prior_running = self.prior_status_was(id, TaskStatus::Running).await;
        let read = self
            .caller
            .shell_stop(id.0, shell_id.to_string(), session_id.to_string())
            .await?;
        Ok(Self::drain_outcome(prior_running, read))
    }

    async fn shell_clear_no_drain(&self, id: TaskId) -> Result<()> {
        self.caller.shell_clear_no_drain(id.0).await?.applied()
    }

    /// `Refused` reads as `NoOp` (the task was not `Running`); once accepted,
    /// `Flipped` and `Deferred` are unambiguous from the row the module reads
    /// back — see `try_record_stop`'s own doc comment in the module for why.
    async fn try_record_stop(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<StopOutcome> {
        Ok(
            match self
                .caller
                .try_record_stop(id.0, encode::stamp(now))
                .await?
            {
                None => StopOutcome::NoOp,
                Some(true) => StopOutcome::Flipped,
                Some(false) => StopOutcome::Deferred,
            },
        )
    }

    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        self.caller
            .record_pre_tool_use(id.0, sub_status.as_str().to_string(), encode::stamp(now))
            .await?
            .applied()
    }

    async fn record_notification(
        &self,
        id: TaskId,
        write: NotificationWrite,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        // Ignore never reaches the store — matches the SQL path's early
        // `return Ok(())`, and saves a round trip for the commonest kind
        // (`auth_success`).
        let mode = match write {
            NotificationWrite::Ignore => return Ok(()),
            NotificationWrite::Clear => "clear",
            NotificationWrite::Raise => "raise",
            NotificationWrite::RaiseIfNoOwnWorkLive => "raise_if_no_own_work_live",
        };
        self.caller
            .record_notification(id.0, mode.to_string(), encode::stamp(now))
            .await?
            .applied()
    }

    /// Resumed vs. Refreshed is NOT decodable from the row the module writes
    /// — both write the identical field set (`status = running`, `sub_status
    /// = active`, `last_pre_tool_use_at`). Classified instead from a PRE-read
    /// (was the task in `Review` just before this call — the same kind of
    /// read [`Self::prior_status_was`] does for the drain methods, just
    /// testing the other status) taken before the call; advisory only,
    /// because the reducer decides `resumed` from its own unambiguous view
    /// and recalculates the epic itself when it is true. See this task's
    /// plan doc, decision 3.
    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<UserPromptOutcome> {
        let prior_review = self.prior_status_was(id, TaskStatus::Review).await;
        // Both timestamps are the SAME event time — mirroring
        // `tasks.rs::record_user_prompt_submit`'s single client `now`, which
        // it formats twice (seconds for `last_pre_tool_use_at`, millis for
        // the void-comparison); the store side uses the millis format for
        // both, since parsing tolerates the extra precision either way.
        let stamp = encode::stamp(now);
        match self
            .caller
            .record_user_prompt_submit(id.0, stamp.clone(), stamp)
            .await?
        {
            ReducerOutcome::Refused(_) => Ok(UserPromptOutcome::NoOp),
            ReducerOutcome::Applied(_) => Ok(if prior_review {
                UserPromptOutcome::Resumed
            } else {
                UserPromptOutcome::Refreshed
            }),
        }
    }

    /// `Applied`/`Refused` map straight onto `true`/`false` via
    /// [`ReducerOutcome::won`] — the exact `try_claim_backlog_task` shape, no
    /// read-back needed.
    async fn mark_pr_learnings_gate_shown(&self, id: TaskId) -> Result<bool> {
        Ok(self
            .caller
            .mark_pr_learnings_gate_shown(id.0, self.now())
            .await?
            .won())
    }

    // -- Feed ingestion (Phase 6c) --------------------------------------------

    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: &[crate::models::FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<Vec<crate::db::RemovedFeedTask>> {
        self.upsert_feed_tasks_inner(epic_id, items, repo_paths, base_branches, true)
            .await
    }

    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: &[crate::models::FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<Vec<crate::db::RemovedFeedTask>> {
        self.upsert_feed_tasks_inner(epic_id, items, repo_paths, base_branches, false)
            .await
    }

    /// Predicts nothing: unlike the two upserts, this delete has no items to
    /// compute a keep-set from — `keep_external_ids` IS the keep-set.
    ///
    /// One `list_tasks()` scan, not one `list_tasks_for_epic` call per child:
    /// the subscription cache has no per-epic index, so asking it once per
    /// child epic would scan every task in the board's view once per child —
    /// O(children × tasks) instead of O(tasks). A single scan filtered by the
    /// child-epic id set stays O(tasks) regardless of subtree fan-out.
    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        keep_external_ids: &[String],
    ) -> Result<Vec<crate::db::RemovedFeedTask>> {
        let children: std::collections::HashSet<EpicId> = self
            .reads
            .list_epics()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.parent_epic_id == Some(parent_id))
            .map(|e| e.id)
            .collect();
        let keep: std::collections::HashSet<&str> =
            keep_external_ids.iter().map(String::as_str).collect();
        let candidates: Vec<crate::models::Task> = self
            .reads
            .list_tasks()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.epic_id.is_some_and(|e| children.contains(&e)))
            .filter(|t| matches!(&t.external_id, Some(e) if !keep.contains(e.as_str())))
            .collect();

        self.caller
            .delete_stale_subtree_feed_tasks(parent_id.0, keep_external_ids.to_vec())
            .await?
            .applied()?;

        Ok(self.confirm_removed(candidates).await)
    }

    async fn create_repo_group_sub_epic(&self, parent_id: EpicId, title: &str) -> Result<EpicId> {
        // `sync.allium: CreatesRequireASettledIdentity`. Required even on the
        // FOUND arm, not only the created one: the module cannot tell this
        // caller which arm it took, so both need the same identity to stamp
        // — see `create_repo_group_sub_epic`'s doc comment in the module for
        // why `created_by` is what makes either answer readable back at all.
        let identity = self
            .require_identity("there is no name to stamp on a repo-group epic; it was not created")
            .await?;
        let id = self
            .caller
            .create_repo_group_sub_epic(parent_id.0, title.to_string(), identity)
            .await?;
        Ok(EpicId(id))
    }

    async fn create_managed_role_epic(
        &self,
        title: &str,
        parent_epic_id: Option<EpicId>,
        role: crate::models::FeedRole,
        feed_command: Option<&str>,
        feed_interval_secs: Option<i64>,
    ) -> Result<EpicId> {
        let identity = self
            .require_identity(
                "there is no name to stamp on a managed-role epic; it was not created",
            )
            .await?;
        let id = self
            .caller
            .create_managed_role_epic(
                title.to_string(),
                parent_epic_id.map(|e| e.0).unwrap_or(0),
                role.as_str().to_string(),
                feed_command.unwrap_or_default().to_string(),
                feed_interval_secs.unwrap_or(0),
                identity,
            )
            .await?;
        Ok(EpicId(id))
    }

    // -- Stragglers ------------------------------------------------------------

    async fn batch_patch_sub_status(&self, updates: &[(TaskId, SubStatus)]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }
        let wire = updates
            .iter()
            .map(|(id, sub_status)| bindings::SubStatusUpdate {
                task_id: id.0,
                sub_status: sub_status.as_str().to_string(),
            })
            .collect();
        self.caller.batch_patch_sub_status(wire).await?.applied()
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
            .respawn_phoenix_successor(predecessor.0, row)
            .await
    }

    // -- Task watchers -----------------------------------------------------------

    async fn create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()> {
        self.caller
            .create_task_watcher(watcher_task_id.0, target_task_id.0)
            .await?
            .applied()
    }

    async fn delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()> {
        self.caller
            .delete_task_watcher(watcher_task_id.0, target_task_id.0)
            .await?
            .applied()
    }

    async fn delete_watches_of_target(&self, target_task_id: TaskId) -> Result<()> {
        self.caller
            .delete_watches_of_target(target_task_id.0)
            .await?
            .applied()
    }

    async fn delete_watches_by_watcher(&self, watcher_task_id: TaskId) -> Result<()> {
        self.caller
            .delete_watches_by_watcher(watcher_task_id.0)
            .await?
            .applied()
    }
}
