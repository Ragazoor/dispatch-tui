//! Project management side-effect commands.

use crate::models::ProjectId;

/// Side-effect commands for the project management domain.
///
/// Wrapped by [`crate::tui::types::Command::Project`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum ProjectCommand {
    Create { name: String },
    Rename { id: ProjectId, name: String },
    Delete { id: ProjectId },
    /// +1 = move down (higher sort_order), -1 = move up (lower sort_order)
    Reorder { id: ProjectId, delta: i8 },
}
