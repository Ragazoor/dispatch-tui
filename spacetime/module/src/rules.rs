//! The data rules the board used to enforce, as pure functions the reducers call.
//!
//! Spec: `docs/specs/store-split.allium` (step M2, task #53255); the rules
//! themselves are in `mcp-task-tools.allium` (`UpdateTaskViaMcp`) and
//! `epics.allium` (`UpdateEpicViaMcp`, `ReparentEpicFromTui`, `RegroupEpic`).
//!
//! Pure on purpose, for the reason `derive_epic_status` is: the reducers supply
//! WHICH rows are visible, and what a rule decides over them is testable
//! without a store. The in-memory reducer caller (`src/sync/memory_caller/`)
//! calls the same functions, so the two cannot disagree about a verdict.
//!
//! **Every refusal's wording is part of the contract.** These strings reach MCP
//! callers as invalid-params errors, and are the ones the services produced
//! before the rules moved here.

use super::*;

/// The version of the data rules this module enforces and a board expects it to.
///
/// **Hand-bumped, in the commit that moves a rule into the module, and by
/// nothing else.** A board compares it for equality with the number it was
/// built with, at every connection attempt, and refuses to run on a difference
/// (`sync.allium: AStoreWithOtherRulesFailsTheAttempt`). That is the only
/// defence against a board on an old build applying old rules to a module that
/// moved on, or the reverse. Not bumped for a new column, a UI change or a bug
/// fix inside a rule: none of those changes who enforces what.
///
/// 1: store-split M2 — sub-status legality, the reparent cycle, repo-group
///    reparent and append-only checks, and repo grouping.
/// 2: store-split M3 — feed routing and reconcile (`ingest_feed`).
/// 3: store-split M4 — dispatch as request rows with a scheduled backstop
///    timer (`dispatch_request.rs`).
/// 4: store-split M5 — wrap-up and exit-session as request rows, the close and
///    the epic chain in the module (`session_request.rs`).
/// 5: store-split M6 — PR state derived from the host's raw `gh` status
///    (`pr_status.rs`).
/// 6: store-split M7 — `close_task` (the done-path close and its `close` row),
///    patch_task's stop_pending clear and phoenix respawn, and
///    `ensure_managed_epics`.
///
/// 7: agents A1 — agent rows and the name ledger, started by the dispatch
///    claim, mirrored and ended by task writes, deleted with their task
///    (`agent.rs`).
/// 8: agents A4 — the live-session facts (window, hook timestamps, subagent
///    count, deferred Stop) are the current agent's, written by the hook
///    reducers, the claim, the close and `patch_task`; the Task columns are
///    never written (`agent.rs`, `agent_state.rs`).
/// 9: agents A5 — `write_task` no longer assigns or checks a task alias, and
///    `backfill_task_aliases` and the `alias` patch field are gone.
/// 10: new-task signal — `unseen_tasks`, `create_task_signalled` and
///    `mark_epic_seen`; task moves, completion and deletion clear the marks
///    (`unseen.rs`).
/// 11: scheduled tasks — `schedule`/`next_due`/`schedule_error`, armed by
///    `create_task` and `patch_task`, carried by the phoenix respawn, consumed
///    by a dispatched report and marked by a failed schedule-origin request;
///    the `schedule` dispatch origin and the epic chain's skip of a waiting
///    task (`schedule.rs`).
/// 12: repo paths carry the host that saved them — `save_repo_path` takes the
///    saving host and stamps it on both arms, so `prune-repo-paths` can leave
///    other hosts' rows alone. A reducer argument an old board does not send.
/// 13: work packages — the title, package, item and link/unlink-overlap checks
///    moved out of the service into `create_work_package_with_links` and
///    `update_work_package_with_links`, one transaction each (`packages.rs`).
///
/// Not a bump: eb73e7eb, which made `write_task` check sub-status legality on
/// every write path and tightened `patch_task`, `close_task` and the epic caps.
/// Each of those rules already lived in the module; the commit closed write
/// paths that skipped them and moved nothing out of the board.
///
/// `0` is what a module that predates the gate reads as, so this is never 0.
pub const RULES_VERSION: i64 = 13;

