//! The per-caller views (`docs/architecture.md`: "Read enforcement: per-caller views").
//!
//! Each view answers one question the board asks, for `ctx.sender()`, and is a
//! thin shell over a function here that is generic over [`StoreReads`]. The
//! module runs it over a [`ViewContext`], the reducers run the predicates it is
//! built from over a [`ReducerContext`], and the in-memory store runs the same
//! function over its own rows (`src/sync/memory_caller/views.rs`), so the
//! module and its double cannot disagree about who sees what.
//!
//! The views narrow by follows and ownership only (`sync.allium`): that is
//! delivery, what a board receives, not access. One person uses the store.
//!
//! Every read is a point read on a btree index, but for `visible_learnings`: a
//! view handle has no `.iter()`, and a range read counts as a full scan, so it
//! re-runs the view on every write to that table
//! (`docs/plans/2026-10-28-per-caller-views-spike.md`).
//!
//! The operator (the publisher identity) is not served by views: it reads the
//! tables directly.

use super::*;
use spacetimedb::ViewContext;
use std::collections::BTreeMap;

/// Indexed reads of the store, as a view or a reducer makes them. One
/// implementation per context (below) and one for the in-memory store.
pub trait StoreReads {
    fn epic(&self, id: i64) -> Option<Epic>;
    fn task(&self, id: i64) -> Option<Task>;
    fn package(&self, id: i64) -> Option<WorkPackage>;
    fn subscriptions_of(&self, subscriber: &str) -> Vec<Subscription>;
    fn epics_under(&self, parent: i64) -> Vec<Epic>;
    fn epics_created_by(&self, who: &str) -> Vec<Epic>;
    fn tasks_in_epic(&self, epic_id: i64) -> Vec<Task>;
    fn tasks_owned_by(&self, who: &str) -> Vec<Task>;
    fn tasks_created_by(&self, who: &str) -> Vec<Task>;
    fn packages_created_by(&self, who: &str) -> Vec<WorkPackage>;
    fn links_of_package(&self, package_id: i64) -> Vec<WorkPackageLink>;
    fn agents_of(&self, task_id: i64) -> Vec<Agent>;
    fn dispatch_requests_of(&self, task_id: i64) -> Vec<DispatchRequest>;
    fn session_requests_of(&self, task_id: i64) -> Vec<SessionRequest>;
    /// Rows on one machine, whatever task they are about.
    fn agents_on_host(&self, host: &str) -> Vec<Agent>;
    fn dispatch_requests_on_host(&self, host: &str) -> Vec<DispatchRequest>;
    fn session_requests_on_host(&self, host: &str) -> Vec<SessionRequest>;
    fn feed_requests_on_host(&self, host: &str) -> Vec<FeedRequest>;
    fn unseen_of(&self, subscriber: &str) -> Vec<UnseenTask>;
    fn bookmarks_of(&self, subscriber: &str) -> Vec<Bookmark>;
    fn subagents_of(&self, task_id: i64) -> Vec<TaskSubagent>;
    /// Rows naming the task as the watcher or as the target.
    fn watchers_of(&self, task_id: i64) -> Vec<TaskWatcher>;
    fn retrievals_of(&self, task_id: i64) -> Vec<LearningRetrieval>;
    fn verdicts_of(&self, task_id: i64) -> Vec<LearningVerdict>;
    fn poll_owners_of(&self, scope_id: i64) -> Vec<PollOwner>;
    /// Every learning: `visible_learnings` serves them all.
    fn all_learnings(&self) -> Vec<Learning>;
    fn hosts_owned_by(&self, owner: &str) -> Vec<Host>;
    fn settings_of(&self, host: &str) -> Vec<Setting>;
    fn usage_events_recorded_by(&self, who: &str) -> Vec<UsageEvent>;
    fn archived_task(&self, id: i64) -> Option<ArchivedTask>;
    /// Every record: the archive listing reads the whole table.
    fn all_archived_tasks(&self) -> Vec<ArchivedTask>;
}

