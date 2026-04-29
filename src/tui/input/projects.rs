//! Projects panel + project create/edit/delete input handlers.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_key_projects_panel(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let len = self.board.projects.len();
                if len > 0 {
                    let next = (self.selection().row(0) + 1).min(len - 1);
                    if next == self.selection().row(0) {
                        return vec![];
                    }
                    self.selection_mut().set_row(0, next);
                    self.projects_panel.list_state.select(Some(next));
                    if let Some(id) = self.selected_project().map(|p| p.id) {
                        return self.update(Message::SelectProject(id));
                    }
                }
                vec![]
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let prev = self.selection().row(0).saturating_sub(1);
                if prev == self.selection().row(0) {
                    return vec![];
                }
                self.selection_mut().set_row(0, prev);
                self.projects_panel.list_state.select(Some(prev));
                if let Some(id) = self.selected_project().map(|p| p.id) {
                    return self.update(Message::SelectProject(id));
                }
                vec![]
            }
            KeyCode::Char('l')
            | KeyCode::Right
            | KeyCode::Enter
            | KeyCode::Char('g')
            | KeyCode::Esc => self.update(Message::NavigateColumn(1)),
            KeyCode::Char('n') => {
                self.input.mode = InputMode::InputProjectName { editing_id: None };
                self.input.buffer.clear();
                vec![]
            }
            KeyCode::Char('r') => {
                if let Some(project) = self.selected_project().cloned() {
                    self.input.mode = InputMode::InputProjectName {
                        editing_id: Some(project.id),
                    };
                    self.input.buffer = project.name;
                }
                vec![]
            }
            KeyCode::Char('d') => {
                if let Some(project) = self.selected_project().cloned() {
                    if !project.is_default {
                        self.input.mode = InputMode::ConfirmDeleteProject1 { id: project.id };
                    } else {
                        return self.update(Message::StatusInfo(
                            "Cannot delete the default project".to_string(),
                        ));
                    }
                }
                vec![]
            }
            KeyCode::Char('J') => {
                if let Some(id) = self.selected_project().map(|p| p.id) {
                    return vec![Command::ReorderProject { id, delta: 1 }];
                }
                vec![]
            }
            KeyCode::Char('K') => {
                if let Some(id) = self.selected_project().map(|p| p.id) {
                    return vec![Command::ReorderProject { id, delta: -1 }];
                }
                vec![]
            }
            KeyCode::Char('q') => self.update(Message::Quit),
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_input_project_name(
        &mut self,
        key: KeyEvent,
        editing_id: Option<i64>,
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Enter => {
                let name = self.input.buffer.trim().to_string();
                self.input.mode = InputMode::Normal;
                self.input.buffer.clear();
                if name.is_empty() {
                    return vec![];
                }
                match editing_id {
                    None => vec![Command::CreateProject { name }],
                    Some(id) => vec![Command::RenameProject { id, name }],
                }
            }
            KeyCode::Esc => {
                self.input.mode = InputMode::Normal;
                self.input.buffer.clear();
                vec![]
            }
            KeyCode::Backspace => {
                self.input.buffer.pop();
                vec![]
            }
            KeyCode::Char(c) => {
                self.input.buffer.push(c);
                vec![]
            }
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_confirm_delete_project1(
        &mut self,
        key: KeyEvent,
        id: i64,
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') => {
                let item_count = self
                    .board
                    .tasks
                    .iter()
                    .filter(|t| t.project_id == id)
                    .count() as u64
                    + self
                        .board
                        .epics
                        .iter()
                        .filter(|e| e.project_id == id)
                        .count() as u64;
                self.input.mode = InputMode::ConfirmDeleteProject2 { id, item_count };
                vec![]
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.input.mode = InputMode::Normal;
                vec![]
            }
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_confirm_delete_project2(
        &mut self,
        key: KeyEvent,
        id: i64,
        _item_count: u64, // read by the renderer via InputMode::ConfirmDeleteProject2 { item_count }
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') => {
                self.input.mode = InputMode::Normal;
                // Navigate away from Projects panel (col 0) to Backlog (col 1)
                let nav = self.update(Message::NavigateColumn(1));
                let mut cmds = vec![Command::DeleteProject { id }];
                cmds.extend(nav);
                cmds
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.input.mode = InputMode::Normal;
                vec![]
            }
            _ => vec![],
        }
    }
}