/// Whether `sub_status` is legal for `status`, in the store's spelling.
///
/// `mcp-task-tools.allium: UpdateTaskViaMcp`. The one home of the rule: the
/// board keeps no copy, and its model tests read its `SubStatus` spellings
/// through [`validate_sub_status`].
fn sub_status_is_legal(status: &str, sub_status: &str) -> bool {
    match status {
        "backlog" | "done" => sub_status == "none",
        "running" => matches!(
            sub_status,
            "active" | "needs_input" | "stale" | "crashed" | "conflict"
        ),
        "review" => matches!(
            sub_status,
            "awaiting_review"
                | "changes_requested"
                | "approved"
                | "conflict"
                | "pr_closed"
                | "pr_unreachable"
        ),
        _ => false,
    }
}

/// Refuse a `sub_status` that is not legal for `status`.
///
/// `status` is the status the row WILL hold: the patch's own when it moves the
/// status, else the row's current one.
pub fn validate_sub_status(status: &str, sub_status: &str) -> Result<(), String> {
    if sub_status_is_legal(status, sub_status) {
        Ok(())
    } else {
        Err(format!(
            "sub_status '{sub_status}' is not valid for status '{status}'"
        ))
    }
}

/// How far up an ancestor chain the cycle walk goes before giving up.
///
/// A chain this deep is already a loop or a corrupt tree, and the walk must
/// terminate on one that got in some other way (a restore, an older board).
const MAX_ANCESTOR_WALK: usize = 1024;

/// Refuse a parent that would put `epic_id` somewhere inside its own ancestry.
///
/// `parent_of` answers an epic's `parent_epic_id` as stored (`0` for a root),
/// `None` for an unknown epic.
/// Walks `proposed_parent`'s ancestors; the epic appearing among them (or being
/// the proposed parent itself) is a cycle.
pub fn validate_epic_parent(
    epic_id: i64,
    proposed_parent: i64,
    parent_of: impl Fn(i64) -> Option<i64>,
) -> Result<(), String> {
    const CYCLE: &str = "Setting this parent would create a cycle in the epic hierarchy";
    if proposed_parent == epic_id {
        return Err(CYCLE.into());
    }
    let mut current = parent_of(proposed_parent);
    for _ in 0..MAX_ANCESTOR_WALK {
        match current {
            None | Some(0) => return Ok(()),
            Some(id) if id == epic_id => return Err(CYCLE.into()),
            Some(id) => current = parent_of(id),
        }
    }
    // The hop cap is spent: the chain above the proposed parent is already a
    // loop or corrupt, and a move under it is not one to wave through.
    Err(CYCLE.into())
}

/// Whether a node of `root`'s subtree sits deeper than `max` levels below it.
///
/// `children_of` answers an epic's direct sub-epics. A subtree this deep is
/// refused by the delete's validation pass: the recursive delete stops at
/// `max`, and stopping silently would strand the rows below it. Visited nodes
/// are not walked twice, so a loop terminates.
pub fn subtree_exceeds_depth(root: i64, children_of: impl Fn(i64) -> Vec<i64>, max: usize) -> bool {
    let mut seen = std::collections::HashSet::from([root]);
    let mut level = vec![root];
    for _ in 0..=max {
        level = level
            .into_iter()
            .flat_map(&children_of)
            .filter(|id| seen.insert(*id))
            .collect();
        if level.is_empty() {
            return false;
        }
    }
    true
}