/// `StoreReads` over a context whose `db` is a table set: the reducers' and the
/// views' handles spell every read the same way.
macro_rules! impl_store_reads {
    ($ctx:ty, $all_archived:expr) => {
        impl StoreReads for $ctx {
            fn epic(&self, id: i64) -> Option<Epic> {
                self.db.epics().id().find(id)
            }
            fn task(&self, id: i64) -> Option<Task> {
                self.db.tasks().id().find(id)
            }
            fn package(&self, id: i64) -> Option<WorkPackage> {
                self.db.work_packages().id().find(id)
            }
            fn subscriptions_of(&self, subscriber: &str) -> Vec<Subscription> {
                self.db
                    .subscriptions()
                    .subscriber()
                    .filter(&subscriber.to_string())
                    .collect()
            }
            fn epics_under(&self, parent: i64) -> Vec<Epic> {
                self.db.epics().parent_epic_id().filter(&parent).collect()
            }
            fn epics_created_by(&self, who: &str) -> Vec<Epic> {
                self.db
                    .epics()
                    .created_by()
                    .filter(&who.to_string())
                    .collect()
            }
            fn tasks_in_epic(&self, epic_id: i64) -> Vec<Task> {
                self.db.tasks().epic_id().filter(&epic_id).collect()
            }
            fn tasks_owned_by(&self, who: &str) -> Vec<Task> {
                self.db.tasks().owner().filter(&who.to_string()).collect()
            }
            fn tasks_created_by(&self, who: &str) -> Vec<Task> {
                self.db
                    .tasks()
                    .created_by()
                    .filter(&who.to_string())
                    .collect()
            }
            fn packages_created_by(&self, who: &str) -> Vec<WorkPackage> {
                self.db
                    .work_packages()
                    .created_by()
                    .filter(&who.to_string())
                    .collect()
            }
            fn links_of_package(&self, package_id: i64) -> Vec<WorkPackageLink> {
                self.db
                    .work_package_links()
                    .work_package_id()
                    .filter(&package_id)
                    .collect()
            }
            fn agents_of(&self, task_id: i64) -> Vec<Agent> {
                self.db.agents().task_id().filter(&task_id).collect()
            }
            fn dispatch_requests_of(&self, task_id: i64) -> Vec<DispatchRequest> {
                self.db
                    .dispatch_requests()
                    .task_id()
                    .filter(&task_id)
                    .collect()
            }
            fn session_requests_of(&self, task_id: i64) -> Vec<SessionRequest> {
                self.db
                    .session_requests()
                    .task_id()
                    .filter(&task_id)
                    .collect()
            }
            fn agents_on_host(&self, host: &str) -> Vec<Agent> {
                self.db.agents().host().filter(&host.to_string()).collect()
            }
            fn dispatch_requests_on_host(&self, host: &str) -> Vec<DispatchRequest> {
                self.db
                    .dispatch_requests()
                    .host()
                    .filter(&host.to_string())
                    .collect()
            }
            fn session_requests_on_host(&self, host: &str) -> Vec<SessionRequest> {
                self.db
                    .session_requests()
                    .host()
                    .filter(&host.to_string())
                    .collect()
            }
            fn feed_requests_on_host(&self, host: &str) -> Vec<FeedRequest> {
                self.db
                    .feed_requests()
                    .host()
                    .filter(&host.to_string())
                    .collect()
            }
            fn unseen_of(&self, subscriber: &str) -> Vec<UnseenTask> {
                self.db
                    .unseen_tasks()
                    .subscriber()
                    .filter(&subscriber.to_string())
                    .collect()
            }
            fn bookmarks_of(&self, subscriber: &str) -> Vec<Bookmark> {
                self.db
                    .bookmarks()
                    .subscriber()
                    .filter(&subscriber.to_string())
                    .collect()
            }
            fn subagents_of(&self, task_id: i64) -> Vec<TaskSubagent> {
                self.db
                    .task_subagents()
                    .task_id()
                    .filter(&task_id)
                    .collect()
            }
            fn watchers_of(&self, task_id: i64) -> Vec<TaskWatcher> {
                let mut rows: Vec<TaskWatcher> = self
                    .db
                    .task_watchers()
                    .watcher_task_id()
                    .filter(&task_id)
                    .collect();
                rows.extend(
                    self.db
                        .task_watchers()
                        .target_task_id()
                        .filter(&task_id)
                        .filter(|w| w.watcher_task_id != task_id),
                );
                rows
            }
            fn retrievals_of(&self, task_id: i64) -> Vec<LearningRetrieval> {
                self.db
                    .learning_retrievals()
                    .task_id()
                    .filter(&task_id)
                    .collect()
            }
            fn verdicts_of(&self, task_id: i64) -> Vec<LearningVerdict> {
                self.db
                    .learning_verdicts()
                    .task_id()
                    .filter(&task_id)
                    .collect()
            }
            fn poll_owners_of(&self, scope_id: i64) -> Vec<PollOwner> {
                self.db.poll_owners().scope_id().filter(&scope_id).collect()
            }
            fn all_learnings(&self) -> Vec<Learning> {
                // A view handle has no `.iter()`; an unbounded range over a
                // live index reads every row.
                self.db
                    .learnings()
                    .author()
                    .filter((
                        std::ops::Bound::<&str>::Unbounded,
                        std::ops::Bound::<&str>::Unbounded,
                    ))
                    .collect()
            }
            fn hosts_owned_by(&self, owner: &str) -> Vec<Host> {
                self.db.hosts().owner().filter(&owner.to_string()).collect()
            }
            fn settings_of(&self, host: &str) -> Vec<Setting> {
                self.db
                    .settings()
                    .host()
                    .filter(&host.to_string())
                    .collect()
            }
            fn usage_events_recorded_by(&self, who: &str) -> Vec<UsageEvent> {
                self.db
                    .usage_events()
                    .recorded_by()
                    .filter(&who.to_string())
                    .collect()
            }
            fn archived_task(&self, id: i64) -> Option<ArchivedTask> {
                self.db.archived_tasks().id().find(id)
            }
            fn all_archived_tasks(&self) -> Vec<ArchivedTask> {
                ($all_archived)(self)
            }
        }
    };
}

