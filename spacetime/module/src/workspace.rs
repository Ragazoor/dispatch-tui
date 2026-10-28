//! Super-admins, workspaces and membership (`workspaces.allium`, "Super-admins",
//! "Workspaces", "Membership").
//!
//! Also `MigrateStoreToWorkspaces`, which puts a pre-workspace store into one
//! default workspace.

use super::*;

/// `workspaces.allium: config.workspace_name_max_length`, in characters.
pub const WORKSPACE_NAME_MAX_LENGTH: usize = 64;

/// `core.allium: WorkspaceRole`, in the store's spelling.
pub const ROLE_ADMIN: &str = "admin";
pub const ROLE_MEMBER: &str = "member";

/// `workspaces.allium: config.default_workspace_name`.
pub const DEFAULT_WORKSPACE_NAME: &str = "Default";

/// Validate a workspace name: trimmed, non-empty, at most
/// [`WORKSPACE_NAME_MAX_LENGTH`] characters, no control or invisible
/// characters. `pub` for `MemoryReducerCaller`.
pub fn validate_workspace_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("a workspace name cannot be empty".into());
    }
    if has_control_or_format_characters(trimmed) {
        return Err(
            "a workspace name cannot contain control or invisible formatting characters".into(),
        );
    }
    let length = trimmed.chars().count();
    if length > WORKSPACE_NAME_MAX_LENGTH {
        return Err(format!(
            "a workspace name is at most {WORKSPACE_NAME_MAX_LENGTH} characters; this one is {length}"
        ));
    }
    Ok(trimmed.to_string())
}

/// Refuse a role that is not `admin` or `member`. `pub` for `MemoryReducerCaller`.
pub fn validate_workspace_role(role: &str) -> Result<(), String> {
    if role == ROLE_ADMIN || role == ROLE_MEMBER {
        Ok(())
    } else {
        Err(format!(
            "a workspace role is '{ROLE_ADMIN}' or '{ROLE_MEMBER}', not '{role}'"
        ))
    }
}

fn profile_of(ctx: &ReducerContext, identity: &str) -> Option<UserProfile> {
    ctx.db
        .user_profiles()
        .identity()
        .find(&identity.to_string())
}

fn require_super_admin(ctx: &ReducerContext) -> Result<(), String> {
    match profile_of(ctx, &caller_identity(ctx)) {
        Some(p) if p.is_super_admin => Ok(()),
        _ => Err("only a super-admin may do this".into()),
    }
}

fn is_workspace_admin(ctx: &ReducerContext, workspace_id: i64, identity: &str) -> bool {
    ctx.db
        .memberships()
        .workspace_id()
        .filter(&workspace_id)
        .any(|m| m.user == identity && m.role == ROLE_ADMIN)
}

/// `is_super_admin(caller()) or workspace.is_admin(caller())`.
fn require_admin_of(ctx: &ReducerContext, workspace_id: i64) -> Result<(), String> {
    let caller = caller_identity(ctx);
    if profile_of(ctx, &caller).is_some_and(|p| p.is_super_admin)
        || is_workspace_admin(ctx, workspace_id, &caller)
    {
        Ok(())
    } else {
        Err("only a super-admin or an admin of this workspace may do this".into())
    }
}

fn workspace_named(ctx: &ReducerContext, name: &str, except: Option<i64>) -> bool {
    ctx.db
        .workspaces()
        .iter()
        .any(|w| w.name == name && Some(w.id) != except)
}

/// `workspaces.allium: ClaimSuperAdmin`, with `AdoptOwnerlessUserBoardTasks`.
#[spacetimedb::reducer]
pub fn claim_super_admin(ctx: &ReducerContext) -> Result<(), String> {
    let identity = caller_identity(ctx);
    let Some(mut profile) = profile_of(ctx, &identity) else {
        return Err("no profile for this identity; reconnect and try again".into());
    };
    if ctx.db.user_profiles().iter().any(|p| p.is_super_admin) {
        return Err("a super-admin already exists".into());
    }
    profile.is_super_admin = true;
    ctx.db.user_profiles().identity().update(profile);
    // An epic-less task with no owner predates owners and has nobody to be
    // private to; the first super-admin takes them.
    let ownerless: Vec<Task> = ctx
        .db
        .tasks()
        .iter()
        .filter(|t| t.epic_id == 0 && t.owner.trim().is_empty())
        .collect();
    for task in ownerless {
        ctx.db.tasks().id().update(Task {
            owner: identity.clone(),
            ..task
        });
    }
    Ok(())
}

/// `workspaces.allium: GrantSuperAdmin`.
#[spacetimedb::reducer]
pub fn grant_super_admin(ctx: &ReducerContext, user: String) -> Result<(), String> {
    require_super_admin(ctx)?;
    let Some(mut profile) = profile_of(ctx, &user) else {
        return Err(format!("no person with identity {user}"));
    };
    profile.is_super_admin = true;
    ctx.db.user_profiles().identity().update(profile);
    Ok(())
}

