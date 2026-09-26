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

use tokio::sync::watch;

use crate::db::LearningFilter;
use crate::models::{Epic, EpicId, Learning, LearningId, LearningRetrieval, Task, TaskId};
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
    pub fn poll_owner(&self, scope: &str, scope_id: i64) -> Option<PollOwnerRow> {
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
