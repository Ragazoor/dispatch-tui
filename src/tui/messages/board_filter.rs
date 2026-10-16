//! Repo-filter overlay messages.

use crate::tui::types::Command;
use crate::tui::App;

#[derive(Debug, Clone)]
pub enum BoardFilterMessage {
    Start,
    Close,
    Toggle(String),
    ToggleMode,
    MoveCursor(isize),
    StartDeleteRepoPath,
    DeleteRepoPath(String),
}

impl BoardFilterMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            BoardFilterMessage::Start => app.handle_start_board_filter(),
            BoardFilterMessage::Close => app.handle_close_board_filter(),
            BoardFilterMessage::Toggle(path) => app.handle_toggle_repo_filter(path),
            BoardFilterMessage::ToggleMode => app.handle_toggle_repo_filter_mode(),
            BoardFilterMessage::MoveCursor(delta) => app.handle_move_repo_cursor(delta),
            BoardFilterMessage::StartDeleteRepoPath => app.handle_start_delete_repo_path(),
            BoardFilterMessage::DeleteRepoPath(path) => app.handle_delete_repo_path(path),
        }
    }
}
