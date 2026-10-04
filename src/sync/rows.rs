//! What the subscription has delivered, as the board's own types.
//!
//! Spec: `docs/specs/sync.allium`'s `BoardReadsFromTheSubscription` and
//! `SubscribedRowsArriveUnasked`.
//!
//! # This is not a cache
//!
//! Nothing here is a second copy of anything. `core.allium`'s shared tables
//! have exactly one copy — the store's — and this is the client end of the live
//! view of it, held in memory for as long as the connection is up and gone the
//! moment it is not. There is no read-through, no staleness window and no
//! fallback to disk: a board with no connection has no rows, which is the
//! bargain Phase 5 of the migration plan records and accepts.
//!
//! Calling it a cache would suggest the one behaviour it must never have —
//! answering from an older copy when the live one is unavailable — so it is
//! called what it is.
//!
//! # Ordering is part of the contract
//!
//! Every accessor returns rows in the order SQLite's corresponding `ORDER BY`
//! returns them, because "renders identically" is a claim about a `Vec`, not
//! about a set. The orderings are stated at each accessor and are the reason
//! this type keeps `BTreeMap`s rather than hash maps: the tiebreak is then the
//! id, deterministically, without a second sort.
//!
//! # The change signal
//!
//! [`SharedRows::changed`] hands out a receiver that fires when anything in
//! here moves. It is what makes a teammate's edit appear without a poll: the
//! board waits on it instead of re-reading on a timer, so a quiet store costs
//! nothing and a busy one redraws as fast as rows arrive. (The board is not
//! otherwise idle — the connection loop still wakes on its own interval to
//! collect a reported drop — but nothing re-reads these rows speculatively.)

use std::collections::BTreeMap;
use std::sync::RwLock;

use chrono::{DateTime, Utc};

use tokio::sync::watch;

use crate::db::{LearningFilter, UsageQuery};
use crate::models::{
    Epic, EpicId, Learning, LearningId, LearningRetrieval, PollScopeId, Task, TaskId, UsageSummary,
};
use crate::spacetime::bindings;

use super::decode;

/// One `repo_paths` row: the shared list of repositories the board offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPathRow {
    pub id: i64,
    pub path: String,
    pub last_used: String,
    pub verify_command: Option<String>,
}

/// One `repo_base_branches` row: a repo's recently-used base branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoBaseBranchRow {
    pub id: i64,
    pub repo_path: String,
    pub branch: String,
    pub last_used: String,
}

/// One `hosts` row: a machine on this board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRow {
    pub id: String,
    pub label: Option<String>,
    pub owner: Option<String>,
}

/// One `poll_owners` row: which host is allowed to run recurring background
/// polling for a task or an epic with no natural owner of its own
/// (`core.allium: PollOwner`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollOwnerRow {
    pub id: i64,
    /// `"task"` or `"epic"` — see `pr-workflow.allium: PollPrStatus` and
    /// `feeds.allium: FeedTick`, the two consumers.
    pub scope: String,
    pub scope_id: i64,
    /// The owning `Host.id`.
    pub host: String,
}

/// One `usage_events` row (Phase 11, task #4915).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageEventRow {
    pub id: i64,
    pub recorded_at: DateTime<Utc>,
    pub category: String,
    pub action: String,
    pub detail: Option<String>,
    pub actor: String,
}

/// One `retired_feed_items` row (task #4971): `core.allium: RetiredFeedItem`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredFeedItemRow {
    pub id: i64,
    pub feed_epic_id: EpicId,
    pub external_id: String,
    /// Audit only; no rule reads this to decide anything (`core.allium`'s
    /// doc comment on `RetiredFeedItem.retired_at`).
    pub retired_at: String,
}

