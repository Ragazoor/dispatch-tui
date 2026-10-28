//! Super-admins, workspaces and membership. Mirrors the module's `workspace.rs`.

use anyhow::Result;

use dispatch_spacetime_module as module;

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

fn refused(why: impl Into<String>) -> Result<ReducerOutcome> {
    Ok(ReducerOutcome::Refused(why.into()))
}

fn applied() -> Result<ReducerOutcome> {
    Ok(ReducerOutcome::Applied(vec![]))
}

impl MemoryReducerCaller {
    fn caller_is_super_admin(&self, tables: &Tables) -> bool {
        tables
            .user_profiles
            .get(&self.sender)
            .is_some_and(|p| p.is_super_admin)
    }

    /// `is_super_admin(caller()) or workspace.is_admin(caller())`.
    fn caller_administers(&self, tables: &Tables, workspace_id: i64) -> bool {
        self.caller_is_super_admin(tables)
            || tables.memberships.values().any(|m| {
                m.workspace_id == workspace_id
                    && m.user == self.sender
                    && m.role == module::ROLE_ADMIN
            })
    }

    fn save_profile(&self, tables: &mut Tables, identity: &str) {
        if let Some(row) = tables.user_profiles.get(identity) {
            self.rows.upsert_user_profile(&row.clone().into());
        }
    }

    pub(super) fn apply_claim_super_admin(&self) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if !tables.user_profiles.contains_key(&self.sender) {
            return refused("no profile for this identity; reconnect and try again");
        }
        if tables.user_profiles.values().any(|p| p.is_super_admin) {
            return refused("a super-admin already exists");
        }
        if let Some(profile) = tables.user_profiles.get_mut(&self.sender) {
            profile.is_super_admin = true;
        }
        let sender = self.sender.clone();
        self.save_profile(&mut tables, &sender);
        let ownerless: Vec<i64> = tables
            .tasks
            .values()
            .filter(|t| t.epic_id == 0 && t.owner.trim().is_empty())
            .map(|t| t.id)
            .collect();
        for id in ownerless {
            if let Some(task) = tables.tasks.get_mut(&id) {
                task.owner = self.sender.clone();
                self.rows.upsert_task(&task.clone().into());
            }
        }
        applied()
    }

    /// `GrantSuperAdmin` when `flag`, `RevokeSuperAdmin` otherwise.
    pub(super) fn apply_set_super_admin(&self, user: String, flag: bool) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if !self.caller_is_super_admin(&tables) {
            return refused("only a super-admin may do this");
        }
        let Some(profile) = tables.user_profiles.get(&user) else {
            return refused(format!("no person with identity {user}"));
        };
        if !flag {
            if !profile.is_super_admin {
                return refused("that person is not a super-admin");
            }
            if tables
                .user_profiles
                .values()
                .filter(|p| p.is_super_admin)
                .count()
                <= 1
            {
                return refused("cannot revoke the last super-admin");
            }
        }
        if let Some(profile) = tables.user_profiles.get_mut(&user) {
            profile.is_super_admin = flag;
        }
        self.save_profile(&mut tables, &user);
        applied()
    }

    pub(super) fn apply_create_workspace(&self, name: String) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if !self.caller_is_super_admin(&tables) {
            return refused("only a super-admin may do this");
        }
        let name = match module::validate_workspace_name(&name) {
            Ok(name) => name,
            Err(why) => return refused(why),
        };
        if tables.workspaces.values().any(|w| w.name == name) {
            return refused(format!("a workspace named '{name}' already exists"));
        }
        let id = Self::assign_id(0, &mut tables.next_workspace_id);
        let row = module::Workspace {
            id,
            name,
            created_by: self.sender.clone(),
            created_at: self.now(),
        };
        tables.workspaces.insert(id, row.clone());
        self.rows.upsert_workspace(&row.into());
        applied()
    }

    pub(super) fn apply_rename_workspace(
        &self,
        workspace_id: i64,
        name: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if !tables.workspaces.contains_key(&workspace_id) {
            return refused(format!("workspace {workspace_id} does not exist"));
        }
        if !self.caller_administers(&tables, workspace_id) {
            return refused("only a super-admin or an admin of this workspace may do this");
        }
        let name = match module::validate_workspace_name(&name) {
            Ok(name) => name,
            Err(why) => return refused(why),
        };
        if tables
            .workspaces
            .values()
            .any(|w| w.name == name && w.id != workspace_id)
        {
            return refused(format!("a workspace named '{name}' already exists"));
        }
        if let Some(workspace) = tables.workspaces.get_mut(&workspace_id) {
            workspace.name = name;
            self.rows.upsert_workspace(&workspace.clone().into());
        }
        applied()
    }

    pub(super) fn apply_delete_workspace(&self, workspace_id: i64) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if !self.caller_is_super_admin(&tables) {
            return refused("only a super-admin may do this");
        }
        if !tables.workspaces.contains_key(&workspace_id) {
            return refused(format!("workspace {workspace_id} does not exist"));
        }
        let memberships: Vec<i64> = tables
            .memberships
            .values()
            .filter(|m| m.workspace_id == workspace_id)
            .map(|m| m.id)
            .collect();
        for id in memberships {
            tables.memberships.remove(&id);
            self.rows.remove_membership(id);
        }
        tables.workspaces.remove(&workspace_id);
        self.rows.remove_workspace(workspace_id);
        applied()
    }

    pub(super) fn apply_add_member(
        &self,
        workspace_id: i64,
        user: String,
        role: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if !tables.workspaces.contains_key(&workspace_id) {
            return refused(format!("workspace {workspace_id} does not exist"));
        }
        if !self.caller_administers(&tables, workspace_id) {
            return refused("only a super-admin or an admin of this workspace may do this");
        }
        if let Err(why) = module::validate_workspace_role(&role) {
            return refused(why);
        }
        if !tables.user_profiles.contains_key(&user) {
            return refused(format!("no person with identity {user}"));
        }
        if tables
            .memberships
            .values()
            .any(|m| m.workspace_id == workspace_id && m.user == user)
        {
            return refused("that person is already a member of this workspace");
        }
        let id = Self::assign_id(0, &mut tables.next_membership_id);
        let row = module::Membership {
            id,
            workspace_id,
            user,
            role,
            created_at: self.now(),
        };
        tables.memberships.insert(id, row.clone());
        self.rows.upsert_membership(&row.into());
        applied()
    }

    pub(super) fn apply_set_member_role(
        &self,
        membership_id: i64,
        role: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let Some(workspace_id) = tables
            .memberships
            .get(&membership_id)
            .map(|m| m.workspace_id)
        else {
            return refused(format!("membership {membership_id} does not exist"));
        };
        if !self.caller_administers(&tables, workspace_id) {
            return refused("only a super-admin or an admin of this workspace may do this");
        }
        if let Err(why) = module::validate_workspace_role(&role) {
            return refused(why);
        }
        if let Some(membership) = tables.memberships.get_mut(&membership_id) {
            membership.role = role;
            self.rows.upsert_membership(&membership.clone().into());
        }
        applied()
    }

    pub(super) fn apply_remove_member(&self, membership_id: i64) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let Some(workspace_id) = tables
            .memberships
            .get(&membership_id)
            .map(|m| m.workspace_id)
        else {
            return refused(format!("membership {membership_id} does not exist"));
        };
        if !self.caller_administers(&tables, workspace_id) {
            return refused("only a super-admin or an admin of this workspace may do this");
        }
        tables.memberships.remove(&membership_id);
        self.rows.remove_membership(membership_id);
        applied()
    }
}
