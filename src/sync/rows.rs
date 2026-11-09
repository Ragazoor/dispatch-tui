//! What the subscription has delivered, as the board's own types.
//!
//! Spec: `docs/specs/sync.allium`'s `AConnectedBoardDrawsOnlyDeliveredRows` and
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
//! Every accessor returns rows in a stated order (each accessor says which),
//! because "renders identically" is a claim about a `Vec`, not about a set. The orderings are stated at each accessor and are the reason
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

use crate::models::{
    Agent, AgentId, DispatchRequest, DispatchRequestId, Epic, EpicId, FeedRequest, FeedRequestId,
    Learning, LearningFilter, LearningId, LearningRetrieval, PollScopeId, SessionRequest,
    SessionRequestId, Task, TaskId, WorkPackage, WorkPackageId, WorkPackageLink,
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
    /// The host that last saved the path; `""` for a row saved before hosts
    /// were recorded.
    pub host: String,
    /// The repository's GitHub path (`org/repo`); `""` when unknown.
    pub github_path: String,
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
    /// The raw row behind each entry of `tasks`. `retire_task` compares against
    /// it, not against the decoded task: two rows that differ only in a column
    /// the decoder ignores decode equal, and the delete half of such an update
    /// would erase the row (`sync.allium: AViewUpdateNeverErasesTheRow`).
    raw_tasks: BTreeMap<i64, bindings::Task>,
    raw_epics: BTreeMap<i64, bindings::Epic>,
    /// Tasks the store delivered that did not decode, as `id -> raw epic_id`
    /// (`0` for none). They are absent from `tasks` but still exist in the
    /// store, so a subtree check has to be able to count them
    /// (epics.allium: `DeleteEpicRefused`). A later good copy or a removal
    /// forgets the entry.
    ///
    /// The value is the raw row, so `retire_task` can tell whether a departing
    /// row is the one the mark belongs to (an undecodable-to-undecodable
    /// update is insert(new) then delete(old)).
    undecodable_tasks: BTreeMap<i64, bindings::Task>,
    epics: BTreeMap<i64, Epic>,
    repo_paths: BTreeMap<i64, RepoPathRow>,
    repo_base_branches: BTreeMap<i64, RepoBaseBranchRow>,
    hosts: BTreeMap<String, HostRow>,
    poll_owners: BTreeMap<i64, PollOwnerRow>,
    /// The raw row behind each `poll_owners` entry; `retire_poll_owner`
    /// compares against it (the decoder drops `claimed_at`).
    raw_poll_owners: BTreeMap<i64, bindings::PollOwner>,
    /// `(scope, scope_id) -> id`, kept in step with `poll_owners` on every
    /// insert/remove. `poll_owner()` is read on every `PollPrStatus`/
    /// `FeedTick` tick and must not degrade to a scan as the table grows.
    poll_owners_by_scope: BTreeMap<(String, i64), i64>,
    /// The knowledge base (Phase 10, task #4914). Arrives through the
    /// `visible_learnings` and `visible_learning_retrievals` views, so the
    /// store has already narrowed it to what this person may see; the only
    /// filtering left is on the way out (`SharedRows::learnings_matching`).
    ///
    /// The embedding travels alongside its `Learning` in one map rather than
    /// in a second one keyed the same way: two maps that must always agree on
    /// which ids exist is the kind of invariant a single map holds for free.
    learnings: BTreeMap<i64, (Learning, Option<Vec<u8>>)>,
    learning_retrievals: BTreeMap<i64, LearningRetrieval>,
    /// Standing verdicts on the tasks this person holds, from the
    /// `visible_learning_verdicts` view. Read only to derive the per-repo
    /// dispatch mute (`core.allium: Learning.off_topic_in_repo`), so they are
    /// held as the store sends them and the verdict string is read there.
    learning_verdicts: BTreeMap<i64, bindings::LearningVerdict>,
    /// The tables the store's task, subscription and settings reads need
    /// beyond the ones above (task #4916). None carries a sentinel, an enum or a timestamp a reader
    /// interprets, so they are held as the store sends them.
    ///
    /// `settings` arrives already scoped to this host by the subscription's
    /// `WHERE host = …`, so nothing here filters by host.
    subscriptions: BTreeMap<String, bindings::Subscription>,
    /// `core.allium: UnseenTask`. Arrive scoped to this person by the
    /// subscription's `WHERE subscriber = …`, so nothing here filters by
    /// subscriber.
    unseen_tasks: BTreeMap<String, bindings::UnseenTask>,
    /// `core.allium: Bookmark`. Arrive scoped to this person by the
    /// `own_bookmarks` view, so nothing here filters by subscriber.
    bookmarks: BTreeMap<String, bindings::Bookmark>,
    /// `workspaces.allium: UserProfile`. Unfiltered (`SubscribeToUserProfiles`):
    /// the directory of identities and names is readable by every client.
    user_profiles: BTreeMap<String, bindings::UserProfile>,
    /// `core.allium: Workspace` and `Membership`. Served through the
    /// `visible_workspaces` and `visible_memberships` views.
    workspaces: BTreeMap<i64, bindings::Workspace>,
    memberships: BTreeMap<i64, bindings::Membership>,
    /// `admin-surfaces.allium: UnseenItem`, from the `unseen_items` view: empty
    /// unless this person is a super-admin. A view row has no primary key, so
    /// it is keyed by (kind, id), which names one item.
    unseen_items: BTreeMap<(String, i64), bindings::UnseenItem>,
    settings: BTreeMap<String, bindings::Setting>,
    /// `work-packages.allium`. Packages arrive scoped to their creator by the
    /// subscription's `WHERE created_by = …`; links arrive unfiltered.
    work_packages: BTreeMap<i64, WorkPackage>,
    /// Keyed by the store's derived link id.
    work_package_links: BTreeMap<String, WorkPackageLink>,
    /// The raw rows behind `work_package_links`: `retire_work_package_link`
    /// compares these, because the decoded link drops `created_at` and `id`.
    raw_work_package_links: BTreeMap<String, bindings::WorkPackageLink>,
    /// `store-split.allium: DispatchRequest`. Arrive scoped to THIS host by the subscription's
    /// `WHERE host = …` (`sync.allium: SubscribeToThisHostsDispatchRequests`),
    /// in every state, so nothing here filters by host or state.
    dispatch_requests: BTreeMap<i64, DispatchRequest>,
    /// `store-split.allium: SessionRequest`. Arrive scoped to THIS host by the subscription's
    /// `WHERE host = …` (`sync.allium: SubscribeToThisHostsSessionRequests`),
    /// in every state, so nothing here filters by host or state.
    session_requests: BTreeMap<i64, SessionRequest>,
    /// Arrive scoped to THIS host by the subscription's `WHERE host = …`
    /// (`sync.allium: SubscribeToThisHostsFeedRequests`), in every state.
    feed_requests: BTreeMap<i64, FeedRequest>,
    /// agents A1. Arrive unfiltered (`sync.allium: SubscribeToAgents`),
    /// current and ended alike.
    agents: BTreeMap<i64, Agent>,
    /// The raw row behind each `agents` entry; `retire_agent` compares
    /// against it (the decoder drops `stop_pending_at`).
    raw_agents: BTreeMap<i64, bindings::Agent>,
    /// `task id -> id of its current agent`, kept in step with `agents` on
    /// every insert and remove. Every task read lays the current agent's
    /// session facts over the task (`models::with_session_of`), so the lookup
    /// must not scan.
    current_agent_by_task: BTreeMap<i64, i64>,
}

