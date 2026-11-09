//! Who may see and change what (`workspaces.allium`: the read predicate,
//! `WritesRequireSight`, "Epics", "Work packages", "The escape hatch").
//!
//! One predicate per entity, applied by every reducer that touches one. A
//! refusal names nothing about the item: an item the caller cannot see is
//! answered exactly as an item that does not exist (`UnseenIsAsMissing`), so a
//! refusal cannot be used to probe for a private one.
//!
//! The operator is exempt. The module's own scheduled calls and the store's
//! publisher identity are the trust boundary (`workspaces.allium`, "READ
//! ENFORCEMENT"), not a person with a membership.

use super::*;
use std::collections::BTreeSet;

/// The most parents a walk up an epic chain follows before it gives up. A
/// cycle cannot be built (`check_epic_patch` refuses one), but a walk that
/// never ends would stall a transaction, so it is bounded.
const PRIVATE_WALK_LIMIT: usize = 64;

/// Whether the caller is the store itself: a scheduled reducer firing. Sees
/// everything. The publisher is NOT an operator here; it is recognised only by
/// [`require_restore_authority`], for the `seed_*` reducers.
pub fn is_operator(ctx: &ReducerContext) -> bool {
    ctx.sender() == ctx.database_identity()
}

/// Whether the caller is the identity that published this database, as
/// recorded by `init`. False for a store that recorded none.
pub fn is_publisher(ctx: &ReducerContext) -> bool {
    ctx.db
        .store_owner()
        .id()
        .find(1)
        .is_some_and(|owner| owner.identity == caller_identity(ctx))
}

/// The identity a write records for a person: the caller's own, unless the
/// caller is the operator, who may name anyone (`workspaces.allium`:
/// "Identities are the caller's"). Pure, so the board's twin calls it too.
pub fn stamped_identity(operator: bool, caller: &str, claimed: String) -> String {
    if operator {
        claimed
    } else {
        caller.to_string()
    }
}

/// [`stamped_identity`] for the current call.
pub fn caller_stamp(ctx: &ReducerContext, claimed: String) -> String {
    stamped_identity(is_operator(ctx), &caller_identity(ctx), claimed)
}

/// Refuse a `seed_*` call from anyone but the store itself or its publisher
/// (`spacetime-seed.allium: SeedRequiresTheOperator`).
pub fn require_restore_authority(ctx: &ReducerContext) -> Result<(), String> {
    if is_operator(ctx) || is_publisher(ctx) {
        Ok(())
    } else {
        Err("only the store's publisher may restore (seed) data".into())
    }
}

/// `is_super_admin(who)`.
pub fn is_super_admin(r: &impl StoreReads, who: &str) -> bool {
    r.profile(who).is_some_and(|p| p.is_super_admin)
}

/// `workspace.has_member(who)`.
pub fn workspace_has_member(r: &impl StoreReads, workspace_id: i64, who: &str) -> bool {
    r.members_of(workspace_id).iter().any(|m| m.user == who)
}

/// `workspace.is_admin(who)`.
pub fn workspace_has_admin(r: &impl StoreReads, workspace_id: i64, who: &str) -> bool {
    r.members_of(workspace_id)
        .iter()
        .any(|m| m.user == who && m.role == ROLE_ADMIN)
}

/// `Epic.private_holder`: the creator of the nearest private epic at or above
/// `epic`, or `None` when the epic is public all the way up. `Some("")` is a
/// private epic with no creator, which nobody holds.
fn private_holder(r: &impl StoreReads, epic: &Epic) -> Option<String> {
    let mut current = epic.clone();
    for _ in 0..PRIVATE_WALK_LIMIT {
        if current.visibility == VISIBILITY_PRIVATE {
            return Some(current.created_by);
        }
        if current.parent_epic_id == 0 {
            return None;
        }
        current = r.epic(current.parent_epic_id)?;
    }
    None
}

/// `Epic.visible_to(who)`.
pub fn epic_visible_to(r: &impl StoreReads, epic: &Epic, who: &str) -> bool {
    if !workspace_has_member(r, epic.workspace_id, who) {
        return false;
    }
    match private_holder(r, epic) {
        None => true,
        Some(holder) => !holder.is_empty() && holder == who,
    }
}

/// Everyone `epic_id` is shown to: the members of its workspace for whom
/// [`epic_visible_to`] holds. Empty for a missing epic. `pub` so
/// `MemoryReducerCaller` marks the same people the module does.
pub fn viewers_of_epic(r: &impl StoreReads, epic_id: i64) -> BTreeSet<String> {
    let Some(epic) = r.epic(epic_id) else {
        return BTreeSet::new();
    };
    r.members_of(epic.workspace_id)
        .into_iter()
        .map(|m| m.user)
        .filter(|user| epic_visible_to(r, &epic, user))
        .collect()
}

/// `Task.visible_to(who)`: an epic task follows its epic, an epic-less task is
/// its owner's alone.
pub fn task_visible_to(r: &impl StoreReads, task: &Task, who: &str) -> bool {
    if task.epic_id == 0 {
        return !task.owner.is_empty() && task.owner == who;
    }
    r.epic(task.epic_id)
        .is_some_and(|epic| epic_visible_to(r, &epic, who))
}

