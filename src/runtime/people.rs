//! People side effects (`docs/specs/workspaces.allium`): the board sets its
//! own display name through the people service.

use super::*;
use crate::tui::commands::PeopleCommand;

impl TuiRuntime {
    pub(super) async fn exec_people_command(&self, app: &mut App, cmd: PeopleCommand) {
        use crate::tui::messages::SystemMessage;
        match cmd {
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
}
