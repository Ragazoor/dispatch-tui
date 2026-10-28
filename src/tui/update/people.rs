//! The person's own display name (`docs/specs/workspaces.allium:
//! SetOwnNameFromBoard`).

use crate::tui::commands::PeopleCommand;

use super::super::types::*;
use super::super::App;

impl App {
    /// `u` in the board's normal mode: open the one-line name input, empty.
    pub(in crate::tui) fn handle_start_set_display_name(&mut self) -> Vec<Command> {
        self.input.clear_buffer();
        self.input.mode = InputMode::InputDisplayName;
        self.status.clear();
        vec![]
    }

    /// Enter in the name input. A blank entry sends nothing.
    pub(in crate::tui) fn handle_submit_display_name(&mut self, name: String) -> Vec<Command> {
        self.input.clear_buffer();
        self.input.mode = InputMode::Normal;
        self.status.clear();
        let name = name.trim().to_string();
        if name.is_empty() {
            return vec![];
        }
        vec![Command::People(PeopleCommand::SetDisplayName(name))]
    }
}
