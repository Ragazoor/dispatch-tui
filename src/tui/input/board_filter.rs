//! Board-filter overlay input handlers, and the repo-path delete confirm.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::types::*;
use super::super::App;
use super::key_event;

impl App {
    /// `action` is the row's action id; `key` supplies the direction and the
    /// digit where one row covers several keys. A tab-specific key on another
    /// tab does nothing and records nothing.
    pub(in crate::tui) fn handle_key_board_filter(
        &mut self,
        key: KeyEvent,
        action: &str,
        label: &str,
    ) -> Vec<Command> {
        use crate::tui::messages::BoardFilterMessage as M;
        let tab = self.filter_overlay.tab;
        let msg = match action {
            "filter_close" => M::Close,
            "filter_back" => M::Back,
            "filter_next_tab" => M::SwitchTab(MoveDirection::Forward),
            "filter_previous_tab" => M::SwitchTab(MoveDirection::Backward),
            "filter_move_cursor" => {
                let down = matches!(key.code, KeyCode::Char('j') | KeyCode::Down);
                M::MoveRowCursor(if down { 1 } else { -1 })
            }
            "filter_toggle_row" => match self.filter_row_toggle() {
                Some(msg) => msg,
                None => return vec![],
            },
            "repo_filter_toggle_repo" if tab == FilterTab::Repos => {
                let KeyCode::Char(c @ '1'..='9') = key.code else {
                    return vec![];
                };
                match self.board.repo_paths.get((c as usize) - ('1' as usize)) {
                    Some(path) => M::Toggle(path.clone()),
                    None => return vec![],
                }
            }
            "repo_filter_toggle_mode" if tab == FilterTab::Repos => M::ToggleMode,
            "repo_filter_delete_repo_path"
                if tab == FilterTab::Repos
                    && self.input.repo_cursor < self.board.repo_paths.len() =>
            {
                M::StartDeleteRepoPath
            }
            "filter_toggle_done_epics" if tab == FilterTab::Epics => M::ToggleShowDoneEpics,
            "filter_search_epics" if tab == FilterTab::Epics => M::StartEpicSearch,
            "filter_fold_epics" if tab == FilterTab::Epics => M::ToggleEpicFold,
            "filter_fold_all_epics" if tab == FilterTab::Epics => M::ToggleAllEpicFolds,
            _ => return vec![],
        };
        self.dispatch_keyed(Message::BoardFilter(msg), action, label)
    }

    /// Space's message for the row under the current tab's cursor, if any.
    fn filter_row_toggle(&self) -> Option<crate::tui::messages::BoardFilterMessage> {
        use crate::tui::messages::BoardFilterMessage as M;
        let overlay = &self.filter_overlay;
        match overlay.tab {
            FilterTab::Repos => self
                .board
                .repo_paths
                .get(self.input.repo_cursor)
                .map(|p| M::Toggle(p.clone())),
            FilterTab::Epics => self
                .filter_epic_rows()
                .get(overlay.epic_cursor)
                .map(|r| M::ToggleEpic(r.id)),
            FilterTab::Packages => self
                .board
                .work_packages
                .get(overlay.package_cursor)
                .map(|p| M::TogglePackage(p.id)),
        }
    }

    /// Keys while typing the Epics tab's title search. Typed characters and
    /// Backspace are text and reach the query without passing through here.
    pub(in crate::tui) fn handle_key_board_filter_search(
        &mut self,
        action: &str,
        label: &str,
    ) -> Vec<Command> {
        use crate::tui::messages::BoardFilterMessage as M;
        let keep = match action {
            "filter_search_commit" => true,
            "filter_search_cancel" => false,
            _ => return vec![],
        };
        self.dispatch_keyed(
            Message::BoardFilter(M::EndEpicSearch { keep }),
            action,
            label,
        )
    }

    pub(in crate::tui) fn handle_key_confirm_delete_repo_path(
        &mut self,
        label: &str,
        yes: bool,
    ) -> Vec<Command> {
        if yes {
            if let Some(path) = self.board.repo_paths.get(self.input.repo_cursor).cloned() {
                return self.dispatch_keyed(
                    Message::BoardFilter(crate::tui::messages::BoardFilterMessage::DeleteRepoPath(
                        path,
                    )),
                    "confirm_delete_repo_path_yes",
                    label,
                );
            }
        }
        // Declined, or the cursor no longer points at a deletable row — the
        // prompt just closes, which is the same outcome as declining.
        self.input.mode = InputMode::BoardFilter;
        vec![key_event("confirm_delete_repo_path_no", label)]
    }
}
