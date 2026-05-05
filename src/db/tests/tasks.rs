use super::*;

#[test]
fn create_and_get() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "My Task",
            description: "A description",
            repo_path: "/repo/path",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let task = db.get_task(id).unwrap().expect("task should exist");
    assert_eq!(task.id, id);
    assert_eq!(task.title, "My Task");
    assert_eq!(task.description, "A description");
    assert_eq!(task.repo_path, "/repo/path");
    assert_eq!(task.status, TaskStatus::Backlog);
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
}

#[test]
fn list_all() {
    let db = in_memory_db();
    db.create_task(CreateTaskRequest {
        title: "Task A",
        description: "desc",
        repo_path: "/a",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        project_id: ProjectId(1),
    })
    .unwrap();
    db.create_task(CreateTaskRequest {
        title: "Task B",
        description: "desc",
        repo_path: "/b",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        project_id: ProjectId(1),
    })
    .unwrap();
    db.create_task(CreateTaskRequest {
        title: "Task C",
        description: "desc",
        repo_path: "/c",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        project_id: ProjectId(1),
    })
    .unwrap();
    let tasks = db.list_all().unwrap();
    assert_eq!(tasks.len(), 3);
    assert_eq!(tasks[0].title, "Task A");
    assert_eq!(tasks[1].title, "Task B");
    assert_eq!(tasks[2].title, "Task C");
}

#[test]
fn list_by_status() {
    let db = in_memory_db();
    let id1 = db
        .create_task(CreateTaskRequest {
            title: "Task A",
            description: "desc",
            repo_path: "/a",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let id2 = db
        .create_task(CreateTaskRequest {
            title: "Task B",
            description: "desc",
            repo_path: "/b",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.create_task(CreateTaskRequest {
        title: "Task C",
        description: "desc",
        repo_path: "/c",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        project_id: ProjectId(1),
    })
    .unwrap();

    db.patch_task(id1, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();
    db.patch_task(id2, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    let running = db.list_by_status(TaskStatus::Running).unwrap();
    assert_eq!(running.len(), 2);

    let backlog = db.list_by_status(TaskStatus::Backlog).unwrap();
    assert_eq!(backlog.len(), 1);
    assert_eq!(backlog[0].title, "Task C");
}

#[test]
fn get_nonexistent() {
    let db = in_memory_db();
    let result = db.get_task(TaskId(9999)).unwrap();
    assert!(result.is_none());
}

#[test]
fn create_task_with_plan() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "Planned Task",
            description: "desc",
            repo_path: "/repo",
            plan: Some("docs/plan.md"),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.plan_path.as_deref(), Some("docs/plan.md"));
}

#[test]
fn create_task_without_plan() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "Simple Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.plan_path.is_none());
}

#[test]
fn find_task_by_plan_returns_match() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "Planned",
            description: "desc",
            repo_path: "/repo",
            plan: Some("/plans/my-plan.md"),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let found = db.find_task_by_plan("/plans/my-plan.md").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, id);
}

#[test]
fn find_task_by_plan_returns_none_when_no_match() {
    let db = in_memory_db();
    db.create_task(CreateTaskRequest {
        title: "Other",
        description: "desc",
        repo_path: "/repo",
        plan: Some("/plans/other.md"),
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        project_id: ProjectId(1),
    })
    .unwrap();

    let found = db.find_task_by_plan("/plans/nonexistent.md").unwrap();
    assert!(found.is_none());
}

#[test]
fn find_task_by_plan_ignores_tasks_without_plan() {
    let db = in_memory_db();
    db.create_task(CreateTaskRequest {
        title: "No Plan",
        description: "desc",
        repo_path: "/repo",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        project_id: ProjectId(1),
    })
    .unwrap();

    let found = db.find_task_by_plan("/plans/any.md").unwrap();
    assert!(found.is_none());
}

#[test]
fn create_task_returning_returns_full_task() {
    let db = in_memory_db();
    let task =
        create_task_returning(&db, "Title", "Desc", "/repo", None, TaskStatus::Backlog).unwrap();
    assert_eq!(task.title, "Title");
    assert_eq!(task.description, "Desc");
    assert_eq!(task.repo_path, "/repo");
    assert_eq!(task.status, TaskStatus::Backlog);
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
    assert!(task.plan_path.is_none());
}

#[test]
fn create_task_returning_with_plan() {
    let db = in_memory_db();
    let task =
        create_task_returning(&db, "T", "D", "/r", Some("plan.md"), TaskStatus::Backlog).unwrap();
    assert_eq!(task.plan_path.as_deref(), Some("plan.md"));
    assert_eq!(task.status, TaskStatus::Backlog);
}

#[test]
fn patch_task_applies_all_fields() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "title",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let patch = TaskPatch::new()
        .status(TaskStatus::Running)
        .plan_path(Some("plan.md"))
        .title("new title");
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.plan_path.as_deref(), Some("plan.md"));
    assert_eq!(task.title, "new title");
    assert_eq!(task.description, "desc"); // unchanged
}

