//! Confirmation dialog handlers (delete, retry, done, etc).

use crossterm::event::{KeyCode, KeyEvent};

use crate::models::{DispatchMode, EpicId, TaskId};

use super::super::types::*;
use super::super::App;
use super::{key_event, key_label};

impl App {
    /// Every confirmation dialog records both outcomes as an
    /// `<action>_yes` / `<action>_no` pair. Dismissing is as much a use of the
    /// dialog as confirming is — a prompt that is nearly always declined is
    /// one worth removing, and only the pair makes that visible.
    pub(in crate::tui) fn confirm_dialog(
        &mut self,
        key: KeyEvent,
        action: &str,
        on_confirm: impl FnOnce(&mut Self) -> Vec<Command>,
    ) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        let label = key_label(key);
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let mut cmds = on_confirm(self);
                cmds.push(key_event(&format!("{action}_yes"), &label));
                cmds
            }
            _ => vec![key_event(&format!("{action}_no"), &label)],
        }
    }

    pub(in crate::tui) fn handle_key_confirm_quit(&mut self, key: KeyEvent) -> Vec<Command> {
        self.confirm_dialog(key, "confirm_quit", |s| {
            // Quitting with a task pinned restores that agent to a standalone
            // window, and a rearrangement in flight makes that impossible to do
            // now. During an entry there is nothing to restore from yet — the
            // join is still running and `active` is still false, so the exit
            // below would issue nothing and dispatch would go away leaving the
            // agent's pane in the board's own window. During a swap the panes
            // have been exchanged but the outgoing task's window may not have
            // been renamed back yet, so quitting leaves two windows sharing a
            // name. Either way, hold the quit; the rearrangement's own settle
            // performs it. See `HoldQuitWhileRearrangementInFlight` in
            // docs/specs/split-pane.allium.
            if let Some(in_flight) = s.board.split.in_flight.as_mut() {
                in_flight.pending_quit = true;
                return vec![];
            }
            s.should_quit = true;
            s.exit_split_if_active()
        })
    }

    /// `tasks.allium: DeleteTask` via `ConfirmDeleteTask` — the id was
    /// captured when 'x' was pressed, so a cursor drift before 'y' cannot
    /// redirect the delete to a different card.
    pub(in crate::tui) fn handle_key_confirm_delete_task(
        &mut self,
        key: KeyEvent,
        id: TaskId,
    ) -> Vec<Command> {
        self.confirm_dialog(key, "confirm_delete", |s| s.handle_delete_task(id))
    }

    pub(in crate::tui) fn handle_key_confirm_retry(
        &mut self,
        key: KeyEvent,
        id: TaskId,
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Char('r') => self.dispatch_keyed(
                Message::Task(crate::tui::messages::TaskMessage::RetryResume(id)),
                "confirm_retry_resume",
                "r",
            ),
            KeyCode::Char('f') => self.dispatch_keyed(
                Message::Task(crate::tui::messages::TaskMessage::RetryFresh(id)),
                "confirm_retry_fresh",
                "f",
            ),
            KeyCode::Esc => self.dispatch_keyed(
                Message::Input(crate::tui::messages::InputMessage::CancelRetry),
                "confirm_retry_no",
                "Esc",
            ),
            _ => vec![],
        }
    }

    /// `tasks.allium: BatchDelete` — reads the current multi-selection at
    /// confirm time (the variant carries no payload).
    pub(in crate::tui) fn handle_key_confirm_batch_delete(
        &mut self,
        key: KeyEvent,
    ) -> Vec<Command> {
        self.confirm_dialog(key, "confirm_delete", |s| s.handle_batch_delete())
    }

    pub(in crate::tui) fn handle_key_confirm_done(&mut self, key: KeyEvent) -> Vec<Command> {
        let label = key_label(key);
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.dispatch_keyed(
                Message::Input(crate::tui::messages::InputMessage::ConfirmDone),
                "confirm_done_yes",
                &label,
            ),
            _ => self.dispatch_keyed(
                Message::Input(crate::tui::messages::InputMessage::CancelDone),
                "confirm_done_no",
                &label,
            ),
        }
    }

    pub(in crate::tui) fn handle_key_confirm_delete_epic(&mut self, key: KeyEvent) -> Vec<Command> {
        self.confirm_dialog(key, "confirm_delete_epic", |s| {
            if let Some(id) = s.selected_epic_id() {
                s.update(Message::Epic(crate::tui::messages::EpicMessage::Delete(id)))
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
        self.confirm_dialog(key, "confirm_detach_tmux", |s| s.detach_tmux_panels(ids))
    }

    /// `epics.allium: EditEpic`'s take-over prompt. `feeds.allium:
    /// OverrideFeedOwner`, `pr-workflow.allium: OverridePrPollOwner`'s
    /// guidance. Declining leaves ownership untouched — the `feed_command`
    /// edit itself already applied before this prompt ever showed.
    pub(in crate::tui) fn handle_key_confirm_override_feed_owner(
        &mut self,
        key: KeyEvent,
    ) -> Vec<Command> {
        let epic_id = match &self.input.mode {
            InputMode::ConfirmOverrideFeedOwner { epic_id, .. } => *epic_id,
            _ => return vec![],
        };
        self.confirm_dialog(key, "confirm_override_feed_owner", |_| {
            vec![Command::Epic(
                crate::tui::commands::EpicCommand::OverrideFeedOwner(epic_id),
            )]
        })
    }

    pub(in crate::tui) fn handle_key_confirm_trust_repo(
        &mut self,
        key: KeyEvent,
        task_id: TaskId,
        mode: DispatchMode,
    ) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        let label = key_label(key);
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.dispatch_keyed(
                Message::Task(crate::tui::messages::TaskMessage::TrustAndDispatch {
                    id: task_id,
                    mode,
                }),
                "confirm_trust_repo_yes",
                &label,
            ),
            _ => vec![key_event("confirm_trust_repo_no", &label)],
        }
    }

    /// The sync confirmation (docs/specs/repo-sync.allium: surface
    /// RepoSyncConfirmation). Nothing is fetched, merged or pushed until it is
    /// confirmed; dismissing leaves the repository untouched.
    pub(in crate::tui) fn handle_key_confirm_repo_sync(
        &mut self,
        key: KeyEvent,
        repo_path: String,
    ) -> Vec<Command> {
        self.confirm_dialog(key, "confirm_repo_sync", |s| {
            s.confirm_repo_sync(&repo_path)
        })
    }

    pub(in crate::tui) fn handle_key_confirm_trust_repo_quick_dispatch(
        &mut self,
        key: KeyEvent,
        draft: TaskDraft,
        epic_id: Option<EpicId>,
    ) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        let label = key_label(key);
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                vec![
                    Command::Task(crate::tui::commands::TaskCommand::TrustAndQuickDispatch {
                        draft,
                        epic_id,
                    }),
                    key_event("confirm_trust_repo_quick_dispatch_yes", &label),
                ]
            }
            _ => vec![key_event("confirm_trust_repo_quick_dispatch_no", &label)],
        }
    }
}
