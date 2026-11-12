//! The task and epic data rules, as pure functions the reducers call.
//!
//! Spec: the rules are in `mcp-task-tools.allium` (`UpdateTaskViaMcp`) and
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
/// 1: sub-status legality, the reparent cycle, repo-group
///    reparent and append-only checks, and repo grouping.
/// 2: feed routing and reconcile (`ingest_feed`).
/// 3: dispatch as request rows with a scheduled backstop
///    timer (`dispatch_request.rs`).
/// 4: wrap-up and exit-session as request rows, the close and
///    the epic chain in the module (`session_request.rs`).
/// 5: PR state derived from the host's raw `gh` status
///    (`pr_status.rs`).
/// 6: `close_task` (the done-path close and its `close` row),
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
/// 49: a Stop whose payload lists background work (shells, workflows,
///    monitors, MCP tasks) defers like one with live subagents; the subagent
///    drain leaves it deferred; idle_prompt is ignored during the wait; and
///    `expire_background_wait` (the module owns the cap) applies it
///    (`agent-health.allium: HookStop`, `ExpireBackgroundWait`). A reducer
///    argument an old board does not send.
/// 48: the learning task scope is gone — `create_learning` refuses
///    `scope=task` (`validate_learning_scope`) and a store from an older module
///    has its task-scoped learnings archived once (`retire_task_scoped_learnings`).
/// 47: a hand move into backlog or done releases a window the board did not
///    see — the store clears the current agent's window and writes a `close`
///    row naming it, so its host kills it
///    (`store-split.allium: HandMoveReleasesAWindowTheBoardDidNotSee`).
/// 46: every created task gets an `assigned` agent in the same transaction
///    (`agents.allium: AssignAgentOnTaskCreated`, A17), the first dispatch
///    claim launches it under its name, and a name is free again once no
///    agent row holds it (A4; the name ledger is retired). The backfill on
///    the first connection after the publish assigns an agent to every task
///    with none. An old board cannot read the `assigned` state.
/// 45: `set_epic_group_by_repo` — the group-by-repo toggle's flag write and its
///    regroup or flatten in one transaction, so a refused move no longer
///    leaves the flag set (`tasks_epics.rs`; `epics.allium:
///    SetEpicGroupByRepoIsOneTransaction`). A reducer the board now calls.
/// 44: `update_task` carries the caller's bookmark (`subscriber`, `bookmarked`)
///    and writes it in the same transaction as the fields, and `set_bookmark`
///    is refused for anyone but the person it belongs to, the operator
///    included (`bookmark.rs`; `mcp-task-tools.allium: UpdateTaskViaMcp`). A
///    reducer argument an old board does not send.
/// 43: repo-path authority — `save_repo_path`, `adopt_repo_path`,
///    `delete_repo_path` and `set_verify_command` (`config.rs`) are refused
///    unless the caller owns the named host; `set_verify_command` takes the
///    host and acts through that host's row of the path, not the first row
///    by path (`core.allium: SavedRepoPath`). The first connection after it
///    also unifies the verify command of linked rows, once
///    (`REPO_PATH_VERIFY_RULES_VERSION`, `config.rs: unify_repo_verify_commands`).
/// 42: three data rules move into the module. A patch that moves `status` and
///    names no `sub_status` resets it to the new status's default
///    (`apply_task_patch`); a create or patch that would give a parent a second
///    epic with one feed role, or a second repo group with one title, is
///    refused (`check_epic_unique_among`, also on a root's move to another
///    workspace, and provisioning reuses a role sub-epic it cannot see); a repo path that learns its GitHub
///    path joins its repository's verify command, and `set_verify_command`
///    reaches every row of the repository (`resaved_verify_command`,
///    `rows_of_the_repository_at`).
/// 41: loss of sight ends claims and marks together — `on_sight_lost` and
///    `on_task_sight_lost` (`config.rs`) release the poll claims and end the
///    unseen marks and bookmarks of whoever no longer sees a task, on every
///    loss-of-sight path including a task move; a detach is refused while
///    another person's host runs the task, and leaves the workspace for the
///    package links and task learnings (`visibility.rs`, `tasks_epics.rs`;
///    `workspaces.allium: ClaimsFollowSight`, `core.allium: Bookmark`).
/// 40: `create_task` and `create_task_signalled` refuse a row naming a host
///    another person owns (`check_new_task_aim`;
///    `tasks.allium: CreateTaskNeverAimsAnotherHostsTeardown`).
/// 39: a task in an epic is detached onto a board by its creator, an admin of
///    its workspace or a super-admin (`task_detach_permitted`), no longer by
///    nobody; the managed-feed settings are written as one change by
///    `set_managed_feed_settings` (`visibility.rs`, `config.rs`).
/// 38: repo paths are one row per `(path, host)` and carry the repository's
///    GitHub path — `save_repo_path` takes it as a new argument, a verify
///    command is shared by every row of one repository, `delete_repo_path`
///    refuses a path only another host has (`config.rs`;
///    `core.allium: SavedRepoPath`). Board and module move together.
/// 37: poll claims and held tasks — a feed epic's claim held by a host whose
///    owner did not set the command, from before the setter rule, is released
///    once when a store moves onto this version
///    (`release_claims_of_non_setter_hosts`, `lib.rs`); task-scoped claims
///    follow sight like epic-scoped ones (`release_task_poll_claims`); a task
///    with a current agent or an open request on a host the caller owns is
///    served to the caller (`hosted_tasks_of`, `views.rs`).
/// 36: bookmarks — `bookmarks`, `set_bookmark` and the `own_bookmarks` view
///    the board subscribes to; entering Done, on any write path, ends every
///    unseen mark and bookmark on the task (`bookmark.rs`, `tasks_epics.rs`;
///    `core.allium: Bookmark`).
/// 35: feed requests — `feed_requests`, `request_feed_refresh` and the
///    claim/report reducers; `ingest_feed` takes the ingesting host and the
///    stale delete writes a teardown request for a task another host holds
///    (`feed_request.rs`, `feed.rs`; `store-split.allium: FeedRequest`). The
///    board subscribes to the new `visible_feed_requests` view.
/// 34: `update_task` takes the caller's host and writes a windowless `close`
///    row when a move into Done leaves a pr-typed url, so the store, not the
///    board's cache, decides `ClosePrOnDone` for an edit (`tasks_epics.rs`,
///    `session_request.rs`; `store-split.allium:
///    UpdateTaskDoneWritesACloseRow`). A reducer argument an old board does
///    not send.
/// 33: repo-path ownership — `save_repo_path` never changes a non-empty host,
///    `adopt_repo_path` stamps only an unowned row, and `delete_repo_path`
///    takes the host it expects to own the row (`config.rs`;
///    `cli.allium: PruneRepoPathsViaCli`).
/// 32: feed command authority and poll claims — a feed epic records who set
///    its command (`feed_command_set_by`) and is claimed only by a host whose
///    owner did; `claim_poll_owner` and `override_poll_owner` need sight of the
///    scope; the poll-owner arm of the command authority is gone; every
///    loss-of-sight path releases the claims it strands
///    (`agent_state.rs`, `tasks_epics.rs`, `visibility.rs`, `feed.rs`,
///    `config.rs`).
/// 31: usage is served on demand. The `own_usage_events` view is gone and the
///    `own_usage_summary` procedure answers `query_usage` with the caller's
///    grouped events, because a view over usage re-ran over a person's whole
///    history on every recorded event (`views.rs`, `config.rs`).
/// 30: the new-task signal clears per task — `mark_task_seen` replaces
///    `mark_epic_seen`, so the cursor resting on a task card ends that one
///    mark and entering an epic ends none (`unseen.rs`;
///    `tasks.allium: SeeTaskByLandingOnIt`).
/// 29: the module stops trusting a client-named owner — a task created with
///    no epic or detached onto a user board takes the caller as `owner`, a
///    created task is stamped `created_by` with the caller,
///    `unsubscribe_from_epic` refuses a subscriber that is not the caller and
///    hides epics the caller cannot see, and `claim_store_owner` records the
///    publisher of a store that has none (`tasks_epics.rs`, `config.rs`,
///    `visibility.rs`, `lib.rs`).
/// 28: the private flip. Every table a per-caller view serves is private, the
///    board reads them through the views, and `own_usage_events` serves a
///    caller the usage events they recorded (`record_usage_event` stamps an
///    appended, indexed `recorded_by`). A super-admin's
///    `visible_workspaces` lists every workspace, through an always-true
///    indexed `listed` column. A board that subscribes to the views needs a
///    module that has them and refuses a subscription to a table now private
///    (`views.rs`).
/// 27: `update_epic` (patch, workspace move and visibility in one transaction)
///    and `update_work_package_with_links` taking the visibility change, so a
///    refused part leaves nothing applied; `set_task_epic` refuses a missing
///    task instead of answering Ok (`tasks_epics.rs`, `packages.rs`).
/// 26: feed authority and poll claims — `remove_member` and
///    `move_epic_to_workspace` release the poll claims of hosts whose owner
///    loses sight; a `feed_command` change needs the creator, the poll
///    owner's owner or a workspace admin; `archive_stale_learnings` archives
///    only what the caller sees; a managed PR Reviews or CVE root is private
///    to its creator (`config.rs`, `workspace.rs`, `visibility.rs`,
///    `tasks_epics.rs`, `learning.rs`, `feed.rs`).
/// 25: one rule for moving work under or out of a private epic — a re-parent
///    or task move that changes who the item is private to needs the authority
///    `set_epic_visibility` needs; a repo-group epic cannot be private and its
///    origin cannot change; a repo-group relink ends the unseen mark; a task
///    moved across workspaces drops its package links and carries its
///    learnings; unlinking an unseen item does nothing; the super-admin hatch
///    deletes only items whose creator has left (`visibility.rs`,
///    `grouping.rs`, `tasks_epics.rs`, `learning.rs`, `packages.rs`).
/// 24: the module stops trusting a client-named identity — the `seed_*`
///    reducers need the store itself or its publisher (`StoreOwner`, stamped by
///    `init`); `subscribe_to_epic` and `mark_epic_seen` refuse a subscriber
///    that is not the caller; `create_epic` and the work-package creators
///    stamp `created_by` with the caller (`seed.rs`, `visibility.rs`).
/// 23: `move_epic_to_workspace` — a root epic and its tree change workspace
///    (`workspaces.allium: MoveEpicToWorkspace`, `visibility.rs`).
/// 22: the per-caller views (`own_epics`, `covered_epics`, `visible_epics` and
///    the rest, `views.rs`) and btree indexes on the columns they read. A board
///    that subscribes through them needs a module that has them. The tables
///    stay public until the board has moved onto the views.
/// 21: `create_learning` stamps a learning's author and workspace itself, and a
///    learning with no workspace is seen only by its author (`learning.rs`,
///    `visibility.rs`).
/// 20: the `unseen_items` view — the private epics and packages nobody can see,
///    for a super-admin and nobody else — and a btree index on the `visibility`
///    column of epics and work packages. A board that subscribes to the view
///    needs a module that has it (`visibility.rs`).
/// 19: visibility on create and change, and sight on writes — a root epic or
///    package names a workspace the caller belongs to, a sub-epic takes its
///    parent's, `set_epic_visibility`, `set_work_package_visibility` and
///    `super_admin_delete_unseen_item`, and every reducer that touches a task,
///    epic, package or learning treats one the caller cannot see as one that
///    does not exist (`visibility.rs`).
/// 18: `update_task` — the board's relink and patch of a task in one
///    transaction, and the completion stamp on entering Done that its patch
///    cannot override (`tasks_epics.rs`).
/// 17: workspace and visibility columns on epics and work packages, a
///    workspace column on learnings, and `MigrateStoreToWorkspaces` on the first
///    connection after the publish; `delete_workspace` refuses an occupied
///    workspace and `remove_member` drops the person's subscriptions and unseen
///    marks in it (`workspace.rs`).
/// 16: super-admins, workspaces and membership — `claim_super_admin` and the
///    grant/revoke pair, `create_workspace`, `rename_workspace`,
///    `delete_workspace`, `add_member`, `set_member_role` and `remove_member`
///    (`workspace.rs`).
/// 15: user profiles — `user_profiles`, registered on every connect, and
///    `set_display_name`, which trims and bounds a person's own name
///    (`user_profile.rs`).
/// 14: learning verdicts — `apply_learning_verdicts` takes the rating task,
///    refuses a learning that task never retrieved, and keeps one verdict per
///    (task, learning) in `learning_verdicts`, so a repeat moves the score by
///    the difference (`learning.rs`).
/// 13: work packages — the title, package, item and link/unlink-overlap checks
///    moved out of the service into `create_work_package_with_links` and
///    `update_work_package_with_links`, one transaction each (`packages.rs`).
///
/// A change to the board's subscription set warrants a bump: a table going
/// private, a view added or removed, a view the board asks for under a new
/// name. 2e681e34 made `subscriptions` private and moved the board to
/// `own_subscriptions` without one, so a board built in between passed the
/// gate and retried a refused subscription forever, with no message saying
/// to upgrade. (Changing which rows an existing view serves is not a bump.)
///
/// Not a bump: a new Task column the board validates, such as `model`. It
/// widens `create_task`'s and `patch_task`'s argument types, but moves no rule
/// into the module; `alias` (fbbd14c6) set the precedent.
///
/// Not a bump: eb73e7eb, which made `write_task` check sub-status legality on
/// every write path and tightened `patch_task`, `close_task` and the epic caps.
/// Each of those rules already lived in the module; the commit closed write
/// paths that skipped them and moved nothing out of the board.
///
/// `0` is what a module that predates the gate reads as, so this is never 0.
pub const RULES_VERSION: i64 = 49;