/// `WorkPackage.visible_to(who)`.
pub fn package_visible_to(r: &impl StoreReads, package: &WorkPackage, who: &str) -> bool {
    workspace_has_member(r, package.workspace_id, who)
        && (package.visibility == VISIBILITY_PUBLIC || package.created_by == who)
}

/// The task `id`, when it exists and the caller may see it. `None` is both
/// "missing" and "unseen": the caller learns neither which.
pub fn visible_task(ctx: &ReducerContext, id: i64) -> Option<Task> {
    let task = ctx.db.tasks().id().find(id)?;
    (is_operator(ctx) || task_visible_to(ctx, &task, &caller_identity(ctx))).then_some(task)
}

/// The epic `id`, when it exists and the caller may see it.
pub fn visible_epic(ctx: &ReducerContext, id: i64) -> Option<Epic> {
    let epic = ctx.db.epics().id().find(id)?;
    (is_operator(ctx) || epic_visible_to(ctx, &epic, &caller_identity(ctx))).then_some(epic)
}

/// The package `id`, when it exists and the caller may see it.
pub fn visible_package(ctx: &ReducerContext, id: i64) -> Option<WorkPackage> {
    let package = ctx.db.work_packages().id().find(id)?;
    (is_operator(ctx) || package_visible_to(ctx, &package, &caller_identity(ctx)))
        .then_some(package)
}

/// `TaskEntersAnEpicTheCallerCanSee`: an epic-less placement needs no check; an
/// epic must be visible. The answer for an unseen epic is the one for a
/// missing epic.
pub fn check_task_may_enter_epic(
    ctx: &ReducerContext,
    task_label: &str,
    epic_id: i64,
) -> Result<(), String> {
    if epic_id == 0 || visible_epic(ctx, epic_id).is_some() {
        return Ok(());
    }
    Err(format!("{task_label}: epic {epic_id} does not exist"))
}

/// Refuse a visibility other than `public` or `private`.
pub fn validate_visibility_value(visibility: &str) -> Result<(), String> {
    if visibility == VISIBILITY_PUBLIC || visibility == VISIBILITY_PRIVATE {
        Ok(())
    } else {
        Err(format!(
            "visibility is '{VISIBILITY_PUBLIC}' or '{VISIBILITY_PRIVATE}', not '{visibility}'"
        ))
    }
}

/// `EpicIsPrivateOnlyWithACreator`.
fn check_private_has_creator(visibility: &str, created_by: &str) -> Result<(), String> {
    if visibility == VISIBILITY_PRIVATE && created_by.is_empty() {
        return Err("a private epic needs a creator".into());
    }
    Ok(())
}

/// `RepoGroupsStayPublic`: a repo-group sub-epic is found by title and origin
/// by whoever routes a task, so it must be one everybody who sees its parent
/// sees.
pub fn check_repo_group_stays_public(origin: &str, visibility: &str) -> Result<(), String> {
    if origin == REPO_GROUP_ORIGIN && visibility == VISIBILITY_PRIVATE {
        return Err("a repo-group epic cannot be private".into());
    }
    Ok(())
}

/// `RepoGroupsStayPublic`: the origin marking a repo group is set by the
/// module's routing and never changed by a patch, in either direction.
pub fn check_origin_patch(current: &str, new: &str) -> Result<(), String> {
    if current != new && (current == REPO_GROUP_ORIGIN || new == REPO_GROUP_ORIGIN) {
        return Err("a repo-group origin is set by routing and cannot be changed".into());
    }
    Ok(())
}

/// `CreateEpicInWorkspace`: settle the row a new epic is stored as. A sub-epic
/// takes its parent's workspace whatever the row names (D5); a root epic names
/// one the caller belongs to. `created_by` is stamped with the caller when the
/// row carries none, so a private epic always has a creator to be private to.
pub fn settle_new_epic(ctx: &ReducerContext, row: Epic) -> Result<Epic, String> {
    validate_visibility_value(&row.visibility)?;
    check_repo_group_stays_public(&row.origin, &row.visibility)?;
    let operator = is_operator(ctx);
    let caller = caller_identity(ctx);
    let (workspace_id, parent_id) = if row.parent_epic_id != 0 {
        let Some(parent) = visible_epic(ctx, row.parent_epic_id) else {
            return Err(format!("epic {} does not exist", row.parent_epic_id));
        };
        (parent.workspace_id, parent.id)
    } else {
        if ctx.db.workspaces().id().find(row.workspace_id).is_none() {
            return Err(format!("workspace {} does not exist", row.workspace_id));
        }
        (row.workspace_id, 0)
    };
    if !operator && !workspace_has_member(ctx, workspace_id, &caller) {
        return Err(format!("workspace {workspace_id} does not exist"));
    }
    // A caller's epic is always theirs: the row's own `created_by` is a
    // client's claim, and a private epic attributed to someone else would
    // block that person's moves. Only the store itself may name a creator.
    let created_by = if operator {
        row.created_by.clone()
    } else {
        caller
    };
    check_private_has_creator(&row.visibility, &created_by)?;
    // `FeedCommandAuthority` at creation: a command on a new epic is its
    // creator's, whatever the row names.
    let feed_command_set_by = if row.feed_command.is_empty() {
        String::new()
    } else {
        created_by.clone()
    };
    Ok(Epic {
        workspace_id,
        parent_epic_id: parent_id,
        created_by,
        feed_command_set_by,
        ..row
    })
}

