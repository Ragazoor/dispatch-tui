//! The per-caller views (`workspaces.allium`: "THE PER-CALLER VIEWS").
//!
//! Each view answers one question the board asks, for `ctx.sender()`, and is a
//! thin shell over a function here that is generic over [`StoreReads`]. The
//! module runs it over a [`ViewContext`], the reducers run the predicates it is
//! built from over a [`ReducerContext`], and the in-memory store runs the same
//! function over its own rows (`src/sync/memory_caller/views.rs`), so the
//! module and its double cannot disagree about who sees what.
//!
//! Every read is a point read on a btree index: a view handle has no `.iter()`,
//! and a range read counts as a full scan, so it would re-run the view on every
//! write to that table (`docs/plans/2026-10-28-per-caller-views-spike.md`).
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
    fn workspace(&self, id: i64) -> Option<Workspace>;
    fn profile(&self, who: &str) -> Option<UserProfile>;
    fn members_of(&self, workspace_id: i64) -> Vec<Membership>;
    fn memberships_of(&self, user: &str) -> Vec<Membership>;
    fn subscriptions_of(&self, subscriber: &str) -> Vec<Subscription>;
    fn epics_under(&self, parent: i64) -> Vec<Epic>;
    fn epics_in_workspace(&self, workspace_id: i64) -> Vec<Epic>;
    fn epics_created_by(&self, who: &str) -> Vec<Epic>;
    fn tasks_in_epic(&self, epic_id: i64) -> Vec<Task>;
    fn tasks_owned_by(&self, who: &str) -> Vec<Task>;
    fn tasks_created_by(&self, who: &str) -> Vec<Task>;
    fn packages_in_workspace(&self, workspace_id: i64) -> Vec<WorkPackage>;
    fn packages_created_by(&self, who: &str) -> Vec<WorkPackage>;
    fn links_of_package(&self, package_id: i64) -> Vec<WorkPackageLink>;
    fn agents_of(&self, task_id: i64) -> Vec<Agent>;
    fn dispatch_requests_of(&self, task_id: i64) -> Vec<DispatchRequest>;
    fn session_requests_of(&self, task_id: i64) -> Vec<SessionRequest>;
    fn unseen_of(&self, subscriber: &str) -> Vec<UnseenTask>;
    fn subagents_of(&self, task_id: i64) -> Vec<TaskSubagent>;
    /// Rows naming the task as the watcher or as the target.
    fn watchers_of(&self, task_id: i64) -> Vec<TaskWatcher>;
    fn retrievals_of(&self, task_id: i64) -> Vec<LearningRetrieval>;
    fn verdicts_of(&self, task_id: i64) -> Vec<LearningVerdict>;
    fn poll_owners_of(&self, scope_id: i64) -> Vec<PollOwner>;
    fn learnings_in_workspace(&self, workspace_id: i64) -> Vec<Learning>;
    fn learnings_by_author(&self, author: &str) -> Vec<Learning>;
    fn hosts_owned_by(&self, owner: &str) -> Vec<Host>;
    fn settings_of(&self, host: &str) -> Vec<Setting>;
}