// A view handle has no `.iter()`, and no view serves the archive (only the
// procedures call `all_archived_tasks`), so the view answers empty.
impl_store_reads!(ReducerContext, |s: &ReducerContext| s
    .db
    .archived_tasks()
    .iter()
    .collect());
impl_store_reads!(ViewContext, |_: &ViewContext| Vec::new());
impl_store_reads!(spacetimedb::TxContext, |s: &spacetimedb::TxContext| s
    .db
    .archived_tasks()
    .iter()
    .collect());

// -- The view bodies ----------------------------------------------------------

/// Rows keyed and ordered by `key`, the first of any duplicates kept.
fn dedup_by<T, K: Ord>(rows: impl IntoIterator<Item = T>, key: impl Fn(&T) -> K) -> Vec<T> {
    rows.into_iter()
        .map(|r| (key(&r), r))
        .collect::<BTreeMap<K, T>>()
        .into_values()
        .collect()
}

/// `own_epics`: what the caller created.
pub fn own_epics_of(r: &impl StoreReads, who: &str) -> Vec<Epic> {
    r.epics_created_by(who)
}

/// `own_tasks`: what the caller created, the epic-less tasks they own, and
/// the tasks run on their hosts ([`hosted_tasks_of`]).
pub fn own_tasks_of(r: &impl StoreReads, who: &str) -> Vec<Task> {
    dedup_by(
        r.tasks_created_by(who)
            .into_iter()
            .chain(r.tasks_owned_by(who))
            .chain(hosted_tasks_of(r, who)),
        |t| t.id,
    )
}

/// `own_packages`: what the caller created.
pub fn own_packages_of(r: &impl StoreReads, who: &str) -> Vec<WorkPackage> {
    r.packages_created_by(who)
}

/// `own_subscriptions`: the caller's own follows of epics that still exist.
pub fn own_subscriptions_of(r: &impl StoreReads, who: &str) -> Vec<Subscription> {
    r.subscriptions_of(who)
        .into_iter()
        .filter(|s| r.epic(s.epic_id).is_some())
        .collect()
}

/// `covered_epics`: the epics the caller follows and every descendant, at any
/// depth.
pub fn covered_epics_of(r: &impl StoreReads, who: &str) -> Vec<Epic> {
    let mut covered: BTreeMap<i64, Epic> = BTreeMap::new();
    let mut pending: Vec<Epic> = r
        .subscriptions_of(who)
        .iter()
        .filter_map(|s| r.epic(s.epic_id))
        .collect();
    while let Some(epic) = pending.pop() {
        let id = epic.id;
        if covered.insert(id, epic).is_some() {
            continue;
        }
        pending.extend(
            r.epics_under(id)
                .into_iter()
                .filter(|child| !covered.contains_key(&child.id)),
        );
    }
    covered.into_values().collect()
}

