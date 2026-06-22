//! Personal TODO overlay side-effect commands.

use crate::models::TodoId;
use crate::service::TodoUpdate;

/// Side-effect commands for the personal TODO view.
///
/// Wrapped by [`crate::tui::types::Command::Todo`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum TodoCommand {
    Load,
    Create { title: String, reopen: bool },
    Update { id: TodoId, update: TodoUpdate },
    Delete(TodoId),
    ClearDone,
    LoadCount,
}
