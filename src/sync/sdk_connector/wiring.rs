//! Row callbacks: pointing the SDK's table events at [`SharedRows`](crate::sync::SharedRows).

use spacetimedb_sdk::{DbContext, Table as _, TableWithPrimaryKey as _};

use super::SpacetimeSdkConnector;
use crate::spacetime::bindings;
use crate::spacetime::bindings::{
    CoveredEpicsTableAccess as _, CoveredTasksTableAccess as _, DbConnection,
    HostsTableAccess as _, OwnEpicsTableAccess as _, OwnPackagesTableAccess as _,
    OwnSettingsTableAccess as _, OwnTasksTableAccess as _, OwnUnseenTasksTableAccess as _,
    RepoBaseBranchesTableAccess as _, RepoPathsTableAccess as _, SubscriptionsTableAccess as _,
    UnseenItemsTableAccess as _, UserProfilesTableAccess as _, VisibleAgentsTableAccess as _,
    VisibleDispatchRequestsTableAccess as _, VisibleLearningRetrievalsTableAccess as _,
    VisibleLearningsTableAccess as _, VisibleMembershipsTableAccess as _,
    VisiblePollOwnersTableAccess as _, VisibleSessionRequestsTableAccess as _,
    VisibleWorkPackageLinksTableAccess as _, VisibleWorkspacesTableAccess as _,
};

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

// One view table's two callbacks. A view's rows have no primary key in the
// bindings, so there is no `on_update`: an update is INSERT(new) then
// DELETE(old), and the delete goes to a retire method that removes the row
// only while it is still the one held (`AViewUpdateNeverErasesTheRow`).
macro_rules! wire_view {
    ($wiring:expr, $db:expr, $table:ident, $upsert:ident, $retire:ident) => {{
        let rows = $wiring.rows.clone();
        $db.$table().on_insert(move |_, row| rows.$upsert(row));
        let rows = $wiring.rows.clone();
        $db.$table().on_delete(move |_, row| rows.$retire(row));
    }};
}

// Two views that can both serve one row (own and covered tasks, or epics):
// [`wire_view!`] for each, except that a delete from one retires the row only
// when the other view's cache no longer holds its id, so a row stays on the
// board until it has left both (`AViewUpdateNeverErasesTheRow`).
macro_rules! wire_paired_views {
    ($wiring:expr, $db:expr, $a:ident, $b:ident, $upsert:ident, $retire:ident) => {{
        wire_paired_views!(@one $wiring, $db, $a, $b, $upsert, $retire);
        wire_paired_views!(@one $wiring, $db, $b, $a, $upsert, $retire);
    }};
    (@one $wiring:expr, $db:expr, $this:ident, $other:ident, $upsert:ident, $retire:ident) => {{
        let rows = $wiring.rows.clone();
        $db.$this().on_insert(move |_, row| rows.$upsert(row));
        let rows = $wiring.rows.clone();
        $db.$this().on_delete(move |ctx, row| {
            if !ctx.db.$other().iter().any(|held| held.id == row.id) {
                rows.$retire(row);
            }
        });
    }};
}

impl SpacetimeSdkConnector {
    /// Point every subscribed table at [`Self::rows`].
    ///
    /// Registered once per connection, right after it is installed and BEFORE
    /// anything is subscribed. The order matters: a subscription applied first
    /// delivers its initial rows through these same callbacks, and callbacks
    /// registered afterwards would miss every row that was already there —
    /// producing a board that is empty until somebody else edits something.
    ///
    /// **An update is an upsert, not a patch** on a public table: the SDK hands
    /// over the old row and the new one, and only the new one is kept. A view
    /// table has no primary key, so it has no update: see [`wire_view!`].
    pub(super) fn wire_rows(&self, connection: &DbConnection) {
        let db = connection.db();
        self.wire_tables(db);
    }

    /// Every table's row callbacks. See [`Self::wire_rows`].
    pub(super) fn wire_tables(&self, db: &bindings::RemoteTables) {
        self.wire_public_tables(db);
        self.wire_board_views(db);
        self.wire_shared_views(db);
    }

    /// Row callbacks for the tables that stay public.
    fn wire_public_tables(&self, db: &bindings::RemoteTables) {
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
            user_profiles,
            upsert_user_profile,
            remove_user_profile,
            |row: &bindings::UserProfile| row.identity.clone()
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

    /// Row callbacks for the views the board itself shows. A task or epic
    /// comes through two views, what the caller created and what they follow,
    /// so a row leaving one is kept while the other still serves it.
    fn wire_board_views(&self, db: &bindings::RemoteTables) {
        wire_paired_views!(self, db, own_tasks, covered_tasks, upsert_task, retire_task);
        wire_paired_views!(self, db, own_epics, covered_epics, upsert_epic, retire_epic);
        wire_view!(
            self,
            db,
            own_packages,
            upsert_work_package,
            retire_work_package
        );
        wire_view!(
            self,
            db,
            visible_work_package_links,
            upsert_work_package_link,
            retire_work_package_link
        );
        wire_view!(self, db, visible_agents, upsert_agent, retire_agent);
    }

    /// Row callbacks for the remaining views.
    fn wire_shared_views(&self, db: &bindings::RemoteTables) {
        wire_view!(
            self,
            db,
            visible_poll_owners,
            upsert_poll_owner,
            retire_poll_owner
        );
        wire_view!(
            self,
            db,
            visible_learnings,
            upsert_learning,
            retire_learning
        );
        wire_view!(
            self,
            db,
            visible_learning_retrievals,
            upsert_learning_retrieval,
            retire_learning_retrieval
        );
        wire_view!(
            self,
            db,
            own_unseen_tasks,
            upsert_unseen_task,
            retire_unseen_task
        );
        wire_view!(
            self,
            db,
            visible_workspaces,
            upsert_workspace,
            retire_workspace
        );
        wire_view!(
            self,
            db,
            visible_memberships,
            upsert_membership,
            retire_membership
        );
        wire_view!(
            self,
            db,
            unseen_items,
            upsert_unseen_item,
            retire_unseen_item
        );
        wire_view!(self, db, own_settings, upsert_setting, retire_setting);
        wire_view!(
            self,
            db,
            visible_dispatch_requests,
            upsert_dispatch_request,
            retire_dispatch_request
        );
        wire_view!(
            self,
            db,
            visible_session_requests,
            upsert_session_request,
            retire_session_request
        );
    }
}
