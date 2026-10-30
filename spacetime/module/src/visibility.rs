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

/// The most parents a walk up an epic chain follows before it gives up. A
/// cycle cannot be built (`check_epic_patch` refuses one), but a walk that
/// never ends would stall a transaction, so it is bounded.
const PRIVATE_WALK_LIMIT: usize = 64;

/// Whether the caller is the store itself: a scheduled reducer firing, or the
/// publisher. Sees everything.
pub fn is_operator(ctx: &ReducerContext) -> bool {
    ctx.sender() == ctx.database_identity()
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

/// `CreateEpicInWorkspace`: settle the row a new epic is stored as. A sub-epic
/// takes its parent's workspace whatever the row names (D5); a root epic names
/// one the caller belongs to. `created_by` is stamped with the caller when the
/// row carries none, so a private epic always has a creator to be private to.
pub fn settle_new_epic(ctx: &ReducerContext, row: Epic) -> Result<Epic, String> {
    validate_visibility_value(&row.visibility)?;
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
    let created_by = if row.created_by.is_empty() && !operator {
        caller
    } else {
        row.created_by.clone()
    };
    check_private_has_creator(&row.visibility, &created_by)?;
    Ok(Epic {
        workspace_id,
        parent_epic_id: parent_id,
        created_by,
        ..row
    })
}

/// `CreateWorkPackageInWorkspace`.
pub fn check_new_package(ctx: &ReducerContext, row: &WorkPackage) -> Result<(), String> {
    validate_visibility_value(&row.visibility)?;
    if ctx.db.workspaces().id().find(row.workspace_id).is_none()
        || (!is_operator(ctx)
            && !workspace_has_member(ctx, row.workspace_id, &caller_identity(ctx)))
    {
        return Err(format!("workspace {} does not exist", row.workspace_id));
    }
    Ok(())
}

/// `EpicStaysInItsWorkspace`: a re-parent stays inside the workspace, under an
/// epic the caller can see. `new_parent` `0` makes it a root epic.
pub fn check_epic_move(ctx: &ReducerContext, epic: &Epic, new_parent: i64) -> Result<(), String> {
    if new_parent == 0 {
        return Ok(());
    }
    let Some(destination) = visible_epic(ctx, new_parent) else {
        return Err(format!("epic {new_parent} does not exist"));
    };
    if destination.workspace_id != epic.workspace_id {
        return Err("an epic never moves to another workspace".into());
    }
    Ok(())
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
    validate_visibility_value(&visibility)?;
    let Some(mut epic) = visible_epic(ctx, epic_id) else {
        return Err(format!("epic {epic_id} does not exist"));
    };
    if !may_set_visibility(ctx, &epic.created_by, epic.workspace_id) {
        return Err("only the creator or an admin may change an epic's visibility".into());
    }
    check_private_has_creator(&visibility, &epic.created_by)?;
    epic.visibility = visibility;
    epic.updated_at = now(ctx);
    ctx.db.epics().id().update(epic);
    Ok(())
}

/// `workspaces.allium: SetWorkPackageVisibility`.
#[spacetimedb::reducer]
pub fn set_work_package_visibility(
    ctx: &ReducerContext,
    work_package_id: i64,
    visibility: String,
) -> Result<(), String> {
    validate_visibility_value(&visibility)?;
    let Some(mut package) = visible_package(ctx, work_package_id) else {
        return Err(format!("no work package {work_package_id}"));
    };
    if !may_set_visibility(ctx, &package.created_by, package.workspace_id) {
        return Err("only the creator or an admin may change a package's visibility".into());
    }
    package.visibility = visibility;
    package.updated_at = now(ctx);
    ctx.db.work_packages().id().update(package);
    Ok(())
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
            if private_holder(ctx, &epic).is_none() {
                return Err("only a private item is deleted this way".into());
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
            if package.visibility != VISIBILITY_PRIVATE {
                return Err("only a private item is deleted this way".into());
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
