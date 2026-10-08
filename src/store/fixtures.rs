//! Test fixtures for the store's request structs.
//!
//! Gated like `crate::models::TaskBuilder`: `cfg(any(test, feature =
//! "test-support"))`, so the `tests/` integration targets can use them and the
//! release binary does not carry them. A test spells out only the fields it
//! cares about and takes the rest with struct-update syntax:
//! `CreateTaskRequest { plan: Some("p.md"), ..CreateTaskRequest::fixture("T", "/repo") }`.

use super::CreateTaskRequest;
use crate::models::{TaskStatus, DEFAULT_BASE_BRANCH};

impl<'a> CreateTaskRequest<'a> {
    /// A backlog task on `main` with no plan, epic, tag or flags.
    pub fn fixture(title: &'a str, repo_path: &'a str) -> Self {
        Self {
            title,
            description: "",
            repo_path,
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: DEFAULT_BASE_BRANCH,
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_task_request_fixture_is_a_plain_backlog_task_on_main() {
        let req = CreateTaskRequest::fixture("T", "/repo");
        assert_eq!(req.title, "T");
        assert_eq!(req.repo_path, "/repo");
        assert_eq!(req.description, "");
        assert_eq!(req.plan, None);
        assert_eq!(req.status, TaskStatus::Backlog);
        assert_eq!(req.base_branch, "main");
        assert_eq!(req.epic_id, None);
        assert_eq!(req.sort_order, None);
        assert_eq!(req.tag, None);
        assert_eq!(req.wrap_up_mode, None);
        assert!(!req.auto_run_plan);
        assert!(!req.phoenix);
    }
}
