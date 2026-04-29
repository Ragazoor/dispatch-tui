//! Repo filter mode + preset/path input handlers.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_key_repo_filter(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                self.update(Message::CloseRepoFilter)
            }
            KeyCode::Char('a') => self.update(Message::ToggleAllRepoFilter),
            KeyCode::Char('j') | KeyCode::Down => self.update(Message::MoveRepoCursor(1)),
            KeyCode::Char('k') | KeyCode::Up => self.update(Message::MoveRepoCursor(-1)),
            KeyCode::Char(' ') => {
                let idx = self.input.repo_cursor;
                if idx < self.board.repo_paths.len() {
                    let path = self.board.repo_paths[idx].clone();
                    self.update(Message::ToggleRepoFilter(path))
                } else {
                    vec![]
                }
            }
            KeyCode::Char(c @ '1'..='9') => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.board.repo_paths.len() {
                    let path = self.board.repo_paths[idx].clone();
                    self.update(Message::ToggleRepoFilter(path))
                } else {
                    vec![]
                }
            }
            KeyCode::Backspace | KeyCode::Delete => self.update(Message::StartDeleteRepoPath),
            KeyCode::Char('s') => self.update(Message::StartSavePreset),
            KeyCode::Char('x') => self.update(Message::StartDeletePreset),
            KeyCode::Char(c @ 'A'..='Z') => {
                let idx = (c as usize) - ('A' as usize);
                if idx < self.filter.presets.len() {
                    let name = self.filter.presets[idx].0.clone();
                    self.update(Message::LoadFilterPreset(name))
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_input_preset_name(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Enter => {
                let name = self.input.buffer.clone();
                self.update(Message::SaveFilterPreset(name))
            }
            KeyCode::Esc => self.update(Message::CancelPresetInput),
            KeyCode::Backspace => self.update(Message::InputBackspace),
            KeyCode::Char(c) => self.update(Message::InputChar(c)),
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_confirm_delete_preset(
        &mut self,
        key: KeyEvent,
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Char(c @ 'A'..='Z') => {
                let idx = (c as usize) - ('A' as usize);
                if idx < self.filter.presets.len() {
                    let name = self.filter.presets[idx].0.clone();
                    self.update(Message::DeleteFilterPreset(name))
                } else {
                    vec![]
                }
            }
            KeyCode::Esc => self.update(Message::CancelPresetInput),
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_confirm_delete_repo_path(
        &mut self,
        key: KeyEvent,
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let idx = self.input.repo_cursor;
                if idx < self.board.repo_paths.len() {
                    let path = self.board.repo_paths[idx].clone();
                    self.update(Message::DeleteRepoPath(path))
                } else {
                    self.input.mode = InputMode::RepoFilter;
                    vec![]
                }
            }
            _ => {
                self.input.mode = InputMode::RepoFilter;
                vec![]
            }
        }
    }
}
