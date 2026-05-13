use crate::models::TaskId;

pub const HEADER_TASK_ID: &str = "X-Caller-Task-Id";
pub const HEADER_KIND: &str = "X-Caller-Kind";
pub const KIND_SESSION: &str = "session";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallerIdentity {
    Task(TaskId),
    Session,
}

impl CallerIdentity {
    /// Parse caller identity from the two optional header values.
    ///
    /// Exactly one of the two headers must be set:
    /// - `Some(task_id_str)` + `None` → `Task(TaskId)`, or `InvalidTaskId` on bad parse.
    /// - `None` + `Some("session")` → `Session`; any other value → `UnknownKind`.
    /// - Both present → `Conflict`.
    /// - Neither present → `Missing`.
    pub fn from_headers(
        task_id_header: Option<&str>,
        kind_header: Option<&str>,
    ) -> Result<Self, IdentityError> {
        match (task_id_header, kind_header) {
            (Some(_), Some(_)) => Err(IdentityError::Conflict),
            (Some(raw), None) => raw
                .parse::<i64>()
                .map(|n| CallerIdentity::Task(TaskId(n)))
                .map_err(|_| IdentityError::InvalidTaskId(raw.to_string())),
            (None, Some(k)) if k == KIND_SESSION => Ok(CallerIdentity::Session),
            (None, Some(other)) => Err(IdentityError::UnknownKind(other.to_string())),
            (None, None) => Err(IdentityError::Missing),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityError {
    Missing,
    Conflict,
    UnknownKind(String),
    InvalidTaskId(String),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::Missing => write!(f, "missing X-Caller-* identity header"),
            IdentityError::Conflict => {
                write!(f, "both X-Caller-Task-Id and X-Caller-Kind set")
            }
            IdentityError::UnknownKind(k) => write!(f, "unknown X-Caller-Kind: {k}"),
            IdentityError::InvalidTaskId(s) => write!(f, "invalid X-Caller-Task-Id: {s}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_header_parses_to_task() {
        assert_eq!(
            CallerIdentity::from_headers(Some("42"), None),
            Ok(CallerIdentity::Task(TaskId(42)))
        );
    }

    #[test]
    fn kind_session_parses() {
        assert_eq!(
            CallerIdentity::from_headers(None, Some("session")),
            Ok(CallerIdentity::Session)
        );
    }

    #[test]
    fn missing_headers_rejected() {
        assert_eq!(
            CallerIdentity::from_headers(None, None),
            Err(IdentityError::Missing)
        );
    }

    #[test]
    fn conflicting_headers_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(Some("1"), Some("session")),
            Err(IdentityError::Conflict)
        ));
    }

    #[test]
    fn non_numeric_task_id_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(Some("not-a-number"), None),
            Err(IdentityError::InvalidTaskId(_))
        ));
    }

    #[test]
    fn unknown_kind_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(None, Some("bogus")),
            Err(IdentityError::UnknownKind(_))
        ));
    }

    #[test]
    fn empty_task_id_header_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(Some(""), None),
            Err(IdentityError::InvalidTaskId(s)) if s.is_empty()
        ));
    }

    #[test]
    fn empty_kind_header_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(None, Some("")),
            Err(IdentityError::UnknownKind(s)) if s.is_empty()
        ));
    }

    #[test]
    fn whitespace_padded_task_id_is_rejected() {
        // Rust's i64::parse rejects leading/trailing whitespace.
        assert!(matches!(
            CallerIdentity::from_headers(Some(" 42 "), None),
            Err(IdentityError::InvalidTaskId(_))
        ));
    }
}
