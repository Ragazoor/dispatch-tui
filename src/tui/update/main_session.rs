use crate::models::expand_tilde;

use super::super::types::*;
use super::super::App;

impl App {
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
        self.input.buffer.clear();
        vec![
            Command::PersistStringSetting {
                key: "main_session.dir".to_string(),
                value: expanded,
            },
            Command::MainSession(crate::tui::commands::MainSessionCommand::Open),
        ]
    }

    pub(in crate::tui) fn handle_main_session_created(&mut self, window: String) -> Vec<Command> {
        self.main_session = Some(window.clone());
        vec![Command::PersistStringSetting {
            key: "main_session.window".to_string(),
            value: window,
        }]
    }
}
