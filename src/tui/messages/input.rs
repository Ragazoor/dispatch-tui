//! Form input, text-entry, and confirmation-flow messages for the task
//! creation, copy, edit, and delete flows.

use crate::models::{TaskTag, WrapUpMode};

use crate::tui::types::Command;
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
    /// A repository's own default branch, come back from
    /// `SettingsCommand::DetectDefaultBranch`. Applied only while the
    /// base-branch step is still open and its buffer still holds `replacing` —
    /// see `DetectedPrefillNeverOverwritesTyping` in docs/specs/dispatch.allium.
    DefaultBranchDetected {
        branch: String,
        replacing: String,
    },
    SubmitWrapUpMode(Option<WrapUpMode>),
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
            InputMessage::StartNewTask => app.handle_start_new_task(),
            InputMessage::CopyTask => app.handle_copy_task(),
            InputMessage::CancelInput => app.handle_cancel_input(),
            InputMessage::SubmitTitle(value) => app.handle_submit_title(value),
            InputMessage::SubmitDescription(value) => app.handle_submit_description(value),
            InputMessage::SubmitRepoPath(value) => app.handle_submit_repo_path(value),
            InputMessage::SubmitTag(tag) => app.handle_submit_tag(tag),
            InputMessage::SubmitBaseBranch(value) => app.handle_submit_base_branch(value),
            InputMessage::DefaultBranchDetected { branch, replacing } => {
                app.handle_default_branch_detected(branch, replacing)
            }
            InputMessage::SubmitWrapUpMode(mode) => app.handle_submit_wrap_up_mode(mode),
            InputMessage::ArmPhoenix => app.handle_arm_phoenix(),
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
                app.handle_start_quick_dispatch_selection()
            }
            InputMessage::SelectQuickDispatchRepo(idx) => {
                app.handle_select_quick_dispatch_repo(idx)
            }
            InputMessage::CancelRetry => app.handle_cancel_retry(),
            InputMessage::ConfirmDone => app.handle_confirm_done(),
            InputMessage::CancelDone => app.handle_cancel_done(),
            InputMessage::ConfirmDetachTmux => app.handle_confirm_detach_tmux(),
        }
    }
}
