//! Row callbacks: pointing the SDK's table events at [`SharedRows`], and the
//! sub-epic walk that widens the subscription as epics arrive.

use spacetimedb_sdk::{DbContext, Table as _, TableWithPrimaryKey as _};
use std::sync::{Arc, Mutex};

use super::subtree_queries;
use super::SpacetimeSdkConnector;
use crate::spacetime::bindings;
use crate::spacetime::bindings::{
    DbConnection, EpicsTableAccess as _, HostsTableAccess as _, LearningRetrievalsTableAccess as _,
    LearningsTableAccess as _, PollOwnersTableAccess as _, RepoBaseBranchesTableAccess as _,
    RepoPathsTableAccess as _, RetiredFeedItemsTableAccess as _, SettingsTableAccess as _,
    SubscriptionHandle, SubscriptionsTableAccess as _, TaskWatchersTableAccess as _,
    TasksTableAccess as _, UsageEventsTableAccess as _,
};
use crate::sync::subtree::SubtreeCover;

impl SpacetimeSdkConnector {
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
    /// [`SharedRows`] is keyed by id and a row's id cannot change.
    pub(super) fn wire_rows(&self, connection: &DbConnection) {
        let db = connection.db();
        self.wire_tables(db);
        self.wire_subtree_walk(db);
    }

    /// Every table's row callbacks. See [`Self::wire_rows`].
    pub(super) fn wire_tables(&self, db: &bindings::RemoteTables) {
        // One table's three callbacks, so "every table gets all three" is
        // structural rather than something a reader verifies by counting.
        //
        // An update is an UPSERT, not a patch: the SDK hands over the old row
        // and the new one, and only the new one is kept — the rows are keyed by
        // id and a row's id cannot change.
        macro_rules! wire {
            ($table:ident, $upsert:ident, $remove:ident, $key:expr) => {{
                let rows = self.rows.clone();
                db.$table().on_insert(move |_, row| rows.$upsert(row));
                let rows = self.rows.clone();
                db.$table().on_update(move |_, _old, new| rows.$upsert(new));
                let rows = self.rows.clone();
                let key = $key;
                db.$table().on_delete(move |_, row| rows.$remove(key(row)));
            }};
        }

        wire!(tasks, upsert_task, remove_task, |row: &bindings::Task| {
            crate::models::TaskId(row.id)
        });
        wire!(epics, upsert_epic, remove_epic, |row: &bindings::Epic| {
            crate::models::EpicId(row.id)
        });
        wire!(
            repo_paths,
            upsert_repo_path,
            remove_repo_path,
            |row: &bindings::RepoPath| row.id
        );
        wire!(
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

        wire!(
            poll_owners,
            upsert_poll_owner,
            remove_poll_owner,
            |row: &bindings::PollOwner| row.id
        );
        wire!(
            learnings,
            upsert_learning,
            remove_learning,
            |row: &bindings::Learning| crate::models::LearningId(row.id)
        );
        wire!(
            learning_retrievals,
            upsert_learning_retrieval,
            remove_learning_retrieval,
            |row: &bindings::LearningRetrieval| row.id
        );
        wire!(
            usage_events,
            upsert_usage_event,
            remove_usage_event,
            |row: &bindings::UsageEvent| row.id
        );
        wire!(
            retired_feed_items,
            upsert_retired_feed_item,
            remove_retired_feed_item,
            |row: &bindings::RetiredFeedItem| row.id
        );

        wire!(
            task_watchers,
            upsert_task_watcher,
            remove_task_watcher,
            |row: &bindings::TaskWatcher| row.id
        );
        wire!(
            subscriptions,
            upsert_subscription,
            remove_subscription,
            |row: &bindings::Subscription| row.id.clone()
        );
        wire!(
            settings,
            upsert_setting,
            remove_setting,
            |row: &bindings::Setting| { row.id.clone() }
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
            if old.parent_epic_id != new.parent_epic_id {
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
    // Checked before the cache is gathered: nearly every arrival — the whole
    // initial load included — sits under an uncovered parent or none.
    if !subtree.cover.covers(row.parent_epic_id) {
        return;
    }
    let newly = subtree
        .cover
        .delivered(row.id, row.parent_epic_id, &known_epics(ctx));
    let queries: Vec<String> = newly.into_iter().flat_map(subtree_queries).collect();
    subscribe_widening(ctx, &mut subtree, queries, "a sub-epic");
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
    if subtree.cover.covers(epic) {
        return;
    }
    let newly = subtree.cover.follow(epic, &known_epics(ctx));
    if newly.is_empty() {
        return;
    }
    let mut queries = vec![format!("SELECT * FROM epics WHERE id = {epic}")];
    queries.extend(newly.into_iter().flat_map(subtree_queries));
    subscribe_widening(ctx, &mut subtree, queries, "a followed epic");
}
