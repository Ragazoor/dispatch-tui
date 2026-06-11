pub mod api;
pub mod embeddings;
pub mod epics;
pub mod learnings;
pub mod repo_index;
pub mod tasks;

pub use api::{EpicServiceApi, TaskServiceApi};
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
    /// Database or internal error; preserves the underlying anyhow source chain.
    Internal(anyhow::Error),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Validation(msg) => write!(f, "{msg}"),
            ServiceError::NotFound(msg) => write!(f, "{msg}"),
            ServiceError::Internal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServiceError::Internal(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for ServiceError {
    fn from(e: anyhow::Error) -> Self {
        ServiceError::Internal(e)
    }
}

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

    /// Convert to `Option<&str>` for use with DB patch builders.
    /// `Set(s)` → `Some(s)`, `Clear` → `None`.
    pub fn as_option(&self) -> Option<&str> {
        match self {
            FieldUpdate::Set(s) => Some(s.as_str()),
            FieldUpdate::Clear => None,
        }
    }
}

/// Set-or-clear for the typed URL field. Mirrors [`FieldUpdate`] but carries a
/// whole [`TaskUrl`](crate::models::TaskUrl) so the url and its type are always
/// updated together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlUpdate {
    Set(crate::models::TaskUrl),
    Clear,
}

impl UrlUpdate {
    /// `Set(u)` → `Some(&u)`, `Clear` → `None`, for the DB patch builder.
    pub fn as_option(&self) -> Option<&crate::models::TaskUrl> {
        match self {
            UrlUpdate::Set(u) => Some(u),
            UrlUpdate::Clear => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod error_tests {
    use super::ServiceError;
    use std::error::Error;

    #[test]
    fn internal_error_preserves_source_chain() {
        let anyhow_err = anyhow::anyhow!("db connection failed");
        let service_err = ServiceError::from(anyhow_err);
        assert!(
            service_err.source().is_some(),
            "ServiceError::Internal should expose its anyhow error as source()"
        );
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
    fn as_option_set_returns_some() {
        let fu = FieldUpdate::Set("https://example.com".to_string());
        assert_eq!(fu.as_option(), Some("https://example.com"));
    }

    #[test]
    fn as_option_clear_returns_none() {
        assert_eq!(FieldUpdate::Clear.as_option(), None);
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
