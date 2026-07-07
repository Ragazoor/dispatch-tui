use crate::models::expand_tilde;

use super::super::types::*;
use super::super::App;

impl App {
    /// Open the repo picker so the user can (re)select the main-session
    /// directory. Driven by the runtime when `:` is pressed and no main-session
    /// window is alive.
    pub(in crate::tui) fn handle_configure_main_session(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::MainSessionDir;
        self.set_status("Type to filter · ↑/↓ navigate · Enter select · Esc cancel".to_string());
        vec![]
    }

    pub(in crate::tui) fn handle_submit_main_session_dir(&mut self, dir: String) -> Vec<Command> {
        let trimmed = dir.trim();
        if trimmed.is_empty() {
            return self.update(Message::Input(
                crate::tui::messages::InputMessage::CancelInput,
            ));
        }
        let expanded = expand_tilde(trimmed);
        self.main_session_dir = Some(expanded.clone());
        self.input.mode = InputMode::Normal;
        self.input.clear_buffer();
        vec![
            Command::PersistStringSetting {
                key: "main_session.dir".to_string(),
                value: expanded,
            },
            Command::MainSession(crate::tui::commands::MainSessionCommand::Create),
        ]
    }
}
