//! Repo filter mode + repo-path delete input handlers.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::types::*;
use super::super::App;
use super::key_event;

impl App {
    /// `action` is the row's action id; `key` supplies the direction and the
    /// digit where one row covers several keys.
    pub(in crate::tui) fn handle_key_repo_filter(
        &mut self,
        key: KeyEvent,
        action: &str,
        label: &str,
    ) -> Vec<Command> {
        match action {
            "repo_filter_close" => self.dispatch_keyed(
                Message::RepoFilter(crate::tui::messages::RepoFilterMessage::Close),
                action,
                label,
            ),
            "repo_filter_move_cursor" => {
                let delta = if matches!(key.code, KeyCode::Char('j') | KeyCode::Down) {
                    1
                } else {
                    -1
                };
                self.dispatch_keyed(
                    Message::RepoFilter(crate::tui::messages::RepoFilterMessage::MoveCursor(delta)),
                    action,
                    label,
                )
            }
            "repo_filter_toggle_repo" => {
                let idx = match key.code {
                    KeyCode::Char(c @ '1'..='9') => (c as usize) - ('1' as usize),
                    _ => self.input.repo_cursor,
                };
                match self.board.repo_paths.get(idx).cloned() {
                    Some(path) => self.dispatch_keyed(
                        Message::RepoFilter(crate::tui::messages::RepoFilterMessage::Toggle(path)),
                        action,
                        label,
                    ),
                    None => vec![],
                }
            }
            "repo_filter_toggle_mode" => self.dispatch_keyed(
                Message::RepoFilter(crate::tui::messages::RepoFilterMessage::ToggleMode),
                action,
                label,
            ),
            "repo_filter_delete_repo_path" => {
                if self.input.repo_cursor < self.board.repo_paths.len() {
                    self.dispatch_keyed(
                        Message::RepoFilter(
                            crate::tui::messages::RepoFilterMessage::StartDeleteRepoPath,
                        ),
                        action,
                        label,
                    )
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_confirm_delete_repo_path(
        &mut self,
        label: &str,
        yes: bool,
    ) -> Vec<Command> {
        if yes {
            if let Some(path) = self.board.repo_paths.get(self.input.repo_cursor).cloned() {
                return self.dispatch_keyed(
                    Message::RepoFilter(crate::tui::messages::RepoFilterMessage::DeleteRepoPath(
                        path,
                    )),
                    "confirm_delete_repo_path_yes",
                    label,
                );
            }
        }
        // Declined, or the cursor no longer points at a deletable row — the
        // prompt just closes, which is the same outcome as declining.
        self.input.mode = InputMode::RepoFilter;
        vec![key_event("confirm_delete_repo_path_no", label)]
    }
}