#[test]
fn patch_task_none_fields_unchanged() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "title",
            description: "desc",
            repo_path: "/repo",
            plan: Some("plan.md"),
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let patch = TaskPatch::new();
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.title, "title");
    assert_eq!(task.plan_path.as_deref(), Some("plan.md"));
    assert_eq!(task.status, TaskStatus::Running);
}

#[test]
fn patch_task_sets_tag() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "title",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(id, &TaskPatch::new().tag(Some(TaskTag::Bug)))
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.tag, Some(TaskTag::Bug));
}

#[test]
fn patch_task_clears_tag() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "title",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(id, &TaskPatch::new().tag(Some(TaskTag::Feature)))
        .unwrap();
    db.patch_task(id, &TaskPatch::new().tag(None)).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.tag.is_none());
}

#[test]
fn has_other_tasks_with_worktree_returns_false_when_no_others() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "Task A",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(
        id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .worktree(Some("/repo/.worktrees/1-task-a"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();

    assert!(!db
        .has_other_tasks_with_worktree("/repo/.worktrees/1-task-a", id)
        .unwrap());
}

#[test]
fn has_other_tasks_with_worktree_returns_true_when_shared() {
    let db = in_memory_db();
    let id1 = db
        .create_task(CreateTaskRequest {
            title: "Task A",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let id2 = db
        .create_task(CreateTaskRequest {
            title: "Task B",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(
        id1,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .worktree(Some("/repo/.worktrees/1-task-a"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();
    db.patch_task(
        id2,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .worktree(Some("/repo/.worktrees/1-task-a"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();

    assert!(db
        .has_other_tasks_with_worktree("/repo/.worktrees/1-task-a", id1)
        .unwrap());
    assert!(db
        .has_other_tasks_with_worktree("/repo/.worktrees/1-task-a", id2)
        .unwrap());
}

#[test]
fn has_other_tasks_with_worktree_ignores_done_tasks() {
    let db = in_memory_db();
    let id1 = db
        .create_task(CreateTaskRequest {
            title: "Task A",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let id2 = db
        .create_task(CreateTaskRequest {
            title: "Task B",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(
        id1,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .worktree(Some("/repo/.worktrees/1-task-a"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();
    db.patch_task(
        id2,
        &TaskPatch::new()
            .status(TaskStatus::Done)
            .worktree(Some("/repo/.worktrees/1-task-a"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();

    assert!(!db
        .has_other_tasks_with_worktree("/repo/.worktrees/1-task-a", id1)
        .unwrap());
}

#[test]
fn patch_task_clears_plan() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "title",
            description: "desc",
            repo_path: "/repo",
            plan: Some("plan.md"),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let patch = TaskPatch::new().plan_path(None);
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.plan_path.is_none());
}

#[test]
fn patch_task_sets_dispatch_fields() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "title",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let patch = TaskPatch::new()
        .worktree(Some("/repo/.worktrees/1-my-task"))
        .tmux_window(Some("session:1-my-task"));
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/1-my-task"));
    assert_eq!(task.tmux_window.as_deref(), Some("session:1-my-task"));
}

#[test]
fn patch_task_clears_dispatch_fields() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "title",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    // Set dispatch fields first
    let patch = TaskPatch::new()
        .worktree(Some("/repo/.worktrees/1-my-task"))
        .tmux_window(Some("session:1-my-task"));
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.worktree.is_some());
    assert!(task.tmux_window.is_some());

    // Clear them
    let patch = TaskPatch::new().worktree(None).tmux_window(None);
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
}

#[test]
fn patch_task_status_and_dispatch_together() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "title",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let patch = TaskPatch::new()
        .status(TaskStatus::Running)
        .worktree(Some("/repo/.worktrees/1-my-task"))
        .tmux_window(Some("session:1-my-task"));
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/1-my-task"));
    assert_eq!(task.tmux_window.as_deref(), Some("session:1-my-task"));
}

#[test]
fn task_patch_status_does_not_set_sub_status() {
    // status() no longer auto-sets sub_status; patch_task handles the default
    let patch = TaskPatch::new().status(TaskStatus::Review);
    assert_eq!(patch.status, Some(TaskStatus::Review));
    assert_eq!(patch.sub_status, None);
}

#[test]
fn task_patch_status_and_sub_status_independent() {
    // Order of builder calls doesn't matter — both fields are set independently
    let patch_a = TaskPatch::new()
        .status(TaskStatus::Running)
        .sub_status(SubStatus::NeedsInput);
    let patch_b = TaskPatch::new()
        .sub_status(SubStatus::NeedsInput)
        .status(TaskStatus::Running);
    assert_eq!(patch_a.status, Some(TaskStatus::Running));
    assert_eq!(patch_a.sub_status, Some(SubStatus::NeedsInput));
    assert_eq!(patch_b.status, Some(TaskStatus::Running));
    assert_eq!(patch_b.sub_status, Some(SubStatus::NeedsInput));
}

#[test]
fn patch_task_status_change_resets_sub_status_in_db() {
    // End-to-end: after a status-only patch, sub_status in DB reflects the new default
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(id, &TaskPatch::default().sub_status(SubStatus::Stale))
        .unwrap();

    db.patch_task(id, &TaskPatch::default().status(TaskStatus::Review))
        .unwrap();

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::AwaitingReview);
}

#[test]
fn update_status_if_matching() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let updated = db
        .update_status_if(id, TaskStatus::Review, TaskStatus::Running)
        .unwrap();
    assert!(updated, "should update when current status matches");

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);
}

#[test]
fn update_status_if_not_matching() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Done,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let updated = db
        .update_status_if(id, TaskStatus::Review, TaskStatus::Running)
        .unwrap();
    assert!(
        !updated,
        "should not update when current status doesn't match"
    );

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done, "status should be unchanged");
}

#[test]
fn update_status_if_nonexistent() {
    let db = in_memory_db();
    let updated = db
        .update_status_if(TaskId(9999), TaskStatus::Review, TaskStatus::Running)
        .unwrap();
    assert!(!updated, "should return false for nonexistent task");
}

#[test]
fn task_roundtrip_with_pr_fields() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "PR task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    db.patch_task(
        id,
        &TaskPatch::new().pr_url(Some("https://github.com/org/repo/pull/42")),
    )
    .unwrap();

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(
        task.pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/42")
    );
}

#[test]
fn task_pr_fields_default_to_none() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "No PR",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.pr_url.is_none());
}

#[test]
fn patch_task_sets_pr_url() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "t",
            description: "d",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    db.patch_task(
        id,
        &TaskPatch::new().pr_url(Some("https://example.com/pull/1")),
    )
    .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.pr_url.as_deref(), Some("https://example.com/pull/1"));
}

#[test]
fn patch_task_sets_sort_order() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(id, &TaskPatch::new().sort_order(Some(500)))
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sort_order, Some(500));
}

#[test]
fn patch_task_clears_sort_order() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(id, &TaskPatch::new().sort_order(Some(100)))
        .unwrap();
    db.patch_task(id, &TaskPatch::new().sort_order(None))
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sort_order, None);
}

#[test]
fn report_usage_first_insert() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "D",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.report_usage(
        id,
        &UsageReport {
            input_tokens: 10_000,
            output_tokens: 2_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        },
    )
    .unwrap();
    let all = db.get_all_usage().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].task_id, id);
    assert_eq!(all[0].input_tokens, 10_000);
    assert_eq!(all[0].output_tokens, 2_000);
    assert_eq!(all[0].cache_read_tokens, 0);
    assert_eq!(all[0].cache_write_tokens, 0);
}

