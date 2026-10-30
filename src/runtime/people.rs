//! People side effects (`docs/specs/workspaces.allium`): the board sets its
//! own display name through the people service.

use super::*;
use crate::tui::commands::PeopleCommand;

impl TuiRuntime {
    pub(super) async fn exec_people_command(&self, app: &mut App, cmd: PeopleCommand) {
        use crate::tui::messages::SystemMessage;
        match cmd {
            PeopleCommand::LoadWorkspaces => {
                let mine = self.people_svc.my_workspaces().await;
                app.update(Message::Input(
                    crate::tui::messages::InputMessage::WorkspacesLoaded(mine),
                ));
            }
            PeopleCommand::LoadAdmin => self.reload_admin(app).await,
            PeopleCommand::Admin(write) => match self.write_admin(write).await {
                Ok(done) => {
                    app.update(Message::System(SystemMessage::StatusInfo(done)));
                    self.reload_admin(app).await;
                }
                Err(e) => {
                    app.update(Message::System(SystemMessage::Error(Self::db_error(
                        "administering workspaces",
                        e,
                    ))));
                }
            },
            PeopleCommand::SetDisplayName(name) => {
                match self.people_svc.set_display_name(&name).await {
                    Ok(saved) => {
                        app.update(Message::System(SystemMessage::StatusInfo(format!(
                            "Display name set to {saved}"
                        ))));
                    }
                    Err(e) => {
                        app.update(Message::System(SystemMessage::Error(Self::db_error(
                            "setting display name",
                            e,
                        ))));
                    }
                }
            }
        }
    }

    /// Read the admin snapshot and hand it to the screen.
    async fn reload_admin(&self, app: &mut App) {
        let snapshot = self.people_svc.admin_snapshot().await;
        app.update(Message::Admin(crate::tui::messages::AdminMessage::Loaded(
            Box::new(snapshot),
        )));
    }

    /// One write of the admin screen; answers the sentence for the status line.
    async fn write_admin(
        &self,
        write: crate::tui::commands::AdminWrite,
    ) -> Result<String, crate::service::ServiceError> {
        use crate::models::MemberRole;
        use crate::tui::commands::AdminWrite as W;
        let svc = &self.people_svc;
        match write {
            W::Claim => {
                svc.claim_super_admin().await?;
                Ok("You are now a super-admin".to_string())
            }
            W::CreateWorkspace(name) => {
                svc.create_workspace(&name).await?;
                Ok(format!("Workspace {} created", name.trim()))
            }
            W::RenameWorkspace { id, name } => {
                svc.rename_workspace(id, &name).await?;
                Ok(format!("Workspace renamed to {}", name.trim()))
            }
            W::DeleteWorkspace(id) => {
                let name = svc.delete_workspace(id).await?;
                Ok(format!("Workspace {name} deleted"))
            }
            W::AddMember {
                workspace_id,
                person,
            } => {
                let name = svc
                    .add_member(workspace_id, &person, MemberRole::Member)
                    .await?;
                Ok(format!("Added {name}"))
            }
            W::SetRole {
                membership_id,
                role,
            } => {
                svc.set_member_role(membership_id, role).await?;
                Ok(format!("Role is now {}", role.as_str()))
            }
            W::RemoveMember(membership_id) => {
                svc.remove_member(membership_id).await?;
                Ok("Member removed".to_string())
            }
            W::SetSuperAdmin { person, granted } => {
                let name = svc.set_super_admin(&person, granted).await?;
                Ok(if granted {
                    format!("{name} is a super-admin")
                } else {
                    format!("{name} is no longer a super-admin")
                })
            }
            W::DeleteUnseen { kind, id } => {
                svc.delete_unseen_item(&kind, id).await?;
                Ok(format!("Deleted {kind} {id}"))
            }
        }
    }
}