/// The rules version that removed the learning task scope: a store stamped
/// with anything older has its task-scoped learnings archived once, on its
/// first connection (`learning.rs: retire_task_scoped_learnings`). Never
/// moves, like the others here.
pub const TASK_SCOPE_REMOVED_RULES_VERSION: i64 = 48;

/// The rules version that added the setter-claims sweep: a store stamped with
/// anything older has its pre-rule feed claims released once, on its first
/// connection (`config.rs: release_claims_of_non_setter_hosts`). Like
/// [`WORKSPACES_RULES_VERSION`] it never moves: a RULES_VERSION bump leaves it.
pub const FEED_SETTER_CLAIMS_RULES_VERSION: i64 = 37;

/// The rules version that made one verify command per repository hold for rows
/// saved before it: a store stamped with anything older has its linked rows
/// unified once, on its first connection (`config.rs:
/// unify_repo_verify_commands`). Never moves, like the two below.
pub const REPO_PATH_VERIFY_RULES_VERSION: i64 = 43;

/// The rules version that added workspaces: a store stamped with anything
/// older is migrated once on its first connection (`workspace.rs:
/// migrate_store_to_workspaces`).
pub const WORKSPACES_RULES_VERSION: i64 = 17;

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

/// `core.allium: UniqueFeedRolePerParent` and `UniqueRepoGroupPerParent`:
/// refuse `candidate` (an epic as it would be stored) when one of `siblings`
/// (the epics under the same parent; `candidate` itself may be among them)
/// already holds its feed role, or is a repo group with its title.
///
/// Siblings are the candidate's own workspace's: a ROOT's role is unique per
/// creator and workspace, since each person's board provisions its own
/// managed roots. Called on a create and on a patch that changes the
/// parent, role, origin or title, so a row that already holds a duplicate
/// stays writable for its other fields.
pub fn check_epic_unique_among(candidate: &Epic, siblings: &[Epic]) -> Result<(), String> {
    // Roots of every workspace share parent 0; only the candidate's own
    // workspace counts (a sub-epic's siblings share it already).
    let others = siblings
        .iter()
        .filter(|e| e.id != candidate.id || candidate.id == 0)
        .filter(|e| e.workspace_id == candidate.workspace_id);
    for other in others {
        if candidate.feed_role != "none"
            && !candidate.feed_role.is_empty()
            && other.feed_role == candidate.feed_role
            && (candidate.parent_epic_id != 0 || other.created_by == candidate.created_by)
        {
            return Err(format!(
                "an epic with feed role '{}' already exists under this parent",
                candidate.feed_role
            ));
        }
        if candidate.origin == REPO_GROUP_ORIGIN
            && other.origin == REPO_GROUP_ORIGIN
            && other.title == candidate.title
        {
            return Err(format!(
                "a repo-group epic titled '{}' already exists under this parent",
                candidate.title
            ));
        }
    }
    Ok(())
}