#[test]
fn report_usage_accumulates() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "D",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.report_usage(
        id,
        &UsageReport {
            input_tokens: 1_000,
            output_tokens: 500,
            cache_read_tokens: 100,
            cache_write_tokens: 50,
        },
    )
    .unwrap();
    db.report_usage(
        id,
        &UsageReport {
            input_tokens: 500,
            output_tokens: 250,
            cache_read_tokens: 50,
            cache_write_tokens: 25,
        },
    )
    .unwrap();
    let all = db.get_all_usage().unwrap();
    assert_eq!(all.len(), 1);
    let u = &all[0];
    assert_eq!(u.input_tokens, 1_500);
    assert_eq!(u.output_tokens, 750);
    assert_eq!(u.cache_read_tokens, 150);
    assert_eq!(u.cache_write_tokens, 75);
}

#[test]
fn get_all_usage_empty() {
    let db = Database::open_in_memory().unwrap();
    assert!(db.get_all_usage().unwrap().is_empty());
}

#[test]
fn task_sub_status_persists() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(id, &TaskPatch::default().sub_status(SubStatus::Stale))
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::Stale);
}

#[test]
fn task_sub_status_defaults_to_none() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::None);
}

#[test]
fn create_task_sets_default_sub_status_for_running() {
    // create_task with status=Running must produce sub_status=active, not 'none'
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::Active);
}