/// Refuse `feed_append_only` together with grouping or a feed role.
///
/// All three arguments are the values the epic will hold AFTER the patch, so
/// the pair is refused whichever flag arrives second.
/// `epics.allium: UpdateEpicViaMcp`, `feeds.allium: AppendOnlyFeed`.
pub fn validate_feed_append_only(
    append_only: bool,
    group_by_repo: bool,
    feed_role: &str,
) -> Result<(), String> {
    if append_only && (group_by_repo || feed_role != "none") {
        return Err(
            "feed_append_only cannot be combined with group_by_repo or a feed role: \
                    grouping keys on the repo an item belongs to, but an append-only feed's \
                    items are events keyed by where in the code they fired, and the one URL \
                    they carry is a configured repo root — so every item would land in the \
                    same sub-epic. To cover several repos, use one flat append-only epic per \
                    repo under a common parent."
                .into(),
        );
    }
    Ok(())
}

/// The refusal for moving an auto-created repo-group sub-epic.
pub const REPO_GROUP_REPARENT_REFUSED: &str = "Cannot reparent an auto-created repo-group sub-epic";

/// The `origin` of an auto-created per-repo sub-epic.
pub const REPO_GROUP_ORIGIN: &str = "repo-group";

/// Whether an epic takes part in MANUAL repo grouping: grouped and not a feed.
/// Feed epics group through ingestion instead.
pub fn can_auto_group(group_by_repo: bool, feed_command: &str) -> bool {
    group_by_repo && feed_command.is_empty()
}

/// What an epic's tasks with no usable repo path are grouped under.
pub const UNKNOWN_REPO_GROUP: &str = "other";

/// The grouping key for a repo path: its final component.
///
/// The twin of `models::repo_name_from_path`, which reads `Path::file_name`;
/// a test in the board's crate compares the two. Like `Path`, a `.` component
/// is skipped (`/x/dispatch/.` is `dispatch`) and a final `..` names nothing.
pub fn repo_name_from_path(repo_path: &str) -> String {
    repo_path
        .rsplit('/')
        .find(|s| !s.is_empty() && *s != ".")
        .filter(|s| *s != "..")
        .unwrap_or(UNKNOWN_REPO_GROUP)
        .to_string()
}

/// Every check `patch_task` makes over a patch before applying it.
///
/// Read against the row as it is NOW, so the verdict is about the true row and
/// not a client's subscription view. A patch that names no `sub_status` is not
/// checked: a status move that leaves a stale sub-status is the caller's to
/// fix, as it was when the service made this check.
pub fn check_task_patch(row: &Task, patch: &TaskPatch) -> Result<(), String> {
    // The move (`set_task_epic`) owns the owner, the repo-group routing and
    // the epic-exists check, so a patch that could change `epic_id` alone
    // would skip all three. No host patch names it (`encode::task_patch` sends `None`).
    if patch.epic_id.is_some() {
        return Err("epic_id cannot be changed by patch_task; use set_task_epic".into());
    }
    if let Some(sub_status) = &patch.sub_status {
        let status = patch.status.as_deref().unwrap_or(&row.status);
        validate_sub_status(status, sub_status)?;
    }
    Ok(())
}

/// The legality check [`write_task`] runs on every write: a row whose
/// `sub_status` MOVES must hold one that is legal for its status.
///
/// Only a move is judged, for the reason `check_task_patch` gives: a status
/// change that leaves the old sub-status behind is the caller's to fix, and a
/// row already holding an odd pair (a restore, an older board) must stay
/// writable for its other fields. An insert is not judged either: a seed
/// carries whatever the old store held.
pub fn check_sub_status_write(prior: Option<&Task>, row: &Task) -> Result<(), String> {
    match prior {
        Some(prior) if prior.sub_status != row.sub_status => {
            validate_sub_status(&row.status, &row.sub_status)
        }
        _ => Ok(()),
    }
}