#[derive(Default)]
struct Rows {
    tasks: BTreeMap<i64, Task>,
    epics: BTreeMap<i64, Epic>,
    repo_paths: BTreeMap<i64, RepoPathRow>,
    repo_base_branches: BTreeMap<i64, RepoBaseBranchRow>,
    hosts: BTreeMap<String, HostRow>,
    poll_owners: BTreeMap<i64, PollOwnerRow>,
    /// `(scope, scope_id) -> id`, kept in step with `poll_owners` on every
    /// insert/remove. `poll_owner()` is read on every `PollPrStatus`/
    /// `FeedTick` tick and must not degrade to a scan as the table grows.
    poll_owners_by_scope: BTreeMap<(String, i64), i64>,
    /// The knowledge base (Phase 10, task #4914). Unlike every table above,
    /// nothing here is scoped by owner or epic — subscribed to unconditionally,
    /// like `repo_paths` — so there is no per-caller filtering to do on the way
    /// in, only on the way out (`SharedRows::learnings_matching`).
    ///
    /// The embedding travels alongside its `Learning` in one map rather than
    /// in a second one keyed the same way: two maps that must always agree on
    /// which ids exist is the kind of invariant a single map holds for free.
    learnings: BTreeMap<i64, (Learning, Option<Vec<u8>>)>,
    learning_retrievals: BTreeMap<i64, LearningRetrieval>,
    /// Telemetry (Phase 11, task #4915). Unconditionally subscribed, like
    /// `learnings` above — nothing here is scoped by owner or host.
    usage_events: BTreeMap<i64, UsageEventRow>,
    /// The three tables `db::SharedReader` needs beyond the ones above (task
    /// #4916). None carries a sentinel, an enum or a timestamp a reader
    /// interprets, so they are held as the store sends them.
    ///
    /// `settings` arrives already scoped to this host by the subscription's
    /// `WHERE host = …`, so nothing here filters by host.
    task_watchers: BTreeMap<i64, bindings::TaskWatcher>,
    subscriptions: BTreeMap<String, bindings::Subscription>,
    settings: BTreeMap<String, bindings::Setting>,
    /// Task #4971. Unconditionally subscribed, like `learnings`/`usage_events`
    /// above — a retirement record is not scoped by owner or host either.
    retired_feed_items: BTreeMap<i64, RetiredFeedItemRow>,
}

impl Rows {
    fn is_empty(&self) -> bool {
        self.tasks.is_empty()
            && self.epics.is_empty()
            && self.repo_paths.is_empty()
            && self.repo_base_branches.is_empty()
            && self.hosts.is_empty()
            && self.poll_owners.is_empty()
            && self.learnings.is_empty()
            && self.learning_retrievals.is_empty()
            && self.usage_events.is_empty()
            && self.task_watchers.is_empty()
            && self.subscriptions.is_empty()
            && self.settings.is_empty()
            && self.retired_feed_items.is_empty()
    }
}

/// The live view of this board's subscriptions.
///
/// Shared between the connection (which writes) and the board (which reads), so
/// every method takes `&self`.
pub struct SharedRows {
    rows: RwLock<Rows>,
    /// Bumped on every mutation. The value is a generation counter rather than
    /// a description of what moved: a board that redraws from the whole set has
    /// no use for a delta, and a signal carrying one would invite a reader to
    /// apply it and drift.
    changed: watch::Sender<u64>,
}

impl Default for SharedRows {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedRows {
    pub fn new() -> Self {
        Self {
            rows: RwLock::new(Rows::default()),
            changed: watch::channel(0).0,
        }
    }

