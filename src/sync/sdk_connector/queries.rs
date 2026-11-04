//! The SQL this board subscribes with.

use crate::sync::SubscriptionRequest;
use anyhow::anyhow;

/// The SQL this board asks the store for.
///
/// Spec: `sync.allium`'s `SubscribeOnceIdentityIsSettled`. Every table that
/// carries a person's work is private (`workspaces.allium`: "THE PER-CALLER
/// VIEWS"), so the board subscribes to the per-caller views over them, which
/// answer for the connection's own identity. A view ask needs no `WHERE` on
/// the identity: the store applies it. What the store walks for the board, the
/// board no longer asks for epic by epic: `covered_epics` is the followed
/// epics and every descendant at any depth, and a follow or an unfollow
/// changes it live.
///
/// Beside the views sit the tables that stay public because they carry
/// nothing a card draws from: the host registry, the directory of people
/// and the shared repo lists.
///
/// **A table absent from this list renders empty.** The board keeps no second
/// copy, so an unasked-for table does not degrade to stale data; it degrades to
/// no data, which on screen is indistinguishable from having none. That is why
/// the set is enumerated here rather than grown as each view is noticed.
///
/// **The identity is validated, not escaped.** It comes from the store as hex
/// and nothing else is a valid identity, so anything outside that alphabet is
/// refused rather than quoted. A quoting rule is a thing to get subtly wrong;
/// a closed alphabet is not.
pub(in crate::sync) fn subscription_queries(
    request: &SubscriptionRequest,
) -> anyhow::Result<Vec<String>> {
    let owner = &request.owner_board;
    if owner.is_empty() || !owner.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "a user identity must be non-empty hexadecimal, got {owner:?}"
        ));
    }

    let host = &request.host;
    // A minted host id is a UUID (`uuid::Uuid::new_v4().to_string()`), not
    // hex-only like the store identity above — hyphens are part of the
    // alphabet here. Still a closed alphabet, so still validated rather than
    // escaped, for the same reason `owner` is.
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(anyhow!(
            "a host id must be non-empty and alphanumeric, got {host:?}"
        ));
    }

    Ok(vec![
        // Public tables.
        //
        // The host registry. Unfiltered on purpose: a task carrying a `host`
        // needs that host to resolve to something, and which machines those
        // are is not knowable in advance.
        "SELECT * FROM hosts".to_string(),
        // The directory of people (`workspaces.allium: UserProfile`, D7).
        // Unfiltered: admins find people by name, and a card names its creator.
        "SELECT * FROM user_profiles".to_string(),
        // The repo lists, unfiltered and deliberately so: a path and a branch
        // name describe the work rather than the person.
        "SELECT * FROM repo_paths".to_string(),
        "SELECT * FROM repo_base_branches".to_string(),
        // The per-caller views.
        //
        // This person's own follows of epics they can still see, so a change
        // made on another of their machines arrives here. `covered_epics`
        // reads the same rows on the store's side, so a follow widens what the
        // views deliver.
        "SELECT * FROM own_subscriptions".to_string(),
        //
        // What the caller created, and the epic-less tasks they own
        // (own_creations), and the epics they follow with every descendant
        // (covered_epics) and those epics' tasks. The two asks overlap on
        // purpose: a row in both is held until it has left both
        // (`AViewUpdateNeverErasesTheRow`).
        "SELECT * FROM own_tasks".to_string(),
        "SELECT * FROM own_epics".to_string(),
        "SELECT * FROM covered_tasks".to_string(),
        "SELECT * FROM covered_epics".to_string(),
        // Work packages are one person's grouping of their own work, and
        // their links are served while both ends are visible to the caller.
        "SELECT * FROM own_packages".to_string(),
        "SELECT * FROM visible_work_package_links".to_string(),
        // Poll ownership claims follow their scope: a claim on an epic or
        // task the caller cannot see is not served.
        "SELECT * FROM visible_poll_owners".to_string(),
        // Workspaces and their memberships, the caller's own.
        "SELECT * FROM visible_workspaces".to_string(),
        "SELECT * FROM visible_memberships".to_string(),
        // Private items nobody can see (`admin-surfaces.allium:
        // UnseenItemListing`): empty unless this person is a super-admin. Ids
        // and creators only, never a title.
        "SELECT * FROM unseen_items".to_string(),
        // This person's own unseen marks (sync.allium: own_unseen). Never
        // another person's.
        "SELECT * FROM own_unseen_tasks".to_string(),
        // This person's own bookmarks (core.allium: Bookmark). Never another
        // person's.
        "SELECT * FROM own_bookmarks".to_string(),
        // Settings (`docs/specs/settings.allium`). The view serves the
        // settings of every host this person owns; a setting is this
        // machine's own, so the ask narrows it by host.
        format!("SELECT * FROM own_settings WHERE host = '{host}'"),
        // The knowledge base (`docs/specs/learnings.allium`): the learnings
        // the caller may see, and the retrievals of the tasks they hold.
        "SELECT * FROM visible_learnings".to_string(),
        "SELECT * FROM visible_learning_retrievals".to_string(),
        // Usage telemetry is NOT subscribed: a view over it would re-run on
        // every recorded event. `query_usage` asks the store on demand
        // (`own_usage_summary`).
        // Dispatch and session requests (store-split M4, M5). Scoped by HOST
        // as well: the view serves every request on a held task, and a request
        // is provisioned only by the machine that made it. Unfiltered by state
        // on purpose: a row leaving a state-filtered query would arrive as a
        // delete, and its outcome would be lost.
        format!("SELECT * FROM visible_dispatch_requests WHERE host = '{host}'"),
        format!("SELECT * FROM visible_session_requests WHERE host = '{host}'"),
        // Feed requests (refresh and teardown): the rows on this host. The
        // view serves every host this person owns; the ask narrows it.
        format!("SELECT * FROM visible_feed_requests WHERE host = '{host}'"),
        // Agents (agents A1): the rows of the tasks the caller holds.
        "SELECT * FROM visible_agents".to_string(),
    ])
}
