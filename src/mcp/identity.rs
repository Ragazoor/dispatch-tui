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
    /// Parse from raw header values. Both inputs are `Option<&str>` because
    /// either header may be absent.
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

#[derive(Debug, PartialEq, Eq)]
pub enum IdentityError {
    Missing,
    Conflict,
    UnknownKind(String),
    InvalidTaskId(String),
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
}
