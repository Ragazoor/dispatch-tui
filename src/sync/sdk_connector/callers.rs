//! [`SdkReducerCaller`]: every reducer call, over the live connection.

use anyhow::anyhow;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use spacetimedb_sdk::Table as _;
use std::sync::Arc;

use super::answer::{awaiting_answer, fire};
use super::outcome::{
    flag_or_refused, generated_id, is_review, matches_create, matches_created_epic,
    matches_created_learning, outcome_of, outcome_with_ids, subagent_drain_read_back,
    value_or_bail,
};
use super::SpacetimeSdkConnector;
use crate::models::{
    EpicId, LearningId, LearningVerdict, NotificationWrite, PollScopeId, RetrievalSource,
    SubStatus, TaskId,
};
use crate::spacetime::bindings;
use crate::spacetime::bindings::{
    apply_learning_verdicts as _, archive_stale_learnings as _, batch_delete as _,
    batch_patch_sub_status as _, claim_backlog_task as _, claim_poll_owner as _,
    clear_setting as _, create_epic as _, create_learning as _, create_managed_role_epic as _,
    create_repo_group_sub_epic as _, create_task as _, create_task_watcher as _, delete_epic as _,
    delete_learning as _, delete_repo_path as _, delete_stale_subtree_feed_tasks as _,
    delete_task as _, delete_task_watcher as _, delete_watches_by_watcher as _,
    delete_watches_of_target as _, drop_closed_retired_feed_items as _,
    mark_pr_learnings_gate_shown as _, override_poll_owner as _, patch_epic as _,
    patch_learning as _, patch_task as _, recalculate_epic_status as _, record_base_branch as _,
    record_learning_retrieval as _, record_notification as _, record_pre_tool_use as _,
    record_usage_event as _, record_user_prompt_submit as _, register_host as _,
    release_backlog_claim as _, rescope_epic_learnings as _, respawn_phoenix_successor as _,
    save_repo_path as _, save_setting as _, set_task_epic as _, set_verify_command as _,
    subagent_clear as _, subagent_clear_and_void_pending_stop as _, subagent_start as _,
    subagent_stop as _, subscribe_to_epic as _, try_record_stop as _, unsubscribe_from_epic as _,
    upsert_feed_tasks as _, upsert_feed_tasks_additive as _, DbConnection, EpicsTableAccess as _,
    LearningsTableAccess as _, TasksTableAccess as _,
};
use crate::sync::encode;
use crate::sync::writes::{DrainReadBack, ReducerCaller, ReducerOutcome};
use crate::sync::SettledIdentity;

/// [`ReducerCaller`] over this connector's live connection.
///
/// Spec: `docs/specs/sync.allium`'s `BoardWritesThroughTheStore`.
///
/// Holds the connector rather than a connection, because the connection is
/// replaced on every reconnect and a caller that captured one would go on
/// talking to a socket that is closed. Every call reads the current one, and
/// finding none is the refusal `AWriteWithNoConnectionIsRefused` demands.
pub struct SdkReducerCaller {
    connector: Arc<SpacetimeSdkConnector>,
    /// Where the session publishes why the connection is down.
    ///
    /// Read only when there is no connection, to make the refusal name the
    /// outage rather than merely report one — `sync.allium`'s
    /// `AWriteWithNoConnectionIsRefused` carries `connection.last_error`, and
    /// the session that owns it is deliberately not reachable from the
    /// transport.
    status: Arc<SettledIdentity>,
}

impl SdkReducerCaller {
    pub fn new(connector: Arc<SpacetimeSdkConnector>, status: Arc<SettledIdentity>) -> Self {
        Self { connector, status }
    }

    /// The live connection, or the refusal.
    ///
    /// ONE PLACE, so no call site can spell "the store is down" differently —
    /// the operator reads this string, and `EveryFailureNamesItself` says it
    /// has to be actionable.
    fn connection(&self) -> anyhow::Result<Arc<DbConnection>> {
        self.connector
            .connection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| match self.status.last_error() {
                Some(why) => anyhow!(
                    "the shared store is unreachable ({why}), so the change was not made \
                     and nothing was queued"
                ),
                // Before the first connection has failed there is nothing to
                // quote, and inventing a cause would be worse than saying only
                // what is certain.
                None => anyhow!(
                    "the shared store is not connected, so the change was not made \
                     and nothing was queued"
                ),
            })
    }
}

