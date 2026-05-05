pub mod epics;
pub mod learnings;
pub mod tasks;

pub use epics::{CreateEpicParams, EpicService, UpdateEpicParams};
pub use learnings::{CreateLearningParams, LearningService, UpdateLearningParams};
pub use tasks::{
    ClaimTaskParams, CreateTaskParams, ListTasksFilter, TaskService, UpdateTaskParams,
};

// ---------------------------------------------------------------------------
// Service error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ServiceError {
    /// Client-provided data is invalid (bad status, missing fields, etc.)
    Validation(String),
    /// Entity not found
    NotFound(String),
    /// Database or internal error
    Internal(String),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Validation(msg) => write!(f, "{msg}"),
            ServiceError::NotFound(msg) => write!(f, "{msg}"),
            ServiceError::Internal(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ServiceError {}

// ---------------------------------------------------------------------------
// FieldUpdate — explicit set-or-clear for nullable string fields
// ---------------------------------------------------------------------------

/// Replaces the `Option<String>` + empty-string sentinel pattern.
/// `Set(value)` sets the field, `Clear` sets it to NULL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldUpdate {
    Set(String),
    Clear,
}