#[test]
fn create_task_sets_default_sub_status_for_backlog() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::None);
}

#[test]
fn create_task_with_epic_sort_tag_single_insert() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: Some(7),
            tag: Some(TaskTag::Bug),
            project_id: ProjectId(1),
        })
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.epic_id, Some(epic.id));
    assert_eq!(task.sort_order, Some(7));
    assert_eq!(task.tag, Some(TaskTag::Bug));
}

#[test]
fn update_status_if_resets_sub_status_to_default() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(id, &TaskPatch::default().sub_status(SubStatus::Stale))
        .unwrap();

    let updated = db
        .update_status_if(id, TaskStatus::Review, TaskStatus::Running)
        .unwrap();
    assert!(updated, "should have updated");

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::AwaitingReview); // default for review
}

#[test]
fn update_status_if_leaves_sub_status_unchanged_when_condition_fails() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(id, &TaskPatch::default().sub_status(SubStatus::Active))
        .unwrap();

    let updated = db
        .update_status_if(id, TaskStatus::Review, TaskStatus::Backlog)
        .unwrap();
    assert!(!updated, "condition was wrong, should not have updated");

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::Active); // unchanged
}

#[test]
fn check_constraint_rejects_review_with_active_substatus() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn().unwrap();
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, sub_status) \
         VALUES ('T', 'D', '/r', 'backlog', 'none')",
        [],
    )
    .unwrap();
    let result = conn.execute(
        "UPDATE tasks SET status = 'review', sub_status = 'active' WHERE id = 1",
        [],
    );
    assert!(
        result.is_err(),
        "CHECK constraint must reject (review, active)"
    );
}

#[test]
fn check_constraint_accepts_review_with_awaiting_review() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn().unwrap();
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, sub_status) \
         VALUES ('T', 'D', '/r', 'backlog', 'none')",
        [],
    )
    .unwrap();
    let result = conn.execute(
        "UPDATE tasks SET status = 'review', sub_status = 'awaiting_review' WHERE id = 1",
        [],
    );
    assert!(result.is_ok(), "valid pair should be accepted");
}

// ---------------------------------------------------------------------------
// Query coverage: delete_task
// ---------------------------------------------------------------------------