    /// A receiver that fires whenever anything here moves.
    ///
    /// `watch` rather than a broadcast queue on purpose: a reader that falls
    /// behind should redraw from the current state once, not replay every
    /// intermediate one. Coalescing is the correct behaviour here, not a
    /// limitation being tolerated.
    pub fn changed(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// The generation the current contents are at. Test scaffolding and
    /// logging; nothing branches on the value.
    pub fn generation(&self) -> u64 {
        *self.changed.borrow()
    }

    /// Apply a mutation, and wake the board only if `f` says something moved.
    ///
    /// The `bool` is not an optimisation of the redraw — that is idempotent and
    /// cheap. It is what keeps the revision number meaning what
    /// [`Self::generation`]'s readers assume: a no-op remove, or a `clear()` of
    /// an already-empty set, would otherwise advance it and make the board's
    /// tick guard re-read a board nothing had changed. `clear()` runs on every
    /// connect and every disconnect, so on a board with no rows that was a
    /// guaranteed spurious redraw per reconnect.
    fn write(&self, f: impl FnOnce(&mut Rows) -> bool) {
        #[allow(clippy::unwrap_used)]
        let mut rows = self.rows.write().unwrap_or_else(|e| e.into_inner());
        let moved = f(&mut rows);
        drop(rows);
        if moved {
            self.changed.send_modify(|generation| *generation += 1);
        }
    }

    fn read<T>(&self, f: impl FnOnce(&Rows) -> T) -> T {
        #[allow(clippy::unwrap_used)]
        let rows = self.rows.read().unwrap_or_else(|e| e.into_inner());
        f(&rows)
    }

    // -- Applying what arrived ---------------------------------------------

    /// Take a row the store delivered, replacing any row with the same id.
    ///
    /// A row that does not decode is DROPPED and logged, not kept in its old
    /// form and not made into an error the caller has to handle. See
    /// `decode`'s header for why refusing beats guessing: a task written by a
    /// newer binary is missing from the board rather than sitting on it under a
    /// plausible wrong status.
    pub fn upsert_task(&self, row: &bindings::Task) {
        if crate::models::is_removed_status(&row.status) {
            // A removed status is dropped on purpose, not a decode failure:
            // no warning, no counter. `RowsWithARemovedStatusAreDropped`.
            tracing::debug!(id = row.id, "skipping a task with a removed status");
            self.remove_task(TaskId(row.id));
            return;
        }
        match decode::task(row) {
            Ok(task) => self.write(|rows| {
                rows.tasks.insert(task.id.0, task);
                true
            }),
            Err(e) => {
                // Counted as well as logged: this is the same bargain
                // `collect_decodable` makes for SQLite's bulk reads, and
                // `db::decode_fallback_count` is the one number that says a
                // board is quietly dropping rows.
                let count = crate::db::bump_decode_fallback();
                tracing::warn!(
                    count,
                    "dropping an undecodable task from the shared store: {e}"
                );
            }
        }
    }

    pub fn remove_task(&self, id: TaskId) {
        self.write(|rows| rows.tasks.remove(&id.0).is_some());
    }

    pub fn upsert_epic(&self, row: &bindings::Epic) {
        if crate::models::is_removed_status(&row.status) {
            tracing::debug!(id = row.id, "skipping an epic with a removed status");
            self.remove_epic(EpicId(row.id));
            return;
        }
        match decode::epic(row) {
            Ok(epic) => self.write(|rows| {
                rows.epics.insert(epic.id.0, epic);
                true
            }),
            Err(e) => {
                // Counted as well as logged: this is the same bargain
                // `collect_decodable` makes for SQLite's bulk reads, and
                // `db::decode_fallback_count` is the one number that says a
                // board is quietly dropping rows.
                let count = crate::db::bump_decode_fallback();
                tracing::warn!(
                    count,
                    "dropping an undecodable epic from the shared store: {e}"
                );
            }
        }
    }

    pub fn remove_epic(&self, id: EpicId) {
        self.write(|rows| rows.epics.remove(&id.0).is_some());
    }

    pub fn upsert_repo_path(&self, row: &bindings::RepoPath) {
        let value = decode::repo_path(row);
        self.write(|rows| {
            rows.repo_paths.insert(value.id, value);
            true
        });
    }

    pub fn remove_repo_path(&self, id: i64) {
        self.write(|rows| rows.repo_paths.remove(&id).is_some());
    }

    pub fn upsert_repo_base_branch(&self, row: &bindings::RepoBaseBranch) {
        let value = decode::repo_base_branch(row);
        self.write(|rows| {
            rows.repo_base_branches.insert(value.id, value);
            true
        });
    }

    pub fn remove_repo_base_branch(&self, id: i64) {
        self.write(|rows| rows.repo_base_branches.remove(&id).is_some());
    }

    pub fn upsert_host(&self, row: &bindings::Host) {
        let value = decode::host(row);
        self.write(|rows| {
            rows.hosts.insert(value.id.clone(), value);
            true
        });
    }

    pub fn remove_host(&self, id: &str) {
        self.write(|rows| rows.hosts.remove(id).is_some());
    }

    pub fn upsert_poll_owner(&self, row: &bindings::PollOwner) {
        let value = decode::poll_owner(row);
        self.write(|rows| {
            rows.poll_owners_by_scope
                .insert((value.scope.clone(), value.scope_id), value.id);
            rows.poll_owners.insert(value.id, value);
            true
        });
    }

    pub fn remove_poll_owner(&self, id: i64) {
        self.write(|rows| match rows.poll_owners.remove(&id) {
            Some(row) => {
                rows.poll_owners_by_scope.remove(&(row.scope, row.scope_id));
                true
            }
            None => false,
        })
    }

    pub fn upsert_learning(&self, row: &bindings::Learning) {
        match decode::learning(row) {
            Ok(learning) => {
                let embedding = row.embedding.clone();
                self.write(|rows| {
                    rows.learnings.insert(learning.id.0, (learning, embedding));
                    true
                })
            }
            Err(e) => {
                let count = crate::db::bump_decode_fallback();
                tracing::warn!(
                    count,
                    "dropping an undecodable learning from the shared store: {e}"
                );
            }
        }
    }

    pub fn remove_learning(&self, id: LearningId) {
        self.write(|rows| rows.learnings.remove(&id.0).is_some());
    }

    pub fn upsert_learning_retrieval(&self, row: &bindings::LearningRetrieval) {
        match decode::learning_retrieval(row) {
            Ok(retrieval) => self.write(|rows| {
                rows.learning_retrievals.insert(retrieval.id, retrieval);
                true
            }),
            Err(e) => {
                let count = crate::db::bump_decode_fallback();
                tracing::warn!(
                    count,
                    "dropping an undecodable learning retrieval from the shared store: {e}"
                );
            }
        }
    }

    pub fn remove_learning_retrieval(&self, id: i64) {
        self.write(|rows| rows.learning_retrievals.remove(&id).is_some());
    }

    pub fn upsert_usage_event(&self, row: &bindings::UsageEvent) {
        match decode::usage_event(row) {
            Ok(event) => self.write(|rows| {
                rows.usage_events.insert(event.id, event);
                true
            }),
            Err(e) => {
                let count = crate::db::bump_decode_fallback();
                tracing::warn!(
                    count,
                    "dropping an undecodable usage event from the shared store: {e}"
                );
            }
        }
    }

    pub fn remove_usage_event(&self, id: i64) {
        self.write(|rows| rows.usage_events.remove(&id).is_some());
    }

    pub fn upsert_task_watcher(&self, row: &bindings::TaskWatcher) {
        self.write(|rows| {
            rows.task_watchers.insert(row.id, row.clone());
            true
        });
    }

    pub fn remove_task_watcher(&self, id: i64) {
        self.write(|rows| rows.task_watchers.remove(&id).is_some());
    }

    pub fn upsert_subscription(&self, row: &bindings::Subscription) {
        self.write(|rows| {
            rows.subscriptions.insert(row.id.clone(), row.clone());
            true
        });
    }

    pub fn remove_subscription(&self, id: String) {
        self.write(|rows| rows.subscriptions.remove(&id).is_some());
    }

    pub fn upsert_setting(&self, row: &bindings::Setting) {
        self.write(|rows| {
            rows.settings.insert(row.id.clone(), row.clone());
            true
        });
    }

    pub fn remove_setting(&self, id: String) {
        self.write(|rows| rows.settings.remove(&id).is_some());
    }

    /// Infallible decode (see [`decode::retired_feed_item`]), so unlike the
    /// upserts above this one never has a dropped-row branch to log.
    pub fn upsert_retired_feed_item(&self, row: &bindings::RetiredFeedItem) {
        let item = decode::retired_feed_item(row);
        self.write(|rows| {
            rows.retired_feed_items.insert(item.id, item);
            true
        });
    }

    pub fn remove_retired_feed_item(&self, id: i64) {
        self.write(|rows| rows.retired_feed_items.remove(&id).is_some());
    }

    /// Drop everything.
    ///
    /// Called when a connection goes down. The rows belonged to that
    /// subscription and the next one re-delivers them from scratch; keeping
    /// them across the gap is exactly the read-through this type does not do,
    /// and would put a board's stale contents on screen with no indication that
    /// nothing behind them is live.
    pub fn clear(&self) {
        self.write(|rows| {
            let had_rows = !rows.is_empty();
            *rows = Rows::default();
            had_rows
        });
    }

    // -- Reading -----------------------------------------------------------

    /// Every task, ordered as `TaskRead::list_all` orders them:
    /// `COALESCE(sort_order, id) ASC, id ASC`.
    pub fn tasks(&self) -> Vec<Task> {
        self.read(|rows| sorted_by_key(rows.tasks.values(), Task::sort_key))
    }

    pub fn task(&self, id: TaskId) -> Option<Task> {
        self.read(|rows| rows.tasks.get(&id.0).cloned())
    }

    /// Tasks under one epic, in the same order.
    pub fn tasks_for_epic(&self, epic: EpicId) -> Vec<Task> {
        self.read(|rows| {
            sorted_by_key(
                rows.tasks.values().filter(|t| t.epic_id == Some(epic)),
                Task::sort_key,
            )
        })
    }

    /// Whether `id` is held, without cloning it.
    pub fn has_task(&self, id: TaskId) -> bool {
        self.read(|rows| rows.tasks.contains_key(&id.0))
    }

    /// Running or Review tasks with a tmux window, ordered by id — the
    /// `list_live_agent_tasks` query. Filtered before anything is cloned: the
    /// agent-tree pane polls this every second.
    pub fn live_agent_tasks(&self) -> Vec<Task> {
        use crate::models::TaskStatus;
        self.read(|rows| {
            rows.tasks
                .values()
                .filter(|t| {
                    matches!(t.status, TaskStatus::Running | TaskStatus::Review)
                        && t.tmux_window.is_some()
                })
                .cloned()
                .collect()
        })
    }

    /// The lowest-id task whose plan is `plan`.
    pub fn task_by_plan(&self, plan: &str) -> Option<Task> {
        self.read(|rows| {
            rows.tasks
                .values()
                .find(|t| t.plan_path.as_deref() == Some(plan))
                .cloned()
        })
    }

    /// Every task with an epic, ordered `epic_id ASC, COALESCE(sort_order, id)
    /// ASC, id ASC` — the `list_all_tasks_with_epic_id` query.
    pub fn tasks_with_epic(&self) -> Vec<Task> {
        self.read(|rows| {
            let mut out: Vec<&Task> = rows
                .tasks
                .values()
                .filter(|t| t.epic_id.is_some())
                .collect();
            out.sort_by_key(|t| (t.epic_id.map(|e| e.0), t.sort_key()));
            out.into_iter().cloned().collect()
        })
    }

    /// Epics whose parent is `parent` (`None` for the roots), in
    /// [`Self::epics`]'s order.
    pub fn epics_with_parent(&self, parent: Option<EpicId>) -> Vec<Epic> {
        self.read(|rows| {
            sorted_by_key(
                rows.epics.values().filter(|e| e.parent_epic_id == parent),
                Epic::sort_key,
            )
        })
    }

    pub fn epics(&self) -> Vec<Epic> {
        self.read(|rows| sorted_by_key(rows.epics.values(), Epic::sort_key))
    }

    pub fn epic(&self, id: EpicId) -> Option<Epic> {
        self.read(|rows| rows.epics.get(&id.0).cloned())
    }

    // `hosts` is DELIVERED and stored but has no accessor, and that is the
    // honest state rather than an omission: a card naming another machine needs
    // the registry present when something finally resolves it, and an accessor
    // written now would be an ordering nothing tests and nothing calls. Phase 6
    // adds the reader together with the test that needs it.
    //
    // `poll_owners` is the exception, added in Phase 7: `PollPrStatus`/
    // `FeedTick` need "who owns this scope?" on every tick, so this reader
    // arrives with its own consumer rather than waiting the way `hosts` is.

    /// The `PollOwner` row for `(scope, scope_id)`, or `None` if unclaimed.
    /// `core.allium: PollOwner`.
    pub fn poll_owner(&self, target: PollScopeId) -> Option<PollOwnerRow> {
        let (scope, scope_id) = target.wire();
        self.read(|rows| {
            let id = rows
                .poll_owners_by_scope
                .get(&(scope.to_string(), scope_id))?;
            rows.poll_owners.get(id).cloned()
        })
    }

    /// The repo paths, most recently used first: `last_used DESC, id ASC`.
    ///
    /// The ascending tiebreak looks backwards beside `base_branches` below, and
    /// is deliberate: `last_used` has whole-second resolution, so paths saved in
    /// one second tie, and SQLite has always broken that tie by rowid ascending.
    /// Matching it is what keeps the picker's order the order it has always had
    /// — see the same `ORDER BY` in `RepoConfigStore::list_repo_paths`.
    pub fn repo_paths(&self) -> Vec<String> {
        self.read(|rows| {
            let mut out: Vec<&RepoPathRow> = rows.repo_paths.values().collect();
            out.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(a.id.cmp(&b.id)));
            out.into_iter().map(|row| row.path.clone()).collect()
        })
    }

