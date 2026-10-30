//! Form input, text-entry, and confirmation-flow messages for the task
//! creation, copy, edit, and delete flows.

use crate::models::{TaskModel, TaskTag, WrapUpMode};

use crate::tui::types::Command;
use crate::tui::update::forms;
use crate::tui::App;

/// Messages targeting the form-input flow.
///
/// Wrapped by [`crate::tui::types::Message::Input`] for dispatch.
#[derive(Debug, Clone)]
pub enum InputMessage {
    StartNewTask,
    CopyTask,
    CancelInput,
    SubmitTitle(String),
    SubmitDescription(String),
    SubmitRepoPath(String),
    SubmitTag(Option<TaskTag>),
    SubmitBaseBranch(String),
    /// `u` on the board (`workspaces.allium: SetOwnNameFromBoard`).
    StartSetDisplayName,
    /// Enter in the display-name input.
    SubmitDisplayName(String),
    /// A repository's own default branch, come back from
    /// `SettingsCommand::DetectDefaultBranch`. Applied only while the
    /// base-branch step is still open and its buffer still holds `replacing` —
    /// see `DetectedPrefillNeverOverwritesTyping` in docs/specs/dispatch.allium.
    DefaultBranchDetected {
        branch: String,
        replacing: String,
    },
    SubmitWrapUpMode(Option<WrapUpMode>),
    /// The model step's answer; `None` is Enter, which keeps the draft's
    /// model (CreateTask: ModelPicker, in `docs/specs/tasks.allium`).
    SubmitModel(Option<TaskModel>),
    /// The person's workspaces, read when a create form opened.
    WorkspacesLoaded(Vec<crate::models::WorkspaceRef>),
    /// Space, `h` or `l` at the placement step.
    PlacementToggleVisibility,
    /// `j` (+1) or `k` (-1) at the placement step.
    PlacementMoveWorkspace(isize),
    /// Enter at the placement step.
    SubmitPlacement,
    /// `p` at the tag picker: arm the phoenix flag and re-open the same step
    /// for the real tag. Carries no payload — there is no message that DISARMS
    /// the flag, because declining it is simply not pressing `p`
    /// (CreateTask: PhoenixArming, in `docs/specs/tasks.allium`).
    ArmPhoenix,
    InputChar(char),
    InputBackspace,
    InputDeleteForward,
    CursorLeft,
    CursorRight,
    CursorWordLeft,
    CursorWordRight,
    CursorHome,
    CursorEnd,
    StartQuickDispatchSelection,
    SelectQuickDispatchRepo(usize),
    CancelRetry,
    ConfirmDone,
    CancelDone,
    ConfirmDetachTmux,
}

impl InputMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            InputMessage::StartNewTask => forms::start_new_task(&mut app.input, &mut app.status),
            InputMessage::CopyTask => match app.selected_task().cloned() {
                Some(task) => forms::copy_task(&mut app.input, &mut app.status, &task),
                None => vec![],
            },
            InputMessage::CancelInput
                if app.input.mode == crate::tui::types::InputMode::InputAdminText =>
            {
                app.cancel_admin_prompt();
                vec![]
            }
            InputMessage::CancelInput => forms::cancel_input(
                &mut app.input,
                &mut app.interaction,
                &mut app.status,
                app.board.work_packages.len(),
            ),
            InputMessage::SubmitTitle(value) => {
                forms::submit_title(&mut app.input, &mut app.status, value)
            }
            InputMessage::SubmitDescription(value) => {
                forms::submit_description(&mut app.input, &mut app.status, value)
            }
            InputMessage::SubmitRepoPath(value) => forms::submit_repo_path(
                &mut app.input,
                &mut app.status,
                &app.board.repo_base_branches,
                value,
            ),
            InputMessage::SubmitTag(tag) => forms::submit_tag(&mut app.input, &mut app.status, tag),
            InputMessage::SubmitBaseBranch(value) => {
                forms::submit_base_branch(&mut app.input, &mut app.status, value)
            }
            InputMessage::StartSetDisplayName => app.handle_start_set_display_name(),
            InputMessage::SubmitDisplayName(name) => app.handle_submit_display_name(name),
            InputMessage::DefaultBranchDetected { branch, replacing } => {
                forms::default_branch_detected(&mut app.input, branch, replacing)
            }
            InputMessage::SubmitWrapUpMode(mode) => {
                forms::submit_wrap_up_mode(&mut app.input, &mut app.status, mode)
            }
            InputMessage::SubmitModel(model) => {
                forms::apply_model(&mut app.input, model);
                app.finish_task_creation()
            }
            InputMessage::WorkspacesLoaded(workspaces) => app.handle_workspaces_loaded(workspaces),
            InputMessage::PlacementToggleVisibility => app.handle_placement_toggle_visibility(),
            InputMessage::PlacementMoveWorkspace(delta) => {
                app.handle_placement_move_workspace(delta)
            }
            InputMessage::SubmitPlacement => app.handle_submit_placement(),
            InputMessage::ArmPhoenix => forms::arm_phoenix(&mut app.input, &mut app.status),
            InputMessage::InputChar(c) => {
                app.input.insert_char(c);
                vec![]
            }
            InputMessage::InputBackspace => {
                app.input.backspace();
                vec![]
            }
            InputMessage::InputDeleteForward => {
                app.input.delete_forward();
                vec![]
            }
            InputMessage::CursorLeft => {
                app.input.cursor_left();
                vec![]
            }
            InputMessage::CursorRight => {
                app.input.cursor_right();
                vec![]
            }
            InputMessage::CursorWordLeft => {
                app.input.cursor_word_left();
                vec![]
            }
            InputMessage::CursorWordRight => {
                app.input.cursor_word_right();
                vec![]
            }
            InputMessage::CursorHome => {
                app.input.cursor_home();
                vec![]
            }
            InputMessage::CursorEnd => {
                app.input.cursor_end();
                vec![]
            }
            InputMessage::StartQuickDispatchSelection => {
                forms::start_quick_dispatch_selection(&mut app.input, &mut app.status)
            }
            InputMessage::SelectQuickDispatchRepo(idx) => {
                match forms::take_quick_dispatch_repo(
                    &mut app.input,
                    &mut app.status,
                    &app.board.repo_paths,
                    idx,
                ) {
                    Some((repo_path, epic_id)) => app.handle_quick_dispatch(repo_path, epic_id),
                    None => vec![],
                }
            }
            InputMessage::CancelRetry => forms::cancel_retry(&mut app.input, &mut app.status),
            InputMessage::ConfirmDone => app.handle_confirm_done(),
            InputMessage::CancelDone => app.handle_cancel_done(),
            InputMessage::ConfirmDetachTmux => app.handle_confirm_detach_tmux(),
        }
    }
}
