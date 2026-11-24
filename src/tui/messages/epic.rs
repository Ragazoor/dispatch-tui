//! Epic CRUD, lifecycle, batch ops, and creation-flow messages.

use crate::models::{Epic, EpicId};

use super::super::types::{Command, MoveDirection, TreeNav};
use crate::tui::update::{epic_input, selection};
use crate::tui::App;

/// Messages targeting the epic domain.
///
/// Wrapped by [`crate::tui::types::Message::Epic`] for dispatch.
#[derive(Debug, Clone)]
pub enum EpicMessage {
    Enter(EpicId),
    Exit,
    /// Leave every epic view at once (`Q`).
    ExitAll,
    /// Open `target`'s epic view with every epic between the current view and
    /// it on the trail (`epics.allium: Epic Navigation`).
    JumpTo(EpicId),
    /// `Enter` on an epic card in the column `status`: jump to the deepest
    /// epic holding the work that puts the card there.
    JumpToDeepest(EpicId, crate::models::TaskStatus),
    Refresh(Vec<Epic>),
    /// The orchestrators and what they keep, from the board reload.
    OrchestrationRefreshed(Box<crate::models::OrchestrationBoard>),
    /// The ready key on a task card (`orchestrators.allium: ReadyShowsOnTheCard`).
    ToggleReady,
    /// The orchestrator key (`OrchestratorKey`).
    OpenOrchestrator,
    /// The resolve key (`ResolveKey`).
    ResolveItemKey,
    /// This machine's orchestrator ended on a done epic (`NotifyEpicFinished`).
    EpicFinished(EpicId),
    /// Splice a single fresh epic into `app.board.epics`.
    Updated(Box<Epic>),
    Created(Box<Epic>),
    Edit(EpicId),
    Edited(crate::tui::EpicEdit),
    Delete(EpicId),
    ToggleGroupByRepo(EpicId),
    ConfirmDelete,
    MoveStatus(EpicId, MoveDirection),
    StartNew,
    SubmitTitle(String),
    ToggleSelect(EpicId),
    StartReparent(EpicId),
    ReparentNavigate(TreeNav),
    ReparentConfirm,
    ReparentExecute,
    ReparentCancel,
    ReparentCancelAll,
    /// `epics.allium: EditEpic`'s take-over prompt: a `feed_command` change
    /// just applied conflicts with an existing `core/PollOwner` claim naming
    /// `other_host`. `feeds.allium: OverrideFeedOwner` is what accepting it
    /// calls.
    FeedOwnerTakeoverOffered {
        epic_id: EpicId,
        other_host: String,
    },
}

impl EpicMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            EpicMessage::Enter(id) => app.handle_enter_epic(id),
            EpicMessage::Exit => app.handle_exit_epic(),
            EpicMessage::ExitAll => app.handle_exit_all_epics(),
            EpicMessage::JumpTo(target) => app.handle_jump_to_epic(target),
            EpicMessage::JumpToDeepest(id, status) => app.handle_jump_to_deepest_epic(id, status),
            EpicMessage::Refresh(epics) => app.handle_refresh_epics(epics),
            EpicMessage::OrchestrationRefreshed(board) => {
                app.handle_orchestration_refreshed(*board)
            }
            EpicMessage::ToggleReady => app.handle_toggle_ready(),
            EpicMessage::OpenOrchestrator => app.handle_open_orchestrator(),
            EpicMessage::ResolveItemKey => app.handle_resolve_item_key(),
            EpicMessage::EpicFinished(epic) => app.handle_epic_finished(epic),
            EpicMessage::Updated(epic) => app.handle_epic_updated(*epic),
            EpicMessage::Created(epic) => app.handle_epic_created(*epic),
            EpicMessage::Edit(id) => app.handle_edit_epic(id),
            EpicMessage::Edited(edit) => app.handle_epic_edited(edit),
            EpicMessage::Delete(id) => app.handle_delete_epic(id),
            EpicMessage::ToggleGroupByRepo(id) => {
                selection::toggle_epic_group_by_repo(&mut app.board.epics, id)
            }
            EpicMessage::ConfirmDelete => app.handle_confirm_delete_epic(),
            EpicMessage::MoveStatus(id, dir) => app.handle_move_epic_status(id, dir),
            EpicMessage::StartNew => {
                epic_input::start_new_epic(&mut app.input, &mut app.status, &app.board.view_mode)
            }
            EpicMessage::SubmitTitle(v) => {
                epic_input::submit_epic_title(&mut app.input, &mut app.status, v)
            }
            EpicMessage::ToggleSelect(id) => {
                app.select.toggle_epic(id);
                vec![]
            }
            EpicMessage::StartReparent(id) => {
                let items =
                    crate::tui::ui::build_reparent_tree(&app.view().reparent_target_epics(id));
                let current_parent = app
                    .board
                    .epics
                    .iter()
                    .find(|e| e.id == id)
                    .and_then(|e| e.parent_epic_id);
                epic_input::start_reparent(
                    &mut app.input,
                    &mut app.interaction,
                    id,
                    items,
                    current_parent,
                );
                vec![]
            }
            EpicMessage::ReparentNavigate(nav) => {
                if epic_input::reparent_navigate(&app.interaction, nav) {
                    app.dirty = true;
                }
                vec![]
            }
            EpicMessage::ReparentConfirm => {
                epic_input::reparent_confirm(
                    &mut app.input,
                    &app.interaction,
                    &mut app.status,
                    &app.board.epics,
                    app.key_table,
                );
                vec![]
            }
            EpicMessage::ReparentExecute => {
                epic_input::reparent_execute(&mut app.input, &mut app.interaction, &mut app.status)
            }
            EpicMessage::ReparentCancel => {
                epic_input::reparent_cancel(&mut app.input, &mut app.interaction, &mut app.status)
            }
            EpicMessage::ReparentCancelAll => epic_input::reparent_cancel_all(
                &mut app.input,
                &mut app.interaction,
                &mut app.status,
            ),
            EpicMessage::FeedOwnerTakeoverOffered {
                epic_id,
                other_host,
            } => epic_input::feed_owner_takeover_offered(
                &mut app.input,
                &mut app.status,
                epic_id,
                other_host,
                app.key_table,
            ),
        }
    }
}