/// `CreateWorkPackageInWorkspace`: check the workspace and return the row to
/// store, `created_by` stamped with the caller (the store itself may name one).
pub fn settle_new_package(ctx: &ReducerContext, row: WorkPackage) -> Result<WorkPackage, String> {
    validate_visibility_value(&row.visibility)?;
    if ctx.db.workspaces().id().find(row.workspace_id).is_none()
        || (!is_operator(ctx)
            && !workspace_has_member(ctx, row.workspace_id, &caller_identity(ctx)))
    {
        return Err(format!("workspace {} does not exist", row.workspace_id));
    }
    let created_by = if is_operator(ctx) {
        row.created_by.clone()
    } else {
        caller_identity(ctx)
    };
    Ok(WorkPackage { created_by, ..row })
}

/// Refuse a `subscriber` the caller is not: a board follows epics and clears
/// unseen marks as itself. The store itself may act for anyone.
pub fn require_own_identity(ctx: &ReducerContext, subscriber: &str) -> Result<(), String> {
    if is_operator(ctx) || subscriber == caller_identity(ctx) {
        Ok(())
    } else {
        Err("a subscriber must be the caller's own identity".into())
    }
}

/// `EpicStaysInItsWorkspace`: a re-parent stays inside the workspace, under an
/// epic the caller can see. `new_parent` `0` makes it a root epic.
///
/// `ReParentingKeepsTheHolder`: a move that changes who the epic is private to
/// (under a private epic, out of one, or between two) is a visibility change,
/// so it needs the authority `SetEpicVisibility` needs.
pub fn check_epic_move(ctx: &ReducerContext, epic: &Epic, new_parent: i64) -> Result<(), String> {
    let destination = if new_parent == 0 {
        None
    } else {
        let Some(destination) = visible_epic(ctx, new_parent) else {
            return Err(format!("epic {new_parent} does not exist"));
        };
        if destination.workspace_id != epic.workspace_id {
            return Err(
                "an epic changes workspace only through move_epic_to_workspace, as a root epic"
                    .into(),
            );
        }
        Some(destination)
    };
    if epic_move_changes_holder(ctx, epic, destination.as_ref())
        && !may_set_visibility(ctx, &epic.created_by, epic.workspace_id)
    {
        return Err(
            "only the creator or an admin may move an epic into or out of a private epic".into(),
        );
    }
    Ok(())
}

/// Whether re-parenting `epic` under `destination` (`None`: to the root)
/// changes who the epic is private to. Shared with `MemoryReducerCaller`.
pub fn epic_move_changes_holder(
    r: &impl StoreReads,
    epic: &Epic,
    destination: Option<&Epic>,
) -> bool {
    let after = if epic.visibility == VISIBILITY_PRIVATE {
        Some(epic.created_by.clone())
    } else {
        destination.and_then(|d| private_holder(r, d))
    };
    after != private_holder(r, epic)
}

/// `ReParentingKeepsTheHolder`, for a task: a move into `epic_id` that changes
/// who the task is private to needs more than sight. A task has no creator, so
/// the authority is: the person it is private to now (their own work is theirs
/// to expose or keep), or an admin of its workspace, or a super-admin. An
/// epic-less task is private to its owner. Shared with `MemoryReducerCaller`.
pub fn task_move_permitted(r: &impl StoreReads, task: &Task, epic_id: i64, caller: &str) -> bool {
    let holder_of = |id: i64| -> Option<String> {
        if id == 0 {
            return Some(task.owner.clone());
        }
        r.epic(id).and_then(|e| private_holder(r, &e))
    };
    let before = holder_of(task.epic_id);
    if before == holder_of(epic_id) || before.as_deref() == Some(caller) {
        return true;
    }
    let workspace = r
        .epic(epic_id)
        .or_else(|| r.epic(task.epic_id))
        .map(|e| e.workspace_id);
    workspace.is_some_and(|w| workspace_has_admin(r, w, caller)) || is_super_admin(r, caller)
}

/// `TaskMovesStayInTheWorkspace`: a task in an epic moves only to another epic
/// of the same workspace, or onto a board as a detach (see
/// [`task_detach_permitted`]). A move into an epic of another workspace leaves
/// it. An epic-less task joins any epic its caller sees. Shared with
/// `MemoryReducerCaller`.
pub fn task_move_leaves_workspace(r: &impl StoreReads, task: &Task, epic_id: i64) -> bool {
    if task.epic_id == 0 {
        return false;
    }
    let Some(source) = r.epic(task.epic_id) else {
        return false;
    };
    match r.epic(epic_id) {
        Some(destination) => destination.workspace_id != source.workspace_id,
        None => false,
    }
}

