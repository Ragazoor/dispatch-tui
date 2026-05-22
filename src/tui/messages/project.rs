//! Project management messages.

use crate::models::{Project, ProjectId};

/// Messages targeting the project management domain.
///
/// Wrapped by [`crate::tui::types::Message::Project`] for dispatch.
#[derive(Debug, Clone)]
pub enum ProjectMessage {
    Updated(Vec<Project>),
    Select(ProjectId),
    Follow(ProjectId),
}
