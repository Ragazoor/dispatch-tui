//! A fluent builder for `Task` test fixtures.
//!
//! One shared fixture instead of a `make_task` per test module. Gated like
//! `test_tmux_window`: `cfg(any(test, feature = "test-support"))`, so the
//! `tests/` integration targets can use it and the release binary does not
//! carry it.
//!
//! Everything not set explicitly comes from `Task::default()`; the one
//! addition is a title of `Task {id}`, so a fixture is distinguishable in a
//! failing assertion without every caller naming it.

use super::{EpicId, SubStatus, Task, TaskId, TaskStatus, TaskTag, TmuxWindow};

#[derive(Debug, Clone)]
pub struct TaskBuilder {
    task: Task,
}

impl TaskBuilder {
    pub fn new(id: i64) -> Self {
        Self {
            task: Task {
                id: TaskId(id),
                title: format!("Task {id}"),
                ..Task::default()
            },
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.task.title = title.into();
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.task.description = description.into();
        self
    }

    pub fn repo_path(mut self, repo_path: impl Into<String>) -> Self {
        self.task.repo_path = repo_path.into();
        self
    }

    pub fn base_branch(mut self, base_branch: impl Into<String>) -> Self {
        self.task.base_branch = base_branch.into();
        self
    }

    pub fn status(mut self, status: TaskStatus) -> Self {
        self.task.status = status;
        self
    }

    pub fn sub_status(mut self, sub_status: SubStatus) -> Self {
        self.task.sub_status = sub_status;
        self
    }

    pub fn epic(mut self, epic_id: Option<i64>) -> Self {
        self.task.epic_id = epic_id.map(EpicId);
        self
    }

    pub fn plan(mut self, plan_path: Option<&str>) -> Self {
        self.task.plan_path = plan_path.map(String::from);
        self
    }

    pub fn tag(mut self, tag: Option<TaskTag>) -> Self {
        self.task.tag = tag;
        self
    }

    pub fn worktree(mut self, worktree: Option<&str>) -> Self {
        self.task.worktree = worktree.map(String::from);
        self
    }

    /// Give the task the worktree path and tmux window a dispatched task has.
    pub fn provisioned(mut self) -> Self {
        let id = self.task.id;
        self.task.worktree = Some(format!("/repo/.worktrees/{}-task-{}", id.0, id.0));
        self.task.tmux_window = Some(TmuxWindow::for_task(id));
        self
    }

    pub fn build(self) -> Task {
        self.task
    }
}

impl From<TaskBuilder> for Task {
    fn from(builder: TaskBuilder) -> Self {
        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_a_titled_backlog_task() {
        let task = TaskBuilder::new(7).build();
        assert_eq!(task.id, TaskId(7));
        assert_eq!(task.title, "Task 7");
        assert_eq!(task.status, TaskStatus::Backlog);
        assert_eq!(task.worktree, None);
        assert_eq!(task.tmux_window, None);
        assert_eq!(task.epic_id, None);
    }

    #[test]
    fn setters_override_only_their_own_field() {
        let task = TaskBuilder::new(1)
            .title("T")
            .description("D")
            .repo_path("/r")
            .base_branch("develop")
            .status(TaskStatus::Running)
            .sub_status(SubStatus::Active)
            .epic(Some(3))
            .plan(Some("p.md"))
            .tag(Some(TaskTag::PrReview))
            .build();
        assert_eq!(task.title, "T");
        assert_eq!(task.description, "D");
        assert_eq!(task.repo_path, "/r");
        assert_eq!(task.base_branch, "develop");
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.sub_status, SubStatus::Active);
        assert_eq!(task.epic_id, Some(EpicId(3)));
        assert_eq!(task.plan_path.as_deref(), Some("p.md"));
        assert_eq!(task.tag, Some(TaskTag::PrReview));
    }

    #[test]
    fn provisioned_sets_worktree_and_window_together() {
        let task = TaskBuilder::new(5).provisioned().build();
        assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/5-task-5"));
        assert_eq!(task.tmux_window, Some(TmuxWindow::for_task(TaskId(5))));
    }
}