/// Whether a move is a detach: a task in an epic onto a user board.
pub fn task_move_is_detach(task: &Task, epic_id: i64) -> bool {
    task.epic_id != 0 && epic_id == 0
}

/// `TaskMovesStayInTheWorkspace`, for a detach: the task's creator, an admin of
/// its workspace or a super-admin. Shared with `MemoryReducerCaller`.
pub fn task_detach_permitted(r: &impl StoreReads, task: &Task, caller: &str) -> bool {
    (!task.created_by.is_empty() && task.created_by == caller)
        || r.epic(task.epic_id)
            .is_some_and(|e| workspace_has_admin(r, e.workspace_id, caller))
        || is_super_admin(r, caller)
}

/// The refusal for a detach by someone with no authority over it.
pub fn task_detach_refusal(task_id: i64) -> String {
    format!("task {task_id}: only its creator or an admin may detach it from its epic")
}

/// A2 (task #98992, resolved with the user): a detach would hide the task from
/// a host that is running it. True when a current agent, or an open dispatch
/// or session request, sits on a host `caller` does not own. Shared with
/// `MemoryReducerCaller`.
pub fn task_detach_strands_a_host(r: &impl StoreReads, task_id: i64, caller: &str) -> bool {
    let mine: std::collections::HashSet<String> = r
        .hosts_owned_by(caller)
        .into_iter()
        .map(|h| h.id)
        .collect();
    r.agents_of(task_id)
        .iter()
        .any(|a| a.state == AGENT_CURRENT && !mine.contains(&a.host))
        || r.dispatch_requests_of(task_id)
            .iter()
            .any(|d| dispatch_request_is_open(&d.state) && !mine.contains(&d.host))
        || r.session_requests_of(task_id)
            .iter()
            .any(|s| session_request_is_open(&s.state) && !mine.contains(&s.host))
}

/// The refusal for a detach that would strand another person's host.
pub fn task_detach_strands_refusal(task_id: i64) -> String {
    format!("task {task_id}: another person's host is running it; detach it once that agent has ended")
}

pub fn check_task_move(ctx: &ReducerContext, task: &Task, epic_id: i64) -> Result<(), String> {
    if is_operator(ctx) {
        return Ok(());
    }
    // A detach is judged by its own authority, not the holder arithmetic below.
    if task_move_is_detach(task, epic_id) {
        let caller = caller_identity(ctx);
        if !task_detach_permitted(ctx, task, &caller) {
            return Err(task_detach_refusal(task.id));
        }
        return if task_detach_strands_a_host(ctx, task.id, &caller) {
            Err(task_detach_strands_refusal(task.id))
        } else {
            Ok(())
        };
    }
    if task_move_leaves_workspace(ctx, task, epic_id) {
        return Err(format!(
            "task {}: a task stays in its workspace; it cannot move into another workspace",
            task.id
        ));
    }
    if task_move_permitted(ctx, task, epic_id, &caller_identity(ctx)) {
        return Ok(());
    }
    Err(format!(
        "task {}: only the person it is private to, or an admin, may move it into or out of a private epic",
        task.id
    ))
}

/// The workspace an item lives in, or `None` for a main-board task
/// (`LinkOnlyInsideTheWorkspace`: `item_workspace`).
fn item_workspace(ctx: &ReducerContext, kind: &str, item_id: i64) -> Option<i64> {
    match kind {
        WORK_PACKAGE_ITEM_EPIC => ctx.db.epics().id().find(item_id).map(|e| e.workspace_id),
        WORK_PACKAGE_ITEM_TASK => {
            let task = ctx.db.tasks().id().find(item_id)?;
            ctx.db
                .epics()
                .id()
                .find(task.epic_id)
                .map(|e| e.workspace_id)
        }
        _ => None,
    }
}

/// `LinkOnlyInsideTheWorkspace`: the caller sees both the package and the
/// item, and the item is in the package's workspace, or is a main-board task.
/// An unseen item is answered as a missing one.
pub fn check_link_allowed(
    ctx: &ReducerContext,
    package: &WorkPackage,
    kind: &str,
    item_id: i64,
) -> Result<(), String> {
    let seen = match kind {
        WORK_PACKAGE_ITEM_TASK => visible_task(ctx, item_id).is_some(),
        WORK_PACKAGE_ITEM_EPIC => visible_epic(ctx, item_id).is_some(),
        other => return Err(format!("unknown work package item kind {other:?}")),
    };
    if !seen {
        return Err(format!("no {kind} {item_id}"));
    }
    match item_workspace(ctx, kind, item_id) {
        Some(workspace) if workspace != package.workspace_id => Err(format!(
            "a work package links only to items in its own workspace; {kind} {item_id} is elsewhere"
        )),
        _ => Ok(()),
    }
}

