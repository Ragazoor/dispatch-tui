//! People and display names (`docs/specs/workspaces.allium`): the one write a
//! board makes about a person is their own name.

use anyhow::Result;

use super::super::Store;

#[async_trait::async_trait]
impl super::super::PeopleStore for Store {
    async fn set_display_name(&self, name: &str) -> Result<String> {
        self.caller
            .set_display_name(name.to_string())
            .await?
            .applied()?;
        Ok(name.trim().to_string())
    }

    async fn my_workspaces(&self) -> Vec<crate::models::WorkspaceRef> {
        let Ok(Some(identity)) = self.identity.user().await else {
            return Vec::new();
        };
        let mine: std::collections::HashSet<i64> = self
            .rows
            .memberships()
            .into_iter()
            .filter(|m| m.user == identity)
            .map(|m| m.workspace_id)
            .collect();
        self.rows
            .workspaces()
            .into_iter()
            .filter(|w| mine.contains(&w.id))
            .map(|w| crate::models::WorkspaceRef {
                id: w.id,
                name: w.name,
            })
            .collect()
    }

    async fn admin_snapshot(&self) -> crate::models::AdminSnapshot {
        use crate::models::{
            shown_name, AdminSnapshot, DirectoryPerson, MemberRole, UnseenItemRow, WorkspaceDetail,
            WorkspaceMember,
        };
        let me = self.identity.user().await.ok().flatten();
        let profiles = self.rows.user_profiles();
        let name_of = |identity: &str| {
            let display = profiles
                .iter()
                .find(|p| p.identity == identity)
                .map_or("", |p| p.display_name.as_str());
            shown_name(display, identity)
        };
        let mut directory: Vec<DirectoryPerson> = profiles
            .iter()
            .map(|p| DirectoryPerson {
                identity: p.identity.clone(),
                shown_name: shown_name(&p.display_name, &p.identity),
                is_super_admin: p.is_super_admin,
            })
            .collect();
        directory.sort_by(|a, b| {
            (a.shown_name.to_lowercase(), &a.identity)
                .cmp(&(b.shown_name.to_lowercase(), &b.identity))
        });
        let i_am_super_admin = me.as_deref().is_some_and(|me| {
            profiles
                .iter()
                .any(|p| p.identity == me && p.is_super_admin)
        });
        let memberships = self.rows.memberships();
        let workspaces_rows = self.rows.workspaces();
        let mut workspaces: Vec<WorkspaceDetail> = workspaces_rows
            .iter()
            .filter(|w| {
                i_am_super_admin
                    || me.as_deref().is_some_and(|me| {
                        memberships
                            .iter()
                            .any(|m| m.workspace_id == w.id && m.user == me)
                    })
            })
            .map(|w| {
                let mut members: Vec<WorkspaceMember> = memberships
                    .iter()
                    .filter(|m| m.workspace_id == w.id)
                    .map(|m| WorkspaceMember {
                        membership_id: m.id,
                        identity: m.user.clone(),
                        shown_name: name_of(&m.user),
                        role: MemberRole::parse(&m.role).unwrap_or(MemberRole::Member),
                    })
                    .collect();
                members.sort_by(|a, b| {
                    (a.shown_name.to_lowercase(), &a.identity)
                        .cmp(&(b.shown_name.to_lowercase(), &b.identity))
                });
                WorkspaceDetail {
                    id: w.id,
                    name: w.name.clone(),
                    members,
                }
            })
            .collect();
        workspaces
            .sort_by(|a, b| (a.name.to_lowercase(), a.id).cmp(&(b.name.to_lowercase(), b.id)));
        let workspace_name = |id: i64| {
            workspaces_rows
                .iter()
                .find(|w| w.id == id)
                .map_or_else(|| format!("workspace {id}"), |w| w.name.clone())
        };
        let unseen = self
            .rows
            .unseen_items()
            .into_iter()
            .map(|i| UnseenItemRow {
                workspace_name: workspace_name(i.workspace_id),
                creator: if i.creator.is_empty() {
                    "nobody".to_string()
                } else {
                    name_of(&i.creator)
                },
                kind: i.kind,
                id: i.item_id,
                workspace_id: i.workspace_id,
                created_at: i.created_at,
            })
            .collect();
        AdminSnapshot {
            me,
            i_am_super_admin,
            claimable: !profiles.iter().any(|p| p.is_super_admin),
            workspaces,
            directory,
            unseen,
        }
    }

    async fn claim_super_admin(&self) -> Result<()> {
        self.caller.claim_super_admin().await?.applied()
    }

    async fn set_super_admin(&self, identity: &str, granted: bool) -> Result<()> {
        let identity = identity.to_string();
        if granted {
            self.caller.grant_super_admin(identity).await?.applied()
        } else {
            self.caller.revoke_super_admin(identity).await?.applied()
        }
    }

    async fn create_workspace(&self, name: &str) -> Result<()> {
        self.caller
            .create_workspace(name.to_string())
            .await?
            .applied()
    }

    async fn rename_workspace(&self, workspace_id: i64, name: &str) -> Result<()> {
        self.caller
            .rename_workspace(workspace_id, name.to_string())
            .await?
            .applied()
    }

    async fn delete_workspace(&self, workspace_id: i64) -> Result<()> {
        self.caller.delete_workspace(workspace_id).await?.applied()
    }

    async fn add_member(
        &self,
        workspace_id: i64,
        identity: &str,
        role: crate::models::MemberRole,
    ) -> Result<()> {
        self.caller
            .add_member(
                workspace_id,
                identity.to_string(),
                role.as_str().to_string(),
            )
            .await?
            .applied()
    }

    async fn set_member_role(
        &self,
        membership_id: i64,
        role: crate::models::MemberRole,
    ) -> Result<()> {
        self.caller
            .set_member_role(membership_id, role.as_str().to_string())
            .await?
            .applied()
    }

    async fn remove_member(&self, membership_id: i64) -> Result<()> {
        self.caller.remove_member(membership_id).await?.applied()
    }

    async fn delete_unseen_item(&self, kind: &str, item_id: i64) -> Result<()> {
        self.caller
            .super_admin_delete_unseen_item(kind.to_string(), item_id)
            .await?
            .applied()
    }
}