/// `covered_tasks`: the tasks of the covered epics.
pub fn covered_tasks_of(r: &impl StoreReads, who: &str) -> Vec<Task> {
    dedup_by(
        covered_epics_of(r, who)
            .iter()
            .flat_map(|e| r.tasks_in_epic(e.id)),
        |t| t.id,
    )
}

/// The epics the board holds: the caller's own and the covered ones. One
/// person uses the store, so this narrows by delivery, not by access.
pub fn held_epics_of(r: &impl StoreReads, who: &str) -> Vec<Epic> {
    dedup_by(
        own_epics_of(r, who)
            .into_iter()
            .chain(covered_epics_of(r, who)),
        |e| e.id,
    )
}

/// The tasks the board holds: the caller's own and the covered ones (and, by
/// [`own_tasks_of`], the ones run on their hosts). The rows about a task are
/// served for these, never for a wider set.
pub fn held_tasks_of(r: &impl StoreReads, who: &str) -> Vec<Task> {
    dedup_by(
        own_tasks_of(r, who)
            .into_iter()
            .chain(covered_tasks_of(r, who)),
        |t| t.id,
    )
}

/// The tasks with a current agent, or an open dispatch or session request, on
/// a host `who` owns.
///
/// A host's reactor serves the requests and agents of its machine, and acts on
/// the task they name. A caller who dispatched a teammate's task from an epic
/// and then unfollowed it holds no follow over the task, so without this arm
/// `get_task` answers None while the agent runs: the wrap-up's rebase fails as
/// "task deleted" and the agent's subagent rows leave the tree.
pub fn hosted_tasks_of(r: &impl StoreReads, who: &str) -> Vec<Task> {
    let task_ids: Vec<i64> = rows_on_owned_hosts(r, who, |h| r.agents_on_host(h))
        .into_iter()
        .filter(|a| a.state == AGENT_CURRENT)
        .map(|a| a.task_id)
        .chain(
            rows_on_owned_hosts(r, who, |h| r.dispatch_requests_on_host(h))
                .into_iter()
                .filter(|d| dispatch_request_is_open(&d.state))
                .map(|d| d.task_id),
        )
        .chain(
            rows_on_owned_hosts(r, who, |h| r.session_requests_on_host(h))
                .into_iter()
                .filter(|s| session_request_is_open(&s.state))
                .map(|s| s.task_id),
        )
        .collect();
    dedup_by(task_ids.into_iter().filter_map(|id| r.task(id)), |t| t.id)
}

/// Rows about a task, one read per held task, ordered by `key`.
fn rows_about_held_tasks<T, K: Ord>(
    r: &impl StoreReads,
    who: &str,
    rows_of: impl Fn(i64) -> Vec<T>,
    key: impl Fn(&T) -> K,
) -> Vec<T> {
    dedup_by(
        held_tasks_of(r, who).iter().flat_map(|t| rows_of(t.id)),
        key,
    )
}

/// Rows on the machines `who` owns, whatever task they are about.
///
/// A request, and the agent it starts, belongs to the machine that serves it.
/// Serving it by the held task alone loses it the moment the caller stops
/// holding the task (an unfollowed epic): the reactor never sees a wrap-up it
/// must act on, and the requester's read-back never finds its own row.
fn rows_on_owned_hosts<T>(
    r: &impl StoreReads,
    who: &str,
    rows_on: impl Fn(&str) -> Vec<T>,
) -> Vec<T> {
    r.hosts_owned_by(who)
        .iter()
        .flat_map(|h| rows_on(&h.id))
        .collect()
}

pub fn agents_of_held(r: &impl StoreReads, who: &str) -> Vec<Agent> {
    dedup_by(
        rows_about_held_tasks(r, who, |t| r.agents_of(t), |a| a.id)
            .into_iter()
            .chain(rows_on_owned_hosts(r, who, |h| r.agents_on_host(h))),
        |a| a.id,
    )
}

pub fn dispatch_requests_of_held(r: &impl StoreReads, who: &str) -> Vec<DispatchRequest> {
    dedup_by(
        rows_about_held_tasks(r, who, |t| r.dispatch_requests_of(t), |d| d.id)
            .into_iter()
            .chain(rows_on_owned_hosts(r, who, |h| {
                r.dispatch_requests_on_host(h)
            })),
        |d| d.id,
    )
}