    /// Every `(repo_path, branch)` pair, ordered `last_used DESC, id DESC`.
    pub fn base_branches(&self) -> Vec<(String, String)> {
        self.read(|rows| {
            let mut out: Vec<&RepoBaseBranchRow> = rows.repo_base_branches.values().collect();
            out.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(b.id.cmp(&a.id)));
            out.into_iter()
                .map(|row| (row.repo_path.clone(), row.branch.clone()))
                .collect()
        })
    }

    /// The watchers of `target`, ordered by watch id.
    pub fn watchers_of(&self, target: TaskId) -> Vec<TaskId> {
        self.read(|rows| {
            rows.task_watchers
                .values()
                .filter(|w| w.target_task_id == target.0)
                .map(|w| TaskId(w.watcher_task_id))
                .collect()
        })
    }

    /// `subscriber`'s followed epic ids, ascending — `ORDER BY epic_id`.
    pub fn subscribed_epics(&self, subscriber: &str) -> Vec<i64> {
        self.read(|rows| {
            let mut ids: Vec<i64> = rows
                .subscriptions
                .values()
                .filter(|s| s.subscriber == subscriber)
                .map(|s| s.epic_id)
                .collect();
            ids.sort_unstable();
            ids
        })
    }

    /// The verify command stored on `path`'s `repo_paths` row, if any.
    pub fn verify_command(&self, path: &str) -> Option<String> {
        self.read(|rows| {
            rows.repo_paths
                .values()
                .find(|row| row.path == path)
                .and_then(|row| row.verify_command.clone())
        })
    }

