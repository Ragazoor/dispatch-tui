//! Repo filter mode + preset/path input handlers.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_key_repo_filter(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => self.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::Close,
            )),
            KeyCode::Char('a') => self.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::ToggleAll,
            )),
            KeyCode::Char('j') | KeyCode::Down => self.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::MoveCursor(1),
            )),
            KeyCode::Char('k') | KeyCode::Up => self.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::MoveCursor(-1),
            )),
            KeyCode::Char(' ') => {
                let idx = self.input.repo_cursor;
                if idx == 0 {
                    self.update(Message::RepoFilter(
                        crate::tui::messages::RepoFilterMessage::ToggleOnlyActive,
                    ))
                } else if idx <= self.board.repo_paths.len() {
                    let path = self.board.repo_paths[idx - 1].clone();
                    self.update(Message::RepoFilter(
                        crate::tui::messages::RepoFilterMessage::Toggle(path),
                    ))
                } else {
                    vec![]
                }
            }
            KeyCode::Char(c @ '1'..='9') => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.board.repo_paths.len() {
                    let path = self.board.repo_paths[idx].clone();
                    self.update(Message::RepoFilter(
                        crate::tui::messages::RepoFilterMessage::Toggle(path),
                    ))
                } else {
                    vec![]
                }
            }
            KeyCode::Tab => self.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::ToggleMode,
            )),
            KeyCode::Backspace | KeyCode::Delete => {
                if self.input.repo_cursor > 0 {
                    self.update(Message::RepoFilter(
                        crate::tui::messages::RepoFilterMessage::StartDeleteRepoPath,
                    ))
                } else {
                    vec![]
                }
            }
            KeyCode::Char('s') => self.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::StartSavePreset,
            )),
            KeyCode::Char('x') => self.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::StartDeletePreset,
            )),
            KeyCode::Char(c @ 'A'..='Z') => {
                let idx = (c as usize) - ('A' as usize);
                if idx < self.filter.presets.len() {
                    let name = self.filter.presets[idx].0.clone();
                    self.update(Message::RepoFilter(
                        crate::tui::messages::RepoFilterMessage::LoadPreset(name),
                    ))
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
                self.update(Message::RepoFilter(
                    crate::tui::messages::RepoFilterMessage::SavePreset(name),
                ))
            }
            KeyCode::Esc => self.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::CancelPresetInput,
            )),
            KeyCode::Backspace => self.update(Message::Input(
                crate::tui::messages::InputMessage::InputBackspace,
            )),
            KeyCode::Char(c) => self.update(Message::Input(
                crate::tui::messages::InputMessage::InputChar(c),
            )),
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
                    self.update(Message::RepoFilter(
                        crate::tui::messages::RepoFilterMessage::DeletePreset(name),
                    ))
                } else {
                    vec![]
                }
            }
            KeyCode::Esc => self.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::CancelPresetInput,
            )),
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
                if idx > 0 && idx <= self.board.repo_paths.len() {
                    let path = self.board.repo_paths[idx - 1].clone();
                    self.update(Message::RepoFilter(
                        crate::tui::messages::RepoFilterMessage::DeleteRepoPath(path),
                    ))
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