/// Whether the caller may act on a linked item: it does not exist (nothing to
/// hide), or it does and the caller sees it. An unseen item is answered as one
/// that is not linked (`LinkOnlyInsideTheWorkspace`, unlinking).
pub fn item_seen(ctx: &ReducerContext, kind: &str, item_id: i64) -> bool {
    match kind {
        WORK_PACKAGE_ITEM_TASK => {
            ctx.db.tasks().id().find(item_id).is_none() || visible_task(ctx, item_id).is_some()
        }
        WORK_PACKAGE_ITEM_EPIC => {
            ctx.db.epics().id().find(item_id).is_none() || visible_epic(ctx, item_id).is_some()
        }
        _ => true,
    }
}

/// May the caller change an item's visibility: its creator, an admin of its
/// workspace, or a super-admin. Sight is checked by the caller first.
fn may_set_visibility(ctx: &ReducerContext, created_by: &str, workspace_id: i64) -> bool {
    let caller = caller_identity(ctx);
    is_operator(ctx)
        || created_by == caller
        || workspace_has_admin(ctx, workspace_id, &caller)
        || is_super_admin(ctx, &caller)
}

/// `workspaces.allium: SetEpicVisibility`.
#[spacetimedb::reducer]
pub fn set_epic_visibility(
    ctx: &ReducerContext,
    epic_id: i64,
    visibility: String,
) -> Result<(), String> {
    set_epic_visibility_row(ctx, epic_id, visibility)
}

/// [`set_epic_visibility`]'s body, shared with `update_epic`.
pub(crate) fn set_epic_visibility_row(
    ctx: &ReducerContext,
    epic_id: i64,
    visibility: String,
) -> Result<(), String> {
    validate_visibility_value(&visibility)?;
    let Some(mut epic) = visible_epic(ctx, epic_id) else {
        return Err(format!("epic {epic_id} does not exist"));
    };
    if !may_set_visibility(ctx, &epic.created_by, epic.workspace_id) {
        return Err("only the creator or an admin may change an epic's visibility".into());
    }
    check_private_has_creator(&visibility, &epic.created_by)?;
    check_repo_group_stays_public(&epic.origin, &visibility)?;
    epic.visibility = visibility;
    epic.updated_at = now(ctx);
    ctx.db.epics().id().update(epic);
    // `ClaimsFollowSight`: a flip to private takes the tree from every other
    // member, and their hosts' claims with it.
    crate::config::on_sight_lost(ctx, &collect_epic_subtree_ids(ctx, epic_id));
    Ok(())
}

/// `workspaces.allium: MoveEpicToWorkspace` (D13): move a root epic and its
/// whole tree to another workspace.
///
/// Sight is not required of a super-admin, so one can rescue an epic stranded
/// in a workspace that does not exist (nobody is a member of it, so nobody
/// sees it). A private epic is the exception: it moves only at its creator's
/// hand, and anyone else is answered as for a missing epic.
#[spacetimedb::reducer]
pub fn move_epic_to_workspace(
    ctx: &ReducerContext,
    epic_id: i64,
    workspace_id: i64,
) -> Result<(), String> {
    move_epic_row(ctx, epic_id, workspace_id)
}