    /// This host's setting `key`.
    pub fn setting(&self, key: &str) -> Option<String> {
        self.read(|rows| {
            rows.settings
                .values()
                .find(|s| s.key == key)
                .map(|s| s.value.clone())
        })
    }

    /// One learning by id.
    pub fn learning(&self, id: LearningId) -> Option<Learning> {
        self.read(|rows| rows.learnings.get(&id.0).map(|(l, _)| l.clone()))
    }

    /// Learnings matching `filter`, ordered `created_at DESC` — the same
    /// order `LearningStore::list_learnings`'s SQL uses.
    pub fn learnings_matching(&self, filter: &LearningFilter) -> Vec<Learning> {
        self.read(|rows| {
            let tag_set: std::collections::HashSet<&str> =
                filter.tags.iter().map(String::as_str).collect();
            let mut out: Vec<&Learning> = rows
                .learnings
                .values()
                .map(|(l, _)| l)
                .filter(|l| filter.status.is_none_or(|s| l.status == s))
                .filter(|l| filter.scope.is_none_or(|s| l.scope == s))
                .filter(|l| {
                    filter
                        .scope_ref
                        .as_deref()
                        .is_none_or(|r| l.scope_ref.as_deref() == Some(r))
                })
                .filter(|l| {
                    tag_set.is_empty() || l.tags.iter().any(|t| tag_set.contains(t.as_str()))
                })
                .collect();
            out.sort_by_key(|l| std::cmp::Reverse(l.created_at));
            if let Some(limit) = filter.limit {
                out.truncate(limit);
            }
            out.into_iter().cloned().collect()
        })
    }

