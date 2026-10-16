//! Board-filter overlay messages: the repo filter, the epic and work package
//! hide-lists, and the overlay's tabs.

use crate::models::{EpicId, WorkPackageId};
use crate::tui::types::{Command, MoveDirection};
use crate::tui::App;

#[derive(Debug, Clone)]
pub enum BoardFilterMessage {
    Start,
    Close,
    Toggle(String),
    ToggleMode,
    /// Move the repo cursor. Shared with the repo-path pickers, which list
    /// repos too; the overlay's own cursor move is [`Self::MoveRowCursor`].
    MoveCursor(isize),
    StartDeleteRepoPath,
    DeleteRepoPath(String),
    /// Move the cursor on whichever tab is showing.
    MoveRowCursor(isize),
    SwitchTab(MoveDirection),
    ToggleEpic(EpicId),
    TogglePackage(WorkPackageId),
    ToggleShowDoneEpics,
    StartEpicSearch,
    /// `z`: fold or unfold the epic row under the cursor.
    ToggleEpicFold,
    /// `Z`: fold every parent, or unfold all when any is folded.
    ToggleAllEpicFolds,
    EpicQueryChar(char),
    EpicQueryBackspace,
    /// Leave the epic search: `keep` holds the query, otherwise it is cleared.
    EndEpicSearch {
        keep: bool,
    },
    /// Esc on the list: clear a standing epic query, or close the overlay.
    Back,
}

impl BoardFilterMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            BoardFilterMessage::Start => app.handle_start_board_filter(),
            BoardFilterMessage::Close => app.handle_close_board_filter(),
            BoardFilterMessage::Toggle(path) => app.handle_toggle_repo_filter(path),
            BoardFilterMessage::ToggleMode => app.handle_toggle_repo_filter_mode(),
            BoardFilterMessage::MoveCursor(delta) => app.handle_move_repo_cursor(delta),
            BoardFilterMessage::StartDeleteRepoPath => app.handle_start_delete_repo_path(),
            BoardFilterMessage::DeleteRepoPath(path) => app.handle_delete_repo_path(path),
            BoardFilterMessage::MoveRowCursor(delta) => app.handle_move_filter_cursor(delta),
            BoardFilterMessage::SwitchTab(direction) => app.handle_switch_filter_tab(direction),
            BoardFilterMessage::ToggleEpic(id) => app.handle_toggle_hidden_epic(id),
            BoardFilterMessage::TogglePackage(id) => app.handle_toggle_hidden_package(id),
            BoardFilterMessage::ToggleShowDoneEpics => app.handle_toggle_show_done_epics(),
            BoardFilterMessage::StartEpicSearch => app.handle_start_epic_search(),
            BoardFilterMessage::ToggleEpicFold => app.handle_toggle_filter_epic_fold(),
            BoardFilterMessage::ToggleAllEpicFolds => app.handle_toggle_all_filter_epic_folds(),
            BoardFilterMessage::EpicQueryChar(c) => app.handle_epic_query_edit(Some(c)),
            BoardFilterMessage::EpicQueryBackspace => app.handle_epic_query_edit(None),
            BoardFilterMessage::EndEpicSearch { keep } => app.handle_end_epic_search(keep),
            BoardFilterMessage::Back => app.handle_board_filter_back(),
        }
    }
}
