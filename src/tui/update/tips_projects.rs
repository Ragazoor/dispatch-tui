//! Tips overlay and project-message handlers.

use crate::models::{Project, ProjectId};

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_show_tips(
        &mut self,
        tips: Vec<crate::tips::Tip>,
        starting_index: usize,
        max_seen_id: u32,
        show_mode: crate::models::TipsShowMode,
    ) -> Vec<Command> {
        self.tips = Some(TipsOverlayState {
            index: starting_index,
            max_seen_id,
            show_mode,
            tips,
        });
        vec![]
    }

    pub(in crate::tui) fn handle_next_tip(&mut self) -> Vec<Command> {
        if let Some(overlay) = &mut self.tips {
            let len = overlay.tips.len();
            if len > 0 {
                overlay.index = (overlay.index + 1) % len;
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_prev_tip(&mut self) -> Vec<Command> {
        if let Some(overlay) = &mut self.tips {
            let len = overlay.tips.len();
            if len > 0 {
                overlay.index = (overlay.index + len - 1) % len;
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_set_tips_mode(
        &mut self,
        mode: crate::models::TipsShowMode,
    ) -> Vec<Command> {
        if let Some(overlay) = &mut self.tips {
            overlay.show_mode = mode;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_close_tips(&mut self) -> Vec<Command> {
        if let Some(overlay) = self.tips.take() {
            let seen_up_to = overlay
                .current_tip()
                .map(|t| t.id.max(overlay.max_seen_id))
                .unwrap_or(overlay.max_seen_id);
            vec![Command::SaveTipsState {
                seen_up_to,
                show_mode: overlay.show_mode,
            }]
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_projects_updated(
        &mut self,
        projects: Vec<Project>,
    ) -> Vec<Command> {
        self.board.projects = projects;
        self.active_is_default = self
            .board
            .projects
            .iter()
            .any(|p| p.id == self.active_project && p.is_default);
        vec![]
    }

    pub(in crate::tui) fn handle_select_project(&mut self, project_id: ProjectId) -> Vec<Command> {
        self.active_project = project_id;
        self.active_is_default = self
            .board
            .projects
            .iter()
            .any(|p| p.id == project_id && p.is_default);
        self.clamp_selection();
        if let Some(idx) = self.board.projects.iter().position(|p| p.id == project_id) {
            self.projects_panel.list_state.select(Some(idx));
        }
        vec![Command::PersistStringSetting {
            key: "last_project".into(),
            value: project_id.to_string(),
        }]
    }

    pub(in crate::tui) fn handle_follow_project(&mut self, project_id: ProjectId) -> Vec<Command> {
        if let Some(idx) = self.board.projects.iter().position(|p| p.id == project_id) {
            self.selection_mut().set_row(0, idx);
            self.projects_panel.list_state.select(Some(idx));
        }
        vec![]
    }
}