/// `workspaces.allium: RevokeSuperAdmin`: never the last one.
#[spacetimedb::reducer]
pub fn revoke_super_admin(ctx: &ReducerContext, user: String) -> Result<(), String> {
    require_super_admin(ctx)?;
    let Some(mut profile) = profile_of(ctx, &user) else {
        return Err(format!("no person with identity {user}"));
    };
    if !profile.is_super_admin {
        return Err("that person is not a super-admin".into());
    }
    if ctx
        .db
        .user_profiles()
        .iter()
        .filter(|p| p.is_super_admin)
        .count()
        <= 1
    {
        return Err("cannot revoke the last super-admin".into());
    }
    profile.is_super_admin = false;
    ctx.db.user_profiles().identity().update(profile);
    Ok(())
}

/// `workspaces.allium: CreateWorkspace`. The creator is not made a member.
#[spacetimedb::reducer]
pub fn create_workspace(ctx: &ReducerContext, name: String) -> Result<(), String> {
    require_super_admin(ctx)?;
    let name = validate_workspace_name(&name)?;
    if workspace_named(ctx, &name, None) {
        return Err(format!("a workspace named '{name}' already exists"));
    }
    ctx.db.workspaces().insert(Workspace {
        id: 0,
        name,
        created_by: caller_identity(ctx),
        created_at: now(ctx),
    });
    Ok(())
}

/// `workspaces.allium: RenameWorkspace`.
#[spacetimedb::reducer]
pub fn rename_workspace(
    ctx: &ReducerContext,
    workspace_id: i64,
    name: String,
) -> Result<(), String> {
    let Some(mut workspace) = ctx.db.workspaces().id().find(workspace_id) else {
        return Err(format!("workspace {workspace_id} does not exist"));
    };
    require_admin_of(ctx, workspace_id)?;
    let name = validate_workspace_name(&name)?;
    if workspace_named(ctx, &name, Some(workspace_id)) {
        return Err(format!("a workspace named '{name}' already exists"));
    }
    workspace.name = name;
    ctx.db.workspaces().id().update(workspace);
    Ok(())
}

/// `workspaces.allium: DeleteWorkspace`. Its memberships go with it.
#[spacetimedb::reducer]
pub fn delete_workspace(ctx: &ReducerContext, workspace_id: i64) -> Result<(), String> {
    require_super_admin(ctx)?;
    if ctx.db.workspaces().id().find(workspace_id).is_none() {
        return Err(format!("workspace {workspace_id} does not exist"));
    }
    // Private epics and packages count: a super-admin empties the workspace
    // first, deleting what they cannot read.
    if ctx
        .db
        .epics()
        .iter()
        .any(|e| e.workspace_id == workspace_id)
        || ctx
            .db
            .work_packages()
            .iter()
            .any(|p| p.workspace_id == workspace_id)
    {
        return Err(format!(
            "workspace {workspace_id} still holds epics or work packages; empty it first"
        ));
    }
    let learnings: Vec<i64> = ctx
        .db
        .learnings()
        .iter()
        .filter(|l| l.workspace_id == workspace_id)
        .map(|l| l.id)
        .collect();
    for id in learnings {
        delete_learning(ctx, id)?;
    }
    let memberships: Vec<i64> = ctx
        .db
        .memberships()
        .workspace_id()
        .filter(&workspace_id)
        .map(|m| m.id)
        .collect();
    for id in memberships {
        ctx.db.memberships().id().delete(id);
    }
    ctx.db.workspaces().id().delete(workspace_id);
    Ok(())
}

/// `workspaces.allium: AddMember`.
#[spacetimedb::reducer]
pub fn add_member(
    ctx: &ReducerContext,
    workspace_id: i64,
    user: String,
    role: String,
) -> Result<(), String> {
    if ctx.db.workspaces().id().find(workspace_id).is_none() {
        return Err(format!("workspace {workspace_id} does not exist"));
    }
    require_admin_of(ctx, workspace_id)?;
    validate_workspace_role(&role)?;
    if profile_of(ctx, &user).is_none() {
        return Err(format!("no person with identity {user}"));
    }
    if ctx
        .db
        .memberships()
        .workspace_id()
        .filter(&workspace_id)
        .any(|m| m.user == user)
    {
        return Err("that person is already a member of this workspace".into());
    }
    ctx.db.memberships().insert(Membership {
        id: 0,
        workspace_id,
        user,
        role,
        created_at: now(ctx),
    });
    Ok(())
}

/// `workspaces.allium: SetMemberRole`. A workspace may be left with no admin.
#[spacetimedb::reducer]
pub fn set_member_role(
    ctx: &ReducerContext,
    membership_id: i64,
    role: String,
) -> Result<(), String> {
    let Some(mut membership) = ctx.db.memberships().id().find(membership_id) else {
        return Err(format!("membership {membership_id} does not exist"));
    };
    require_admin_of(ctx, membership.workspace_id)?;
    validate_workspace_role(&role)?;
    membership.role = role;
    ctx.db.memberships().id().update(membership);
    Ok(())
}

