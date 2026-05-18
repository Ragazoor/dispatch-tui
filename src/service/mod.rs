pub mod embeddings;
pub mod epics;
pub mod learnings;
pub mod tasks;

pub use epics::{CreateEpicParams, EpicService, UpdateEpicParams};
pub use learnings::{CreateLearningParams, LearningService, UpdateLearningParams};
pub use tasks::{
    ClaimTaskParams, CreateTaskParams, ListTasksFilter, TaskService, UpdateTaskParams,
    UpdateTaskResult,
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

impl FieldUpdate {
    /// Translate the legacy MCP convention where `Some("")` means "clear to
    /// NULL", `Some(v)` means "set to v", and `None` means "do not touch" into
    /// an `Option<FieldUpdate>` patch value.
    pub fn from_optional_string(s: Option<String>) -> Option<FieldUpdate> {
        match s {
            None => None,
            Some(v) if v.is_empty() => Some(FieldUpdate::Clear),
            Some(v) => Some(FieldUpdate::Set(v)),
        }
    }
}

#[cfg(test)]
mod field_update_tests {
    use super::FieldUpdate;

    #[test]
    fn from_optional_string_none_is_noop() {
        assert_eq!(FieldUpdate::from_optional_string(None), None);
    }

    #[test]
    fn from_optional_string_empty_clears() {
        assert_eq!(
            FieldUpdate::from_optional_string(Some(String::new())),
            Some(FieldUpdate::Clear)
        );
    }

    #[test]
    fn from_optional_string_value_sets() {
        assert_eq!(
            FieldUpdate::from_optional_string(Some("https://x".to_string())),
            Some(FieldUpdate::Set("https://x".to_string()))
        );
    }
}