#[test]
fn delete_task_removes_task() {
    let db = in_memory_db();
    let id = db
        .create_task(CreateTaskRequest {
            title: "Doomed",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    assert!(db.get_task(id).unwrap().is_some());

    db.delete_task(id).unwrap();
    assert!(db.get_task(id).unwrap().is_none());
}

#[test]
fn delete_task_nonexistent_errors() {
    let db = in_memory_db();
    let result = db.delete_task(TaskId(9999));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// upsert_feed_tasks
// ---------------------------------------------------------------------------

fn make_feed_item(external_id: &str, title: &str) -> crate::models::FeedItem {
    crate::models::FeedItem {
        external_id: external_id.to_string(),
        title: title.to_string(),
        description: "desc".to_string(),
        url: String::new(),
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Bug,
    }
}

/// Build a parallel vec of "main" base branches for tests that don't
/// exercise the per-task base_branch path.
fn main_branches(n: usize) -> Vec<String> {
    vec!["main".to_string(); n]
}

#[test]
fn upsert_feed_tasks_creates_tasks() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let items = vec![
        make_feed_item("ext-1", "Task One"),
        make_feed_item("ext-2", "Task Two"),
    ];
    let repo_paths = vec!["/repo".to_string(), "/repo".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks.len(), 2);
    let mut titles: Vec<&str> = tasks.iter().map(|t| t.title.as_str()).collect();
    titles.sort();
    assert_eq!(titles, vec!["Task One", "Task Two"]);
    assert!(tasks.iter().all(|t| t.status == TaskStatus::Backlog));
    assert!(tasks
        .iter()
        .all(|t| t.external_id.as_deref() == Some("ext-1")
            || t.external_id.as_deref() == Some("ext-2")));
}

#[test]
fn upsert_feed_tasks_idempotent() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let items = vec![make_feed_item("ext-1", "Task One")];
    let repo_paths = vec!["/repo".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .unwrap();
    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks.len(), 1, "second call should not create duplicate");
    assert_eq!(tasks[0].title, "Task One");
}

#[test]
fn upsert_feed_tasks_preserves_status() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let items = vec![make_feed_item("ext-1", "Original Title")];

    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .unwrap();

    // Simulate user moving task to Running
    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    db.patch_task(tasks[0].id, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    // Re-run upsert with updated title and different status
    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "Updated Title".to_string(),
        description: "new desc".to_string(),
        url: String::new(),
        status: TaskStatus::Done, // feed says done; user status should be preserved
        tag: crate::models::TaskTag::Bug,
    }];
    db.upsert_feed_tasks(epic.id, &updated, &["/repo".to_string()], &main_branches(1))
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Updated Title", "title should be updated");
    assert_eq!(
        tasks[0].description, "new desc",
        "description should be updated"
    );
    assert_eq!(
        tasks[0].status,
        TaskStatus::Running,
        "user-managed status must be preserved"
    );
}

#[test]
fn upsert_feed_tasks_adds_new_items() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();

    db.upsert_feed_tasks(
        epic.id,
        &[make_feed_item("ext-1", "First")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .unwrap();

    db.upsert_feed_tasks(
        epic.id,
        &[
            make_feed_item("ext-1", "First"),
            make_feed_item("ext-2", "Second"),
        ],
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
    )
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks.len(), 2, "new item should be created on second call");
}

#[test]
fn upsert_feed_tasks_removes_stale_items() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();

    // First fetch: two items
    db.upsert_feed_tasks(
        epic.id,
        &[
            make_feed_item("ext-1", "First"),
            make_feed_item("ext-2", "Second"),
        ],
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
    )
    .unwrap();
    assert_eq!(db.list_tasks_for_epic(epic.id).unwrap().len(), 2);

    // Second fetch: only ext-1 remains in the feed
    db.upsert_feed_tasks(
        epic.id,
        &[make_feed_item("ext-1", "First")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks.len(), 1, "stale feed task should be removed");
    assert_eq!(tasks[0].external_id.as_deref(), Some("ext-1"));
}

#[test]
fn upsert_feed_tasks_uses_resolved_repo_path() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/epic-repo", None, ProjectId(1))
        .unwrap();
    let items = vec![make_feed_item("ext-1", "Task One")];
    let repo_paths = vec!["/resolved/local/repo".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks[0].repo_path, "/resolved/local/repo");
}

#[test]
fn upsert_feed_tasks_stores_empty_sentinel_when_unresolved() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/epic-repo", None, ProjectId(1))
        .unwrap();
    let items = vec![make_feed_item("ext-1", "Task One")];
    let repo_paths = vec!["".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks[0].repo_path, "");
}

#[test]
fn upsert_feed_tasks_on_conflict_does_not_update_repo_path() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/epic-repo", None, ProjectId(1))
        .unwrap();
    let items = vec![make_feed_item("ext-1", "Original")];

    // First upsert: resolved path stored
    db.upsert_feed_tasks(
        epic.id,
        &items,
        &["/first/path".to_string()],
        &main_branches(1),
    )
    .unwrap();
    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks[0].repo_path, "/first/path");

    // Second upsert: different path provided — ON CONFLICT should NOT update repo_path
    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "Updated Title".to_string(),
        description: "new desc".to_string(),
        url: String::new(),
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Bug,
    }];
    db.upsert_feed_tasks(
        epic.id,
        &updated,
        &["/second/path".to_string()],
        &main_branches(1),
    )
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks[0].title, "Updated Title");
    assert_eq!(
        tasks[0].repo_path, "/first/path",
        "repo_path must not be updated on conflict"
    );
}