/// Every check `patch_epic` makes over a patch before applying it.
///
/// `parent_of` answers an epic's stored `parent_epic_id` for the cycle walk
/// (`0` for a root), `None` for an unknown epic. The append-only pair is checked only when the patch
/// touches one of its three inputs, so an unrelated write to an epic that
/// already holds the combination (a restore, an older board) is not blocked.
pub fn check_epic_patch(
    row: &Epic,
    patch: &EpicPatch,
    parent_of: impl Fn(i64) -> Option<i64>,
) -> Result<(), String> {
    if let Some(parent) = patch.parent_epic_id {
        if row.origin == REPO_GROUP_ORIGIN {
            return Err(REPO_GROUP_REPARENT_REFUSED.into());
        }
        if parent != 0 {
            validate_epic_parent(row.id, parent, parent_of)?;
        }
    }
    if patch.feed_append_only.is_some()
        || patch.group_by_repo.is_some()
        || patch.feed_role.is_some()
    {
        validate_feed_append_only(
            patch.feed_append_only.unwrap_or(row.feed_append_only),
            patch.group_by_repo.unwrap_or(row.group_by_repo),
            patch.feed_role.as_deref().unwrap_or(&row.feed_role),
        )?;
    }
    Ok(())
}

/// The `feed_role` of the epic a PR-review feed is attached to.
pub const ROLE_REVIEWS_PARENT: &str = "reviews-parent";
/// The `feed_role` of the CVE feed's root epic.
pub const ROLE_CVE: &str = "cve";
/// The `feed_role` of the sub-epic for reviews requested of the user.
pub const ROLE_MY_REVIEWS: &str = "my-reviews";
/// The `feed_role` of the sub-epic for reviews requested of the user's team.
pub const ROLE_TEAM_REVIEWS: &str = "team-reviews";
/// The `feed_role` of the sub-epic for bot-authored PRs.
pub const ROLE_BOTS: &str = "bots";

/// Whether a PR item is kept out of the review subtree altogether.
///
/// `feeds.allium: ExcludeFromReviews`. Evaluated before [`route_signals`], so
/// an excluded item is never routed, inserted or kept. Signals are in the
/// store's spelling (kebab-case, as `Signal` serialises).
///
/// 1. `author-me` excludes unconditionally.
/// 2. `approved` excludes unless `direct-request` or `team-request` rescues it.
pub fn excluded_from_reviews(signals: &[String]) -> bool {
    let has = |s: &str| signals.iter().any(|x| x == s);
    if has("author-me") {
        return true;
    }
    has("approved") && !has("direct-request") && !has("team-request")
}

/// The role a PR's signals route it to: one of [`ROLE_MY_REVIEWS`],
/// [`ROLE_TEAM_REVIEWS`], [`ROLE_BOTS`]. Total, first match wins.
///
/// `feeds.allium: RouteSignals`. Engagement (a review or comment that is not
/// on the user's own PR) beats the bot rule; `org-review` counts as a direct
/// request; `approved` is not read here.
pub fn route_signals(signals: &[String]) -> &'static str {
    let has = |s: &str| signals.iter().any(|x| x == s);
    let engaged = (has("reviewed") || has("commented")) && !has("author-me");
    if engaged {
        ROLE_MY_REVIEWS
    } else if has("author-bot") {
        ROLE_BOTS
    } else if has("direct-request") || has("org-review") {
        ROLE_MY_REVIEWS
    } else if has("team-request") {
        ROLE_TEAM_REVIEWS
    } else {
        ROLE_MY_REVIEWS
    }
}

/// `"org/repo"` from a GitHub URL, or `None` for anything else.
///
/// The twin of `models::extract_github_repo`; a test in the board's crate
/// compares the two.
pub fn extract_github_repo(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://github.com/")?;
    let rest = rest.trim_end_matches('/');
    let slash = rest.find('/')?;
    let after_org = &rest[slash + 1..];
    if after_org.is_empty() {
        return None;
    }
    let end = after_org.find('/').unwrap_or(after_org.len());
    if after_org[..end].is_empty() {
        return None;
    }
    Some(&rest[..slash + 1 + end])
}

/// The grouping key for a feed item's URL: the repo name, else
/// [`UNKNOWN_REPO_GROUP`]. The twin of `models::repo_name_from_url`.
pub fn repo_name_from_url(url: &str) -> String {
    extract_github_repo(url)
        .and_then(|s| s.split('/').next_back())
        .unwrap_or(UNKNOWN_REPO_GROUP)
        .to_string()
}