pub fn session_requests_of_held(r: &impl StoreReads, who: &str) -> Vec<SessionRequest> {
    dedup_by(
        rows_about_held_tasks(r, who, |t| r.session_requests_of(t), |s| s.id)
            .into_iter()
            .chain(rows_on_owned_hosts(r, who, |h| {
                r.session_requests_on_host(h)
            })),
        |s| s.id,
    )
}

/// Feed requests are about no held task (a teardown's task is deleted), so a
/// caller sees only the rows on the hosts it owns.
pub fn feed_requests_of_owner(r: &impl StoreReads, who: &str) -> Vec<FeedRequest> {
    dedup_by(
        rows_on_owned_hosts(r, who, |h| r.feed_requests_on_host(h)),
        |f| f.id,
    )
}

pub fn subagents_of_held(r: &impl StoreReads, who: &str) -> Vec<TaskSubagent> {
    // No primary key: a row is the (task, agent, session) it names.
    rows_about_held_tasks(
        r,
        who,
        |t| r.subagents_of(t),
        |s| (s.task_id, s.agent_id.clone(), s.session_id.clone()),
    )
}

pub fn watchers_of_held(r: &impl StoreReads, who: &str) -> Vec<TaskWatcher> {
    rows_about_held_tasks(r, who, |t| r.watchers_of(t), |w| w.id)
}

pub fn retrievals_of_held(r: &impl StoreReads, who: &str) -> Vec<LearningRetrieval> {
    rows_about_held_tasks(r, who, |t| r.retrievals_of(t), |l| l.id)
}

pub fn verdicts_of_held(r: &impl StoreReads, who: &str) -> Vec<LearningVerdict> {
    rows_about_held_tasks(r, who, |t| r.verdicts_of(t), |l| l.id)
}

/// `own_bookmarks`: the caller's own bookmarks, on tasks that still exist
/// (`core.allium: Bookmark`).
pub fn own_bookmarks_of(r: &impl StoreReads, who: &str) -> Vec<Bookmark> {
    dedup_by(
        r.bookmarks_of(who)
            .into_iter()
            .filter(|b| r.task(b.task_id).is_some()),
        |b| b.id.clone(),
    )
}

/// `own_unseen_tasks`: the caller's own marks, on tasks that still exist.
pub fn own_unseen_of(r: &impl StoreReads, who: &str) -> Vec<UnseenTask> {
    dedup_by(
        r.unseen_of(who)
            .into_iter()
            .filter(|u| r.task(u.task_id).is_some()),
        |u| u.id.clone(),
    )
}

/// `visible_poll_owners`: a claim follows its scope. An epic-scoped row is
/// served while the caller holds the epic, a task-scoped one the task.
pub fn poll_owners_visible_to(r: &impl StoreReads, who: &str) -> Vec<PollOwner> {
    let epic_rows = held_epics_of(r, who)
        .into_iter()
        .flat_map(|e| r.poll_owners_of(e.id))
        .filter(|p| p.scope == POLL_SCOPE_EPIC);
    let task_rows = held_tasks_of(r, who)
        .into_iter()
        .flat_map(|t| r.poll_owners_of(t.id))
        .filter(|p| p.scope == POLL_SCOPE_TASK);
    dedup_by(epic_rows.chain(task_rows), |p| p.id)
}

/// `visible_work_package_links`: the links of the packages the caller holds,
/// which are the ones they created.
pub fn links_visible_to(r: &impl StoreReads, who: &str) -> Vec<WorkPackageLink> {
    dedup_by(
        own_packages_of(r, who)
            .iter()
            .flat_map(|p| r.links_of_package(p.id)),
        |l| l.id.clone(),
    )
}

/// `visible_learnings`: every learning. One person uses the store, so none is
/// withheld; a full read is the accepted cost.
pub fn learnings_visible_to(r: &impl StoreReads) -> Vec<Learning> {
    r.all_learnings()
}

/// `own_settings`: the settings of the hosts the caller owns (D9).
pub fn settings_visible_to(r: &impl StoreReads, who: &str) -> Vec<Setting> {
    dedup_by(
        r.hosts_owned_by(who)
            .iter()
            .flat_map(|h| r.settings_of(&h.id)),
        |s| s.id.clone(),
    )
}