    /// Every approved, non-task-scoped learning with a stored embedding, with
    /// its raw bytes — the RAG candidate pool. Ordered `id ASC`, matching
    /// `list_all_approved_non_task_learnings`'s SQL.
    pub fn approved_non_task_learnings_with_embedding(&self) -> Vec<(Learning, Vec<u8>)> {
        self.read(|rows| {
            rows.learnings
                .values()
                .filter(|(l, emb)| is_approved_non_task(l) && emb.is_some())
                .map(|(l, emb)| (l.clone(), emb.clone().unwrap_or_default()))
                .collect()
        })
    }

    /// Approved, non-task-scoped learnings with no embedding stored yet —
    /// the backfill job's worklist. Ordered `id ASC`.
    pub fn learnings_missing_embedding(&self) -> Vec<Learning> {
        self.read(|rows| {
            rows.learnings
                .values()
                .filter(|(l, emb)| emb.is_none() && is_approved_non_task(l))
                .map(|(l, _)| l.clone())
                .collect()
        })
    }

    /// Retrievals recorded for `task_id`, ordered `id ASC`.
    pub fn retrievals_for_task(&self, task_id: TaskId) -> Vec<LearningRetrieval> {
        self.read(|rows| {
            sorted_by_key(
                rows.learning_retrievals
                    .values()
                    .filter(|r| r.task_id == task_id),
                |r| r.id,
            )
        })
    }

