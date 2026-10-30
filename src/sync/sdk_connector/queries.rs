//! The SQL this board subscribes with.

use crate::sync::SubscriptionRequest;
use anyhow::anyhow;

/// The SQL this board asks the store for.
///
/// Three things vary, as `sync.allium`'s `SubscribeOnceIdentityIsSettled`
/// requires: this person's own user board, everything they created themselves
/// (own_creations), and the epics they follow. Beside them sit the tables that
/// have no per-person or per-epic dimension at all — the host registry, this
/// person's own subscription rows, their checklist, and the shared repo lists.
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

    let mut queries = vec![
        // The host registry. Unfiltered on purpose: a task carrying a `host`
        // needs that host to resolve to something, and which machines those
        // are is not knowable in advance.
        "SELECT * FROM hosts".to_string(),
        // Poll ownership claims. Unfiltered for the same reason `hosts` is:
        // every host's tick needs to know who owns EVERY scope, not only the
        // ones it happens to already hold, to tell "unclaimed" from "someone
        // else's" (`core.allium: PollOwner`).
        "SELECT * FROM poll_owners".to_string(),
        // This person's own subscription rows, so a change made on another of
        // their machines arrives here.
        format!("SELECT * FROM subscriptions WHERE subscriber = '{owner}'"),
        // This person's own unseen marks (sync.allium: own_unseen). Never
        // another person's.
        format!("SELECT * FROM unseen_tasks WHERE subscriber = '{owner}'"),
        // The directory of people (`workspaces.allium: UserProfile`, D7).
        // Unfiltered: admins find people by name, and a card names its creator.
        "SELECT * FROM user_profiles".to_string(),
        // Workspaces and their memberships (`workspaces.allium`). Unfiltered
        // until the read-enforcement task replaces them with views.
        "SELECT * FROM workspaces".to_string(),
        "SELECT * FROM memberships".to_string(),
        // Private items nobody can see (`admin-surfaces.allium:
        // UnseenItemListing`): a per-caller view, empty unless this person is
        // a super-admin. Ids and creators only, never a title.
        "SELECT * FROM unseen_items".to_string(),
        // The user board: epic-less tasks this person owns.
        format!("SELECT * FROM tasks WHERE owner = '{owner}'"),
        // own_creations (sync.allium: SubscribeOnceIdentityIsSettled):
        // unconditional, unlike the per-epic asks below — a task or epic this
        // person created is in the subscription cache the moment a create
        // answers, regardless of which epic it landed in or whether anyone
        // follows it yet. This is what makes the reducer-completion read-back
        // in `generated_id` (below) actually work for an epic and for a task
        // in an unfollowed epic, neither of which `owner`/`epic_id` cover.
        format!("SELECT * FROM tasks WHERE created_by = '{owner}'"),
        format!("SELECT * FROM epics WHERE created_by = '{owner}'"),
        // The repo lists, unfiltered and deliberately so. Neither table has an
        // owner to filter on and neither wants one: a path and a branch name
        // describe the work rather than the person, and a colleague adding a
        // repo is a colleague saying where the code lives. Nothing private
        // travels in them, so the containment claim above is untouched.
        "SELECT * FROM repo_paths".to_string(),
        "SELECT * FROM repo_base_branches".to_string(),
        // Settings (`docs/specs/settings.allium`). Scoped by HOST, not by
        // owner: a setting is this machine's own, and
        // host is known even before an identity settles, unlike everything
        // above that filters by `owner`.
        format!("SELECT * FROM settings WHERE host = '{host}'"),
        // The knowledge base (`docs/specs/learnings.allium`'s Storage Backend
        // section). Unfiltered, like `repo_paths` above and for the same
        // reason: a learning's visibility is governed entirely by its own
        // scope/scope_ref, not by who created it or which machine is asking.
        "SELECT * FROM learnings".to_string(),
        "SELECT * FROM learning_retrievals".to_string(),
        // Usage telemetry (Phase 11, task #4915). Unfiltered, like the
        // knowledge base above: append-only with no user-observable rule
        // beyond "recorded", and nothing scopes it by owner or host.
        "SELECT * FROM usage_events".to_string(),
        // Retired feed items (`core.allium: RetiredFeedItem`, task #4971).
        // Unfiltered: every host on the board must refuse the same ids, or a
        // second host's feed cycle would re-insert what the first one's human
        // deleted — see the module's own doc comment on `RetiredFeedItem`.
        "SELECT * FROM retired_feed_items".to_string(),
        // Work packages (`docs/specs/work-packages.allium`). A package is one
        // person's grouping of their own work, so it is scoped to its creator,
        // the way `own_creations` scopes epics above.
        format!("SELECT * FROM work_packages WHERE created_by = '{owner}'"),
        // Their links, unfiltered, like the repo lists: a row is two ids and
        // a kind, and the store's SQL cannot join it to the packages above to
        // narrow it. A link whose package or item this board does not hold
        // draws nothing (`work-packages.allium: PackageViewCards`).
        "SELECT * FROM work_package_links".to_string(),
        // Dispatch requests (store-split M4, `sync.allium:
        // SubscribeToThisHostsDispatchRequests`). Scoped by HOST, like
        // settings: a request is always provisioned by the machine that made
        // it, so this delivers both the work this host must provision and the
        // outcome of every request it made. Unfiltered by state on purpose: a
        // row leaving a state-filtered query would arrive as a delete, and its
        // outcome would be lost.
        format!("SELECT * FROM dispatch_requests WHERE host = '{host}'"),
        // Session requests (store-split M5, `sync.allium:
        // SubscribeToThisHostsSessionRequests`). Scoped by HOST like the
        // dispatch requests: a wrap-up or exit-session request names the
        // machine holding the task's worktree and window, so this delivers the
        // rebase and teardown this host must do, the outcome `wrap_up` waits
        // on, and the done wrap_up rows `exit_session`'s no-answer settling
        // reads. Unfiltered by state for the same reason. The module's
        // `session_request_timeouts` schedule table is its own, never asked
        // for.
        format!("SELECT * FROM session_requests WHERE host = '{host}'"),
        // Agents (agents A1, `sync.allium: SubscribeToAgents`). Unfiltered,
        // like the work package links: the store's SQL cannot join an agent
        // to the tasks this board holds, and a row carries a task id, a name,
        // a host and a window, never the task's text. A row whose task this
        // board does not hold draws nothing. The name ledger is not asked
        // for: only the module's picker reads it.
        "SELECT * FROM agents".to_string(),
    ];

    // The epic ids are integers by type, so they need no validation beyond
    // being integers. Each followed epic brings its own row plus its subtree
    // asks; everything deeper arrives as the connector widens the
    // subscription (`SubtreeCover`).
    for epic in &request.epics {
        queries.push(format!("SELECT * FROM epics WHERE id = {epic}"));
        queries.extend(subtree_queries(*epic));
    }

    Ok(queries)
}

/// What a covered epic is asked for beyond its own row: its direct tasks and
/// its direct sub-epics.
///
/// Spec: `sync.allium`'s `ASubEpicOfAFollowedEpicIsAskedForToo`. The second
/// query is what walks the tree — the store's SQL cannot follow a parent
/// chain, so each level is asked for by the one above it, and each sub-epic
/// that arrives is asked for in turn by [`SubtreeCover`](crate::sync::subtree::SubtreeCover). The sub-epic's own
/// row needs no query of its own: its parent's sub-epics ask already covers it.
pub(in crate::sync) fn subtree_queries(epic: i64) -> Vec<String> {
    vec![
        format!("SELECT * FROM tasks WHERE epic_id = {epic}"),
        format!("SELECT * FROM epics WHERE parent_epic_id = {epic}"),
    ]
}