/// What `own_usage_summary` asks of the caller's events.
#[derive(Clone, Debug, Default)]
pub struct UsageFilter {
    pub category: Option<String>,
    pub actor: Option<String>,
    /// Inclusive lower bound, in the store's fixed-width `recorded_at` text,
    /// which sorts the same as the instants it spells.
    pub since: Option<String>,
    pub limit: i64,
}

/// One group of the caller's usage events: what `own_usage_summary` returns.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, PartialEq)]
pub struct UsageSummaryRow {
    pub category: String,
    pub action: String,
    pub detail: Option<String>,
    pub actor: String,
    pub count: i64,
    pub last_used: String,
}

/// `own_usage_summary`: the events the caller recorded, grouped by
/// category, action, detail and actor, rarest first. An event names no task,
/// so the held-task rule cannot apply; the caller sees their own.
///
/// Served on demand, never as a view: a view over usage is recomputed whole on
/// every recorded event (`docs/architecture.md`, "Read enforcement: per-caller views").
pub fn usage_summary_for(
    r: &impl StoreReads,
    who: &str,
    filter: &UsageFilter,
) -> Vec<UsageSummaryRow> {
    type Key = (String, String, Option<String>, String);
    let mut groups: std::collections::HashMap<Key, (i64, String)> =
        std::collections::HashMap::new();
    // One index read: each row once.
    for e in r.usage_events_recorded_by(who) {
        if filter.category.as_ref().is_some_and(|c| *c != e.category)
            || filter.actor.as_ref().is_some_and(|a| *a != e.actor)
            || filter.since.as_ref().is_some_and(|s| e.recorded_at < *s)
        {
            continue;
        }
        let entry = groups
            .entry((e.category, e.action, e.detail, e.actor))
            .or_insert((0, String::new()));
        entry.0 += 1;
        if e.recorded_at > entry.1 {
            entry.1 = e.recorded_at;
        }
    }
    let mut out: Vec<UsageSummaryRow> = groups
        .into_iter()
        .map(
            |((category, action, detail, actor), (count, last_used))| UsageSummaryRow {
                category,
                action,
                detail,
                actor,
                count,
                last_used,
            },
        )
        .collect();
    // Rarest first; the key breaks ties so the order is the same every call.
    out.sort_by(|a, b| {
        (a.count, &a.category, &a.action, &a.detail, &a.actor).cmp(&(
            b.count,
            &b.category,
            &b.action,
            &b.detail,
            &b.actor,
        ))
    });
    out.truncate(filter.limit.clamp(1, 500) as usize);
    out
}

/// One task that stops a delete: what `check_deletable` returns per blocker.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, PartialEq)]
pub struct DeleteBlocker {
    /// The epic whose delete this task blocks, or 0 when the task itself was
    /// asked to be deleted.
    pub epic_id: i64,
    pub task_id: i64,
    pub title: String,
    pub status: String,
}

/// The answer of `check_deletable`: why a delete would be refused, read from
/// the store's true rows. Empty means the reducers would not refuse it.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, PartialEq, Default)]
pub struct DeleteCheck {
    /// For each requested epic, every task in its subtree that is not done,
    /// lowest id first; for each requested task, the task itself when it is
    /// not done.
    pub blockers: Vec<DeleteBlocker>,
    /// A refusal that names no task: a subtree nested too deep to delete.
    pub refusal: Option<String>,
}