    /// Aggregated usage rows matching `query`, ordered `count ASC` — the same
    /// order `UsageStore::query_usage`'s SQL uses, and for the same reason:
    /// the rarest features surface first as pruning candidates.
    ///
    /// The `GROUP BY category, action, detail, actor` / `COUNT(*)` /
    /// `MAX(recorded_at)` the SQL path expresses declaratively has to be done
    /// by hand here — a subscription is rows, not a query engine — but it is
    /// the same aggregation over the same rows, so the two backends answer
    /// the same summary for the same events.
    pub fn usage_summary(&self, query: &UsageQuery) -> Vec<UsageSummary> {
        /// `(category, action, detail, actor) -> (count, last_used)`.
        type UsageGroups = std::collections::HashMap<
            (String, String, Option<String>, String),
            (i64, DateTime<Utc>),
        >;

        self.read(|rows| {
            let mut groups: UsageGroups = std::collections::HashMap::new();

            for event in rows.usage_events.values() {
                if let Some(cat) = &query.category {
                    if &event.category != cat {
                        continue;
                    }
                }
                if let Some(actor) = &query.actor {
                    if &event.actor != actor {
                        continue;
                    }
                }
                if let Some(since) = query.since {
                    if event.recorded_at < since {
                        continue;
                    }
                }
                let key = (
                    event.category.clone(),
                    event.action.clone(),
                    event.detail.clone(),
                    event.actor.clone(),
                );
                let entry = groups.entry(key).or_insert((0, event.recorded_at));
                entry.0 += 1;
                entry.1 = entry.1.max(event.recorded_at);
            }

            let mut out: Vec<UsageSummary> = groups
                .into_iter()
                .map(
                    |((category, action, detail, actor), (count, last_used))| UsageSummary {
                        category,
                        action,
                        detail,
                        actor,
                        count,
                        last_used,
                    },
                )
                .collect();
            out.sort_by_key(|s| s.count);
            let limit = query.limit.unwrap_or(50).clamp(1, 500);
            out.truncate(limit);
            out
        })
    }

