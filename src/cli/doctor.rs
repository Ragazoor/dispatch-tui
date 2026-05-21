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

/// Check worktrees: compare task.worktree DB values against disk, and scan
/// .worktrees/ directories for paths that don't match any DB row.
pub fn check_worktrees(tasks: &[crate::models::Task], repo_paths: &[String]) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Collect set of all worktree paths known to the DB.
    let db_worktrees: std::collections::HashSet<String> = tasks
        .iter()
        .filter_map(|t| t.worktree.clone())
        .collect();

    // DB orphans: task.worktree set but path doesn't exist on disk.
    for task in tasks {
        let Some(ref wt) = task.worktree else { continue };
        let path = crate::models::expand_tilde(wt);
        if !std::path::Path::new(&path).exists() {
            findings.push(Finding {
                check: "worktrees",
                status: FindingStatus::Error,
                target: wt.clone(),
                message: format!("task #{} claims worktree but path does not exist", task.id.0),
                repair_available: true,
            });
        }
    }

    // Disk orphans: directories under .worktrees/ with no matching DB row.
    for repo in repo_paths {
        let expanded = crate::models::expand_tilde(repo);
        let worktrees_dir = std::path::Path::new(&expanded).join(".worktrees");
        let Ok(entries) = std::fs::read_dir(&worktrees_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let path_str = p.to_string_lossy().to_string();
            if !db_worktrees.contains(&path_str) {
                findings.push(Finding {
                    check: "worktrees",
                    status: FindingStatus::Warn,
                    target: path_str,
                    message: "directory exists on disk but no task row has this worktree path"
                        .to_string(),
                    repair_available: true,
                });
            }
        }
    }

    findings
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

    mod worktrees_tests {
        use super::*;
        use tempfile::TempDir;

        fn task_with_worktree(id: i64, worktree: &str) -> crate::models::Task {
            crate::models::Task {
                id: crate::models::TaskId(id),
                title: format!("task {id}"),
                description: String::new(),
                repo_path: "/repo".to_string(),
                status: crate::models::TaskStatus::Running,
                worktree: Some(worktree.to_string()),
                tmux_window: Some(format!("task-{id}")),
                plan_path: None,
                epic_id: None,
                sub_status: crate::models::SubStatus::Active,
                pr_url: None,
                tag: None,
                sort_order: None,
                base_branch: "main".to_string(),
                external_id: None,
                project_id: crate::models::ProjectId(1),
                labels: vec![],
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                last_pre_tool_use_at: None,
                last_notification_at: None,
                wrap_up_mode: None,
            }
        }

        #[test]
        fn db_orphan_when_path_missing() {
            let findings = check_worktrees(
                &[task_with_worktree(1, "/nonexistent/task-1")],
                &["/repo".to_string()],
            );
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].status, FindingStatus::Error);
            assert_eq!(findings[0].check, "worktrees");
            assert!(findings[0].repair_available);
            assert!(findings[0].target.contains("task-1"));
        }

        #[test]
        fn ok_when_worktree_path_exists() {
            let dir = TempDir::new().unwrap();
            let path = dir.path().to_str().unwrap().to_string();
            let task = task_with_worktree(2, &path);
            let findings = check_worktrees(&[task], &["/repo".to_string()]);
            let errors: Vec<_> = findings.iter().filter(|f| f.status == FindingStatus::Error).collect();
            assert!(errors.is_empty(), "expected no errors for existing path, got: {errors:?}");
        }

        #[test]
        fn disk_orphan_when_dir_not_in_db() {
            let repo_dir = TempDir::new().unwrap();
            let worktrees_dir = repo_dir.path().join(".worktrees");
            std::fs::create_dir_all(&worktrees_dir).unwrap();
            let orphan = worktrees_dir.join("task-99");
            std::fs::create_dir_all(&orphan).unwrap();

            let findings = check_worktrees(&[], &[repo_dir.path().to_str().unwrap().to_string()]);
            assert_eq!(findings.len(), 1, "expected one finding for disk orphan");
            assert_eq!(findings[0].status, FindingStatus::Warn);
            assert!(findings[0].repair_available);
            assert!(findings[0].target.contains("task-99"));
        }

        #[test]
        fn no_findings_for_task_without_worktree() {
            let mut task = task_with_worktree(3, "/some/path");
            task.worktree = None;
            let findings = check_worktrees(&[task], &[]);
            assert!(findings.is_empty());
        }
    }
}