#[test]
fn upsert_feed_tasks_mixed_batch_resolved_and_unresolved() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/epic-repo", None, ProjectId(1))
        .unwrap();
    let items = vec![
        make_feed_item("ext-1", "Resolved Task"),
        make_feed_item("ext-2", "Unresolved Task"),
    ];
    let repo_paths = vec!["/matched/local/path".to_string(), "".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    let resolved = tasks
        .iter()
        .find(|t| t.external_id.as_deref() == Some("ext-1"))
        .unwrap();
    let unresolved = tasks
        .iter()
        .find(|t| t.external_id.as_deref() == Some("ext-2"))
        .unwrap();
    assert_eq!(resolved.repo_path, "/matched/local/path");
    assert_eq!(unresolved.repo_path, "");
}

#[test]
fn upsert_feed_tasks_stores_per_task_base_branch() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let items = vec![
        make_feed_item("ext-1", "Master Task"),
        make_feed_item("ext-2", "Develop Task"),
        make_feed_item("ext-3", "Main Task"),
    ];
    let repo_paths = vec![
        "/repo-a".to_string(),
        "/repo-b".to_string(),
        "/repo-c".to_string(),
    ];
    let base_branches = vec![
        "master".to_string(),
        "develop".to_string(),
        "main".to_string(),
    ];

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &base_branches)
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    let by_ext = |ext: &str| {
        tasks
            .iter()
            .find(|t| t.external_id.as_deref() == Some(ext))
            .unwrap()
    };
    assert_eq!(by_ext("ext-1").base_branch, "master");
    assert_eq!(by_ext("ext-2").base_branch, "develop");
    assert_eq!(by_ext("ext-3").base_branch, "main");
}

#[test]
fn upsert_feed_tasks_does_not_remove_manual_tasks() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();

    // Manually created task linked to the epic (no external_id)
    let manual_task_id = db
        .create_task(CreateTaskRequest {
            title: "Manual",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Feed fetch with one item
    db.upsert_feed_tasks(
        epic.id,
        &[make_feed_item("ext-1", "Feed Task")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .unwrap();

    // Feed fetch returns nothing — only manual task should survive
    db.upsert_feed_tasks(epic.id, &[], &[], &[]).unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "manual task should survive empty feed fetch"
    );
    assert_eq!(tasks[0].id, manual_task_id);
}

#[test]
fn upsert_feed_tasks_persists_tag() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let items = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "Tagged".to_string(),
        description: "".to_string(),
        url: String::new(),
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::PrReview,
    }];

    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].tag, Some(crate::models::TaskTag::PrReview));
}

#[test]
fn upsert_feed_tasks_updates_tag_on_conflict() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let initial = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "T".to_string(),
        description: "".to_string(),
        url: String::new(),
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::PrReview,
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .unwrap();

    // Re-emit the same item with a different tag — feed is the source of truth.
    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "T".to_string(),
        description: "".to_string(),
        url: String::new(),
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Fix,
    }];
    db.upsert_feed_tasks(epic.id, &updated, &["/repo".to_string()], &main_branches(1))
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].tag, Some(crate::models::TaskTag::Fix));
}

// ---------------------------------------------------------------------------
// patch_struct! macro correctness — has_changes() and setter coverage
// ---------------------------------------------------------------------------

#[test]
fn task_patch_default_has_no_changes() {
    assert!(!TaskPatch::default().has_changes());
}