/// `delete_task`, `delete_epic` and `batch_delete`'s validation pass, read-only
/// and over one snapshot: the rules those reducers apply before they delete
/// anything. A row that is gone is skipped, as in the reducers (the delete is then a no-op). Mirrored by
/// `MemoryReducerCaller::apply_check_deletable`.
pub fn delete_check_for(r: &impl StoreReads, task_ids: &[i64], epic_ids: &[i64]) -> DeleteCheck {
    let mut check = DeleteCheck::default();
    for &id in epic_ids {
        if r.epic(id).is_none() {
            continue;
        }
        let mut doomed = std::collections::HashSet::new();
        let mut stack = vec![id];
        while let Some(next) = stack.pop() {
            if doomed.insert(next) {
                stack.extend(r.epics_under(next).into_iter().map(|e| e.id));
            }
        }
        if crate::rules::subtree_exceeds_depth(
            id,
            |epic| r.epics_under(epic).into_iter().map(|e| e.id).collect(),
            crate::tasks_epics::MAX_EPIC_DEPTH,
        ) {
            check.refusal = Some(format!(
                "epic {id}: cannot delete a subtree nested more than {} levels deep",
                crate::tasks_epics::MAX_EPIC_DEPTH
            ));
            return check;
        }
        // Every undone task, lowest id first, not just the lowest: a board
        // that cannot decode the lowest one still names its lowest readable
        // blocker (`epics.allium: DeleteEpicRefused`).
        let mut undone: Vec<_> = doomed
            .iter()
            .flat_map(|e| r.tasks_in_epic(*e))
            .filter(|t| t.status != DONE)
            .collect();
        undone.sort_by_key(|t| t.id);
        check
            .blockers
            .extend(undone.into_iter().map(|t| DeleteBlocker {
                epic_id: id,
                task_id: t.id,
                title: t.title,
                status: t.status,
            }));
    }
    for &id in task_ids {
        let Some(task) = r.task(id) else { continue };
        if task.status != DONE {
            check.blockers.push(DeleteBlocker {
                epic_id: 0,
                task_id: task.id,
                title: task.title,
                status: task.status,
            });
        }
    }
    check
}

// -- The views ----------------------------------------------------------------

fn sender(ctx: &ViewContext) -> String {
    ctx.sender().to_hex().to_string()
}

#[spacetimedb::view(accessor = own_epics, public)]
fn own_epics(ctx: &ViewContext) -> Vec<Epic> {
    own_epics_of(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = own_tasks, public)]
fn own_tasks(ctx: &ViewContext) -> Vec<Task> {
    own_tasks_of(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = own_packages, public)]
fn own_packages(ctx: &ViewContext) -> Vec<WorkPackage> {
    own_packages_of(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = covered_epics, public)]
fn covered_epics(ctx: &ViewContext) -> Vec<Epic> {
    covered_epics_of(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = covered_tasks, public)]
fn covered_tasks(ctx: &ViewContext) -> Vec<Task> {
    covered_tasks_of(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_agents, public)]
fn visible_agents(ctx: &ViewContext) -> Vec<Agent> {
    agents_of_held(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_dispatch_requests, public)]
fn visible_dispatch_requests(ctx: &ViewContext) -> Vec<DispatchRequest> {
    dispatch_requests_of_held(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_session_requests, public)]
fn visible_session_requests(ctx: &ViewContext) -> Vec<SessionRequest> {
    session_requests_of_held(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_feed_requests, public)]
fn visible_feed_requests(ctx: &ViewContext) -> Vec<FeedRequest> {
    feed_requests_of_owner(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = own_subscriptions, public)]
fn own_subscriptions(ctx: &ViewContext) -> Vec<Subscription> {
    own_subscriptions_of(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = own_unseen_tasks, public)]
fn own_unseen_tasks(ctx: &ViewContext) -> Vec<UnseenTask> {
    own_unseen_of(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = own_bookmarks, public)]
fn own_bookmarks(ctx: &ViewContext) -> Vec<Bookmark> {
    own_bookmarks_of(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_task_subagents, public)]
fn visible_task_subagents(ctx: &ViewContext) -> Vec<TaskSubagent> {
    subagents_of_held(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_task_watchers, public)]
fn visible_task_watchers(ctx: &ViewContext) -> Vec<TaskWatcher> {
    watchers_of_held(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_learning_retrievals, public)]
fn visible_learning_retrievals(ctx: &ViewContext) -> Vec<LearningRetrieval> {
    retrievals_of_held(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_learning_verdicts, public)]
fn visible_learning_verdicts(ctx: &ViewContext) -> Vec<LearningVerdict> {
    verdicts_of_held(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_poll_owners, public)]
fn visible_poll_owners(ctx: &ViewContext) -> Vec<PollOwner> {
    poll_owners_visible_to(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_work_package_links, public)]
fn visible_work_package_links(ctx: &ViewContext) -> Vec<WorkPackageLink> {
    links_visible_to(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_learnings, public)]
fn visible_learnings(ctx: &ViewContext) -> Vec<Learning> {
    learnings_visible_to(ctx)
}

#[spacetimedb::view(accessor = own_settings, public)]
fn own_settings(ctx: &ViewContext) -> Vec<Setting> {
    settings_visible_to(ctx, &sender(ctx))
}
