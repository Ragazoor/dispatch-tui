//! Confirmation dialog handlers (delete, archive, retry, done, merge, wrap-up, etc).

use crossterm::event::{KeyCode, KeyEvent};

use crate::models::TaskId;

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn confirm_dialog(
        &mut self,
        key: KeyEvent,
        on_confirm: impl FnOnce(&mut Self) -> Vec<Command>,
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.input.mode = InputMode::Normal;
                self.clear_status();
                on_confirm(self)
            }
            _ => {
                self.input.mode = InputMode::Normal;
                self.clear_status();
                vec![]
            }
        }
    }

    pub(in crate::tui) fn handle_key_confirm_quit(&mut self, key: KeyEvent) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            s.should_quit = true;
            s.exit_split_if_active()
        })
    }

    pub(in crate::tui) fn handle_key_confirm_delete(&mut self, key: KeyEvent) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            if s.show_archived() {
                s.confirm_delete_archived()
            } else {
                s.confirm_delete_selected()
            }
        })
    }

    pub(in crate::tui) fn confirm_delete_archived(&mut self) -> Vec<Command> {
        self.archived_tasks()
            .get(self.selected_archive_row())
            .map(|t| t.id)
            .map(|id| self.update(Message::DeleteTask(id)))
            .unwrap_or_default()
    }

    pub(in crate::tui) fn confirm_delete_selected(&mut self) -> Vec<Command> {
        self.selected_task()
            .map(|t| t.id)
            .map(|id| self.update(Message::DeleteTask(id)))
            .unwrap_or_default()
    }

    pub(in crate::tui) fn handle_key_confirm_retry(
        &mut self,
        key: KeyEvent,
        id: TaskId,
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Char('r') => self.update(Message::RetryResume(id)),
            KeyCode::Char('f') => self.update(Message::RetryFresh(id)),
            KeyCode::Esc => self.update(Message::CancelRetry),
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_confirm_archive(
        &mut self,
        key: KeyEvent,
        task_id: Option<TaskId>,
    ) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            if s.has_selection() {
                let mut cmds = Vec::new();
                if !s.select.tasks.is_empty() {
                    let ids: Vec<_> = s.select.tasks.iter().copied().collect();
                    cmds.extend(s.update(Message::BatchArchiveTasks(ids)));
                }
                if !s.select.epics.is_empty() {
                    let ids: Vec<_> = s.select.epics.iter().copied().collect();
                    cmds.extend(s.update(Message::BatchArchiveEpics(ids)));
                }
                cmds
            } else if let Some(id) = task_id {
                s.update(Message::ArchiveTask(id))
            } else {
                vec![]
            }
        })
    }

    pub(in crate::tui) fn handle_key_confirm_done(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.update(Message::ConfirmDone),
            _ => self.update(Message::CancelDone),
        }
    }

    pub(in crate::tui) fn handle_key_confirm_merge_pr(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.update(Message::ConfirmMergePr),
            _ => self.update(Message::CancelMergePr),
        }
    }

    pub(in crate::tui) fn handle_key_confirm_delete_epic(&mut self, key: KeyEvent) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            if let Some(id) = s.selected_epic_id() {
                s.update(Message::DeleteEpic(id))
            } else {
                vec![]
            }
        })
    }

    pub(in crate::tui) fn handle_key_confirm_archive_epic(
        &mut self,
        key: KeyEvent,
    ) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            if let Some(id) = s.selected_epic_id() {
                s.update(Message::ArchiveEpic(id))
            } else {
                vec![]
            }
        })
    }

    pub(in crate::tui) fn handle_key_confirm_detach_tmux(&mut self, key: KeyEvent) -> Vec<Command> {
        let ids = match &self.input.mode {
            InputMode::ConfirmDetachTmux(ids) => ids.clone(),
            _ => return vec![],
        };
        self.confirm_dialog(key, |s| s.detach_tmux_panels(ids))
    }

    pub(in crate::tui) fn handle_key_confirm_edit_task(
        &mut self,
        key: KeyEvent,
        id: TaskId,
    ) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            if let Some(task) = s.board.tasks.iter().find(|t| t.id == id) {
                vec![Command::PopOutEditor(EditKind::TaskEdit(task.clone()))]
            } else {
                vec![]
            }
        })
    }

    pub(in crate::tui) fn handle_key_confirm_wrap_up(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('r') => self.update(Message::WrapUpRebase),
            KeyCode::Char('p') => self.update(Message::WrapUpPr),
            KeyCode::Esc => self.update(Message::CancelWrapUp),
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_confirm_epic_wrap_up(
        &mut self,
        key: KeyEvent,
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Char('r') => self.update(Message::EpicWrapUpRebase),
            KeyCode::Char('p') => self.update(Message::EpicWrapUpPr),
            KeyCode::Esc => self.update(Message::CancelEpicWrapUp),
            _ => vec![],
        }
    }
}