    /// Of `external_ids`, the subset retired under `feed_epic_id` AND absent
    /// from every task anywhere in `feed_epic_id`'s subtree — the read twin of
    /// `db::TaskCrud::retired_without_task`'s SQLite recursive-CTE query, done
    /// in Rust over the rows a standing subscription already holds, the same
    /// reasoning [`Self::usage_summary`] documents.
    pub fn retired_without_task(
        &self,
        feed_epic_id: EpicId,
        external_ids: &[String],
    ) -> Vec<String> {
        // Built outside the read lock (via the ordinary `epics()` accessor,
        // which takes its own short-lived lock) so the subtree walk reuses
        // `crate::models::descendant_epic_ids` instead of a hand-rolled DFS.
        let epics = self.epics();
        let subtree = crate::models::descendant_epic_ids(feed_epic_id, &epics);
        self.read(|rows| {
            let wanted: std::collections::HashSet<&str> =
                external_ids.iter().map(String::as_str).collect();
            // Survivor external_ids for the subtree, computed once so the
            // filter below is an O(1) membership check per candidate rather
            // than a full scan of every task on the board per candidate.
            let survivors: std::collections::HashSet<&str> = rows
                .tasks
                .values()
                .filter(|t| t.epic_id.is_some_and(|e| subtree.contains(&e)))
                .filter_map(|t| t.external_id.as_deref())
                .collect();
            rows.retired_feed_items
                .values()
                .filter(|r| {
                    r.feed_epic_id == feed_epic_id && wanted.contains(r.external_id.as_str())
                })
                .filter(|r| !survivors.contains(r.external_id.as_str()))
                .map(|r| r.external_id.clone())
                .collect()
        })
    }
}

/// Shared by [`SharedRows::approved_non_task_learnings_with_embedding`] and
/// [`SharedRows::learnings_missing_embedding`] — the two differ only in
/// whether they want a stored embedding or its absence.
fn is_approved_non_task(l: &Learning) -> bool {
    use crate::models::{LearningScope, LearningStatus};
    l.status == LearningStatus::Approved && l.scope != LearningScope::Task
}

/// Sort by `(key, id)`, where the `BTreeMap`'s own iteration order already
/// supplies the id tiebreak — so this is a stable sort on the key alone.
///
/// The sort runs over REFERENCES and clones once at the end. A `Task` is some
/// two dozen `String`/`Vec<String>` fields, so sorting the values themselves
/// memmoves hundreds of bytes per swap, `O(n log n)` times, having already deep
/// cloned every row. `repo_paths` and `base_branches` below already do it this
/// way; this is the generic helper catching up.
fn sorted_by_key<'a, T: Clone + 'a>(
    values: impl Iterator<Item = &'a T>,
    key: impl Fn(&T) -> i64,
) -> Vec<T> {
    let mut out: Vec<&T> = values.collect();
    out.sort_by_key(|value| key(value));
    out.into_iter().cloned().collect()
}
