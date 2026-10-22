//! Row callbacks: pointing the SDK's table events at [`SharedRows`](crate::sync::SharedRows), and the
//! sub-epic walk that widens the subscription as epics arrive.

use spacetimedb_sdk::{DbContext, Table as _, TableWithPrimaryKey as _};
use std::sync::{Arc, Mutex};

use super::subtree_queries;
use super::SpacetimeSdkConnector;
use crate::spacetime::bindings;
use crate::spacetime::bindings::{
    AgentsTableAccess as _, DbConnection, DispatchRequestsTableAccess as _, EpicsTableAccess as _,
    HostsTableAccess as _, LearningRetrievalsTableAccess as _, LearningsTableAccess as _,
    PollOwnersTableAccess as _, RepoBaseBranchesTableAccess as _, RepoPathsTableAccess as _,
    RetiredFeedItemsTableAccess as _, SessionRequestsTableAccess as _, SettingsTableAccess as _,
    SubscriptionHandle, SubscriptionsTableAccess as _, TasksTableAccess as _,
    UnseenTasksTableAccess as _, UsageEventsTableAccess as _, WorkPackageLinksTableAccess as _,
    WorkPackagesTableAccess as _,
};
use crate::sync::subtree::SubtreeCover;

// One table's three callbacks, so "every table gets all three" is
// structural rather than something a reader verifies by counting.
//
// An update is an UPSERT, not a patch: the SDK hands over the old row
// and the new one, and only the new one is kept — the rows are keyed by
// id and a row's id cannot change.
macro_rules! wire {
    ($wiring:expr, $db:expr, $table:ident, $upsert:ident, $remove:ident, $key:expr) => {{
        let rows = $wiring.rows.clone();
        $db.$table().on_insert(move |_, row| rows.$upsert(row));
        let rows = $wiring.rows.clone();
        $db.$table()
            .on_update(move |_, _old, new| rows.$upsert(new));
        let rows = $wiring.rows.clone();
        let key = $key;
        $db.$table().on_delete(move |_, row| rows.$remove(key(row)));
    }};
}

impl SpacetimeSdkConnector {
    /// Re-ask for the sub-epic levels of epics this connection already holds,
    /// after a replacing subscribe reset the walk. See [`reseed_queries`].
    pub(super) fn reseed_widenings(&self, connection: &DbConnection) {
        let known: Vec<(i64, i64)> = connection
            .db()
            .epics()
            .iter()
            .map(|epic| (epic.id, epic.parent_epic_id))
            .collect();
        let mut subtree = self.subtree.lock().unwrap_or_else(|e| e.into_inner());
        let queries = reseed_queries(&mut subtree, &known);
        if queries.is_empty() {
            return;
        }
        let handle = connection
            .subscription_builder()
            .on_error(|_ctx, error| {
                tracing::warn!("re-asking for sub-epics after a resubscribe failed: {error}");
            })
            .subscribe(queries);
        subtree.widenings.push(handle);
    }

    /// Point every subscribed table at [`Self::rows`].
    ///
    /// Registered once per connection, right after it is installed and BEFORE
    /// anything is subscribed. The order matters: a subscription applied first
    /// delivers its initial rows through these same callbacks, and callbacks
    /// registered afterwards would miss every row that was already there —
    /// producing a board that is empty until somebody else edits something.
    ///
    /// **An update is an upsert, not a patch.** The SDK hands over the old row
    /// and the new one; only the new one is kept, because
    /// [`SharedRows`](crate::sync::SharedRows) is keyed by id and a row's id cannot change.
    pub(super) fn wire_rows(&self, connection: &DbConnection) {
        let db = connection.db();
        self.wire_tables(db);
        self.wire_subtree_walk(db);
    }

    /// Every table's row callbacks. See [`Self::wire_rows`].
    pub(super) fn wire_tables(&self, db: &bindings::RemoteTables) {
        self.wire_board_tables(db);
        self.wire_shared_tables(db);
    }

    /// Row callbacks for the tables the board itself shows.
    fn wire_board_tables(&self, db: &bindings::RemoteTables) {
        wire!(
            self,
            db,
            tasks,
            upsert_task,
            remove_task,
            |row: &bindings::Task| { crate::models::TaskId(row.id) }
        );
        wire!(
            self,
            db,
            epics,
            upsert_epic,
            remove_epic,
            |row: &bindings::Epic| { crate::models::EpicId(row.id) }
        );
        wire!(
            self,
            db,
            repo_paths,
            upsert_repo_path,
            remove_repo_path,
            |row: &bindings::RepoPath| row.id
        );
        wire!(
            self,
            db,
            repo_base_branches,
            upsert_repo_base_branch,
            remove_repo_base_branch,
            |row: &bindings::RepoBaseBranch| row.id
        );
        // `hosts` is written out rather than passed through `wire!`. Its key is
        // a borrowed `&str` rather than an owned id, and a closure returning a
        // borrow of its own argument needs a higher-ranked bound the macro's
        // `$key:expr` cannot carry. Three lines of repetition beat a macro
        // contorted to fit one caller.
        let rows = self.rows.clone();
        db.hosts().on_insert(move |_, row| rows.upsert_host(row));
        let rows = self.rows.clone();
        db.hosts()
            .on_update(move |_, _old, new| rows.upsert_host(new));
        let rows = self.rows.clone();
        db.hosts()
            .on_delete(move |_, row| rows.remove_host(&row.id));
    }

