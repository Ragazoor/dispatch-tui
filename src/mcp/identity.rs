use crate::models::TaskId;

pub const HEADER_TASK_ID: &str = "X-Caller-Task-Id";
pub const HEADER_KIND: &str = "X-Caller-Kind";
pub const KIND_SESSION: &str = "session";
/// The orchestrator's caller header (`orchestrators.allium:
/// ItsOwnCallerIdentity`), carrying its name.
pub const HEADER_ORCHESTRATOR: &str = "X-Caller-Orchestrator";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallerIdentity {
    Task(TaskId),
    /// An epic's orchestrator on this machine, by its name.
    Orchestrator(String),
    Session,
}

impl CallerIdentity {
    /// Parse caller identity from the three optional header values.
    ///
    /// Exactly one of the three headers must be set:
    /// - the task id → `Task(TaskId)`, or `InvalidTaskId` on bad parse.
    /// - the kind `"session"` → `Session`; any other value → `UnknownKind`.
    /// - the orchestrator's name → `Orchestrator`; a blank one →
    ///   `InvalidOrchestrator`.
    /// - More than one present → `Conflict`.
    /// - None present → `Missing`.
    pub fn from_headers(
        task_id_header: Option<&str>,
        kind_header: Option<&str>,
        orchestrator_header: Option<&str>,
    ) -> Result<Self, IdentityError> {
        let present = [task_id_header, kind_header, orchestrator_header]
            .iter()
            .filter(|h| h.is_some())
            .count();
        if present > 1 {
            return Err(IdentityError::Conflict);
        }
        match (task_id_header, kind_header, orchestrator_header) {
            (Some(raw), _, _) => raw
                .parse::<TaskId>()
                .map(CallerIdentity::Task)
                .map_err(|_| IdentityError::InvalidTaskId(raw.to_string())),
            (_, Some(k), _) if k == KIND_SESSION => Ok(CallerIdentity::Session),
            (_, Some(other), _) => Err(IdentityError::UnknownKind(other.to_string())),
            (_, _, Some(name)) if name.trim().is_empty() => {
                Err(IdentityError::InvalidOrchestrator(name.to_string()))
            }
            (_, _, Some(name)) => Ok(CallerIdentity::Orchestrator(name.trim().to_string())),
            (None, None, None) => Err(IdentityError::Missing),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityError {
    Missing,
    Conflict,
    UnknownKind(String),
    InvalidTaskId(String),
    InvalidOrchestrator(String),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::Missing => write!(f, "missing X-Caller-* identity header"),
            IdentityError::Conflict => {
                write!(f, "more than one of X-Caller-Task-Id, X-Caller-Kind and X-Caller-Orchestrator set")
            }
            IdentityError::UnknownKind(k) => write!(f, "unknown X-Caller-Kind: {k}"),
            IdentityError::InvalidTaskId(s) => write!(f, "invalid X-Caller-Task-Id: {s}"),
            IdentityError::InvalidOrchestrator(s) => {
                write!(f, "invalid X-Caller-Orchestrator: {s:?}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_header_parses_to_task() {
        assert_eq!(
            CallerIdentity::from_headers(Some("42"), None, None),
            Ok(CallerIdentity::Task(TaskId(42)))
        );
    }

    #[test]
    fn kind_session_parses() {
        assert_eq!(
            CallerIdentity::from_headers(None, Some("session"), None),
            Ok(CallerIdentity::Session)
        );
    }

    #[test]
    fn missing_headers_rejected() {
        assert_eq!(
            CallerIdentity::from_headers(None, None, None),
            Err(IdentityError::Missing)
        );
    }

    #[test]
    fn conflicting_headers_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(Some("1"), Some("session"), None),
            Err(IdentityError::Conflict)
        ));
    }

    #[test]
    fn non_numeric_task_id_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(Some("not-a-number"), None, None),
            Err(IdentityError::InvalidTaskId(_))
        ));
    }

    #[test]
    fn unknown_kind_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(None, Some("bogus"), None),
            Err(IdentityError::UnknownKind(_))
        ));
    }

    #[test]
    fn empty_task_id_header_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(Some(""), None, None),
            Err(IdentityError::InvalidTaskId(s)) if s.is_empty()
        ));
    }

    #[test]
    fn empty_kind_header_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(None, Some(""), None),
            Err(IdentityError::UnknownKind(s)) if s.is_empty()
        ));
    }

    #[test]
    fn orchestrator_header_parses_to_its_name() {
        assert_eq!(
            CallerIdentity::from_headers(None, None, Some("zlata2")),
            Ok(CallerIdentity::Orchestrator("zlata2".into()))
        );
    }

    #[test]
    fn orchestrator_beside_another_header_conflicts() {
        assert_eq!(
            CallerIdentity::from_headers(Some("1"), None, Some("zlata2")),
            Err(IdentityError::Conflict)
        );
        assert_eq!(
            CallerIdentity::from_headers(None, Some("session"), Some("zlata2")),
            Err(IdentityError::Conflict)
        );
    }

    #[test]
    fn blank_orchestrator_header_rejected() {
        assert!(matches!(
            CallerIdentity::from_headers(None, None, Some("  ")),
            Err(IdentityError::InvalidOrchestrator(_))
        ));
    }

    #[test]
    fn whitespace_padded_task_id_is_rejected() {
        // Rust's i64::parse rejects leading/trailing whitespace.
        assert!(matches!(
            CallerIdentity::from_headers(Some(" 42 "), None, None),
            Err(IdentityError::InvalidTaskId(_))
        ));
    }
}
