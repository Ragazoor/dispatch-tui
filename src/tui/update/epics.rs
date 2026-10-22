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
        let mut cmds = self.push_epic_view(epic_id);
        cmds.extend(self.see_epic(epic_id));
        cmds
    }

    fn push_epic_view(&mut self, epic_id: EpicId) -> Vec<Command> {
        self.push_view(|selection, parent| ViewMode::Epic {
            epic_id,
            selection,
            parent,
        })
    }

    /// `tasks.allium: SeeTasksByEnteringTheirEpic`: the epic's own tasks stop
    /// being unseen. Dropped locally at once so the mark does not outlive the
    /// keypress, and sent to the store, which clears it for this person
    /// everywhere. Tasks in child epics keep their mark.
    fn see_epic(&mut self, epic_id: EpicId) -> Vec<Command> {
        if self.board.unseen_tasks.is_empty() {
            return vec![];
        }
        let seen: Vec<TaskId> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.epic_id == Some(epic_id))
            .map(|t| t.id)
            .collect();
        if !seen.iter().any(|id| self.board.unseen_tasks.contains(id)) {
            return vec![];
        }
        for id in &seen {
            self.board.unseen_tasks.remove(id);
        }
        vec![Command::Epic(crate::tui::commands::EpicCommand::MarkSeen(
            epic_id,
        ))]
    }

    pub(in crate::tui) fn handle_unseen_refreshed(&mut self, ids: Vec<TaskId>) -> Vec<Command> {
        self.board.unseen_tasks = ids.into_iter().collect();
        self.dirty = true;
        vec![]
    }

    /// Open a nested view (an epic or a work package) over the current one,
    /// with a fresh cursor and the current view kept as its parent, so q
    /// restores it.
    pub(in crate::tui) fn push_view(
        &mut self,
        view: impl FnOnce(BoardSelection, Box<ViewMode>) -> ViewMode,
    ) -> Vec<Command> {
        let parent = Box::new(self.board.view_mode.clone());
        self.board.view_mode = view(BoardSelection::new_for_epic(), parent);
        self.invalidate_layout_cache();
        self.dirty = true;
        vec![]
    }

    /// Leave one level: an epic view, or a package view
    /// (`work-packages.allium: ExitPackage`). Anything else is left as it is.
    pub(in crate::tui) fn handle_exit_epic(&mut self) -> Vec<Command> {
        match std::mem::take(&mut self.board.view_mode) {
            ViewMode::Epic { parent, .. } | ViewMode::Package { parent, .. } => {
                self.board.view_mode = *parent;
                self.invalidate_layout_cache();
                self.dirty = true;
            }
            other => self.board.view_mode = other,
        }
        vec![]
    }

    /// Back to the main board from any depth of epic and package views.
    pub(in crate::tui) fn handle_exit_all_epics(&mut self) -> Vec<Command> {
        while matches!(
            self.board.view_mode,
            ViewMode::Epic { .. } | ViewMode::Package { .. }
        ) {
            self.handle_exit_epic();
        }
        vec![]
    }

    /// Open `target`'s epic view, entering each epic between the current view
    /// and `target` so that `q` climbs back one level at a time.
    pub(in crate::tui) fn handle_jump_to_epic(&mut self, target: EpicId) -> Vec<Command> {
        self.jump_to_epic_from(target, None)
    }

    /// [`Self::handle_jump_to_epic`], with the trail's first level named when
    /// the current view is not an epic view. A package view holds linked epics
    /// from anywhere in the tree, so a jump from one starts its trail at the
    /// linked epic pressed rather than at the tree's root
    /// (`work-packages.allium: EnterEpicFromPackage`).
    fn jump_to_epic_from(&mut self, target: EpicId, first: Option<EpicId>) -> Vec<Command> {
        let path = crate::models::epic_path(target, &self.board.epics);
        let start = match self.current_epic_id() {
            Some(cur) => path.iter().position(|&e| e == cur).map_or(0, |i| i + 1),
            None => first
                .and_then(|f| path.iter().position(|&e| e == f))
                .unwrap_or(0),
        };
        let below = &path[start..];
        // Every level is entered so `q` climbs back one at a time, but only the
        // epic whose view finally opens is SEEN: the epics passed over were
        // never looked at (`tasks.allium: SeeTasksByEnteringTheirEpic`).
        for &id in below {
            self.push_epic_view(id);
        }
        below
            .last()
            .map_or_else(Vec::new, |&last| self.see_epic(last))
    }

    pub(in crate::tui) fn handle_jump_to_deepest_epic(
        &mut self,
        epic: EpicId,
        status: TaskStatus,
    ) -> Vec<Command> {
        let target =
            crate::models::deepest_epic_with(epic, status, &self.board.epics, &self.board.tasks);
        let in_package = matches!(self.board.view_mode, ViewMode::Package { .. });
        self.jump_to_epic_from(target, in_package.then_some(epic))
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
            if !self.board.epic_subtree_all_done(id) {
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
                crate::tui::commands::PersistFields::from_task(task).releasing_tmux_window(),
            )));
        }
        cmds
    }
}