impl Rows {
    /// Drop `agent` from the current-agent index, if it is what it holds.
    fn index_agent_out(&mut self, agent: i64) {
        if let Some(old) = self.agents.get(&agent) {
            if self.current_agent_by_task.get(&old.task_id.0) == Some(&agent) {
                self.current_agent_by_task.remove(&old.task_id.0);
            }
        }
    }

    /// `task` as the board shows it: with its current agent's session facts.
    fn view(&self, task: &Task) -> Task {
        let agent = self
            .current_agent_by_task
            .get(&task.id.0)
            .and_then(|id| self.agents.get(id));
        crate::models::with_session_of(task, agent)
    }

    /// The window of `task`'s current agent, without cloning anything.
    fn current_window_of(&self, task: TaskId) -> bool {
        self.current_agent_by_task
            .get(&task.0)
            .and_then(|id| self.agents.get(id))
            .is_some_and(|a| a.tmux_window.is_some())
    }

    fn is_empty(&self) -> bool {
        self.tasks.is_empty()
            && self.epics.is_empty()
            && self.repo_paths.is_empty()
            && self.repo_base_branches.is_empty()
            && self.hosts.is_empty()
            && self.poll_owners.is_empty()
            && self.learnings.is_empty()
            && self.learning_retrievals.is_empty()
            && self.learning_verdicts.is_empty()
            && self.subscriptions.is_empty()
            && self.unseen_tasks.is_empty()
            && self.bookmarks.is_empty()
            && self.user_profiles.is_empty()
            && self.workspaces.is_empty()
            && self.memberships.is_empty()
            && self.unseen_items.is_empty()
            && self.settings.is_empty()
            && self.work_packages.is_empty()
            && self.work_package_links.is_empty()
            && self.raw_work_package_links.is_empty()
            && self.dispatch_requests.is_empty()
            && self.session_requests.is_empty()
            && self.feed_requests.is_empty()
            && self.agents.is_empty()
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
        let mut rows = self.rows.write().unwrap_or_else(|e| e.into_inner());
        let moved = f(&mut rows);
        drop(rows);
        if moved {
            self.changed.send_modify(|generation| *generation += 1);
        }
    }

