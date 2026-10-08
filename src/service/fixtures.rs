//! Test fixtures for the service layer's create params.
//!
//! Gated like `crate::models::TaskBuilder`: `cfg(any(test, feature =
//! "test-support"))`. A test spells out only the fields it cares about and
//! takes the rest with struct-update syntax:
//! `CreateEpicParams { parent_epic_id: Some(p), ..CreateEpicParams::fixture("Child") }`.

use super::{CreateEpicParams, CreateTaskParams};

impl CreateEpicParams {
    /// A top-level epic with no description, sort order or feed.
    pub fn fixture(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: String::new(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
        }
    }
}

impl CreateTaskParams {
    /// A task on the repo's default branch with no description, plan, epic,
    /// tag or flags.
    pub fn fixture(title: impl Into<String>, repo_path: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: String::new(),
            repo_path: repo_path.into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
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
    fn create_epic_params_fixture_is_a_plain_top_level_epic() {
        let p = CreateEpicParams::fixture("E");
        assert_eq!(p.title, "E");
        assert_eq!(p.description, "");
        assert_eq!(p.sort_order, None);
        assert_eq!(p.parent_epic_id, None);
        assert_eq!(p.feed_command, None);
        assert_eq!(p.feed_interval_secs, None);
    }

    #[test]
    fn create_task_params_fixture_is_a_plain_task_on_the_default_branch() {
        let p = CreateTaskParams::fixture("T", "/repo");
        assert_eq!(p.title, "T");
        assert_eq!(p.repo_path, "/repo");
        assert_eq!(p.description, "");
        assert_eq!(p.plan_path, None);
        assert_eq!(p.epic_id, None);
        assert_eq!(p.sort_order, None);
        assert_eq!(p.tag, None);
        assert_eq!(p.base_branch, None);
        assert_eq!(p.wrap_up_mode, None);
        assert!(!p.auto_run_plan);
        assert!(!p.phoenix);
    }
}