    /// Row callbacks for the remaining shared tables.
    fn wire_shared_tables(&self, db: &bindings::RemoteTables) {
        wire!(
            self,
            db,
            poll_owners,
            upsert_poll_owner,
            remove_poll_owner,
            |row: &bindings::PollOwner| row.id
        );
        wire!(
            self,
            db,
            learnings,
            upsert_learning,
            remove_learning,
            |row: &bindings::Learning| crate::models::LearningId(row.id)
        );
        wire!(
            self,
            db,
            learning_retrievals,
            upsert_learning_retrieval,
            remove_learning_retrieval,
            |row: &bindings::LearningRetrieval| row.id
        );
        wire!(
            self,
            db,
            usage_events,
            upsert_usage_event,
            remove_usage_event,
            |row: &bindings::UsageEvent| row.id
        );
        wire!(
            self,
            db,
            retired_feed_items,
            upsert_retired_feed_item,
            remove_retired_feed_item,
            |row: &bindings::RetiredFeedItem| row.id
        );

        wire!(
            self,
            db,
            subscriptions,
            upsert_subscription,
            remove_subscription,
            |row: &bindings::Subscription| row.id.clone()
        );
        wire!(
            self,
            db,
            unseen_tasks,
            upsert_unseen_task,
            remove_unseen_task,
            |row: &bindings::UnseenTask| row.id.clone()
        );
        wire!(
            self,
            db,
            settings,
            upsert_setting,
            remove_setting,
            |row: &bindings::Setting| { row.id.clone() }
        );
        wire!(
            self,
            db,
            work_packages,
            upsert_work_package,
            remove_work_package,
            |row: &bindings::WorkPackage| crate::models::WorkPackageId(row.id)
        );
        wire!(
            self,
            db,
            work_package_links,
            upsert_work_package_link,
            remove_work_package_link,
            |row: &bindings::WorkPackageLink| row.id.clone()
        );
        wire!(
            self,
            db,
            dispatch_requests,
            upsert_dispatch_request,
            remove_dispatch_request,
            |row: &bindings::DispatchRequest| row.id
        );
        wire!(
            self,
            db,
            session_requests,
            upsert_session_request,
            remove_session_request,
            |row: &bindings::SessionRequest| row.id
        );
        wire!(
            self,
            db,
            agents,
            upsert_agent,
            remove_agent,
            |row: &bindings::Agent| crate::models::AgentId(row.id)
        );
    }

    /// The sub-epic walk: about the ASK rather than the rows, so apart from
    /// the table wiring. A follow widens the ask, on the initial load and live
    /// alike — see `follow_epic`.
    pub(super) fn wire_subtree_walk(&self, db: &bindings::RemoteTables) {
        let subtree = Arc::clone(&self.subtree);
        db.subscriptions()
            .on_insert(move |ctx, row| follow_epic(ctx, &subtree, row.epic_id));
        let subtree = Arc::clone(&self.subtree);
        db.epics()
            .on_insert(move |ctx, row| widen_subtree(ctx, &subtree, row));
        let subtree = Arc::clone(&self.subtree);
        db.epics().on_update(move |ctx, old, new| {
            if moved_under_new_parent(old.parent_epic_id, new.parent_epic_id) {
                widen_subtree(ctx, &subtree, new);
            }
        });
    }
}

/// One connection's sub-epic walk: what is covered, and the subscriptions
/// that widened the ask to reach it.
///
/// The handles are kept for the reason [`SpacetimeSdkConnector::subscription`]
/// keeps its own: dropping one does not unsubscribe, so the next walk must be
/// able to end this one's.
#[derive(Default)]
pub(super) struct Subtree {
    pub(super) cover: SubtreeCover,
    pub(super) widenings: Vec<SubscriptionHandle>,
}

/// An epic row arrived or moved: if it now sits under a covered epic, ask
/// for its subtree — and for that of any descendant this board already holds.
///
/// Runs on the SDK's thread, inside a row callback, which is why the answer
/// is a fresh subscription rather than a reply to anybody. A widening that
/// fails is logged and not retried: the rows it would have brought stay
/// missing until the next connection re-walks the tree (the rule's
/// "NOT RETRIED" clause).
fn widen_subtree(ctx: &bindings::EventContext, subtree: &Mutex<Subtree>, row: &bindings::Epic) {
    let mut subtree = subtree.lock().unwrap_or_else(|e| e.into_inner());
    let queries = widen_queries(&mut subtree, row.id, row.parent_epic_id, &known_epics(ctx));
    subscribe_widening(ctx, &mut subtree, queries, "a sub-epic");
}