    fn read<T>(&self, f: impl FnOnce(&Rows) -> T) -> T {
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
                rows.undecodable_tasks.remove(&task.id.0);
                rows.raw_tasks.insert(task.id.0, row.clone());
                rows.tasks.insert(task.id.0, task);
                true
            }),
            Err(e) => {
                super::drop_undecodable("task", &e);
                // Dropped from the board, but remembered by id and epic so a
                // delete pre-check can still count it.
                self.write(|rows| {
                    rows.tasks.remove(&row.id);
                    rows.raw_tasks.remove(&row.id);
                    rows.undecodable_tasks.insert(row.id, row.clone());
                    false
                });
            }
        }
    }

    pub fn remove_task(&self, id: TaskId) {
        self.write(|rows| {
            let undecodable = rows.undecodable_tasks.remove(&id.0).is_some();
            rows.raw_tasks.remove(&id.0);
            rows.tasks.remove(&id.0).is_some() || undecodable
        });
    }

    /// A task row left a per-caller view: remove it, unless the row held now is
    /// no longer the one that left.
    ///
    /// A view's rows have no primary key in the bindings, so an update arrives
    /// as INSERT(new) then DELETE(old) with the same id, and the SDK's callback
    /// order for the pair is not ours to rely on. [`Self::remove_task`] on the
    /// delete would erase the row the insert just stored. This removes only
    /// when the stored RAW row equals the one that left. The raw row, not the
    /// decoded task: rows that differ only in a column the decoder ignores
    /// decode equal, and would erase the update (task 98793).
    /// `AViewUpdateNeverErasesTheRow` in `sync.allium`.
    pub fn retire_task(&self, old: &bindings::Task) {
        if decode::task(old).is_err() {
            // Never stored decoded; only its undecodable mark can remain.
            let id = old.id;
            self.write(|rows| remove_if_held(&mut rows.undecodable_tasks, &id, old));
            return;
        }
        self.write(|rows| {
            let removed = remove_if_held(&mut rows.raw_tasks, &old.id, old);
            if removed {
                rows.tasks.remove(&old.id);
            }
            removed
        });
    }

    /// An epic row left a per-caller view. As [`Self::retire_task`].
    pub fn retire_epic(&self, old: &bindings::Epic) {
        if decode::epic(old).is_err() {
            return;
        }
        self.write(|rows| {
            let removed = remove_if_held(&mut rows.raw_epics, &old.id, old);
            if removed {
                rows.epics.remove(&old.id);
            }
            removed
        });
    }

    pub fn upsert_epic(&self, row: &bindings::Epic) {
        if crate::models::is_removed_status(&row.status) {
            tracing::debug!(id = row.id, "skipping an epic with a removed status");
            self.remove_epic(EpicId(row.id));
            return;
        }
        match decode::epic(row) {
            Ok(epic) => self.write(|rows| {
                rows.raw_epics.insert(epic.id.0, row.clone());
                rows.epics.insert(epic.id.0, epic);
                true
            }),
            Err(e) => {
                super::drop_undecodable("epic", &e);
                // As `upsert_task`: a row that stops decoding leaves the
                // board rather than lingering in its old form.
                self.write(|rows| {
                    rows.raw_epics.remove(&row.id);
                    rows.epics.remove(&row.id).is_some()
                });
            }
        }
    }

    pub fn remove_epic(&self, id: EpicId) {
        self.write(|rows| {
            rows.raw_epics.remove(&id.0);
            rows.epics.remove(&id.0).is_some()
        });
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
            rows.raw_poll_owners.insert(value.id, row.clone());
            rows.poll_owners.insert(value.id, value);
            true
        });
    }

    pub fn remove_poll_owner(&self, id: i64) {
        self.write(|rows| match rows.poll_owners.remove(&id) {
            Some(row) => {
                rows.raw_poll_owners.remove(&id);
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
                super::drop_undecodable("learning", &e);
                // As `upsert_task`: a row that stops decoding leaves the
                // board rather than lingering in its old form.
                self.write(|rows| rows.learnings.remove(&row.id).is_some());
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
                super::drop_undecodable("learning retrieval", &e);
                self.write(|rows| rows.learning_retrievals.remove(&row.id).is_some());
            }
        }
    }

    pub fn remove_learning_retrieval(&self, id: i64) {
        self.write(|rows| rows.learning_retrievals.remove(&id).is_some());
    }

    pub fn upsert_learning_verdict(&self, row: &bindings::LearningVerdict) {
        self.write(|rows| {
            rows.learning_verdicts.insert(row.id, row.clone());
            true
        });
    }

    pub fn remove_learning_verdict(&self, id: i64) {
        self.write(|rows| rows.learning_verdicts.remove(&id).is_some());
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

    pub fn retire_subscription(&self, old: &bindings::Subscription) {
        self.write(|rows| remove_if_held(&mut rows.subscriptions, &old.id, old));
    }

    pub fn upsert_unseen_task(&self, row: &bindings::UnseenTask) {
        self.write(|rows| {
            rows.unseen_tasks.insert(row.id.clone(), row.clone());
            true
        });
    }

    pub fn remove_unseen_task(&self, id: String) {
        self.write(|rows| rows.unseen_tasks.remove(&id).is_some());
    }

    pub fn upsert_bookmark(&self, row: &bindings::Bookmark) {
        self.write(|rows| {
            rows.bookmarks.insert(row.id.clone(), row.clone());
            true
        });
    }

    pub fn remove_bookmark(&self, id: String) {
        self.write(|rows| rows.bookmarks.remove(&id).is_some());
    }

    /// The tasks this person has bookmarked, ascending (`core.allium:
    /// Bookmark`). The rows are already this person's own.
    pub fn bookmarked_task_ids(&self) -> Vec<TaskId> {
        self.read(|rows| {
            let mut ids: Vec<TaskId> = rows.bookmarks.values().map(|b| TaskId(b.task_id)).collect();
            ids.sort_unstable();
            ids
        })
    }

    /// The tasks this person has not yet seen, ascending
    /// (`core.allium: UnseenTask`). The rows are already this person's own.
    pub fn unseen_task_ids(&self) -> Vec<TaskId> {
        self.read(|rows| {
            let mut ids: Vec<TaskId> = rows
                .unseen_tasks
                .values()
                .map(|u| TaskId(u.task_id))
                .collect();
            ids.sort_unstable();
            ids
        })
    }

    pub fn upsert_user_profile(&self, row: &bindings::UserProfile) {
        self.write(|rows| {
            rows.user_profiles.insert(row.identity.clone(), row.clone());
            true
        });
    }

    pub fn remove_user_profile(&self, identity: String) {
        self.write(|rows| rows.user_profiles.remove(&identity).is_some());
    }

    pub fn upsert_workspace(&self, row: &bindings::Workspace) {
        self.write(|rows| {
            rows.workspaces.insert(row.id, row.clone());
            true
        });
    }

    pub fn remove_workspace(&self, id: i64) {
        self.write(|rows| rows.workspaces.remove(&id).is_some());
    }

    pub fn upsert_membership(&self, row: &bindings::Membership) {
        self.write(|rows| {
            rows.memberships.insert(row.id, row.clone());
            true
        });
    }

    pub fn remove_membership(&self, id: i64) {
        self.write(|rows| rows.memberships.remove(&id).is_some());
    }

    /// A row entered the `unseen_items` view. An update is INSERT(new) then
    /// DELETE(old) on the same key, so this replaces what is held.
    pub fn upsert_unseen_item(&self, row: &bindings::UnseenItem) {
        self.write(|rows| {
            rows.unseen_items
                .insert((row.kind.clone(), row.item_id), row.clone());
            true
        });
    }

    /// A row left the `unseen_items` view: removed only while the held row is
    /// the one that left, so the DELETE of an update never erases the INSERT
    /// that replaced it (`sync.allium: AViewUpdateNeverErasesTheRow`).
    pub fn retire_unseen_item(&self, old: &bindings::UnseenItem) {
        self.write(|rows| {
            remove_if_held(
                &mut rows.unseen_items,
                &(old.kind.clone(), old.item_id),
                old,
            )
        });
    }

    /// Replace the whole listing. A test double that has no view to push rows
    /// recomputes the listing after a write and hands it over here.
    pub fn replace_unseen_items(&self, items: Vec<bindings::UnseenItem>) {
        self.write(|rows| {
            let next: BTreeMap<(String, i64), bindings::UnseenItem> = items
                .into_iter()
                .map(|i| ((i.kind.clone(), i.item_id), i))
                .collect();
            if rows.unseen_items == next {
                return false;
            }
            rows.unseen_items = next;
            true
        });
    }

    /// Every unseen item, ordered by (kind, id). Empty for anyone but a
    /// super-admin.
    pub fn unseen_items(&self) -> Vec<bindings::UnseenItem> {
        self.read(|rows| rows.unseen_items.values().cloned().collect())
    }

    /// Every workspace, by id.
    pub fn workspaces(&self) -> Vec<bindings::Workspace> {
        self.read(|rows| rows.workspaces.values().cloned().collect())
    }

    /// Every membership, by id.
    pub fn memberships(&self) -> Vec<bindings::Membership> {
        self.read(|rows| rows.memberships.values().cloned().collect())
    }

    /// Every profile, by identity: the directory of people.
    pub fn user_profiles(&self) -> Vec<bindings::UserProfile> {
        self.read(|rows| rows.user_profiles.values().cloned().collect())
    }

    /// Whether `identity` holds the super-admin flag.
    pub fn is_super_admin(&self, identity: &str) -> bool {
        self.read(|rows| {
            rows.user_profiles
                .get(identity)
                .is_some_and(|p| p.is_super_admin)
        })
    }

    /// The display name `identity` has set, or `""` when none is set or no
    /// profile is held.
    pub fn display_name_of(&self, identity: &str) -> String {
        self.read(|rows| {
            rows.user_profiles
                .get(identity)
                .map(|p| p.display_name.clone())
                .unwrap_or_default()
        })
    }

    /// `identity` as a card or tool result names them
    /// (`workspaces.allium: ShownName`).
    pub fn shown_name_of(&self, identity: &str) -> String {
        self.read(|rows| {
            let name = rows
                .user_profiles
                .get(identity)
                .map_or("", |p| p.display_name.as_str());
            crate::models::shown_name(name, identity)
        })
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

    pub fn upsert_work_package(&self, row: &bindings::WorkPackage) {
        match decode::work_package(row) {
            Ok(package) => self.write(|rows| {
                rows.work_packages.insert(package.id.0, package);
                true
            }),
            Err(e) => {
                super::drop_undecodable("work package", &e);
                self.write(|rows| rows.work_packages.remove(&row.id).is_some());
            }
        }
    }

    pub fn remove_work_package(&self, id: WorkPackageId) {
        self.write(|rows| rows.work_packages.remove(&id.0).is_some());
    }

    pub fn upsert_work_package_link(&self, row: &bindings::WorkPackageLink) {
        match decode::work_package_link(row) {
            Ok(link) => self.write(|rows| {
                rows.work_package_links.insert(row.id.clone(), link);
                rows.raw_work_package_links
                    .insert(row.id.clone(), row.clone());
                true
            }),
            Err(e) => {
                super::drop_undecodable("work package link", &e);
                self.write(|rows| {
                    rows.raw_work_package_links.remove(&row.id);
                    rows.work_package_links.remove(&row.id).is_some()
                });
            }
        }
    }

    pub fn remove_work_package_link(&self, id: String) {
        self.write(|rows| {
            rows.raw_work_package_links.remove(&id);
            rows.work_package_links.remove(&id).is_some()
        });
    }

    /// A dispatch request row (`store-split.allium: DispatchRequest`). An undecodable one is dropped,
    /// as a task is: read as open or as finished it would drive the dispatch
    /// feedback wrongly either way.
    pub fn upsert_dispatch_request(&self, row: &bindings::DispatchRequest) {
        match decode::dispatch_request(row) {
            Ok(request) => self.write(|rows| {
                rows.dispatch_requests.insert(request.id.0, request);
                true
            }),
            Err(e) => {
                super::drop_undecodable("dispatch request", &e);
                self.write(|rows| rows.dispatch_requests.remove(&row.id).is_some());
            }
        }
    }

    pub fn remove_dispatch_request(&self, id: i64) {
        self.write(|rows| rows.dispatch_requests.remove(&id).is_some());
    }

    /// An agent row (agents A1). An undecodable one is dropped, as a task is.
    pub fn upsert_agent(&self, row: &bindings::Agent) {
        match decode::agent(row) {
            Ok(agent) => self.write(|rows| {
                rows.index_agent_out(agent.id.0);
                if agent.is_current() {
                    rows.current_agent_by_task
                        .insert(agent.task_id.0, agent.id.0);
                }
                rows.raw_agents.insert(agent.id.0, row.clone());
                rows.agents.insert(agent.id.0, agent);
                true
            }),
            Err(e) => {
                super::drop_undecodable("agent", &e);
                self.write(|rows| {
                    rows.index_agent_out(row.id);
                    rows.raw_agents.remove(&row.id);
                    rows.agents.remove(&row.id).is_some()
                });
            }
        }
    }

    pub fn remove_agent(&self, id: AgentId) {
        self.write(|rows| {
            rows.index_agent_out(id.0);
            rows.raw_agents.remove(&id.0);
            rows.agents.remove(&id.0).is_some()
        });
    }

    /// A session request row (`store-split.allium: SessionRequest`). An undecodable one is dropped,
    /// as a dispatch request is: read as open or as finished it would drive the
    /// `wrap_up` wait and the exit token wrongly either way.
    pub fn upsert_session_request(&self, row: &bindings::SessionRequest) {
        match decode::session_request(row) {
            Ok(request) => self.write(|rows| {
                rows.session_requests.insert(request.id.0, request);
                true
            }),
            Err(e) => {
                super::drop_undecodable("session request", &e);
                self.write(|rows| rows.session_requests.remove(&row.id).is_some());
            }
        }
    }

    /// A feed request row. An undecodable one is dropped, as a session request
    /// is.
    pub fn upsert_feed_request(&self, row: &bindings::FeedRequest) {
        match decode::feed_request(row) {
            Ok(request) => self.write(|rows| {
                rows.feed_requests.insert(request.id.0, request);
                true
            }),
            Err(e) => {
                super::drop_undecodable("feed request", &e);
                self.write(|rows| rows.feed_requests.remove(&row.id).is_some());
            }
        }
    }

    pub fn remove_feed_request(&self, id: i64) {
        self.write(|rows| rows.feed_requests.remove(&id).is_some());
    }

    pub fn retire_feed_request(&self, old: &bindings::FeedRequest) {
        let Ok(old) = decode::feed_request(old) else {
            return;
        };
        self.write(|rows| remove_if_held(&mut rows.feed_requests, &old.id.0, &old));
    }

    /// Every feed request row this subscription holds — this host's, in every
    /// state — oldest first (by id).
    pub fn feed_requests(&self) -> Vec<FeedRequest> {
        self.read(|rows| rows.feed_requests.values().cloned().collect())
    }

    /// One feed request row by id.
    pub fn feed_request(&self, id: FeedRequestId) -> Option<FeedRequest> {
        self.read(|rows| rows.feed_requests.get(&id.0).cloned())
    }

    pub fn remove_session_request(&self, id: i64) {
        self.write(|rows| rows.session_requests.remove(&id).is_some());
    }

    // -- Rows that left a per-caller view ---------------------------------
    //
    // A view's rows have no primary key in the bindings, so an update arrives
    // as INSERT(new) then DELETE(old) with the same id, in an order that is not
    // ours to rely on. Each `retire_*` below removes the id only while the row
    // held is the one that left (`sync.allium: AViewUpdateNeverErasesTheRow`);
    // a row that cannot be decoded was never stored, so nothing of it is held.

    pub fn retire_work_package(&self, old: &bindings::WorkPackage) {
        let Ok(old) = decode::work_package(old) else {
            return;
        };
        self.write(|rows| remove_if_held(&mut rows.work_packages, &old.id.0, &old));
    }

    /// Compared RAW: the decoded link drops `created_at`, so rows that differ
    /// only there decode equal and a decoded compare erased the update.
    pub fn retire_work_package_link(&self, old: &bindings::WorkPackageLink) {
        if decode::work_package_link(old).is_err() {
            return;
        }
        self.write(|rows| {
            let removed = remove_if_held(&mut rows.raw_work_package_links, &old.id, old);
            if removed {
                rows.work_package_links.remove(&old.id);
            }
            removed
        });
    }

    pub fn retire_poll_owner(&self, old: &bindings::PollOwner) {
        let decoded = decode::poll_owner(old);
        self.write(|rows| {
            let held = remove_if_held(&mut rows.raw_poll_owners, &old.id, old);
            if held {
                rows.poll_owners.remove(&old.id);
                rows.poll_owners_by_scope
                    .remove(&(decoded.scope.clone(), decoded.scope_id));
            }
            held
        });
    }

    pub fn retire_learning(&self, old: &bindings::Learning) {
        let Ok(decoded) = decode::learning(old) else {
            return;
        };
        self.write(|rows| {
            let held = rows
                .learnings
                .get(&decoded.id.0)
                .is_some_and(|(l, embedding)| *l == decoded && *embedding == old.embedding);
            if held {
                rows.learnings.remove(&decoded.id.0);
            }
            held
        });
    }

    pub fn retire_learning_retrieval(&self, old: &bindings::LearningRetrieval) {
        let Ok(old) = decode::learning_retrieval(old) else {
            return;
        };
        self.write(|rows| remove_if_held(&mut rows.learning_retrievals, &old.id, &old));
    }

    pub fn retire_learning_verdict(&self, old: &bindings::LearningVerdict) {
        self.write(|rows| remove_if_held(&mut rows.learning_verdicts, &old.id, old));
    }

    pub fn retire_unseen_task(&self, old: &bindings::UnseenTask) {
        self.write(|rows| remove_if_held(&mut rows.unseen_tasks, &old.id, old));
    }

    pub fn retire_bookmark(&self, old: &bindings::Bookmark) {
        self.write(|rows| remove_if_held(&mut rows.bookmarks, &old.id, old));
    }

    pub fn retire_workspace(&self, old: &bindings::Workspace) {
        self.write(|rows| remove_if_held(&mut rows.workspaces, &old.id, old));
    }

    pub fn retire_membership(&self, old: &bindings::Membership) {
        self.write(|rows| remove_if_held(&mut rows.memberships, &old.id, old));
    }

    pub fn retire_setting(&self, old: &bindings::Setting) {
        self.write(|rows| remove_if_held(&mut rows.settings, &old.id, old));
    }

    pub fn retire_dispatch_request(&self, old: &bindings::DispatchRequest) {
        let Ok(old) = decode::dispatch_request(old) else {
            return;
        };
        self.write(|rows| remove_if_held(&mut rows.dispatch_requests, &old.id.0, &old));
    }

    pub fn retire_session_request(&self, old: &bindings::SessionRequest) {
        let Ok(old) = decode::session_request(old) else {
            return;
        };
        self.write(|rows| remove_if_held(&mut rows.session_requests, &old.id.0, &old));
    }

    pub fn retire_agent(&self, old: &bindings::Agent) {
        if decode::agent(old).is_err() {
            return;
        }
        self.write(|rows| {
            if rows.raw_agents.get(&old.id) == Some(old) {
                rows.index_agent_out(old.id);
                rows.raw_agents.remove(&old.id);
                rows.agents.remove(&old.id);
                true
            } else {
                false
            }
        });
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
        self.read(|rows| {
            sorted_by_key(rows.tasks.values(), Task::sort_key)
                .iter()
                .map(|t| rows.view(t))
                .collect()
        })
    }

    pub fn task(&self, id: TaskId) -> Option<Task> {
        self.read(|rows| rows.tasks.get(&id.0).map(|t| rows.view(t)))
    }

    /// Tasks under one epic, in the same order.
    pub fn tasks_for_epic(&self, epic: EpicId) -> Vec<Task> {
        self.read(|rows| {
            sorted_by_key(
                rows.tasks.values().filter(|t| t.epic_id == Some(epic)),
                Task::sort_key,
            )
            .iter()
            .map(|t| rows.view(t))
            .collect()
        })
    }

    /// Ids of the epic's tasks the store delivered but that did not decode,
    /// ascending. Not on the board; counted by the delete pre-check.
    pub fn undecodable_task_ids_for_epic(&self, epic: EpicId) -> Vec<TaskId> {
        self.read(|rows| {
            rows.undecodable_tasks
                .iter()
                .filter(|(_, t)| t.epic_id == epic.0)
                .map(|(id, _)| TaskId(*id))
                .collect()
        })
    }

    /// Whether `id` is held, without cloning it.
    pub fn has_task(&self, id: TaskId) -> bool {
        self.read(|rows| rows.tasks.contains_key(&id.0))
    }

    /// The agent-tree pane's agents, ordered by id — the `list_agent_tasks`
    /// query: Running or Review tasks with a tmux window (`Task::is_live_agent`)
    /// and those with a worktree and none (`Task::is_detached`), split in one
    /// pass under one read, so a task is never in both lists or neither.
    /// Filtered before anything is cloned: every shown pane polls this each
    /// second.
    pub fn agent_tasks(&self) -> (Vec<Task>, Vec<Task>) {
        use crate::models::TaskStatus;
        self.read(|rows| {
            let (mut live, mut detached) = (Vec::new(), Vec::new());
            for t in rows.tasks.values() {
                if !matches!(t.status, TaskStatus::Running | TaskStatus::Review) {
                    continue;
                }
                if rows.current_window_of(t.id) {
                    live.push(rows.view(t));
                } else if t.worktree.is_some() {
                    detached.push(rows.view(t));
                }
            }
            (live, detached)
        })
    }

    /// The lowest-id task whose plan is `plan`.
    pub fn task_by_plan(&self, plan: &str) -> Option<Task> {
        self.read(|rows| {
            rows.tasks
                .values()
                .find(|t| t.plan_path.as_deref() == Some(plan))
                .map(|t| rows.view(t))
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
            out.into_iter().map(|t| rows.view(t)).collect()
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

    /// Every work package, oldest first (by id). `created_by` is the creator
    /// AS SHOWN (`workspaces.allium: CreatorIsShownByName`): their display
    /// name, or the identity's prefix when they have set none.
    pub fn work_packages(&self) -> Vec<WorkPackage> {
        self.read(|rows| {
            rows.work_packages
                .values()
                .cloned()
                .map(|mut package| {
                    if let Some(identity) = package.created_by.take() {
                        let name = rows
                            .user_profiles
                            .get(&identity)
                            .map_or("", |p| p.display_name.as_str());
                        package.created_by = Some(crate::models::shown_name(name, &identity));
                    }
                    package
                })
                .collect()
        })
    }

    /// Every work package link, ordered by package id, then the item's kind
    /// and id — the order the derived key would sort in if its ids were
    /// numbers rather than text.
    pub fn work_package_links(&self) -> Vec<WorkPackageLink> {
        self.read(|rows| {
            let mut out: Vec<WorkPackageLink> = rows.work_package_links.values().copied().collect();
            out.sort_by_key(|l| (l.work_package_id, l.item));
            out
        })
    }

    // `hosts` has one reader, `host_owner`: whose machine is this host, for
    // the feed gate (`feeds.allium: FeedCommandRunsOnSettersHost`).
    //
    // `poll_owners` is the exception, added in Phase 7: `PollPrStatus`/
    // `FeedTick` need "who owns this scope?" on every tick, so this reader
    // arrives with its own consumer rather than waiting the way `hosts` is.

    /// The owner of the host `host_id`, or `None` when the host is unknown or
    /// has no owner. `core.allium: Host.owner`.
    pub fn host_owner(&self, host_id: &str) -> Option<String> {
        self.read(|rows| rows.hosts.get(host_id).and_then(|h| h.owner.clone()))
    }

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

    /// One dispatch request row by id.
    pub fn dispatch_request(&self, id: DispatchRequestId) -> Option<DispatchRequest> {
        self.read(|rows| rows.dispatch_requests.get(&id.0).cloned())
    }

    /// Every dispatch request row this subscription holds — this host's, in
    /// every state — oldest first (by id).
    pub fn dispatch_requests(&self) -> Vec<DispatchRequest> {
        self.read(|rows| rows.dispatch_requests.values().cloned().collect())
    }

    /// Every agent row, current and ended, oldest first (by id).
    pub fn agents(&self) -> Vec<Agent> {
        self.read(|rows| rows.agents.values().cloned().collect())
    }

    /// `current_agent(task)`: the task's one current agent, if this
    /// subscription holds it (`agents.allium: OneCurrentAgentPerTask`).
    pub fn current_agent(&self, task: TaskId) -> Option<Agent> {
        self.read(|rows| {
            rows.current_agent_by_task
                .get(&task.0)
                .and_then(|id| rows.agents.get(id))
                .cloned()
        })
    }

    /// One session request row by id.
    pub fn session_request(&self, id: SessionRequestId) -> Option<SessionRequest> {
        self.read(|rows| rows.session_requests.get(&id.0).cloned())
    }

    /// Every session request row this subscription holds — this host's, in
    /// every state — oldest first (by id).
    pub fn session_requests(&self) -> Vec<SessionRequest> {
        self.read(|rows| rows.session_requests.values().cloned().collect())
    }

    /// `task`'s most recent dispatch request (the highest id), if this
    /// subscription holds one.
    pub fn latest_dispatch_request_for_task(&self, task: TaskId) -> Option<DispatchRequest> {
        self.read(|rows| {
            rows.dispatch_requests
                .values()
                .rev()
                .find(|r| r.task_id == task)
                .cloned()
        })
    }

    /// The repo paths, most recently used first: `last_used DESC, id ASC`.
    ///
    /// The ascending tiebreak looks backwards beside `base_branches` below, and
    /// is deliberate: `last_used` has whole-second resolution, so paths saved in
    /// one second tie, and the retired SQLite store broke that tie by rowid
    /// ascending. Matching it is what keeps the picker's order the order it has
    /// always had.
    pub fn repo_paths(&self) -> Vec<String> {
        self.read(|rows| {
            let mut out: Vec<&RepoPathRow> = rows.repo_paths.values().collect();
            out.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(a.id.cmp(&b.id)));
            // One row per (path, host): a path several hosts saved is listed once.
            let mut seen = std::collections::HashSet::new();
            out.into_iter()
                .filter(|row| seen.insert(row.path.as_str()))
                .map(|row| row.path.clone())
                .collect()
        })
    }

    /// The paths `host` saved, in [`Self::repo_paths`]'s order. A row with no
    /// recorded host belongs to no one, so it is in nobody's list; see
    /// [`Self::repo_paths_unowned`].
    pub fn repo_paths_saved_by(&self, host: &str) -> Vec<String> {
        self.read(|rows| {
            let mut out: Vec<&RepoPathRow> = rows
                .repo_paths
                .values()
                .filter(|row| !row.host.is_empty() && row.host == host)
                .collect();
            out.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(a.id.cmp(&b.id)));
            out.into_iter().map(|row| row.path.clone()).collect()
        })
    }

    /// The paths saved before rows carried a host, in [`Self::repo_paths`]'s
    /// order. No host owns them; every host's prune may judge them.
    pub fn repo_paths_unowned(&self) -> Vec<String> {
        self.read(|rows| {
            let mut out: Vec<&RepoPathRow> = rows
                .repo_paths
                .values()
                .filter(|row| row.host.is_empty())
                .collect();
            out.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(a.id.cmp(&b.id)));
            out.into_iter().map(|row| row.path.clone()).collect()
        })
    }

    /// Every `(repo_path, branch)` pair, ordered `last_used DESC, id DESC`.
    pub fn base_branches(&self) -> Vec<(String, String)> {
        self.read(|rows| {
            let mut out: Vec<&RepoBaseBranchRow> = rows.repo_base_branches.values().collect();
            out.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(b.id.cmp(&a.id)));
            // A branch recorded through one clone is offered for every clone
            // of the same repository (shared non-empty github_path).
            let mut pairs: Vec<(String, String)> = Vec::new();
            for row in out {
                let gh = rows
                    .repo_paths
                    .values()
                    .find(|p| p.path == row.repo_path && !p.github_path.is_empty())
                    .map(|p| p.github_path.as_str());
                let mut targets = vec![row.repo_path.as_str()];
                if let Some(gh) = gh {
                    targets.extend(
                        rows.repo_paths
                            .values()
                            .filter(|p| p.github_path == gh)
                            .map(|p| p.path.as_str()),
                    );
                }
                for path in targets {
                    let pair = (path.to_string(), row.branch.clone());
                    if !pairs.contains(&pair) {
                        pairs.push(pair);
                    }
                }
            }
            pairs
        })
    }

    /// `subscriber`'s followed epic ids, ascending — `ORDER BY epic_id`.
    pub fn subscribed_epics(&self, subscriber: &str) -> Vec<EpicId> {
        self.read(|rows| {
            let mut ids: Vec<EpicId> = rows
                .subscriptions
                .values()
                .filter(|s| s.subscriber == subscriber)
                .map(|s| EpicId(s.epic_id))
                .collect();
            ids.sort_unstable();
            ids
        })
    }

    /// The verify command stored on `host`'s `repo_paths` row of `path`, else
    /// on an unowned row of it (saved before hosts were recorded), if any. A
    /// row only another host has is not read: two machines may hold different
    /// repositories at one path. Mirrors the module's `set_verify_command`.
    pub fn verify_command(&self, path: &str, host: &str) -> Option<String> {
        self.read(|rows| {
            rows.repo_paths
                .values()
                .find(|row| row.path == path && row.host == host)
                .or_else(|| {
                    rows.repo_paths
                        .values()
                        .find(|row| row.path == path && row.host.is_empty())
                })
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

    /// The learnings off-topic in `repo_path`: at least `threshold`
    /// `not_applicable` verdicts from tasks in that repo and no `helped` one
    /// (`core.allium: Learning.off_topic_in_repo`). A verdict whose task the
    /// board no longer holds does not count; `wrong` counts neither way.
    pub fn learnings_off_topic_in_repo(
        &self,
        repo_path: &str,
        threshold: usize,
    ) -> std::collections::BTreeSet<LearningId> {
        self.read(|rows| {
            let mut counts: BTreeMap<i64, (usize, usize)> = BTreeMap::new();
            for v in rows.learning_verdicts.values() {
                let in_repo = rows
                    .tasks
                    .get(&v.task_id)
                    .is_some_and(|t| t.repo_path == repo_path);
                if !in_repo {
                    continue;
                }
                let entry = counts.entry(v.learning_id).or_default();
                use crate::models::LearningVerdict as V;
                match V::parse(&v.verdict) {
                    Ok(V::NotApplicable) => entry.0 += 1,
                    Ok(V::Helped) => entry.1 += 1,
                    Ok(V::Wrong) | Err(_) => {}
                }
            }
            counts
                .into_iter()
                .filter(|(_, (off_topic, helped))| *off_topic >= threshold && *helped == 0)
                .map(|(id, _)| LearningId(id))
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

/// Remove `key` from `map` only while the row held there equals `old`: the one
/// rule behind every `retire_*` (`sync.allium: AViewUpdateNeverErasesTheRow`).
/// Whether anything was removed, which is what `SharedRows::write` wakes on.
fn remove_if_held<K: Ord, V: PartialEq>(map: &mut BTreeMap<K, V>, key: &K, old: &V) -> bool {
    let held = map.get(key) == Some(old);
    if held {
        map.remove(key);
    }
    held
}