/// [`move_epic_to_workspace`]'s body, shared with `update_epic`.
pub(crate) fn move_epic_row(
    ctx: &ReducerContext,
    epic_id: i64,
    workspace_id: i64,
) -> Result<(), String> {
    let caller = caller_identity(ctx);
    let missing = || format!("epic {epic_id} does not exist");
    let Some(epic) = ctx.db.epics().id().find(epic_id) else {
        return Err(missing());
    };
    let privileged = is_operator(ctx) || is_super_admin(ctx, &caller);
    let holder_ok = match private_holder(ctx, &epic) {
        None => true,
        Some(holder) => is_operator(ctx) || (!holder.is_empty() && holder == caller),
    };
    if !holder_ok || (!privileged && !epic_visible_to(ctx, &epic, &caller)) {
        return Err(missing());
    }
    if epic.parent_epic_id != 0 {
        let mut root = epic.clone();
        for _ in 0..PRIVATE_WALK_LIMIT {
            match ctx.db.epics().id().find(root.parent_epic_id) {
                Some(parent) if root.parent_epic_id != 0 => root = parent,
                _ => break,
            }
        }
        return Err(format!(
            "epic {epic_id} is a sub-epic; move its root epic {} instead",
            root.id
        ));
    }
    if ctx.db.workspaces().id().find(workspace_id).is_none() {
        return Err(format!("workspace {workspace_id} does not exist"));
    }
    if workspace_id == epic.workspace_id {
        return Err(format!(
            "epic {epic_id} is already in workspace {workspace_id}"
        ));
    }
    let admin_of_both = ctx.db.workspaces().id().find(epic.workspace_id).is_some()
        && workspace_has_admin(ctx, epic.workspace_id, &caller)
        && workspace_has_admin(ctx, workspace_id, &caller);
    if !privileged && !admin_of_both {
        return Err("only a super-admin, or an admin of both workspaces, may move an epic".into());
    }
    // `core.allium: UniqueFeedRolePerParent`: the root as it would land.
    let landed = Epic {
        workspace_id,
        ..epic.clone()
    };
    check_epic_unique_among(&landed, &ctx.epics_under(0))?;

    let tree = crate::tasks_epics::collect_epic_subtree_ids(ctx, epic_id);
    // A private sub-epic is its creator's: the move must not carry it off,
    // and the refusal names nothing about it.
    let someone_elses = tree.iter().any(|id| {
        ctx.db.epics().id().find(*id).is_some_and(|node| {
            private_holder(ctx, &node).is_some_and(|h| h.is_empty() || h != caller)
        })
    });
    if someone_elses && !is_operator(ctx) {
        return Err(missing());
    }
    let mut task_ids = std::collections::HashSet::new();
    for id in &tree {
        for task in ctx.db.tasks().epic_id().filter(id) {
            task_ids.insert(task.id);
        }
    }
    let stamp = now(ctx);
    for id in &tree {
        if let Some(node) = ctx.db.epics().id().find(*id) {
            ctx.db.epics().id().update(Epic {
                workspace_id,
                updated_at: stamp.clone(),
                ..node
            });
        }
    }
    let in_tree = |l: &Learning| {
        let id = l.scope_ref.as_deref().and_then(|r| r.parse::<i64>().ok());
        match (l.scope.as_str(), id) {
            ("epic", Some(id)) => tree.contains(&id),
            ("task", Some(id)) => task_ids.contains(&id),
            _ => false,
        }
    };
    crate::learning::update_matching_learnings(ctx, in_tree, |row| Learning {
        workspace_id,
        ..row
    });
    for id in &tree {
        crate::packages::delete_work_package_links_to(ctx, WORK_PACKAGE_ITEM_EPIC, *id);
    }
    for id in &task_ids {
        crate::packages::delete_work_package_links_to(ctx, WORK_PACKAGE_ITEM_TASK, *id);
    }
    let stale: Vec<String> = ctx
        .db
        .subscriptions()
        .iter()
        .filter(|s| {
            tree.contains(&s.epic_id) && !workspace_has_member(ctx, workspace_id, &s.subscriber)
        })
        .map(|s| s.id)
        .collect();
    for id in stale {
        ctx.db.subscriptions().id().delete(&id);
    }
    // `MoveEpicToWorkspace` (V6): a host whose owner is not in the destination
    // can no longer read the tree, so it must not keep the feed's claim.
    let tree_set: std::collections::HashSet<i64> = tree.iter().copied().collect();
    crate::config::release_poll_claims(ctx, &tree_set, |_, owner| {
        !workspace_has_member(ctx, workspace_id, owner)
    });
    crate::config::release_task_poll_claims(ctx, &tree_set, |_, owner| {
        !workspace_has_member(ctx, workspace_id, owner)
    });
    crate::bookmark::drop_marks_where(ctx, None, |task_id, subscriber| {
        task_ids.contains(&task_id) && !workspace_has_member(ctx, workspace_id, subscriber)
    });
    Ok(())
}

/// Every refusal `set_work_package_visibility` can make, raised without a
/// write so `update_work_package_with_links` can ask before its first one.
pub(crate) fn check_package_visibility_change(
    ctx: &ReducerContext,
    work_package_id: i64,
    visibility: &str,
) -> Result<(), String> {
    validate_visibility_value(visibility)?;
    let Some(package) = visible_package(ctx, work_package_id) else {
        return Err(format!("no work package {work_package_id}"));
    };
    if !may_set_visibility(ctx, &package.created_by, package.workspace_id) {
        return Err("only the creator or an admin may change a package's visibility".into());
    }
    Ok(())
}

/// `workspaces.allium: SetWorkPackageVisibility`.
#[spacetimedb::reducer]
pub fn set_work_package_visibility(
    ctx: &ReducerContext,
    work_package_id: i64,
    visibility: String,
) -> Result<(), String> {
    check_package_visibility_change(ctx, work_package_id, &visibility)?;
    let Some(mut package) = visible_package(ctx, work_package_id) else {
        return Err(format!("no work package {work_package_id}"));
    };
    package.visibility = visibility;
    package.updated_at = now(ctx);
    ctx.db.work_packages().id().update(package);
    Ok(())
}

/// `SuperAdminDeletesUnseenItem`: a private epic whose holder is no longer a
/// member of its workspace. Shared with `MemoryReducerCaller`.
pub fn epic_is_unseen(r: &impl StoreReads, epic: &Epic) -> bool {
    private_holder(r, epic)
        .is_some_and(|h| h.is_empty() || !workspace_has_member(r, epic.workspace_id, &h))
}

/// `SuperAdminDeletesUnseenItem`: a private package whose creator is no longer
/// a member of its workspace.
pub fn package_is_unseen(r: &impl StoreReads, package: &WorkPackage) -> bool {
    package.visibility == VISIBILITY_PRIVATE
        && !workspace_has_member(r, package.workspace_id, &package.created_by)
}

