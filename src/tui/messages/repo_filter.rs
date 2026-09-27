//! Repo-filter overlay messages.

use crate::tui::types::Command;
use crate::tui::App;

#[derive(Debug, Clone)]
pub enum RepoFilterMessage {
    Start,
    Close,
    Toggle(String),
    ToggleMode,
    MoveCursor(isize),
    StartDeleteRepoPath,
    DeleteRepoPath(String),
    ToggleOnlyActive,
}

impl RepoFilterMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            RepoFilterMessage::Start => app.handle_start_repo_filter(),
            RepoFilterMessage::Close => app.handle_close_repo_filter(),
            RepoFilterMessage::Toggle(path) => app.handle_toggle_repo_filter(path),
            RepoFilterMessage::ToggleMode => app.handle_toggle_repo_filter_mode(),
            RepoFilterMessage::ToggleOnlyActive => app.handle_toggle_only_active(),
            RepoFilterMessage::MoveCursor(delta) => app.handle_move_repo_cursor(delta),
            RepoFilterMessage::StartDeleteRepoPath => app.handle_start_delete_repo_path(),
            RepoFilterMessage::DeleteRepoPath(path) => app.handle_delete_repo_path(path),
        }
    }
}
