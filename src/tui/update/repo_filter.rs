//! Repo filter handlers.

use super::super::types::*;
use super::super::{filtered_repos, has_new_repo_option, App};

impl App {
    pub(in crate::tui) fn handle_start_repo_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::RepoFilter;
        self.input.repo_cursor = 0;
        vec![]
    }

    pub(in crate::tui) fn handle_move_repo_cursor(&mut self, delta: isize) -> Vec<Command> {
        let count = if let Some(candidates) = self.picker_candidates() {
            let filtered = filtered_repos(candidates, &self.input.buffer);
            let extra = has_new_repo_option(&self.input.buffer, &filtered) as usize;
            filtered.len() + extra
        } else {
            self.board.repo_paths.len()
        };
        if count == 0 {
            return vec![];
        }
        self.input.repo_cursor =
            (self.input.repo_cursor as isize + delta).rem_euclid(count as isize) as usize;
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_close_repo_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.sync_board_selection();
        self.reset_column_scroll();
        let mut paths: Vec<_> = self.filter.repos.iter().cloned().collect();
        paths.sort();
        let value = serde_json::to_string(&paths).unwrap_or_else(|_| "[]".to_string());
        let mode_value = self.filter.mode.as_str();
        vec![
            Command::Settings(
                crate::tui::commands::SettingsCommand::PersistStringSetting {
                    key: "repo_filter".to_string(),
                    value,
                },
            ),
            Command::Settings(
                crate::tui::commands::SettingsCommand::PersistStringSetting {
                    key: "repo_filter_mode".to_string(),
                    value: mode_value.to_string(),
                },
            ),
        ]
    }

    pub(in crate::tui) fn handle_toggle_repo_filter(&mut self, path: String) -> Vec<Command> {
        if self.filter.repos.contains(&path) {
            self.filter.repos.remove(&path);
        } else {
            self.filter.repos.insert(path);
        }
        self.sync_board_selection();
        self.reset_column_scroll();
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_toggle_repo_filter_mode(&mut self) -> Vec<Command> {
        self.filter.mode = match self.filter.mode {
            RepoFilterMode::Include => RepoFilterMode::Exclude,
            RepoFilterMode::Exclude => RepoFilterMode::Include,
        };
        self.sync_board_selection();
        self.reset_column_scroll();
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_toggle_only_active(&mut self) -> Vec<Command> {
        self.filter.only_active = !self.filter.only_active;
        self.sync_board_selection();
        self.reset_column_scroll();
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_start_delete_repo_path(&mut self) -> Vec<Command> {
        if self.board.repo_paths.is_empty() {
            return vec![];
        }
        self.input.mode = InputMode::ConfirmDeleteRepoPath;
        vec![]
    }

    pub(in crate::tui) fn handle_delete_repo_path(&mut self, path: String) -> Vec<Command> {
        self.filter.repos.remove(&path);
        self.input.mode = InputMode::RepoFilter;
        self.status.set("Deleted repo path".to_string());
        vec![Command::RepoFilter(
            crate::tui::commands::RepoFilterCommand::DeleteRepoPath(path),
        )]
    }
}
