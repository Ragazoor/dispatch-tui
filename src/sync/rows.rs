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

use crate::models::{Epic, EpicId, Task, TaskId, Todo, TodoId};
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

#[derive(Default)]
struct Rows {
    tasks: BTreeMap<i64, Task>,
    epics: BTreeMap<i64, Epic>,
    todos: BTreeMap<i64, Todo>,
    repo_paths: BTreeMap<i64, RepoPathRow>,
    repo_base_branches: BTreeMap<i64, RepoBaseBranchRow>,
    hosts: BTreeMap<String, HostRow>,
}

impl Rows {
    fn is_empty(&self) -> bool {
        self.tasks.is_empty()
            && self.epics.is_empty()
            && self.todos.is_empty()
            && self.repo_paths.is_empty()
            && self.repo_base_branches.is_empty()
            && self.hosts.is_empty()
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

    pub fn upsert_todo(&self, row: &bindings::Todo) {
        match decode::todo(row) {
            Ok(todo) => self.write(|rows| {
                rows.todos.insert(todo.id.0, todo);
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
                    "dropping an undecodable todo from the shared store: {e}"
                );
            }
        }
    }

    pub fn remove_todo(&self, id: TodoId) {
        self.write(|rows| rows.todos.remove(&id.0).is_some());
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

    /// Every todo, ordered as `TodoStore::list_todos` orders them:
    /// `sort_order ASC, id ASC`.
    pub fn todos(&self) -> Vec<Todo> {
        self.read(|rows| sorted_by_key(rows.todos.values(), |t| t.sort_order))
    }

    // `hosts` is DELIVERED and stored but has no accessor, and that is the
    // honest state rather than an omission: a card naming another machine needs
    // the registry present when something finally resolves it, and an accessor
    // written now would be an ordering nothing tests and nothing calls. Phase 6
    // adds the reader together with the test that needs it.

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
