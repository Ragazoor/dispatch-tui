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
        if tables
            .epics
            .values()
            .any(|e| e.workspace_id == workspace_id)
            || tables
                .work_packages
                .values()
                .any(|p| p.workspace_id == workspace_id)
        {
            return refused(format!(
                "workspace {workspace_id} still holds epics or work packages; empty it first"
            ));
        }
        let learnings: Vec<i64> = tables
            .learnings
            .values()
            .filter(|l| l.workspace_id == workspace_id)
            .map(|l| l.id)
            .collect();
        for id in learnings {
            self.delete_learning_cascade(&mut tables, id);
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
        let Some(membership) = tables.memberships.remove(&membership_id) else {
            return applied();
        };
        self.rows.remove_membership(membership_id);
        self.drop_follows_in_workspace(&mut tables, &membership.user, workspace_id);
        applied()
    }

    /// `RemoveMember`: end `user`'s subscriptions and unseen marks inside
    /// `workspace_id`, and nothing in their other workspaces.
    fn drop_follows_in_workspace(&self, tables: &mut Tables, user: &str, workspace_id: i64) {
        let in_workspace = |tables: &Tables, epic_id: i64| {
            tables
                .epics
                .get(&epic_id)
                .is_some_and(|e| e.workspace_id == workspace_id)
        };
        let subscriptions: Vec<String> = tables
            .subscriptions
            .values()
            .filter(|s| s.subscriber == user && in_workspace(tables, s.epic_id))
            .map(|s| s.id.clone())
            .collect();
        for id in subscriptions {
            tables.subscriptions.remove(&id);
            self.rows.remove_subscription(id);
        }
        let marks: Vec<String> = tables
            .unseen_tasks
            .values()
            .filter(|u| {
                u.subscriber == user
                    && tables
                        .tasks
                        .get(&u.task_id)
                        .is_some_and(|t| t.epic_id != 0 && in_workspace(tables, t.epic_id))
            })
            .map(|u| u.id.clone())
            .collect();
        for id in marks {
            tables.unseen_tasks.remove(&id);
            self.rows.remove_unseen_task(id);
        }
    }

    /// `MigrateStoreToWorkspaces`, as the module's connect handler runs it on
    /// the first connection after the publish. A memory store has no connect
    /// event, so a fixture calls this where one would. Does nothing once a
    /// workspace exists.
    pub fn migrate_store_to_workspaces(&self) {
        let mut tables = self.lock();
        if !tables.workspaces.is_empty() {
            return;
        }
        let stamp = self.now();
        let id = Self::assign_id(0, &mut tables.next_workspace_id);
        let default = module::Workspace {
            id,
            name: module::DEFAULT_WORKSPACE_NAME.to_string(),
            // `store_publisher()`: a memory store has no database identity.
            created_by: String::new(),
            created_at: stamp.clone(),
        };
        tables.workspaces.insert(id, default.clone());
        self.rows.upsert_workspace(&default.into());
        for epic in tables.epics.values_mut() {
            epic.workspace_id = id;
            epic.visibility = module::VISIBILITY_PUBLIC.to_string();
            self.rows.upsert_epic(&epic.clone().into());
        }
        for package in tables.work_packages.values_mut() {
            package.workspace_id = id;
            package.visibility = module::VISIBILITY_PUBLIC.to_string();
            self.rows.upsert_work_package(&package.clone().into());
        }
        for learning in tables.learnings.values_mut().filter(|l| l.scope != "user") {
            learning.workspace_id = id;
            self.rows.upsert_learning(&learning.clone().into());
        }
        let mut people = std::collections::BTreeSet::new();
        people.extend(tables.hosts.values().map(|h| h.owner.clone()));
        for task in tables.tasks.values() {
            people.insert(task.owner.clone());
            people.insert(task.created_by.clone());
        }
        people.extend(tables.epics.values().map(|e| e.created_by.clone()));
        people.extend(tables.work_packages.values().map(|p| p.created_by.clone()));
        people.extend(tables.subscriptions.values().map(|s| s.subscriber.clone()));
        people.retain(|p| !p.trim().is_empty());
        for person in people {
            if !tables.user_profiles.contains_key(&person) {
                let profile = module::UserProfile {
                    identity: person.clone(),
                    display_name: String::new(),
                    is_super_admin: false,
                    created_at: stamp.clone(),
                };
                tables.user_profiles.insert(person.clone(), profile.clone());
                self.rows.upsert_user_profile(&profile.into());
            }
            let membership_id = Self::assign_id(0, &mut tables.next_membership_id);
            let membership = module::Membership {
                id: membership_id,
                workspace_id: id,
                user: person,
                role: module::ROLE_MEMBER.to_string(),
                created_at: stamp.clone(),
            };
            tables.memberships.insert(membership_id, membership.clone());
            self.rows.upsert_membership(&membership.into());
        }
    }
}
