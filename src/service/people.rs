//! People and display names (`docs/specs/workspaces.allium`): the service a
//! keypress and an MCP call both set the caller's own name through. The data
//! rules (trimmed, non-empty, at most 64 characters) live in the store's
//! `set_display_name` reducer.

use std::sync::Arc;

use crate::models::{resolve_person, AdminSnapshot, DirectoryPerson, MemberRole};
use crate::store;

use super::ServiceError;

pub struct PeopleService {
    db: Arc<dyn store::PeopleStore>,
}

impl PeopleService {
    pub fn new(db: Arc<dyn store::PeopleStore>) -> Self {
        Self { db }
    }

    /// `SetDisplayName`: answers the saved (trimmed) name.
    pub async fn set_display_name(&self, name: &str) -> Result<String, ServiceError> {
        Ok(self.db.set_display_name(name).await?)
    }

    /// The workspaces the caller belongs to: what a root epic or package may
    /// be created in.
    pub async fn my_workspaces(&self) -> Vec<crate::models::WorkspaceRef> {
        self.db.my_workspaces().await
    }

    /// What the admin screen and the list tools show
    /// (`admin-surfaces.allium: OpenAdminScreen`, `ListWorkspacesTool`).
    pub async fn admin_snapshot(&self) -> AdminSnapshot {
        self.db.admin_snapshot().await
    }

    /// `workspaces.allium: ClaimSuperAdmin`.
    pub async fn claim_super_admin(&self) -> Result<(), ServiceError> {
        Ok(self.db.claim_super_admin().await?)
    }

    /// `GrantSuperAdmin` or `RevokeSuperAdmin` for the person `person` names
    /// (`admin-surfaces.allium: PersonResolution`). Granting a super-admin or
    /// revoking a non-super-admin changes nothing and succeeds. Answers the
    /// person's shown name.
    pub async fn set_super_admin(
        &self,
        person: &str,
        granted: bool,
    ) -> Result<String, ServiceError> {
        let who = self.resolve(person).await?;
        if who.is_super_admin != granted {
            self.db.set_super_admin(&who.identity, granted).await?;
        }
        Ok(who.shown_name)
    }

    /// `CreateWorkspace`.
    pub async fn create_workspace(&self, name: &str) -> Result<(), ServiceError> {
        Ok(self.db.create_workspace(name).await?)
    }

    /// `RenameWorkspace`.
    pub async fn rename_workspace(
        &self,
        workspace_id: i64,
        name: &str,
    ) -> Result<(), ServiceError> {
        self.known_workspace(workspace_id).await?;
        Ok(self.db.rename_workspace(workspace_id, name).await?)
    }

    /// `DeleteWorkspace`. Answers the workspace's name.
    pub async fn delete_workspace(&self, workspace_id: i64) -> Result<String, ServiceError> {
        let name = self.known_workspace(workspace_id).await?;
        self.db.delete_workspace(workspace_id).await?;
        Ok(name)
    }

    /// `AddMember` for the person `person` names. Answers their shown name.
    pub async fn add_member(
        &self,
        workspace_id: i64,
        person: &str,
        role: MemberRole,
    ) -> Result<String, ServiceError> {
        self.known_workspace(workspace_id).await?;
        let who = self.resolve(person).await?;
        self.db
            .add_member(workspace_id, &who.identity, role)
            .await?;
        Ok(who.shown_name)
    }

    /// One call for the three membership changes of
    /// `admin-surfaces.allium: SetWorkspaceMemberTool`: `Some(role)` adds the
    /// person or changes their role, `None` removes them. Answers a sentence
    /// saying what happened.
    pub async fn set_member(
        &self,
        workspace_id: i64,
        person: &str,
        role: Option<MemberRole>,
    ) -> Result<String, ServiceError> {
        let workspace = self.known_workspace(workspace_id).await?;
        let who = self.resolve(person).await?;
        let existing = self
            .db
            .admin_snapshot()
            .await
            .workspaces
            .into_iter()
            .find(|w| w.id == workspace_id)
            .and_then(|w| w.members.into_iter().find(|m| m.identity == who.identity));
        match (role, existing) {
            (None, Some(m)) => {
                self.db.remove_member(m.membership_id).await?;
                Ok(format!("Removed {} from {workspace}", who.shown_name))
            }
            (None, None) => Err(ServiceError::Validation(format!(
                "{} is not a member of {workspace}",
                who.shown_name
            ))),
            (Some(role), Some(m)) => {
                if m.role != role {
                    self.db.set_member_role(m.membership_id, role).await?;
                }
                Ok(format!(
                    "{} is {} of {workspace}",
                    who.shown_name,
                    role.as_str()
                ))
            }
            (Some(role), None) => {
                self.db
                    .add_member(workspace_id, &who.identity, role)
                    .await?;
                Ok(format!(
                    "Added {} to {workspace} as {}",
                    who.shown_name,
                    role.as_str()
                ))
            }
        }
    }

    /// `SetMemberRole` for a membership the admin screen holds.
    pub async fn set_member_role(
        &self,
        membership_id: i64,
        role: MemberRole,
    ) -> Result<(), ServiceError> {
        Ok(self.db.set_member_role(membership_id, role).await?)
    }

    /// `RemoveMember` for a membership the admin screen holds.
    pub async fn remove_member(&self, membership_id: i64) -> Result<(), ServiceError> {
        Ok(self.db.remove_member(membership_id).await?)
    }

    /// `SuperAdminDeletesUnseenItem`. `kind` is `epic` or `work_package`.
    pub async fn delete_unseen_item(&self, kind: &str, item_id: i64) -> Result<(), ServiceError> {
        if kind != "epic" && kind != "work_package" {
            return Err(ServiceError::Validation(format!(
                "kind is 'epic' or 'work_package', not '{kind}'"
            )));
        }
        Ok(self.db.delete_unseen_item(kind, item_id).await?)
    }

    async fn resolve(&self, person: &str) -> Result<DirectoryPerson, ServiceError> {
        let snapshot = self.db.admin_snapshot().await;
        resolve_person(&snapshot.directory, person)
            .cloned()
            .map_err(ServiceError::Validation)
    }

    /// The name of a workspace the caller can see, or the refusal a missing
    /// one gets (an unseen workspace is answered as a missing one).
    async fn known_workspace(&self, workspace_id: i64) -> Result<String, ServiceError> {
        self.db
            .admin_snapshot()
            .await
            .workspaces
            .into_iter()
            .find(|w| w.id == workspace_id)
            .map(|w| w.name)
            .ok_or_else(|| {
                ServiceError::NotFound(format!(
                    "no workspace {workspace_id}; list_workspaces shows the ones you can see"
                ))
            })
    }
}