/// `StoreReads` over a context whose `db` is a table set: the reducers' and the
/// views' handles spell every read the same way.
macro_rules! impl_store_reads {
    ($ctx:ty) => {
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
            fn workspace(&self, id: i64) -> Option<Workspace> {
                self.db.workspaces().id().find(id)
            }
            fn profile(&self, who: &str) -> Option<UserProfile> {
                self.db.user_profiles().identity().find(&who.to_string())
            }
            fn members_of(&self, workspace_id: i64) -> Vec<Membership> {
                self.db
                    .memberships()
                    .workspace_id()
                    .filter(&workspace_id)
                    .collect()
            }
            fn memberships_of(&self, user: &str) -> Vec<Membership> {
                self.db
                    .memberships()
                    .user()
                    .filter(&user.to_string())
                    .collect()
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
            fn epics_in_workspace(&self, workspace_id: i64) -> Vec<Epic> {
                self.db
                    .epics()
                    .workspace_id()
                    .filter(&workspace_id)
                    .collect()
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
            fn packages_in_workspace(&self, workspace_id: i64) -> Vec<WorkPackage> {
                self.db
                    .work_packages()
                    .workspace_id()
                    .filter(&workspace_id)
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
            fn unseen_of(&self, subscriber: &str) -> Vec<UnseenTask> {
                self.db
                    .unseen_tasks()
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
            fn learnings_in_workspace(&self, workspace_id: i64) -> Vec<Learning> {
                self.db
                    .learnings()
                    .workspace_id()
                    .filter(&workspace_id)
                    .collect()
            }
            fn learnings_by_author(&self, author: &str) -> Vec<Learning> {
                self.db
                    .learnings()
                    .author()
                    .filter(&author.to_string())
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
        }
    };
}

impl_store_reads!(ReducerContext);
impl_store_reads!(ViewContext);

// -- The view bodies ----------------------------------------------------------

/// Rows keyed and ordered by `key`, the first of any duplicates kept.
fn dedup_by<T, K: Ord>(rows: impl IntoIterator<Item = T>, key: impl Fn(&T) -> K) -> Vec<T> {
    rows.into_iter()
        .map(|r| (key(&r), r))
        .collect::<BTreeMap<K, T>>()
        .into_values()
        .collect()
}

/// `own_epics`: what the caller created, while it is still visible to them.
pub fn own_epics_of(r: &impl StoreReads, who: &str) -> Vec<Epic> {
    dedup_by(
        r.epics_created_by(who)
            .into_iter()
            .filter(|e| epic_visible_to(r, e, who)),
        |e| e.id,
    )
}

/// `own_tasks`: what the caller created, and the epic-less tasks they own,
/// while visible.
pub fn own_tasks_of(r: &impl StoreReads, who: &str) -> Vec<Task> {
    dedup_by(
        r.tasks_created_by(who)
            .into_iter()
            .chain(r.tasks_owned_by(who))
            .filter(|t| task_visible_to(r, t, who)),
        |t| t.id,
    )
}

/// `own_packages`: what the caller created, while visible.
pub fn own_packages_of(r: &impl StoreReads, who: &str) -> Vec<WorkPackage> {
    dedup_by(
        r.packages_created_by(who)
            .into_iter()
            .filter(|p| package_visible_to(r, p, who)),
        |p| p.id,
    )
}

/// `covered_epics`: the epics the caller follows and every descendant, at any
/// depth. The walk stops at an epic the caller cannot see: a private epic hides
/// its whole subtree, so nothing below it is theirs either.
pub fn covered_epics_of(r: &impl StoreReads, who: &str) -> Vec<Epic> {
    let mut covered: BTreeMap<i64, Epic> = BTreeMap::new();
    let mut pending: Vec<Epic> = r
        .subscriptions_of(who)
        .iter()
        .filter_map(|s| r.epic(s.epic_id))
        .filter(|e| epic_visible_to(r, e, who))
        .collect();
    while let Some(epic) = pending.pop() {
        let id = epic.id;
        if covered.insert(id, epic).is_some() {
            continue;
        }
        pending.extend(
            r.epics_under(id)
                .into_iter()
                .filter(|child| !covered.contains_key(&child.id) && epic_visible_to(r, child, who)),
        );
    }
    covered.into_values().collect()
}

/// `covered_tasks`: the tasks of the covered epics. A task in a visible epic is
/// visible, so no per-task check is made.
pub fn covered_tasks_of(r: &impl StoreReads, who: &str) -> Vec<Task> {
    dedup_by(
        covered_epics_of(r, who)
            .iter()
            .flat_map(|e| r.tasks_in_epic(e.id)),
        |t| t.id,
    )
}

/// The workspaces `who` belongs to.
fn workspaces_of(r: &impl StoreReads, who: &str) -> Vec<i64> {
    let mut ids: Vec<i64> = r
        .memberships_of(who)
        .iter()
        .map(|m| m.workspace_id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// `visible_epics`: every epic the caller sees in a workspace they belong to.
pub fn visible_epics_of(r: &impl StoreReads, who: &str) -> Vec<Epic> {
    dedup_by(
        workspaces_of(r, who)
            .into_iter()
            .flat_map(|ws| r.epics_in_workspace(ws))
            .filter(|e| epic_visible_to(r, e, who)),
        |e| e.id,
    )
}

/// `visible_tasks`: the tasks of the visible epics.
pub fn visible_tasks_of(r: &impl StoreReads, who: &str) -> Vec<Task> {
    dedup_by(
        visible_epics_of(r, who)
            .iter()
            .flat_map(|e| r.tasks_in_epic(e.id)),
        |t| t.id,
    )
}

/// `visible_packages`: every package the caller sees in their workspaces.
pub fn visible_packages_of(r: &impl StoreReads, who: &str) -> Vec<WorkPackage> {
    dedup_by(
        workspaces_of(r, who)
            .into_iter()
            .flat_map(|ws| r.packages_in_workspace(ws))
            .filter(|p| package_visible_to(r, p, who)),
        |p| p.id,
    )
}

/// The tasks the board holds: the caller's own and the covered ones. The rows
/// about a task are served for these, never for a wider set.
pub fn held_tasks_of(r: &impl StoreReads, who: &str) -> Vec<Task> {
    dedup_by(
        own_tasks_of(r, who)
            .into_iter()
            .chain(covered_tasks_of(r, who)),
        |t| t.id,
    )
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

pub fn agents_of_held(r: &impl StoreReads, who: &str) -> Vec<Agent> {
    rows_about_held_tasks(r, who, |t| r.agents_of(t), |a| a.id)
}

pub fn dispatch_requests_of_held(r: &impl StoreReads, who: &str) -> Vec<DispatchRequest> {
    rows_about_held_tasks(r, who, |t| r.dispatch_requests_of(t), |d| d.id)
}

pub fn session_requests_of_held(r: &impl StoreReads, who: &str) -> Vec<SessionRequest> {
    rows_about_held_tasks(r, who, |t| r.session_requests_of(t), |s| s.id)
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

/// `own_unseen_tasks`: the caller's own marks, on tasks they can still see.
pub fn own_unseen_of(r: &impl StoreReads, who: &str) -> Vec<UnseenTask> {
    dedup_by(
        r.unseen_of(who).into_iter().filter(|u| {
            r.task(u.task_id)
                .is_some_and(|t| task_visible_to(r, &t, who))
        }),
        |u| u.id.clone(),
    )
}

/// `visible_poll_owners`: a claim follows its scope. An epic-scoped row is
/// served while the epic is visible, a task-scoped one while the task is held.
pub fn poll_owners_visible_to(r: &impl StoreReads, who: &str) -> Vec<PollOwner> {
    let epic_rows = visible_epics_of(r, who)
        .into_iter()
        .flat_map(|e| r.poll_owners_of(e.id))
        .filter(|p| p.scope == POLL_SCOPE_EPIC);
    let task_rows = held_tasks_of(r, who)
        .into_iter()
        .flat_map(|t| r.poll_owners_of(t.id))
        .filter(|p| p.scope == POLL_SCOPE_TASK);
    dedup_by(epic_rows.chain(task_rows), |p| p.id)
}

/// `visible_work_package_links`: a link is served when the caller sees both the
/// package and the item it names.
pub fn links_visible_to(r: &impl StoreReads, who: &str) -> Vec<WorkPackageLink> {
    dedup_by(
        visible_packages_of(r, who)
            .iter()
            .flat_map(|p| r.links_of_package(p.id))
            .filter(|l| match l.kind.as_str() {
                WORK_PACKAGE_ITEM_TASK => r
                    .task(l.item_id)
                    .is_some_and(|t| task_visible_to(r, &t, who)),
                WORK_PACKAGE_ITEM_EPIC => r
                    .epic(l.item_id)
                    .is_some_and(|e| epic_visible_to(r, &e, who)),
                _ => false,
            }),
        |l| l.id.clone(),
    )
}

/// `visible_learnings`: the learnings `LearningVisibility` lets the caller see.
pub fn learnings_visible_to(r: &impl StoreReads, who: &str) -> Vec<Learning> {
    dedup_by(
        workspaces_of(r, who)
            .into_iter()
            .flat_map(|ws| r.learnings_in_workspace(ws))
            .chain(r.learnings_by_author(who))
            // A learning that predates workspaces has no author.
            .chain(r.learnings_by_author(""))
            .filter(|l| learning_seen_by(r, l, who)),
        |l| l.id,
    )
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

/// `visible_workspaces`: the workspaces the caller belongs to. A super-admin's
/// list of every workspace needs a column a view can range over, which the
/// table has not (open question in `workspaces.allium`).
pub fn workspaces_visible_to(r: &impl StoreReads, who: &str) -> Vec<Workspace> {
    dedup_by(
        workspaces_of(r, who)
            .into_iter()
            .filter_map(|id| r.workspace(id)),
        |w| w.id,
    )
}

/// `visible_memberships`: the membership rows of the visible workspaces.
pub fn memberships_visible_to(r: &impl StoreReads, who: &str) -> Vec<Membership> {
    dedup_by(
        workspaces_visible_to(r, who)
            .iter()
            .flat_map(|w| r.members_of(w.id)),
        |m| m.id,
    )
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

#[spacetimedb::view(accessor = visible_epics, public)]
fn visible_epics(ctx: &ViewContext) -> Vec<Epic> {
    visible_epics_of(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_tasks, public)]
fn visible_tasks(ctx: &ViewContext) -> Vec<Task> {
    visible_tasks_of(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_packages, public)]
fn visible_packages(ctx: &ViewContext) -> Vec<WorkPackage> {
    visible_packages_of(ctx, &sender(ctx))
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

#[spacetimedb::view(accessor = own_unseen_tasks, public)]
fn own_unseen_tasks(ctx: &ViewContext) -> Vec<UnseenTask> {
    own_unseen_of(ctx, &sender(ctx))
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
    learnings_visible_to(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = own_settings, public)]
fn own_settings(ctx: &ViewContext) -> Vec<Setting> {
    settings_visible_to(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_workspaces, public)]
fn visible_workspaces(ctx: &ViewContext) -> Vec<Workspace> {
    workspaces_visible_to(ctx, &sender(ctx))
}

#[spacetimedb::view(accessor = visible_memberships, public)]
fn visible_memberships(ctx: &ViewContext) -> Vec<Membership> {
    memberships_visible_to(ctx, &sender(ctx))
}