/// One reducer call whose only answer is "did it work?".
///
/// Written as a macro because the body is identical fifteen times over and the
/// only things that vary are the reducer's name and its arguments. Spelled out
/// fifteen times it would be fifteen chances to forget the callback.
macro_rules! answered_call {
    ($self:ident, $what:expr, $reducer:ident ( $($arg:expr),* $(,)? )) => {{
        let connection = $self.connection()?;
        awaiting_answer($what, move |tx| {
            connection
                .reducers
                .$reducer($($arg,)* move |_, result| {
                    fire(tx, outcome_of(result));
                })
        })
        .await
    }};
}

/// A create-shaped reducer call: send it, then read the generated id back off
/// the transaction by matching the row this board just sent.
///
/// The sibling of [`answered_call!`] for the calls whose answer is an id. What
/// varies is the reducer, the table to search and the predicate that picks
/// out the caller's own row; everything else — the callback, the read-back
/// and the refusal text of [`generated_id`] — is written once here.
macro_rules! created_call {
    ($self:ident, $what:expr, $label:expr, $reducer:ident ( $($arg:expr),* $(,)? ), $table:ident, |$row:ident| $pred:expr) => {{
        let connection = $self.connection()?;
        let answer = awaiting_answer($what, move |tx| {
            connection
                .reducers
                .$reducer($($arg,)* move |ctx, result| {
                    fire(
                        tx,
                        outcome_with_ids(result, || {
                            ctx.db
                                .$table()
                                .iter()
                                .filter(|$row| $pred)
                                .map(|$row| $row.id)
                                .collect()
                        }),
                    );
                })
        })
        .await?;
        generated_id(answer, $label)
    }};
}

#[async_trait]
impl ReducerCaller for SdkReducerCaller {
    /// Create a task and read its generated id back off the transaction.
    ///
    /// # How a reducer answers, given that it cannot
    ///
    /// The callback runs with a view of the database AFTER this transaction, so
    /// the row is there — the problem is saying which one it is. The row is
    /// matched on the fields this board just sent: the title, the repo, the
    /// owner, the epic, the creator and the creation instant, which is this
    /// board's clock to the millisecond. The highest matching id is taken.
    ///
    /// **The tie is real and it is benign.** Two identical creates from the
    /// same board inside one millisecond produce two indistinguishable rows,
    /// and this returns the later one's id. Both were genuinely created and
    /// both are the caller's; returning either returns a task the caller just
    /// made. What it cannot do is return somebody else's row, because the
    /// creator and the creation instant are ours.
    async fn create_task(&self, row: bindings::Task) -> anyhow::Result<TaskId> {
        let wanted = row.clone();
        created_call!(
            self,
            "the new task",
            "task",
            create_task_then(row),
            tasks,
            |t| matches_create(t, &wanted)
        )
        .map(TaskId)
    }

    async fn patch_task(
        &self,
        id: TaskId,
        patch: bindings::TaskPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the task change", patch_task_then(id.0, patch))
    }

    async fn delete_task(&self, id: TaskId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the task deletion", delete_task_then(id.0))
    }