/// `workspaces.allium: RemoveMember`. The person's epics, tasks and packages
/// stay where they are; dropping their subscriptions and unseen marks in the
/// workspace waits for `Epic.workspace` (see the module header).
#[spacetimedb::reducer]
pub fn remove_member(ctx: &ReducerContext, membership_id: i64) -> Result<(), String> {
    let Some(membership) = ctx.db.memberships().id().find(membership_id) else {
        return Err(format!("membership {membership_id} does not exist"));
    };
    require_admin_of(ctx, membership.workspace_id)?;
    ctx.db.memberships().id().delete(membership_id);
    drop_follows_in_workspace(ctx, &membership.user, membership.workspace_id);
    Ok(())
}

/// `RemoveMember`: end `user`'s subscriptions and unseen marks inside
/// `workspace_id`, and nothing in their other workspaces.
fn drop_follows_in_workspace(ctx: &ReducerContext, user: &str, workspace_id: i64) {
    let in_workspace = |epic_id: i64| {
        ctx.db
            .epics()
            .id()
            .find(epic_id)
            .is_some_and(|e| e.workspace_id == workspace_id)
    };
    let subscriptions: Vec<String> = ctx
        .db
        .subscriptions()
        .iter()
        .filter(|s| s.subscriber == user && in_workspace(s.epic_id))
        .map(|s| s.id)
        .collect();
    for id in subscriptions {
        ctx.db.subscriptions().id().delete(&id);
    }
    let marks: Vec<String> = ctx
        .db
        .unseen_tasks()
        .iter()
        .filter(|u| {
            u.subscriber == user
                && ctx
                    .db
                    .tasks()
                    .id()
                    .find(u.task_id)
                    .is_some_and(|t| t.epic_id != 0 && in_workspace(t.epic_id))
        })
        .map(|u| u.id)
        .collect();
    for id in marks {
        ctx.db.unseen_tasks().id().delete(&id);
    }
}

/// `workspaces.allium: MigrateStoreToWorkspaces` (D12). Does nothing once any
/// workspace exists. `pub` for `MemoryReducerCaller`.
pub fn migrate_store_to_workspaces(ctx: &ReducerContext) {
    if ctx.db.workspaces().iter().next().is_some() {
        return;
    }
    let stamp = now(ctx);
    // `store_publisher()`: the store itself, in `caller_identity`'s spelling.
    let default = ctx.db.workspaces().insert(Workspace {
        id: 0,
        name: DEFAULT_WORKSPACE_NAME.into(),
        created_by: ctx.database_identity().to_hex().to_string(),
        created_at: stamp.clone(),
    });
    let epics: Vec<Epic> = ctx.db.epics().iter().collect();
    for epic in epics {
        ctx.db.epics().id().update(Epic {
            workspace_id: default.id,
            visibility: VISIBILITY_PUBLIC.into(),
            ..epic
        });
    }
    let packages: Vec<WorkPackage> = ctx.db.work_packages().iter().collect();
    for package in packages {
        ctx.db.work_packages().id().update(WorkPackage {
            workspace_id: default.id,
            visibility: VISIBILITY_PUBLIC.into(),
            ..package
        });
    }
    let learnings: Vec<Learning> = ctx
        .db
        .learnings()
        .iter()
        .filter(|l| l.scope != "user")
        .collect();
    for learning in learnings {
        ctx.db.learnings().id().update(Learning {
            workspace_id: default.id,
            ..learning
        });
    }
    for person in known_identities(ctx) {
        if profile_of(ctx, &person).is_none() {
            ctx.db.user_profiles().insert(UserProfile {
                identity: person.clone(),
                display_name: String::new(),
                is_super_admin: false,
                created_at: stamp.clone(),
            });
        }
        ctx.db.memberships().insert(Membership {
            id: 0,
            workspace_id: default.id,
            user: person,
            role: ROLE_MEMBER.into(),
            created_at: stamp.clone(),
        });
    }
}

/// Every distinct non-empty `Host.owner`, `Task.owner`, `Task.created_by`,
/// `Epic.created_by`, `WorkPackage.created_by` and `Subscription.subscriber`.
fn known_identities(ctx: &ReducerContext) -> std::collections::BTreeSet<String> {
    let mut people = std::collections::BTreeSet::new();
    people.extend(ctx.db.hosts().iter().map(|h| h.owner));
    for task in ctx.db.tasks().iter() {
        people.insert(task.owner);
        people.insert(task.created_by);
    }
    people.extend(ctx.db.epics().iter().map(|e| e.created_by));
    people.extend(ctx.db.work_packages().iter().map(|p| p.created_by));
    people.extend(ctx.db.subscriptions().iter().map(|s| s.subscriber));
    people.retain(|p| !p.trim().is_empty());
    people
}
