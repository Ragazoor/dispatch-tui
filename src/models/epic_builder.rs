//! A fluent builder for `Epic` test fixtures — the twin of
//! [`super::TaskBuilder`], gated the same way.
//!
//! A fresh builder is a manual backlog epic titled `Epic {id}`, with no plan,
//! parent or feed, and both timestamps at the moment of construction.

use chrono::Utc;

use super::{Epic, EpicId, EpicOrigin, FeedRole, TaskStatus};

#[derive(Debug, Clone)]
pub struct EpicBuilder {
    epic: Epic,
}

impl EpicBuilder {
    pub fn new(id: i64) -> Self {
        let now = Utc::now();
        Self {
            epic: Epic {
                id: EpicId(id),
                title: format!("Epic {id}"),
                description: String::new(),
                status: TaskStatus::Backlog,
                plan_path: None,
                sort_order: None,
                completed_at: None,
                auto_dispatch: false,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                group_by_repo: false,
                feed_append_only: false,
                feed_role: FeedRole::None,
                origin: EpicOrigin::Manual,
                created_at: now,
                updated_at: now,
            },
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.epic.title = title.into();
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.epic.description = description.into();
        self
    }

    pub fn status(mut self, status: TaskStatus) -> Self {
        self.epic.status = status;
        self
    }

    pub fn plan(mut self, plan_path: Option<&str>) -> Self {
        self.epic.plan_path = plan_path.map(String::from);
        self
    }

    pub fn sort_order(mut self, sort_order: Option<i64>) -> Self {
        self.epic.sort_order = sort_order;
        self
    }

    pub fn auto_dispatch(mut self, auto_dispatch: bool) -> Self {
        self.epic.auto_dispatch = auto_dispatch;
        self
    }

    pub fn parent(mut self, parent: Option<i64>) -> Self {
        self.epic.parent_epic_id = parent.map(EpicId);
        self
    }

    pub fn feed_command(mut self, feed_command: Option<&str>) -> Self {
        self.epic.feed_command = feed_command.map(String::from);
        self
    }

    pub fn feed_interval_secs(mut self, feed_interval_secs: Option<i64>) -> Self {
        self.epic.feed_interval_secs = feed_interval_secs;
        self
    }

    pub fn group_by_repo(mut self, group_by_repo: bool) -> Self {
        self.epic.group_by_repo = group_by_repo;
        self
    }

    pub fn build(self) -> Epic {
        self.epic
    }
}

impl From<EpicBuilder> for Epic {
    fn from(builder: EpicBuilder) -> Self {
        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_a_titled_manual_backlog_epic() {
        let epic = EpicBuilder::new(4).build();
        assert_eq!(epic.id, EpicId(4));
        assert_eq!(epic.title, "Epic 4");
        assert_eq!(epic.description, "");
        assert_eq!(epic.status, TaskStatus::Backlog);
        assert_eq!(epic.plan_path, None);
        assert_eq!(epic.sort_order, None);
        assert_eq!(epic.completed_at, None);
        assert!(!epic.auto_dispatch);
        assert_eq!(epic.parent_epic_id, None);
        assert_eq!(epic.feed_command, None);
        assert_eq!(epic.feed_interval_secs, None);
        assert!(!epic.group_by_repo);
        assert!(!epic.feed_append_only);
        assert_eq!(epic.feed_role, FeedRole::None);
        assert_eq!(epic.origin, EpicOrigin::Manual);
        assert_eq!(epic.created_at, epic.updated_at);
    }

    #[test]
    fn setters_override_only_their_own_field() {
        let epic = EpicBuilder::new(1)
            .title("T")
            .description("D")
            .status(TaskStatus::Running)
            .plan(Some("p.md"))
            .sort_order(Some(9))
            .auto_dispatch(true)
            .parent(Some(2))
            .feed_command(Some("echo hi"))
            .feed_interval_secs(Some(30))
            .group_by_repo(true)
            .build();
        assert_eq!(epic.title, "T");
        assert_eq!(epic.description, "D");
        assert_eq!(epic.status, TaskStatus::Running);
        assert_eq!(epic.plan_path.as_deref(), Some("p.md"));
        assert_eq!(epic.sort_order, Some(9));
        assert!(epic.auto_dispatch);
        assert_eq!(epic.parent_epic_id, Some(EpicId(2)));
        assert_eq!(epic.feed_command.as_deref(), Some("echo hi"));
        assert_eq!(epic.feed_interval_secs, Some(30));
        assert!(epic.group_by_repo);
        assert!(!epic.feed_append_only);
    }
}
