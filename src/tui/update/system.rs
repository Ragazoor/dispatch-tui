//! System / status / editor / quick-dispatch / misc handlers.

use crate::models::{EpicId, TaskId, DEFAULT_BASE_BRANCH, DEFAULT_QUICK_TASK_TITLE};

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_error(&mut self, msg: String) -> Vec<Command> {
        self.status.error_popup = Some(msg);
        vec![]
    }

    pub(in crate::tui) fn handle_task_edited(&mut self, edit: TaskEdit) -> Vec<Command> {
        if let Some(t) = self.find_task_mut(edit.id) {
            t.title = edit.title;
            t.description = edit.description;
            t.repo_path = edit.repo_path;
            t.status = edit.status;
            t.plan_path = edit.plan_path;
            t.tag = edit.tag;
            if let Some(bb) = edit.base_branch {
                t.base_branch = bb;
            }
            t.updated_at = chrono::Utc::now();
        }
        self.sync_board_selection();
        vec![]
    }

    pub(in crate::tui) fn handle_repo_paths_updated(&mut self, paths: Vec<String>) -> Vec<Command> {
        self.board.repo_paths = paths;
        if !self.board.repo_paths.is_empty() {
            self.input.repo_cursor = self.input.repo_cursor.min(self.board.repo_paths.len() - 1);
        } else {
            self.input.repo_cursor = 0;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_quick_dispatch(
        &mut self,
        repo_path: String,
        epic_id: Option<EpicId>,
    ) -> Vec<Command> {
        vec![Command::QuickDispatch {
            draft: TaskDraft {
                title: DEFAULT_QUICK_TASK_TITLE.to_string(),
                description: String::new(),
                repo_path,
                tag: None,
                base_branch: DEFAULT_BASE_BRANCH.to_string(),
            },
            epic_id,
        }]
    }

    pub(in crate::tui) fn handle_dismiss_error(&mut self) -> Vec<Command> {
        self.status.error_popup = None;
        vec![]
    }

    pub(in crate::tui) fn handle_status_info(&mut self, msg: String) -> Vec<Command> {
        self.set_status(msg);
        vec![]
    }

    pub(in crate::tui) fn handle_toggle_help(&mut self) -> Vec<Command> {
        if self.input.mode == InputMode::Help {
            self.input.mode = InputMode::Normal;
        } else {
            self.input.mode = InputMode::Help;
        }
        vec![]
    }

    pub(in crate::tui) fn exit_split_if_active(&mut self) -> Vec<Command> {
        if !self.board.split.active {
            return vec![];
        }
        let pane_id = match self.board.split.right_pane_id.take() {
            Some(id) => id,
            None => return vec![],
        };
        let restore_window = self
            .board
            .split
            .pinned_task_id
            .and_then(|id| self.find_task(id))
            .and_then(|t| t.tmux_window.clone());
        vec![Command::ExitSplitMode {
            pane_id,
            restore_window,
        }]
    }

    pub(in crate::tui) fn finish_task_creation(&mut self, repo_path: String) -> Vec<Command> {
        let draft = self.input.task_draft.take().unwrap_or_default();
        self.input.mode = InputMode::Normal;
        self.clear_status();
        let epic_id = match self.effective_view_mode() {
            ViewMode::Epic { epic_id, .. } => Some(*epic_id),
            _ => None,
        };
        vec![
            Command::InsertTask { draft, epic_id },
            Command::SaveRepoPath(repo_path),
        ]
    }

    pub(in crate::tui) fn handle_dispatch_failed(&mut self, id: TaskId) -> Vec<Command> {
        self.dispatching.remove(&id);
        vec![]
    }

    pub(in crate::tui) fn handle_mark_dispatching(&mut self, id: TaskId) -> Vec<Command> {
        self.dispatching.insert(id);
        vec![]
    }

    pub(in crate::tui) fn handle_description_editor_result(
        &mut self,
        value: String,
    ) -> Vec<Command> {
        match self.input.mode {
            InputMode::InputDescription => self.handle_submit_description(value),
            InputMode::InputEpicDescription => self.handle_submit_epic_description(value),
            _ => vec![],
        }
    }

    /// Router for editor results that come back from a pop-out editor. Each
    /// `EditKind` is finalized by a `FinalizeEditorResult` command dispatched
    /// to the runtime, except the `Description` variant which threads straight
    /// through the existing description-flow messages.
    pub(in crate::tui) fn handle_editor_result(
        &mut self,
        kind: EditKind,
        outcome: EditorOutcome,
    ) -> Vec<Command> {
        match (&kind, &outcome) {
            (EditKind::Description { .. }, EditorOutcome::Saved(text)) => {
                let text = crate::editor::parse_description_editor_output(text);
                self.update(Message::DescriptionEditorResult(text))
            }
            (EditKind::Description { .. }, EditorOutcome::Cancelled) => {
                self.update(Message::CancelInput)
            }
            _ => vec![Command::FinalizeEditorResult { kind, outcome }],
        }
    }

    pub(in crate::tui) fn handle_message_received(&mut self, id: TaskId) -> Vec<Command> {
        self.agents
            .message_flash
            .insert(id, std::time::Instant::now());
        vec![]
    }

    pub(in crate::tui) fn handle_open_in_browser(&self, url: String) -> Vec<Command> {
        vec![Command::OpenInBrowser { url }]
    }
}