/// `workspaces.allium: SuperAdminDeletesUnseenItem`: delete a private epic
/// (with its subtree) or a private package without reading it. Refused for an
/// item that is public, so the escape hatch is no way around `DeleteEpic`'s
/// guard for ordinary items.
#[spacetimedb::reducer]
pub fn super_admin_delete_unseen_item(
    ctx: &ReducerContext,
    kind: String,
    item_id: i64,
) -> Result<(), String> {
    if !is_operator(ctx) && !is_super_admin(ctx, &caller_identity(ctx)) {
        return Err("only a super-admin may do this".into());
    }
    match kind.as_str() {
        WORK_PACKAGE_ITEM_EPIC => {
            let Some(epic) = ctx.db.epics().id().find(item_id) else {
                return Err(format!("no epic {item_id}"));
            };
            // `SuperAdminDeletesUnseenItem`: an item nobody can see, not one
            // its creator still holds. The listing (`unseen_items_of`) shows
            // the same set.
            if !epic_is_unseen(ctx, &epic) {
                return Err("only an item nobody can see is deleted this way".into());
            }
            let parent = epic.parent_epic_id;
            // The escape hatch widens who may delete, not what may be torn
            // down: the every-task-done guard of `delete_epic` still holds.
            let doomed = crate::tasks_epics::validate_epic_deletable(ctx, item_id)?;
            delete_epic_subtree_with_feed_cleanup(ctx, item_id, &doomed);
            recalculate_epic_chain(ctx, parent);
            Ok(())
        }
        "work_package" => {
            let Some(package) = ctx.db.work_packages().id().find(item_id) else {
                return Err(format!("no work package {item_id}"));
            };
            if !package_is_unseen(ctx, &package) {
                return Err("only an item nobody can see is deleted this way".into());
            }
            delete_package_rows(ctx, item_id);
            Ok(())
        }
        other => Err(format!(
            "an unseen item is an epic or a work_package, not {other:?}"
        )),
    }
}

/// An epic delete cascades through its whole subtree, so it is refused when the
/// subtree holds an epic the caller cannot see: a private sub-epic of a public
/// epic is somebody's, and a delete of its parent must not take it. The refusal
/// names no detail of the hidden epic.
pub(crate) fn check_subtree_all_seen(
    ctx: &ReducerContext,
    id: i64,
    doomed: &std::collections::HashSet<i64>,
) -> Result<(), String> {
    if is_operator(ctx) {
        return Ok(());
    }
    let caller = caller_identity(ctx);
    let all_seen = doomed.iter().all(|epic_id| {
        ctx.db
            .epics()
            .id()
            .find(*epic_id)
            .is_none_or(|e| epic_visible_to(ctx, &e, &caller))
    });
    if all_seen {
        Ok(())
    } else {
        Err(format!("epic {id}: cannot delete right now"))
    }
}

/// Whether the caller sees a learning (`workspaces.allium:
/// LearningVisibility`).
pub fn learning_scope_seen(ctx: &ReducerContext, learning: &Learning) -> bool {
    learning_seen_by(ctx, learning, &caller_identity(ctx))
}

/// Whether `who` sees a learning (`workspaces.allium: LearningVisibility`): an
/// epic- or task-scoped learning only while its item is visible, a learning
/// with a workspace only to that workspace's members, and a learning with none
/// only to its author. A learning that predates workspaces has no author and
/// no workspace, and stays visible to everyone.
pub fn learning_seen_by(r: &impl StoreReads, learning: &Learning, caller: &str) -> bool {
    let item_id = learning
        .scope_ref
        .as_deref()
        .and_then(|r| r.parse::<i64>().ok());
    let item_seen = match (learning.scope.as_str(), item_id) {
        ("epic", Some(id)) => r.epic(id).is_none_or(|e| epic_visible_to(r, &e, caller)),
        ("task", Some(id)) => r.task(id).is_none_or(|t| task_visible_to(r, &t, caller)),
        _ => true,
    };
    item_seen
        && if learning.workspace_id == 0 {
            learning.author.is_empty() || learning.author == caller
        } else {
            workspace_has_member(r, learning.workspace_id, caller)
        }
}

/// The workspace a new learning belongs to, or `0` for none
/// (`workspaces.allium: LearningVisibility`). A recording task the caller
/// cannot see counts for nothing, so naming someone else's task places no
/// learning in their workspace.
pub(crate) fn learning_workspace(ctx: &ReducerContext, row: &Learning, caller: &str) -> i64 {
    let id_of = |r: &Option<String>| r.as_deref().and_then(|r| r.parse::<i64>().ok());
    let epic_workspace = |epic_id: i64| {
        ctx.db
            .epics()
            .id()
            .find(epic_id)
            .map_or(0, |e| e.workspace_id)
    };
    match row.scope.as_str() {
        "epic" => id_of(&row.scope_ref).map_or(0, epic_workspace),
        "task" => id_of(&row.scope_ref)
            .and_then(|id| ctx.db.tasks().id().find(id))
            .map_or(0, |t| epic_workspace(t.epic_id)),
        "repo" => row
            .source_task_id
            .and_then(|id| ctx.db.tasks().id().find(id))
            .filter(|t| task_visible_to(ctx, t, caller))
            .map_or(0, |t| epic_workspace(t.epic_id)),
        _ => 0,
    }
}