#[test]
fn task_patch_each_setter_marks_has_changes() {
    assert!(TaskPatch::new().status(TaskStatus::Running).has_changes());
    assert!(TaskPatch::new().plan_path(Some("p")).has_changes());
    assert!(TaskPatch::new().plan_path(None).has_changes());
    assert!(TaskPatch::new().title("t").has_changes());
    assert!(TaskPatch::new().description("d").has_changes());
    assert!(TaskPatch::new().repo_path("/r").has_changes());
    assert!(TaskPatch::new().worktree(Some("w")).has_changes());
    assert!(TaskPatch::new().worktree(None).has_changes());
    assert!(TaskPatch::new().tmux_window(Some("tw")).has_changes());
    assert!(TaskPatch::new().tmux_window(None).has_changes());
    assert!(TaskPatch::new().sub_status(SubStatus::Active).has_changes());
    assert!(TaskPatch::new().pr_url(Some("u")).has_changes());
    assert!(TaskPatch::new().pr_url(None).has_changes());
    assert!(TaskPatch::new().tag(Some(TaskTag::Bug)).has_changes());
    assert!(TaskPatch::new().tag(None).has_changes());
    assert!(TaskPatch::new().sort_order(Some(1)).has_changes());
    assert!(TaskPatch::new().sort_order(None).has_changes());
    assert!(TaskPatch::new().base_branch("main").has_changes());
    assert!(TaskPatch::new().external_id(Some("x")).has_changes());
    assert!(TaskPatch::new().external_id(None).has_changes());
    assert!(TaskPatch::new().project_id(ProjectId(1)).has_changes());
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

mod property_tests {
    use super::*;
    use proptest::prelude::*;

    /// Build a `TaskPatch` with the subset of fields indicated by `bits`.
    /// Each bit (0-12) maps to one field in `has_changes()` order.
    fn taskpatch_from_bits(bits: u16) -> TaskPatch<'static> {
        let mut p = TaskPatch::new();
        if bits & (1 << 0) != 0 {
            p = p.status(crate::models::TaskStatus::Backlog);
        }
        if bits & (1 << 1) != 0 {
            p = p.plan_path(Some("plan.md"));
        }
        if bits & (1 << 2) != 0 {
            p = p.title("t");
        }
        if bits & (1 << 3) != 0 {
            p = p.description("d");
        }
        if bits & (1 << 4) != 0 {
            p = p.repo_path("/repo");
        }
        if bits & (1 << 5) != 0 {
            p = p.worktree(Some(".wt"));
        }
        if bits & (1 << 6) != 0 {
            p = p.tmux_window(Some("w"));
        }
        if bits & (1 << 7) != 0 {
            p = p.sub_status(crate::models::SubStatus::Active);
        }
        if bits & (1 << 8) != 0 {
            p = p.pr_url(Some("https://github.com/pr/1"));
        }
        if bits & (1 << 9) != 0 {
            p = p.tag(Some(crate::models::TaskTag::Bug));
        }
        if bits & (1 << 10) != 0 {
            p = p.sort_order(Some(1));
        }
        if bits & (1 << 11) != 0 {
            p = p.base_branch("main");
        }
        if bits & (1 << 12) != 0 {
            p = p.external_id(Some("ext-1"));
        }
        p
    }

    /// Build an `EpicPatch` with the subset of fields indicated by `bits`.
    /// Each bit (0-8) maps to one field in `has_changes()` order.
    fn epicpatch_from_bits(bits: u16) -> EpicPatch<'static> {
        let mut p = EpicPatch::new();
        if bits & (1 << 0) != 0 {
            p = p.title("epic title");
        }
        if bits & (1 << 1) != 0 {
            p = p.description("desc");
        }
        if bits & (1 << 2) != 0 {
            p = p.status(crate::models::TaskStatus::Running);
        }
        if bits & (1 << 3) != 0 {
            p = p.plan_path(Some("plan.md"));
        }
        if bits & (1 << 4) != 0 {
            p = p.sort_order(Some(1));
        }
        if bits & (1 << 5) != 0 {
            p = p.repo_path("/repo");
        }
        if bits & (1 << 6) != 0 {
            p = p.auto_dispatch(true);
        }
        if bits & (1 << 7) != 0 {
            p = p.feed_command(Some("cmd"));
        }
        if bits & (1 << 8) != 0 {
            p = p.feed_interval_secs(Some(60));
        }
        p
    }

    proptest! {
        #[test]
        fn taskpatch_has_changes_iff_any_field_set(bits in 0u16..8192) {
            let patch = taskpatch_from_bits(bits);
            prop_assert_eq!(patch.has_changes(), bits != 0);
        }

        #[test]
        fn epicpatch_has_changes_iff_any_field_set(bits in 0u16..512) {
            let patch = epicpatch_from_bits(bits);
            prop_assert_eq!(patch.has_changes(), bits != 0);
        }
    }
}
