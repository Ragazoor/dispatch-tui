//! Repo filter mode + repo-path delete input handlers.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::types::*;
use super::super::App;
use super::{key_event, key_label};

impl App {
    pub(in crate::tui) fn handle_key_repo_filter(&mut self, key: KeyEvent) -> Vec<Command> {
        let label = key_label(key);
        match key.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => self.dispatch_keyed(
                Message::RepoFilter(crate::tui::messages::RepoFilterMessage::Close),
                "repo_filter_close",
                &label,
            ),
            KeyCode::Char('j') | KeyCode::Down => self.dispatch_keyed(
                Message::RepoFilter(crate::tui::messages::RepoFilterMessage::MoveCursor(1)),
                "repo_filter_move_cursor",
                &label,
            ),
            KeyCode::Char('k') | KeyCode::Up => self.dispatch_keyed(
                Message::RepoFilter(crate::tui::messages::RepoFilterMessage::MoveCursor(-1)),
                "repo_filter_move_cursor",
                &label,
            ),
            KeyCode::Char(' ') => {
                match self.board.repo_paths.get(self.input.repo_cursor).cloned() {
                    Some(path) => self.dispatch_keyed(
                        Message::RepoFilter(crate::tui::messages::RepoFilterMessage::Toggle(path)),
                        "repo_filter_toggle_repo",
                        &label,
                    ),
                    None => vec![],
                }
            }
            KeyCode::Char(c @ '1'..='9') => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.board.repo_paths.len() {
                    let path = self.board.repo_paths[idx].clone();
                    self.dispatch_keyed(
                        Message::RepoFilter(crate::tui::messages::RepoFilterMessage::Toggle(path)),
                        "repo_filter_toggle_repo",
                        &label,
                    )
                } else {
                    vec![]
                }
            }
            KeyCode::Tab => self.dispatch_keyed(
                Message::RepoFilter(crate::tui::messages::RepoFilterMessage::ToggleMode),
                "repo_filter_toggle_mode",
                &label,
            ),
            KeyCode::Backspace | KeyCode::Delete => {
                if self.input.repo_cursor < self.board.repo_paths.len() {
                    self.dispatch_keyed(
                        Message::RepoFilter(
                            crate::tui::messages::RepoFilterMessage::StartDeleteRepoPath,
                        ),
                        "repo_filter_delete_repo_path",
                        &label,
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
        key: KeyEvent,
    ) -> Vec<Command> {
        let label = key_label(key);
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(path) = self.board.repo_paths.get(self.input.repo_cursor).cloned() {
                    self.dispatch_keyed(
                        Message::RepoFilter(
                            crate::tui::messages::RepoFilterMessage::DeleteRepoPath(path),
                        ),
                        "confirm_delete_repo_path_yes",
                        &label,
                    )
                } else {
                    // Cursor no longer points at a deletable row — the prompt
                    // just closes, which is the same outcome as declining.
                    self.input.mode = InputMode::RepoFilter;
                    vec![key_event("confirm_delete_repo_path_no", &label)]
                }
            }
            _ => {
                self.input.mode = InputMode::RepoFilter;
                vec![key_event("confirm_delete_repo_path_no", &label)]
            }
        }
    }
}