/// The learning `id`, when it exists and the caller may see it.
pub fn visible_learning(ctx: &ReducerContext, id: i64) -> Option<Learning> {
    let learning = ctx.db.learnings().id().find(id)?;
    (is_operator(ctx) || learning_scope_seen(ctx, &learning)).then_some(learning)
}

/// A module-made sub-epic (feed grouping, role routing) lives in its parent's
/// workspace (`SubEpicSharesItsParentsWorkspace`). A root epic is left as is.
pub(crate) fn inherit_workspace(ctx: &ReducerContext, row: Epic) -> Epic {
    match ctx.db.epics().id().find(row.parent_epic_id) {
        Some(parent) if row.parent_epic_id != 0 => Epic {
            workspace_id: parent.workspace_id,
            ..row
        },
        _ => row,
    }
}

/// The lowest-numbered workspace `who` belongs to.
pub(crate) fn first_workspace_of(ctx: &ReducerContext, who: &str) -> Option<i64> {
    ctx.db
        .memberships()
        .user()
        .filter(&who.to_string())
        .map(|m| m.workspace_id)
        .min()
}

/// One row of the `unseen_items` view (`admin-surfaces.allium: UnseenItem`):
/// a private epic or package nobody can see. Never a title or any content.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, PartialEq, Eq)]
pub struct UnseenItem {
    /// `"epic"` or `"work_package"`, as `super_admin_delete_unseen_item` takes it.
    pub kind: String,
    pub item_id: i64,
    pub workspace_id: i64,
    /// The creator's identity; `""` for a private item with no creator.
    pub creator: String,
    pub created_at: String,
}

/// `admin-surfaces.allium: UnseenItemListingContents`, over what a view can
/// read, so the view and `MemoryReducerCaller` run one definition. A view reads
/// through indexes only, so the caller hands in the private epics and packages
/// (a point read on the `visibility` index) and a way to find one epic by id.
/// An item is unseen when its creator is not a member of its workspace. An
/// epic under a private ancestor is part of that ancestor's row, so only the
/// topmost private epic is listed. Ordered by workspace id, kind, then id.
pub fn unseen_items_of(
    private_epics: &[Epic],
    private_packages: &[WorkPackage],
    epic_by_id: impl Fn(i64) -> Option<Epic>,
    is_member: impl Fn(i64, &str) -> bool,
) -> Vec<UnseenItem> {
    let under_private = |epic: &Epic| {
        let mut parent = epic.parent_epic_id;
        for _ in 0..PRIVATE_WALK_LIMIT {
            let Some(p) = epic_by_id(parent) else {
                return false;
            };
            if p.visibility == VISIBILITY_PRIVATE {
                return true;
            }
            parent = p.parent_epic_id;
        }
        false
    };
    let mut items: Vec<UnseenItem> = private_epics
        .iter()
        .filter(|e| !under_private(e) && !is_member(e.workspace_id, &e.created_by))
        .map(|e| UnseenItem {
            kind: WORK_PACKAGE_ITEM_EPIC.to_string(),
            item_id: e.id,
            workspace_id: e.workspace_id,
            creator: e.created_by.clone(),
            created_at: e.created_at.clone(),
        })
        .chain(
            private_packages
                .iter()
                .filter(|p| !is_member(p.workspace_id, &p.created_by))
                .map(|p| UnseenItem {
                    kind: "work_package".to_string(),
                    item_id: p.id,
                    workspace_id: p.workspace_id,
                    creator: p.created_by.clone(),
                    created_at: p.created_at.clone(),
                }),
        )
        .collect();
    items.sort_by(|a, b| {
        (a.workspace_id, &a.kind, a.item_id).cmp(&(b.workspace_id, &b.kind, b.item_id))
    });
    items
}

/// `admin-surfaces.allium: UnseenItemListingContents`, served per caller: the
/// private items nobody can see, for a super-admin, and nothing for anyone
/// else. Every read is a point read: a non-super-admin's run reads one profile
/// row, so a write to epics does not re-run it, and a super-admin's reads the
/// private rows only.
#[spacetimedb::view(accessor = unseen_items, public)]
fn unseen_items(ctx: &spacetimedb::ViewContext) -> Vec<UnseenItem> {
    let who = ctx.sender().to_hex().to_string();
    if !ctx
        .db
        .user_profiles()
        .identity()
        .find(&who)
        .is_some_and(|p| p.is_super_admin)
    {
        return Vec::new();
    }
    let private = VISIBILITY_PRIVATE.to_string();
    let epics: Vec<Epic> = ctx.db.epics().visibility().filter(&private).collect();
    let packages: Vec<WorkPackage> = ctx
        .db
        .work_packages()
        .visibility()
        .filter(&private)
        .collect();
    unseen_items_of(
        &epics,
        &packages,
        |id| ctx.db.epics().id().find(id),
        |workspace_id, user| {
            ctx.db
                .memberships()
                .workspace_id()
                .filter(&workspace_id)
                .any(|m| m.user == user)
        },
    )
}