/// The queries an epic row's arrival adds to the ask: none unless it now sits
/// under a covered epic. The decision of [`widen_subtree`], apart from the
/// SDK so it runs without a connection.
pub(super) fn widen_queries(
    subtree: &mut Subtree,
    id: i64,
    parent: i64,
    known: &[(i64, i64)],
) -> Vec<String> {
    // Checked before the cache is gathered by the caller's `known`: nearly
    // every arrival — the whole initial load included — sits under an
    // uncovered parent or none.
    if !subtree.cover.covers(parent) {
        return Vec::new();
    }
    let newly = subtree.cover.delivered(id, parent, known);
    newly.into_iter().flat_map(subtree_queries).collect()
}

/// The queries a replacing subscribe must add for epics the connection
/// already holds.
///
/// `reset_subtree` ends every widening, and the deeper levels would otherwise
/// come back only from `on_insert` callbacks — which rows already in the SDK
/// cache never fire. So a still-followed epic's sub-epic tasks would vanish
/// until reconnect. One pass is enough in any order: an epic under a covered
/// parent covers its cached descendants too, so one met earlier is picked up
/// then.
pub(super) fn reseed_queries(subtree: &mut Subtree, known: &[(i64, i64)]) -> Vec<String> {
    known
        .iter()
        .flat_map(|&(id, parent)| widen_queries(subtree, id, parent, known))
        .collect()
}

/// Whether an epic update re-parented the epic, the only update that can
/// bring it under a covered epic.
pub(super) fn moved_under_new_parent(old_parent: i64, new_parent: i64) -> bool {
    old_parent != new_parent
}

/// Every `(id, parent)` pair the connection holds, for [`SubtreeCover`].
///
/// The SDK's own cache, not `SharedRows`: it already holds the arriving row
/// when a callback fires, and callback order between the two epic handlers is
/// not something to depend on.
fn known_epics(ctx: &bindings::EventContext) -> Vec<(i64, i64)> {
    ctx.db
        .epics()
        .iter()
        .map(|epic| (epic.id, epic.parent_epic_id))
        .collect()
}

/// Send one widening subscription, and keep its handle with the walk so the
/// next `subscribe` unsubscribes it. A widening the store refuses is logged,
/// not retried.
fn subscribe_widening(
    ctx: &bindings::EventContext,
    subtree: &mut Subtree,
    queries: Vec<String>,
    what: &'static str,
) {
    if queries.is_empty() {
        return;
    }
    let handle = ctx
        .subscription_builder()
        .on_error(move |_ctx, error| {
            tracing::warn!("widening the subscription to {what} failed: {error}");
        })
        .subscribe(queries);
    subtree.widenings.push(handle);
}

/// A `Subscription` row arrived: ask for the epic it follows, and its tree.
///
/// Spec: `sync.allium`'s `ASubscriptionRowWidensTheAsk`. This is how followed
/// epics reach the ask at all. The session builds its first subscription
/// before any row has arrived, so the followed-epic list it can read then is
/// empty; the subscription rows come in with that first subscription, and
/// each one widens it from here. The same callback is what makes an epic
/// followed a moment ago — on this machine or another of this person's —
/// arrive without a reconnect.
///
/// Only widens, like `widen_subtree`: an unfollow leaves the epic asked for
/// until the next connection starts a fresh walk (`SubtreeCover`'s "only
/// grows").
fn follow_epic(ctx: &bindings::EventContext, subtree: &Mutex<Subtree>, epic: i64) {
    let mut subtree = subtree.lock().unwrap_or_else(|e| e.into_inner());
    let queries = follow_queries(&mut subtree, epic, &known_epics(ctx));
    subscribe_widening(ctx, &mut subtree, queries, "a followed epic");
}

/// The queries following `epic` adds to the ask: the epic itself and its
/// tree, or none when it is already covered. The decision of [`follow_epic`],
/// apart from the SDK so it runs without a connection.
pub(super) fn follow_queries(
    subtree: &mut Subtree,
    epic: i64,
    known: &[(i64, i64)],
) -> Vec<String> {
    if subtree.cover.covers(epic) {
        return Vec::new();
    }
    let newly = subtree.cover.follow(epic, known);
    if newly.is_empty() {
        return Vec::new();
    }
    let mut queries = vec![format!("SELECT * FROM epics WHERE id = {epic}")];
    queries.extend(newly.into_iter().flat_map(subtree_queries));
    queries
}