/// Whether a patch moved one of the inputs of [`check_epic_unique_among`].
pub fn epic_uniqueness_inputs_changed(before: &Epic, after: &Epic) -> bool {
    before.parent_epic_id != after.parent_epic_id
        || before.workspace_id != after.workspace_id
        || before.feed_role != after.feed_role
        || before.origin != after.origin
        || before.title != after.title
}

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
    // The store owns `owner` (`set_task_epic` stamps it); a client-named one
    // would orphan the task, since `AdoptOwnerless` adopts only empty owners.
    if patch.owner.is_some() {
        return Err("owner cannot be changed by patch_task; the store sets it".into());
    }
    if let Some(sub_status) = &patch.sub_status {
        let status = patch.status.as_deref().unwrap_or(&row.status);
        validate_sub_status(status, sub_status)?;
    }
    Ok(())
}

/// `PatchTaskNeverAimsAnotherHostsTeardown`: a patch that CHANGES a task's
/// `host`, `repo_path` or `worktree`, or names a `tmux_window`, is refused while the host that holds the
/// task now belongs to another person, and a patch that names a new `host` is
/// refused when that host belongs to another person. A feed teardown snapshots
/// exactly these three columns and asks that host to remove the worktree, so
/// letting anyone who sees the task rewrite them would let them aim another
/// person's host at a live worktree.
///
/// A host nobody registered has no owner, and no teardown request is ever
/// written for it (the module writes one only for a host the ingesting caller
/// owns), so it is not a target and is not judged. A value the patch repeats is
/// no change (the editor sends the repo on every save), and the repo of a task
/// no host holds stays editable by anyone who sees it. `owner_of` answers a
/// host id's owning identity, `None` for an unregistered host.
pub fn check_task_aim(
    row: &Task,
    patch: &TaskPatch,
    caller: &str,
    owner_of: impl Fn(&str) -> Option<String>,
) -> Result<(), String> {
    let changes = |new: &Option<String>, old: &str| new.as_deref().is_some_and(|v| v != old);
    let host_changes = changes(&patch.host, &row.host);
    // The window lives on the current agent, not the row, and a teardown kills
    // the window it names: naming one is a change; clearing one is not.
    let names_a_window = patch.tmux_window.as_deref().is_some_and(|w| !w.is_empty());
    if !(host_changes
        || names_a_window
        || changes(&patch.repo_path, &row.repo_path)
        || changes(&patch.worktree, &row.worktree))
    {
        return Ok(());
    }
    let someone_elses = |host: &str| owner_of(host).is_some_and(|owner| owner != caller);
    if !row.host.is_empty() && someone_elses(&row.host) {
        return Err(format!(
            "task {} is held by host {}, which belongs to another person, so its host, repo and worktree cannot be changed",
            row.id, row.host
        ));
    }
    if host_changes {
        let new = patch.host.as_deref().unwrap_or_default();
        if someone_elses(new) {
            return Err(format!(
                "host {new} belongs to another person, so a task cannot be moved onto it"
            ));
        }
    }
    Ok(())
}

/// What `create_task` and `create_task_signalled` check of a new row: it may
/// not name a host that belongs to another person.
///
/// A feed teardown asks the host named in the row to remove the worktree named
/// in it, so a creator who could name another person's host would aim it at a
/// live worktree (`tasks.allium: CreateTaskNeverAimsAnotherHostsTeardown`).
/// A host nobody registered has no owner and is not judged.
pub fn check_new_task_aim(
    row: &Task,
    caller: &str,
    owner_of: impl Fn(&str) -> Option<String>,
) -> Result<(), String> {
    if !row.host.is_empty() && owner_of(&row.host).is_some_and(|owner| owner != caller) {
        return Err(format!(
            "host {} belongs to another person, so a task cannot be created on it",
            row.host
        ));
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
