use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Ok,
    Warn,
    Error,
}

impl FindingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
    pub fn is_problem(&self) -> bool {
        matches!(self, Self::Warn | Self::Error)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub check: &'static str,
    pub status: FindingStatus,
    pub target: String,
    pub message: String,
    pub repair_available: bool,
}

/// Format findings as human-readable lines.
pub fn format_human(findings: &[Finding]) -> String {
    findings
        .iter()
        .map(|f| {
            let status = match f.status {
                FindingStatus::Ok => "ok   ",
                FindingStatus::Warn => "warn ",
                FindingStatus::Error => "error",
            };
            format!("{}  {}  {}  {}", status, f.check, f.target, f.message)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Serialize findings to a JSON array string.
pub fn format_json(findings: &[Finding]) -> String {
    serde_json::to_string_pretty(findings).unwrap_or_else(|_| "[]".to_string())
}

/// Returns true if any finding is warn or error.
pub fn has_problems(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.status.is_problem())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn ok_finding() -> Finding {
        Finding {
            check: "hooks",
            status: FindingStatus::Ok,
            target: "/repo".to_string(),
            message: "core.hooksPath = .githooks".to_string(),
            repair_available: false,
        }
    }

    fn warn_finding() -> Finding {
        Finding {
            check: "worktrees",
            status: FindingStatus::Warn,
            target: "/repo/.worktrees/task-5".to_string(),
            message: "directory exists but no matching DB row".to_string(),
            repair_available: true,
        }
    }

    #[test]
    fn finding_status_as_str() {
        assert_eq!(FindingStatus::Ok.as_str(), "ok");
        assert_eq!(FindingStatus::Warn.as_str(), "warn");
        assert_eq!(FindingStatus::Error.as_str(), "error");
    }

    #[test]
    fn finding_status_is_problem() {
        assert!(!FindingStatus::Ok.is_problem());
        assert!(FindingStatus::Warn.is_problem());
        assert!(FindingStatus::Error.is_problem());
    }

    #[test]
    fn format_human_ok_line() {
        let out = format_human(&[ok_finding()]);
        assert!(out.starts_with("ok   "), "expected 'ok   ' prefix, got: {out}");
        assert!(out.contains("hooks"), "expected check name, got: {out}");
        assert!(out.contains("/repo"), "expected target, got: {out}");
    }

    #[test]
    fn format_human_warn_line() {
        let out = format_human(&[warn_finding()]);
        assert!(out.starts_with("warn "), "expected 'warn ' prefix, got: {out}");
    }

    #[test]
    fn format_human_multiple_lines() {
        let out = format_human(&[ok_finding(), warn_finding()]);
        assert_eq!(out.lines().count(), 2);
    }

    #[test]
    fn format_json_is_valid_array() {
        let json = format_json(&[ok_finding(), warn_finding()]);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["check"], "hooks");
        assert_eq!(parsed[0]["status"], "ok");
        assert_eq!(parsed[0]["repair_available"], false);
        assert_eq!(parsed[1]["status"], "warn");
        assert_eq!(parsed[1]["repair_available"], true);
    }

    #[test]
    fn has_problems_false_when_all_ok() {
        assert!(!has_problems(&[ok_finding(), ok_finding()]));
    }

    #[test]
    fn has_problems_true_when_any_warn() {
        assert!(has_problems(&[ok_finding(), warn_finding()]));
    }

    #[test]
    fn has_problems_empty_slice_is_false() {
        assert!(!has_problems(&[]));
    }
}