    async fn set_task_epic(
        &self,
        id: TaskId,
        epic_id: Option<EpicId>,
        owner: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the epic move",
            set_task_epic_then(id.0, encode::epic_ref(epic_id), owner)
        )
    }

    async fn claim_backlog_task(&self, id: TaskId, host: String) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the claim", claim_backlog_task_then(id.0, host))
    }

    async fn release_backlog_claim(&self, id: TaskId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the claim release", release_backlog_claim_then(id.0))
    }

    /// The epic twin of [`Self::create_task`], matched the same way.
    async fn create_epic(&self, row: bindings::Epic) -> anyhow::Result<EpicId> {
        let wanted = row.clone();
        created_call!(
            self,
            "the new epic",
            "epic",
            create_epic_then(row),
            epics,
            |e| matches_created_epic(e, &wanted)
        )
        .map(EpicId)
    }

    async fn patch_epic(
        &self,
        id: EpicId,
        patch: bindings::EpicPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the epic change", patch_epic_then(id.0, patch))
    }

    async fn delete_epic(&self, id: EpicId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the epic deletion", delete_epic_then(id.0))
    }

    async fn batch_delete(
        &self,
        task_ids: Vec<TaskId>,
        epic_ids: Vec<EpicId>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the batch delete",
            batch_delete_then(
                task_ids.into_iter().map(|id| id.0).collect(),
                epic_ids.into_iter().map(|id| id.0).collect()
            )
        )
    }

    async fn recalculate_epic_status(&self, id: EpicId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the epic recalculation",
            recalculate_epic_status_then(id.0)
        )
    }

    async fn save_repo_path(
        &self,
        path: String,
        last_used: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the repo path",
            save_repo_path_then(path, encode::stamp(last_used))
        )
    }

    async fn delete_repo_path(&self, path: String) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the repo path removal", delete_repo_path_then(path))
    }

    async fn set_verify_command(
        &self,
        path: String,
        command: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the verify command",
            set_verify_command_then(path, command)
        )
    }

    async fn record_base_branch(
        &self,
        repo_path: String,
        branch: String,
        last_used: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the base branch",
            record_base_branch_then(repo_path, branch, encode::stamp(last_used))
        )
    }

    async fn subscribe_to_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the subscription",
            subscribe_to_epic_then(subscriber, epic_id.0)
        )
    }

    async fn unsubscribe_from_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the unsubscribe",
            unsubscribe_from_epic_then(subscriber, epic_id.0)
        )
    }

    async fn save_setting(
        &self,
        host: String,
        key: String,
        value: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the setting", save_setting_then(host, key, value))
    }

    async fn clear_setting(&self, host: String, key: String) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the setting clear", clear_setting_then(host, key))
    }

    // -- Learnings and retrievals (Phase 10, task #4914) ----------------------

    /// Create a learning and read its generated id back off the transaction —
    /// the same mechanism [`Self::create_task`] uses, matched on content
    /// instead of on identity because a learning is not owned by anyone.
    async fn create_learning(&self, row: bindings::Learning) -> anyhow::Result<LearningId> {
        let wanted = row.clone();
        created_call!(
            self,
            "the new learning",
            "learning",
            create_learning_then(row),
            learnings,
            |l| matches_created_learning(l, &wanted)
        )
        .map(LearningId)
    }

    async fn patch_learning(
        &self,
        id: LearningId,
        patch: bindings::LearningPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the learning change",
            patch_learning_then(id.0, patch)
        )
    }

    async fn delete_learning(&self, id: LearningId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the learning deletion", delete_learning_then(id.0))
    }

    async fn rescope_epic_learnings(
        &self,
        from: EpicId,
        to: EpicId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the learning re-scope",
            rescope_epic_learnings_then(from.0, to.0)
        )
    }

    async fn record_learning_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the learning retrieval",
            record_learning_retrieval_then(task_id.0, learning_id.0, source.as_str().to_string())
        )
    }

    async fn apply_learning_verdicts(
        &self,
        verdicts: Vec<(LearningId, LearningVerdict)>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the learning verdicts",
            apply_learning_verdicts_then(encode::verdict_inputs(&verdicts))
        )
    }

    async fn archive_stale_learnings(
        &self,
        cutoff: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the stale-learning sweep",
            archive_stale_learnings_then(encode::stamp(cutoff))
        )
    }

    // -- Usage events (Phase 11, task #4915) ---------------------------------

    async fn record_usage_event(
        &self,
        row: bindings::UsageEvent,
        cap: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the usage event", record_usage_event_then(row, cap))
    }

    // -- Agent session state (Phase 6b) --------------------------------------
    //
    // Every method below acts on a row whose id is already known — never a
    // generated one — so its answer is a read of THAT row by primary key,
    // inside the `_then` callback, whose view is "after this transaction" the
    // same way `create_task`'s is. No subscription-widening question arises:
    // a task with a live hook firing against it is one this board is already
    // running, hence already subscribed.
    //
    // Six of these answer with a bespoke type (`i64`, `DrainReadBack`,
    // `Option<bool>`) rather than `ReducerOutcome`'s `Vec<i64>`, because each
    // has a genuine fact to read back and a positional slot is exactly the
    // kind of thing a transposed index compiles cleanly through. None of them
    // has an application-level refusal that `ReducerOutcome::Refused` needs
    // to carry EXCEPT `try_record_stop`, whose `None` plays that role
    // directly. The other five never refuse (they mirror SQL paths with no
    // `requires` guard at all), so an `InternalError` from the SDK is folded
    // into a genuine `anyhow::Error` here rather than into "nothing
    // happened" — unlike [`outcome_of`]'s fold, which is correct for every
    // reducer that DOES have an ordinary refusal to answer with.

    /// `live_subagents` after the write. Never refuses: starting a subagent has no precondition.
    async fn subagent_start(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
        started_at: DateTime<Utc>,
    ) -> anyhow::Result<i64> {
        let connection = self.connection()?;
        let (task_id, started_at) = (task_id.0, encode::subagent_started_at(started_at));
        awaiting_answer("the subagent start", move |tx| {
            connection.reducers.subagent_start_then(
                task_id,
                agent_id,
                session_id,
                started_at,
                move |ctx, result| {
                    fire(
                        tx,
                        value_or_bail(result, "the subagent start", || {
                            ctx.db
                                .tasks()
                                .id()
                                .find(&task_id)
                                .map_or(0, |t| t.live_subagents)
                        }),
                    );
                },
            )
        })
        .await?
    }

    /// The live subagent count and whether the row is now in `review`, after
    /// the write. Never refuses: an unrecognised
    /// `agent_id` is a no-op rather than an error.
    async fn subagent_stop(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
    ) -> anyhow::Result<DrainReadBack> {
        let connection = self.connection()?;
        let task_id = task_id.0;
        awaiting_answer("the subagent stop", move |tx| {
            connection.reducers.subagent_stop_then(
                task_id,
                agent_id,
                session_id,
                move |ctx, result| {
                    fire(
                        tx,
                        value_or_bail(result, "the subagent stop", || {
                            subagent_drain_read_back(ctx, task_id)
                        }),
                    );
                },
            )
        })
        .await?
    }

    /// Same shape as [`Self::subagent_stop`].
    async fn subagent_clear(&self, task_id: TaskId) -> anyhow::Result<DrainReadBack> {
        let connection = self.connection()?;
        let task_id = task_id.0;
        awaiting_answer("the subagent clear", move |tx| {
            connection
                .reducers
                .subagent_clear_then(task_id, move |ctx, result| {
                    fire(
                        tx,
                        value_or_bail(result, "the subagent clear", || {
                            subagent_drain_read_back(ctx, task_id)
                        }),
                    );
                })
        })
        .await?
    }

    async fn subagent_clear_and_void_pending_stop(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the subagent clear",
            subagent_clear_and_void_pending_stop_then(task_id.0)
        )
    }

    /// `None` when the task was not `Running` (this task's refusal, and the
    /// ONE agent-session-state method here with an application-level "no").
    /// Otherwise `Some(is_review)` — `true` if the row is now in `review`,
    /// `false` if it deferred instead. See `try_record_stop`'s doc comment in
    /// the module for why refusing the precondition is what makes this
    /// unambiguous.
    async fn try_record_stop(
        &self,
        id: TaskId,
        stop_pending_at: DateTime<Utc>,
    ) -> anyhow::Result<Option<bool>> {
        let connection = self.connection()?;
        let (id, stop_pending_at) = (id.0, encode::stamp(stop_pending_at));
        awaiting_answer("the stop", move |tx| {
            connection
                .reducers
                .try_record_stop_then(id, stop_pending_at, move |ctx, result| {
                    fire(
                        tx,
                        flag_or_refused(result, || {
                            ctx.db
                                .tasks()
                                .id()
                                .find(&id)
                                .is_some_and(|t| is_review(&t.status))
                        }),
                    );
                })
        })
        .await
    }

    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the activity stamp",
            record_pre_tool_use_then(id.0, sub_status.as_str().to_string(), encode::stamp(at))
        )
    }

    async fn record_notification(
        &self,
        id: TaskId,
        mode: NotificationWrite,
        at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        let Some(mode) = encode::notification_mode(mode) else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        answered_call!(
            self,
            "the notification",
            record_notification_then(id.0, mode.to_string(), encode::stamp(at))
        )
    }

    /// Plain applied/refused — `Resumed` vs. `Refreshed` is not decodable
    /// here at all (both write the identical row); `ReducerWriter` classifies
    /// it from a pre-read instead. See its own doc comment.
    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        activity_at: DateTime<Utc>,
        prompt_at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the prompt",
            record_user_prompt_submit_then(
                id.0,
                encode::stamp(activity_at),
                encode::stamp(prompt_at)
            )
        )
    }

    async fn mark_pr_learnings_gate_shown(
        &self,
        id: TaskId,
        at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the PR learnings gate",
            mark_pr_learnings_gate_shown_then(id.0, encode::stamp(at))
        )
    }

    // -- Feed ingestion (Phase 6c) --------------------------------------------
    //
    // Plain applied-or-refused calls: which rows a stale-delete removed is
    // decoded in `ReducerWriter`, from its own pre-read, not here — see this
    // task's plan doc, decision 1.

    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the feed upsert",
            upsert_feed_tasks_then(epic_id.0, items, created_by)
        )
    }

    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the additive feed upsert",
            upsert_feed_tasks_additive_then(epic_id.0, items, created_by)
        )
    }

    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        keep_external_ids: Vec<String>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the stale feed task cleanup",
            delete_stale_subtree_feed_tasks_then(parent_id.0, keep_external_ids)
        )
    }

    async fn drop_closed_retired_feed_items(
        &self,
        feed_epic_id: EpicId,
        keep_external_ids: Vec<String>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the closed-retirement cleanup",
            drop_closed_retired_feed_items_then(feed_epic_id.0, keep_external_ids)
        )
    }

    /// Find-or-create: matched by the domain key `(parent_id, title)` rather
    /// than content/timestamp — exact, not a tie-break, because that pair is
    /// genuinely unique (the module's own `create_repo_group_sub_epic` doc
    /// comment). Covers both arms: a FOUND epic answers with its existing id
    /// the same way a freshly created one answers with its new one.
    async fn create_repo_group_sub_epic(
        &self,
        parent_id: EpicId,
        title: String,
        created_by: String,
    ) -> anyhow::Result<EpicId> {
        let parent_id = parent_id.0;
        let wanted_title = title.clone();
        created_call!(
            self,
            "the repo-group epic",
            "repo-group epic",
            create_repo_group_sub_epic_then(parent_id, title, created_by),
            epics,
            |e| e.parent_epic_id == parent_id
                && e.title == wanted_title
                && e.origin == "repo-group"
        )
        .map(EpicId)
    }

    /// The managed-role twin, matched on `(parent_epic_id, feed_role)` — the
    /// module's own uniqueness key for this find-or-create, same reasoning as
    /// [`Self::create_repo_group_sub_epic`].
    async fn create_managed_role_epic(
        &self,
        title: String,
        parent_epic_id: Option<EpicId>,
        role: String,
        feed_command: String,
        feed_interval_secs: i64,
        created_by: String,
    ) -> anyhow::Result<EpicId> {
        let parent_epic_id = encode::epic_ref(parent_epic_id);
        let wanted_role = role.clone();
        created_call!(
            self,
            "the managed-role epic",
            "managed-role epic",
            create_managed_role_epic_then(
                title,
                parent_epic_id,
                role,
                feed_command,
                feed_interval_secs,
                created_by
            ),
            epics,
            |e| e.parent_epic_id == parent_epic_id && e.feed_role == wanted_role
        )
        .map(EpicId)
    }

    // -- Task watchers -----------------------------------------------------------

    async fn create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the watch",
            create_task_watcher_then(watcher_task_id.0, target_task_id.0)
        )
    }

    async fn delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the watch removal",
            delete_task_watcher_then(watcher_task_id.0, target_task_id.0)
        )
    }

    async fn delete_watches_of_target(
        &self,
        target_task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the target's watches",
            delete_watches_of_target_then(target_task_id.0)
        )
    }

    async fn delete_watches_by_watcher(
        &self,
        watcher_task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the watcher's watches",
            delete_watches_by_watcher_then(watcher_task_id.0)
        )
    }

    async fn claim_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let (scope, scope_id) = target.wire();
        answered_call!(
            self,
            "the poll claim",
            claim_poll_owner_then(scope.to_string(), scope_id, host)
        )
    }

    async fn override_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let (scope, scope_id) = target.wire();
        answered_call!(
            self,
            "the poll override",
            override_poll_owner_then(scope.to_string(), scope_id, host)
        )
    }

    // -- Stragglers ------------------------------------------------------------

    async fn batch_patch_sub_status(
        &self,
        updates: Vec<(TaskId, SubStatus)>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the sub-status batch",
            batch_patch_sub_status_then(encode::sub_status_updates(&updates))
        )
    }

    /// The phoenix twin of [`Self::create_task`]: matched the same way, on
    /// the fields the caller chose, not on `predecessor` — the successor is
    /// as much "the caller's own" as any other create, and a tie between two
    /// identical successors is the same benign case `matches_create` already
    /// accepts.
    async fn respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        successor: bindings::Task,
    ) -> anyhow::Result<TaskId> {
        let predecessor = predecessor.0;
        let wanted = successor.clone();
        created_call!(
            self,
            "the phoenix successor",
            "phoenix successor",
            respawn_phoenix_successor_then(predecessor, successor),
            tasks,
            |t| matches_create(t, &wanted)
        )
        .map(TaskId)
    }

    // -- Host registry (Phase 6c) -----------------------------------------------

    async fn register_host(
        &self,
        id: String,
        label: String,
        owner: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the host registration",
            register_host_then(id, label, owner)
        )
    }
}
