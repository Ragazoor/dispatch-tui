//! Form input, text-entry, and confirmation-flow messages for the task
//! creation, copy, edit, and delete flows.

use crate::models::{TaskTag, WrapUpMode};

/// Messages targeting the form-input flow.
///
/// Wrapped by [`crate::tui::types::Message::Input`] for dispatch.
#[derive(Debug, Clone)]
pub enum InputMessage {
    StartNewTask,
    CopyTask,
    CancelInput,
    ConfirmDeleteStart,
    ConfirmDeleteYes,
    CancelDelete,
    SubmitTitle(String),
    SubmitDescription(String),
    SubmitRepoPath(String),
    SubmitTag(Option<TaskTag>),
    SubmitBaseBranch(String),
    SubmitWrapUpMode(Option<WrapUpMode>),
    InputChar(char),
    InputBackspace,
    StartQuickDispatchSelection,
    SelectQuickDispatchRepo(usize),
    CancelRetry,
    ConfirmDone,
    CancelDone,
    ConfirmDetachTmux,
}
