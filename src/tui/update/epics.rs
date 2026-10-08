//! Epic-related message handlers: lifecycle, batch ops, creation flow.

use std::collections::HashSet;

use crate::models::{descendant_epic_ids, Epic, EpicId, TaskId, TaskStatus};

use super::super::types::*;
use super::super::{truncate_title, App, TITLE_DISPLAY_LENGTH};

impl App {
    // -----------------------------------------------------------------------
    // Epic handlers
    // -----------------------------------------------------------------------

    pub(in crate::tui) fn handle_enter_epic(&mut self, epic_id: EpicId) -> Vec<Command> {
        let parent = Box::new(self.board.view_mode.clone());
        self.board.view_mode = ViewMode::Epic {
            epic_id,
            selection: BoardSelection::new_for_epic(),
            parent,
        };
        self.invalidate_layout_cache();
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_exit_epic(&mut self) -> Vec<Command> {
        if let ViewMode::Epic { parent, .. } = std::mem::take(&mut self.board.view_mode) {
            self.board.view_mode = *parent;
            self.invalidate_layout_cache();
            self.dirty = true;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_exit_all_epics(&mut self) -> Vec<Command> {
        while matches!(self.board.view_mode, ViewMode::Epic { .. }) {
            self.handle_exit_epic();
        }
        vec![]
    }

    /// Open `target`'s epic view, entering each epic between the current view
    /// and `target` so that `q` climbs back one level at a time.
    pub(in crate::tui) fn handle_jump_to_epic(&mut self, target: EpicId) -> Vec<Command> {
        let path = crate::models::epic_path(target, &self.board.epics);
        let start = self
            .current_epic_id()
            .and_then(|cur| path.iter().position(|&e| e == cur))
            .map_or(0, |i| i + 1);
        let below = &path[start..];
        for &id in below {
            self.handle_enter_epic(id);
        }
        vec![]
    }

    pub(in crate::tui) fn handle_jump_to_deepest_epic(
        &mut self,
        epic: EpicId,
        status: TaskStatus,
    ) -> Vec<Command> {
        let target =
            crate::models::deepest_epic_with(epic, status, &self.board.epics, &self.board.tasks);
        self.handle_jump_to_epic(target)
    }

    pub(in crate::tui) fn handle_refresh_epics(&mut self, epics: Vec<Epic>) -> Vec<Command> {
        self.board.epics = epics;
        let valid_ids: HashSet<EpicId> = self.board.epics.iter().map(|e| e.id).collect();
        self.select.epics.retain(|id| valid_ids.contains(id));
        self.invalidate_layout_cache();
        vec![]
    }

    /// Splice a single fresh epic into the in-memory list, replacing the row
    /// with a matching id or appending if it's newly-created.
    pub(in crate::tui) fn handle_epic_updated(&mut self, epic: Epic) -> Vec<Command> {
        if let Some(slot) = self.board.epics.iter_mut().find(|e| e.id == epic.id) {
            *slot = epic;
        } else {
            self.board.epics.push(epic);
        }
        self.invalidate_layout_cache();
        vec![]
    }

    pub(in crate::tui) fn handle_epic_created(&mut self, epic: Epic) -> Vec<Command> {
        self.board.epics.push(epic);
        self.invalidate_layout_cache();
        vec![]
    }

    pub(in crate::tui) fn handle_edit_epic(&mut self, id: EpicId) -> Vec<Command> {
        if let Some(epic) = self.board.epics.iter().find(|e| e.id == id) {
            vec![Command::Editor(
                crate::tui::commands::EditorCommand::PopOut(EditKind::EpicEdit(Box::new(
                    epic.clone(),
                ))),
            )]
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_epic_edited(&mut self, epic: Epic) -> Vec<Command> {
        if let Some(slot) = self.board.epics.iter_mut().find(|e| e.id == epic.id) {
            *slot = epic;
        }
        vec![]
    }

    /// `epics.allium: EditEpic`'s take-over prompt. Shows a y/n confirmation
    /// naming the conflicting owner; the `feed_command` edit that triggered
    /// this has already applied by the time this fires — see
    /// `TuiRuntime::finalize_epic_edit`, which is the only emitter.
    pub(in crate::tui) fn handle_feed_owner_takeover_offered(
        &mut self,
        epic_id: EpicId,
        other_host: String,
    ) -> Vec<Command> {
        self.status.set(format!(
            "This feed is currently owned by host {other_host} — take over polling? [y/N]"
        ));
        self.input.mode = InputMode::ConfirmOverrideFeedOwner {
            epic_id,
            other_host,
        };
        vec![]
    }

    pub(in crate::tui) fn handle_delete_epic(&mut self, id: EpicId) -> Vec<Command> {
        let mut cmds = self.teardown_epic_subtree(id, crate::tui::commands::DeleteGuard::Epic(id));
        cmds.push(Command::Epic(crate::tui::commands::EpicCommand::Delete(id)));
        cmds
    }

    /// Board-mutation and best-effort teardown for `id`'s whole subtree,
    /// shared by [`Self::handle_delete_epic`] (single-epic `x`, which still
    /// issues its own `EpicCommand::Delete` afterwards) and
    /// `handle_batch_delete`'s atomic path (which bundles every epic's
    /// subtree into ONE `TaskCommand::BatchDelete` call instead — see
    /// `docs/specs/tasks.allium: BatchDelete`).
    pub(in crate::tui) fn teardown_epic_subtree(
        &mut self,
        id: EpicId,
        guard: crate::tui::commands::DeleteGuard,
    ) -> Vec<Command> {
        let mut cmds = Vec::new();
        // The DB delete drops the whole subtree (`delete_epic_recursive` walks
        // parent_epic_id depth-first), so cleanup must cover the same subtree —
        // covering only direct children would delete a nested subtask's row
        // while leaving its worktree and tmux window with nothing referencing
        // them. See DeleteEpic in docs/specs/epics.allium.
        let subtree = descendant_epic_ids(id, &self.board.epics);
        let in_subtree =
            |t: &crate::models::Task| t.epic_id.is_some_and(|eid| subtree.contains(&eid));
        let subtask_ids: Vec<TaskId> = self
            .board
            .tasks
            .iter()
            .filter(|t| in_subtree(t))
            .map(|t| t.id)
            .collect();
        for task_id in subtask_ids {
            if let Some(task) = self.find_task_mut(task_id) {
                // DeleteEpic is exempt from the pointer gate: the epic delete
                // drops every subtask row in one operation, so there is nothing
                // left that could hold a retryable pointer — and nothing to
                // write back on success either. The failure is still reported
                // and logged. See WorktreeReleaseIsGated in
                // docs/specs/tasks.allium.
                let cleanup = Self::take_cleanup(
                    task,
                    crate::tui::commands::CleanupFollowUp::Nothing,
                    Some(guard.clone()),
                );
                if let Some(c) = cleanup {
                    cmds.push(c);
                }
                self.clear_agent_tracking(task_id);
            }
            // split-pane.allium: SplitPaneRespawnOnWindowCleared lists DeleteTask
            // as a trigger, and a subtask going with its epic is no different —
            // a pinned task inside the deleted subtree must not leave the pane
            // pointing at a row that no longer exists.
            cmds.extend(self.maybe_respawn_split_pane(task_id));
        }
        self.board.epics.retain(|e| !subtree.contains(&e.id));
        self.board.tasks.retain(|t| !in_subtree(t));
        // If we were viewing this epic, exit
        if matches!(&self.board.view_mode, ViewMode::Epic { epic_id, .. } if *epic_id == id) {
            self.handle_exit_epic();
        }
        self.sync_board_selection();
        cmds
    }

    pub(in crate::tui) fn handle_confirm_delete_epic(&mut self) -> Vec<Command> {
        if let Some(ColumnItem::Epic(epic)) = self.selected_column_item() {
            let id = epic.id;
            if !self.epic_subtree_all_done(id) {
                let title = truncate_title(&epic.title, TITLE_DISPLAY_LENGTH);
                self.status.set(format!(
                    "Cannot delete epic {title}: unfinished work in its subtree"
                ));
                return vec![];
            }
            let title = truncate_title(&epic.title, TITLE_DISPLAY_LENGTH);
            self.input.mode = InputMode::ConfirmDeleteEpic;
            self.status
                .set(format!("Delete epic {title} and subtasks? [y/n]"));
        }
        vec![]
    }

    /// `epics.allium: ConfirmDeleteEpic`'s guard — every task anywhere in
    /// `id`'s subtree, at any depth, is done. An epic with no tasks at all
    /// (an empty subtree) qualifies vacuously.
    pub(in crate::tui) fn epic_subtree_all_done(&self, id: EpicId) -> bool {
        let subtree = descendant_epic_ids(id, &self.board.epics);
        self.board
            .tasks
            .iter()
            .filter(|t| t.epic_id.is_some_and(|eid| subtree.contains(&eid)))
            .all(|t| t.status == TaskStatus::Done)
    }

    pub(in crate::tui) fn handle_move_epic_status(
        &mut self,
        id: EpicId,
        direction: MoveDirection,
    ) -> Vec<Command> {
        let Some(epic) = self.board.epics.iter_mut().find(|e| e.id == id) else {
            return vec![];
        };
        let new_status = match direction {
            MoveDirection::Forward => epic.status.next(),
            MoveDirection::Backward => epic.status.prev(),
        };
        if new_status == epic.status {
            return vec![];
        }
        epic.status = new_status;
        let mut cmds = vec![Command::Epic(crate::tui::commands::EpicCommand::Persist {
            id,
            status: Some(new_status),
            sort_order: None,
            completed_at: None,
        })];

        // Moving to Done cleans up all subtask tmux windows
        if new_status == TaskStatus::Done {
            cmds.extend(self.release_subtask_windows(id));
        }
        self.sync_board_selection();
        cmds
    }

    /// Take every direct subtask's tmux window, returning the commands that
    /// kill each window and persist its task without it.
    fn release_subtask_windows(&mut self, id: EpicId) -> Vec<Command> {
        let subtask_ids: Vec<TaskId> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.epic_id == Some(id) && t.tmux_window.is_some())
            .map(|t| t.id)
            .collect();
        let mut cmds = Vec::new();
        for task_id in subtask_ids {
            let Some(task) = self.find_task_mut(task_id) else {
                continue;
            };
            let Some(window) = task.tmux_window.take() else {
                continue;
            };
            cmds.push(Command::Task(
                crate::tui::commands::TaskCommand::KillTmuxWindow { window },
            ));
            cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                crate::tui::commands::PersistFields::from_task(task),
            )));
        }
        cmds
    }

    pub(in crate::tui) fn handle_start_new_epic(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::InputEpicTitle;
        self.input.clear_buffer();
        let parent_epic_id = if let ViewMode::Epic { epic_id, .. } = self.board.view_mode {
            Some(epic_id)
        } else {
            None
        };
        self.input.epic_draft = Some(EpicDraft {
            parent_epic_id,
            ..Default::default()
        });
        self.status.set("Epic title: ".to_string());
        vec![]
    }

    pub(in crate::tui) fn handle_submit_epic_title(&mut self, value: String) -> Vec<Command> {
        self.input.clear_buffer();
        if value.is_empty() {
            self.input.mode = InputMode::Normal;
            self.status.clear();
            vec![]
        } else {
            let parent_epic_id = self
                .input
                .epic_draft
                .as_ref()
                .and_then(|d| d.parent_epic_id);
            self.input.epic_draft = Some(EpicDraft {
                title: value,
                description: String::new(),
                parent_epic_id,
            });
            self.input.mode = InputMode::InputEpicDescription;
            self.status
                .set("Opening editor for description...".to_string());
            vec![Command::Editor(
                crate::tui::commands::EditorCommand::PopOut(EditKind::Description {
                    is_epic: true,
                }),
            )]
        }
    }

    pub(in crate::tui) fn handle_submit_epic_description(&mut self, value: String) -> Vec<Command> {
        self.input.clear_buffer();
        if let Some(ref mut draft) = self.input.epic_draft {
            draft.description = value;
        }
        self.finish_epic_creation()
    }

    // -----------------------------------------------------------------------
    // Reparent epic handlers
    // -----------------------------------------------------------------------

    pub(in crate::tui) fn handle_start_reparent(&mut self, epic_id: EpicId) -> Vec<Command> {
        let mut tree_state = tui_tree_widget::TreeState::default();
        tree_state.select_first();
        let eligible = self.view().reparent_target_epics(epic_id);
        let items = crate::tui::ui::build_reparent_tree(&eligible);
        self.interaction.reparent_picker = Some(crate::tui::ReparentPickerState {
            epic_id,
            tree_state: std::cell::RefCell::new(tree_state),
            items,
        });
        self.input.mode = InputMode::ReparentEpic(epic_id);
        vec![]
    }

    pub(in crate::tui) fn handle_reparent_navigate(&mut self, nav: TreeNav) -> Vec<Command> {
        if let Some(picker) = &self.interaction.reparent_picker {
            crate::tui::types::apply_tree_nav(&mut picker.tree_state.borrow_mut(), nav);
            self.dirty = true;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_reparent_confirm(&mut self) -> Vec<Command> {
        let epic_id = match self.input.mode {
            InputMode::ReparentEpic(id) => id,
            _ => return vec![],
        };

        let selected_id: Option<String> = self
            .interaction
            .reparent_picker
            .as_ref()
            .and_then(|p| p.tree_state.borrow().selected().last().cloned());

        let new_parent: Option<EpicId> = match selected_id.as_deref() {
            Some(s) if s != crate::tui::types::REPARENT_NO_PARENT_SENTINEL => s
                .strip_prefix("epic:")
                .and_then(|n| n.parse::<i64>().ok())
                .map(EpicId),
            _ => None,
        };

        let moving_title = self
            .board
            .epics
            .iter()
            .find(|e| e.id == epic_id)
            .map(|e| truncate_title(&e.title, TITLE_DISPLAY_LENGTH))
            .unwrap_or_default();

        let msg = match new_parent {
            None => format!("Make {moving_title} a root epic? [y/n]"),
            Some(pid) => {
                let parent_label = self
                    .board
                    .epics
                    .iter()
                    .find(|e| e.id == pid)
                    .map(|e| truncate_title(&e.title, TITLE_DISPLAY_LENGTH))
                    .unwrap_or_else(|| format!("\"epic #{}\"", pid.0));
                format!("Reparent {moving_title} under {parent_label}? [y/n]")
            }
        };

        self.input.mode = InputMode::ConfirmReparentEpic {
            epic_id,
            new_parent,
        };
        self.status.set(msg);
        vec![]
    }

    fn clear_reparent_state(&mut self) {
        self.input.mode = InputMode::Normal;
        self.interaction.reparent_picker = None;
        self.status.clear();
    }

    pub(in crate::tui) fn handle_reparent_execute(&mut self) -> Vec<Command> {
        let (epic_id, new_parent) = match self.input.mode {
            InputMode::ConfirmReparentEpic {
                epic_id,
                new_parent,
            } => (epic_id, new_parent),
            _ => return vec![],
        };
        self.clear_reparent_state();
        vec![Command::Epic(crate::tui::commands::EpicCommand::Reparent {
            id: epic_id,
            new_parent,
        })]
    }

    /// Cancel the reparent flow entirely (Esc/q from ConfirmReparentEpic),
    /// returning to Normal mode and clearing the picker.
    pub(in crate::tui) fn handle_reparent_cancel_all(&mut self) -> Vec<Command> {
        self.clear_reparent_state();
        vec![]
    }

    pub(in crate::tui) fn handle_reparent_cancel(&mut self) -> Vec<Command> {
        match self.input.mode {
            InputMode::ConfirmReparentEpic { epic_id, .. } => {
                self.input.mode = InputMode::ReparentEpic(epic_id);
                self.status.clear();
            }
            InputMode::ReparentEpic(_) => {
                self.input.mode = InputMode::Normal;
                self.interaction.reparent_picker = None;
            }
            _ => {}
        }
        vec![]
    }
}
