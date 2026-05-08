#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::Arc;

use super::{ClaimTaskParams, CreateTaskParams, ListTasksFilter, TaskService, UpdateTaskParams};
use crate::db::{self, Database, ProjectCrud, TaskCrud};
use crate::models::{EpicId, ProjectId, SubStatus, TaskId, TaskStatus, TaskTag, UsageReport};
use crate::service::epics::{CreateEpicParams, EpicService, UpdateEpicParams};
use crate::service::{FieldUpdate, ServiceError};

fn test_db() -> Arc<dyn db::TaskStore> {
    Arc::new(Database::open_in_memory().unwrap())
}

fn task_svc(db: &Arc<dyn db::TaskStore>) -> TaskService {
    let d: Arc<dyn db::TaskAndEpicStore> = db.clone();
    TaskService::new(d)
}

fn epic_svc(db: &Arc<dyn db::TaskStore>) -> EpicService {
    let d: Arc<dyn db::EpicCrud> = db.clone();
    EpicService::new(d)
}

// -- TaskService ----------------------------------------------------------

#[test]
fn create_and_get_task() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "Test".into(),
            description: "desc".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let task = svc.get_task(id).unwrap();
    assert_eq!(task.title, "Test");
    assert_eq!(task.status, TaskStatus::Backlog);
}

#[test]
fn create_task_with_tag() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "Bug fix".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: Some(5),
            tag: Some(TaskTag::Bug),
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let task = svc.get_task(id).unwrap();
    assert_eq!(task.tag, Some(TaskTag::Bug));
    assert_eq!(task.sort_order, Some(5));
}

#[test]
fn create_task_with_sort_order() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "Sorted".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: Some(42),
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let task = svc.get_task(id).unwrap();
    assert_eq!(task.sort_order, Some(42));
}

#[test]
fn update_task_status() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .unwrap();

    let task = svc.get_task(id).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

// Note: Done/Archived restriction moved to MCP handler layer.
// The service now allows any status transition (TUI needs it).

#[test]
fn update_task_no_fields_returns_error() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let err = svc.update_task(UpdateTaskParams::for_task(id)).unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[test]
fn update_task_params_builder_compiles() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .unwrap();

    let task = svc.get_task(id).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

#[test]
fn update_task_invalid_substatus_for_status() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // active is not valid for backlog
    let err = svc
        .update_task(UpdateTaskParams::for_task(id).sub_status(SubStatus::Active))
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[test]
fn claim_task_success() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let task = svc
        .claim_task(ClaimTaskParams {
            task_id: id,
            worktree: "/repo/.worktrees/feature".into(),
            tmux_window: "win1".into(),
        })
        .unwrap();
    assert_eq!(task.title, "T");

    // Verify it was actually updated
    let task = svc.get_task(id).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/feature"));
}

#[test]
fn claim_task_wrong_repo() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo-a".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let err = svc
        .claim_task(ClaimTaskParams {
            task_id: id,
            worktree: "/repo-b/.worktrees/feature".into(),
            tmux_window: "win1".into(),
        })
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[test]
fn claim_task_not_backlog() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Move to running first
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .unwrap();

    let err = svc
        .claim_task(ClaimTaskParams {
            task_id: id,
            worktree: "/repo/.worktrees/feature".into(),
            tmux_window: "win1".into(),
        })
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[test]
fn list_tasks_with_filter() {
    let db = test_db();
    let svc = task_svc(&db);

    svc.create_task(CreateTaskParams {
        title: "T1".into(),
        description: "".into(),
        repo_path: "/repo".into(),
        plan_path: None,
        epic_id: None,
        sort_order: None,
        tag: None,
        base_branch: None,
        project_id: ProjectId(1),
    })
    .unwrap();

    let tasks = svc
        .list_tasks(ListTasksFilter {
            statuses: Some(vec![TaskStatus::Backlog]),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let tasks = svc
        .list_tasks(ListTasksFilter {
            statuses: Some(vec![TaskStatus::Running]),
            ..Default::default()
        })
        .unwrap();
    assert!(tasks.is_empty());
}

#[test]
fn get_task_not_found() {
    let db = test_db();
    let svc = task_svc(&db);
    let err = svc.get_task(TaskId(999)).unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[test]
fn report_usage_for_nonexistent_task() {
    let db = test_db();
    let svc = task_svc(&db);
    let err = svc
        .report_usage(
            TaskId(999),
            &UsageReport {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            },
        )
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[test]
fn update_task_with_epic_linkage() {
    let db = test_db();
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams {
            title: "Epic".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let id = task_svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    task_svc
        .update_task(UpdateTaskParams::for_task(id).epic_id(epic.id))
        .unwrap();

    let task = task_svc.get_task(id).unwrap();
    assert_eq!(task.epic_id, Some(epic.id));
}

#[test]
fn update_task_status_recalculates_parent_epic() {
    // Status-change branch of recalculate_epic_for_task: an epic that
    // contains a single task should follow the task's status.
    let db = test_db();
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let id = task_svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    task_svc
        .update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .unwrap();

    let refreshed = epic_svc.get_epic(epic.id).unwrap();
    assert_eq!(refreshed.status, TaskStatus::Running);
}

#[test]
fn update_task_relink_recalculates_old_and_new_epic() {
    // Linkage-change branch of recalculate_epic_for_task: moving a Running
    // task between two epics should leave the old epic empty (Backlog) and
    // the new epic Running.
    let db = test_db();
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic_a = epic_svc
        .create_epic(CreateEpicParams {
            title: "A".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let epic_b = epic_svc
        .create_epic(CreateEpicParams {
            title: "B".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let id = task_svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic_a.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    task_svc
        .update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .unwrap();

    // Sanity: epic A is now Running.
    assert_eq!(
        epic_svc.get_epic(epic_a.id).unwrap().status,
        TaskStatus::Running
    );

    task_svc
        .update_task(UpdateTaskParams::for_task(id).epic_id(epic_b.id))
        .unwrap();

    assert_eq!(
        epic_svc.get_epic(epic_a.id).unwrap().status,
        TaskStatus::Backlog
    );
    assert_eq!(
        epic_svc.get_epic(epic_b.id).unwrap().status,
        TaskStatus::Running
    );
}

// -- EpicService ----------------------------------------------------------

#[test]
fn create_and_get_epic() {
    let db = test_db();
    let svc = epic_svc(&db);

    let epic = svc
        .create_epic(CreateEpicParams {
            title: "Epic 1".into(),
            description: "desc".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let fetched = svc.get_epic(epic.id).unwrap();
    assert_eq!(fetched.title, "Epic 1");
}

#[test]
fn get_epic_not_found() {
    let db = test_db();
    let svc = epic_svc(&db);
    let err = svc.get_epic(EpicId(999)).unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[test]
fn update_epic_status() {
    let db = test_db();
    let svc = epic_svc(&db);

    let epic = svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.update_epic(UpdateEpicParams {
        epic_id: epic.id,
        title: None,
        description: None,
        status: Some(TaskStatus::Running),
        plan_path: None,
        sort_order: None,
        repo_path: None,
        auto_dispatch: None,
        feed_command: None,
        feed_interval_secs: None,
        project_id: None,
    })
    .unwrap();

    let updated = svc.get_epic(epic.id).unwrap();
    assert_eq!(updated.status, TaskStatus::Running);
}

#[test]
fn update_epic_no_fields_returns_error() {
    let db = test_db();
    let svc = epic_svc(&db);

    let epic = svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let err = svc
        .update_epic(UpdateEpicParams {
            epic_id: epic.id,
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: None,
        })
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[test]
fn update_epic_auto_dispatch_persists() {
    let db = test_db();
    let svc = epic_svc(&db);

    let epic = svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    assert!(db.get_epic(epic.id).unwrap().unwrap().auto_dispatch);

    svc.update_epic(UpdateEpicParams {
        epic_id: epic.id,
        title: None,
        description: None,
        status: None,
        plan_path: None,
        sort_order: None,
        repo_path: None,
        auto_dispatch: Some(false),
        feed_command: None,
        feed_interval_secs: None,
        project_id: None,
    })
    .unwrap();

    assert!(!db.get_epic(epic.id).unwrap().unwrap().auto_dispatch);
}

#[test]
fn list_epics_with_progress() {
    let db = test_db();
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    task_svc
        .create_task(CreateTaskParams {
            title: "Sub1".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let list = epic_svc.list_epics_with_progress().unwrap();
    assert_eq!(list.len(), 1);
    let (_, done, total) = &list[0];
    assert_eq!(*done, 0);
    assert_eq!(*total, 1);
}

#[test]
fn list_epics_with_progress_multiple_epics() {
    let db = test_db();
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let e1 = epic_svc
        .create_epic(CreateEpicParams {
            title: "E1".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let e2 = epic_svc
        .create_epic(CreateEpicParams {
            title: "E2".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // 2 tasks in E1
    let t1 = task_svc
        .create_task(CreateTaskParams {
            title: "T1".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(e1.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    task_svc
        .create_task(CreateTaskParams {
            title: "T2".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(e1.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    // 1 task in E2
    task_svc
        .create_task(CreateTaskParams {
            title: "T3".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(e2.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Mark T1 as done
    task_svc
        .update_task(UpdateTaskParams::for_task(t1).status(TaskStatus::Done))
        .unwrap();

    let list = epic_svc.list_epics_with_progress().unwrap();
    assert_eq!(list.len(), 2);
    let e1_progress = list.iter().find(|(e, _, _)| e.id == e1.id).unwrap();
    assert_eq!(e1_progress.1, 1); // 1 done
    assert_eq!(e1_progress.2, 2); // 2 total
    let e2_progress = list.iter().find(|(e, _, _)| e.id == e2.id).unwrap();
    assert_eq!(e2_progress.1, 0);
    assert_eq!(e2_progress.2, 1);
}

#[test]
fn update_task_status_recalculates_epic() {
    let db = test_db();
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let task_id = task_svc
        .create_task(CreateTaskParams {
            title: "Sub".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    task_svc
        .update_task(UpdateTaskParams::for_task(task_id).status(TaskStatus::Done))
        .unwrap();

    let updated_epic = epic_svc.get_epic(epic.id).unwrap();
    assert_eq!(updated_epic.status, TaskStatus::Done);
}

#[test]
fn get_epic_with_subtasks() {
    let db = test_db();
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    task_svc
        .create_task(CreateTaskParams {
            title: "Sub".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let (e, subtasks) = epic_svc.get_epic_with_subtasks(epic.id).unwrap();
    assert_eq!(e.title, "E");
    assert_eq!(subtasks.len(), 1);
}

// -- next_backlog_task -----------------------------------------------------

#[test]
fn next_backlog_task_returns_first_by_sort_order() {
    let db = test_db();
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    task_svc
        .create_task(CreateTaskParams {
            title: "Second".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: Some(20),
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    task_svc
        .create_task(CreateTaskParams {
            title: "First".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: Some(10),
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let next = task_svc.next_backlog_task(epic.id).unwrap();
    assert_eq!(next.unwrap().title, "First");
}

#[test]
fn next_backlog_task_skips_non_backlog() {
    let db = test_db();
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let id = task_svc
        .create_task(CreateTaskParams {
            title: "Running".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: Some(1),
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Move to running
    task_svc
        .update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .unwrap();

    let next = task_svc.next_backlog_task(epic.id).unwrap();
    assert!(next.is_none());
}

#[test]
fn next_backlog_task_epic_not_found() {
    let db = test_db();
    let svc = task_svc(&db);
    let err = svc.next_backlog_task(EpicId(999)).unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

// -- create_task_returning ---------------------------------------------------

#[test]
fn create_task_returning_gives_full_task() {
    let db = test_db();
    let svc = task_svc(&db);

    let task = svc
        .create_task_returning(CreateTaskParams {
            title: "Full task".into(),
            description: "desc".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: Some(TaskTag::Feature),
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    assert_eq!(task.title, "Full task");
    assert_eq!(task.description, "desc");
    assert_eq!(task.tag, Some(TaskTag::Feature));
    assert_eq!(task.status, TaskStatus::Backlog);
}

#[test]
fn create_task_returning_with_epic() {
    let db = test_db();
    let tsvc = task_svc(&db);
    let esvc = epic_svc(&db);

    let epic = esvc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let task = tsvc
        .create_task_returning(CreateTaskParams {
            title: "Sub".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    assert_eq!(task.epic_id, Some(epic.id));
}

#[test]
fn create_task_returning_sets_all_optional_fields_atomically() {
    let db = test_db();
    let tsvc = task_svc(&db);
    let esvc = epic_svc(&db);

    let epic = esvc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let task = tsvc
        .create_task_returning(CreateTaskParams {
            title: "Atomic".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: Some(3),
            tag: Some(TaskTag::Feature),
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    assert_eq!(task.epic_id, Some(epic.id));
    assert_eq!(task.sort_order, Some(3));
    assert_eq!(task.tag, Some(TaskTag::Feature));
}

// -- delete_task -------------------------------------------------------------

#[test]
fn delete_task_removes_it() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.delete_task(id).unwrap();

    let err = svc.get_task(id).unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[test]
fn delete_task_not_found() {
    let db = test_db();
    let svc = task_svc(&db);
    let err = svc.delete_task(TaskId(999)).unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

// -- update_task with worktree/tmux_window -----------------------------------

#[test]
fn update_task_sets_worktree_and_tmux_window() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into()))
            .tmux_window(FieldUpdate::Set("task-1".into())),
    )
    .unwrap();

    let task = svc.get_task(id).unwrap();
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/feat"));
    assert_eq!(task.tmux_window.as_deref(), Some("task-1"));
}

#[test]
fn update_task_clears_worktree() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Set worktree
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into()))
            .tmux_window(FieldUpdate::Set("task-1".into())),
    )
    .unwrap();

    // Clear worktree via FieldUpdate::Clear
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Done)
            .worktree(FieldUpdate::Clear)
            .tmux_window(FieldUpdate::Clear),
    )
    .unwrap();

    let task = svc.get_task(id).unwrap();
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
}

// -- update_task allows done/archived (MCP restriction moved to handler) -----

#[test]
fn update_task_allows_done_status() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .unwrap();

    let task = svc.get_task(id).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
}

// -- delete_epic -------------------------------------------------------------

#[test]
fn delete_epic_removes_it() {
    let db = test_db();
    let svc = epic_svc(&db);

    let epic = svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.delete_epic(epic.id).unwrap();

    let err = svc.get_epic(epic.id).unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[test]
fn delete_epic_not_found() {
    let db = test_db();
    let svc = epic_svc(&db);
    let err = svc.delete_epic(EpicId(999)).unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

// --- FieldUpdate ---

#[test]
fn field_update_set_has_value() {
    let fu: FieldUpdate = FieldUpdate::Set("hello".to_string());
    assert!(matches!(fu, FieldUpdate::Set(ref s) if s == "hello"));
}

#[test]
fn field_update_clear_is_clear() {
    let fu: FieldUpdate = FieldUpdate::Clear;
    assert!(matches!(fu, FieldUpdate::Clear));
}

#[test]
fn update_task_worktree_set_persists() {
    let db = test_db();
    let svc = task_svc(&db);
    let id = svc
        .create_task(CreateTaskParams {
            title: "t".into(),
            description: "d".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/wt".to_string()))
            .tmux_window(FieldUpdate::Set("win".to_string())),
    )
    .unwrap();
    let task = db.get_task(TaskId(id.0)).unwrap().unwrap();
    assert_eq!(task.worktree.as_deref(), Some("/wt"));
    assert_eq!(task.tmux_window.as_deref(), Some("win"));
}

#[test]
fn update_task_worktree_clear_sets_null() {
    let db = test_db();
    let svc = task_svc(&db);
    let id = svc
        .create_task(CreateTaskParams {
            title: "t".into(),
            description: "d".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    // First set a value
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/wt".to_string()))
            .tmux_window(FieldUpdate::Set("win".to_string())),
    )
    .unwrap();
    // Then clear it
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .worktree(FieldUpdate::Clear)
            .tmux_window(FieldUpdate::Clear),
    )
    .unwrap();
    let task = db.get_task(TaskId(id.0)).unwrap().unwrap();
    assert_eq!(task.worktree, None);
    assert_eq!(task.tmux_window, None);
}

#[test]
fn update_task_pr_url_set_and_clear() {
    let db = test_db();
    let svc = task_svc(&db);
    let id = svc
        .create_task(CreateTaskParams {
            title: "t".into(),
            description: "d".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    // Set PR URL
    svc.update_task(UpdateTaskParams::for_task(id).pr_url(FieldUpdate::Set(
        "https://github.com/org/repo/pull/1".to_string(),
    )))
    .unwrap();
    let task = db.get_task(TaskId(id.0)).unwrap().unwrap();
    assert_eq!(
        task.pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/1")
    );
    // Clear PR URL
    svc.update_task(UpdateTaskParams::for_task(id).pr_url(FieldUpdate::Clear))
        .unwrap();
    let task = db.get_task(TaskId(id.0)).unwrap().unwrap();
    assert_eq!(task.pr_url, None);
}

#[test]
fn list_tasks_filters_by_epic_id() {
    let db = test_db();
    let svc = task_svc(&db);
    let esvc = epic_svc(&db);

    let epic = esvc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let id1 = svc
        .create_task(CreateTaskParams {
            title: "In epic".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let _id2 = svc
        .create_task(CreateTaskParams {
            title: "No epic".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let tasks = svc
        .list_tasks(ListTasksFilter {
            epic_id: Some(epic.id),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, id1);
}

#[test]
fn list_tasks_excludes_archived_by_default() {
    let db = test_db();
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Archived))
        .unwrap();

    let tasks = svc
        .list_tasks(ListTasksFilter {
            ..Default::default()
        })
        .unwrap();
    assert!(tasks.is_empty());
}

#[test]
fn list_tasks_filters_by_project_id() {
    let db = test_db();
    let svc = task_svc(&db);

    svc.create_task(CreateTaskParams {
        title: "P1 task".into(),
        description: "".into(),
        repo_path: "/repo".into(),
        plan_path: None,
        epic_id: None,
        sort_order: None,
        tag: None,
        base_branch: None,
        project_id: ProjectId(1),
    })
    .unwrap();

    svc.create_task(CreateTaskParams {
        title: "P2 task".into(),
        description: "".into(),
        repo_path: "/repo".into(),
        plan_path: None,
        epic_id: None,
        sort_order: None,
        tag: None,
        base_branch: None,
        project_id: ProjectId(2),
    })
    .unwrap();

    let tasks = svc
        .list_tasks(ListTasksFilter {
            project_id: Some(ProjectId(2)),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "P2 task");
}

#[test]
fn list_tasks_filters_by_repo_paths() {
    let db = test_db();
    let svc = task_svc(&db);

    svc.create_task(CreateTaskParams {
        title: "Repo A".into(),
        description: "".into(),
        repo_path: "/repo/a".into(),
        plan_path: None,
        epic_id: None,
        sort_order: None,
        tag: None,
        base_branch: None,
        project_id: ProjectId(1),
    })
    .unwrap();

    svc.create_task(CreateTaskParams {
        title: "Repo B".into(),
        description: "".into(),
        repo_path: "/repo/b".into(),
        plan_path: None,
        epic_id: None,
        sort_order: None,
        tag: None,
        base_branch: None,
        project_id: ProjectId(1),
    })
    .unwrap();

    let tasks = svc
        .list_tasks(ListTasksFilter {
            repo_paths: Some(vec!["/repo/a".to_string()]),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Repo A");
}

#[test]
fn list_tasks_excludes_caller_task() {
    let db = test_db();
    let svc = task_svc(&db);

    let id1 = svc
        .create_task(CreateTaskParams {
            title: "T1".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.create_task(CreateTaskParams {
        title: "T2".into(),
        description: "".into(),
        repo_path: "/repo".into(),
        plan_path: None,
        epic_id: None,
        sort_order: None,
        tag: None,
        base_branch: None,
        project_id: ProjectId(1),
    })
    .unwrap();

    let tasks = svc
        .list_tasks(ListTasksFilter {
            exclude_task_id: Some(id1),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "T2");
}

#[test]
fn validate_send_message_missing_worktree() {
    let db = test_db();
    let svc = task_svc(&db);

    let from_id = svc
        .create_task(CreateTaskParams {
            title: "Sender".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Target task has no worktree (still backlog)
    let to_id = svc
        .create_task(CreateTaskParams {
            title: "Receiver".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let err = svc.validate_send_message(from_id, to_id).unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
    assert!(err.to_string().contains("no worktree"));
}

#[test]
fn validate_send_message_missing_tmux_window() {
    let db = test_db();
    let svc = task_svc(&db);

    let from_id = svc
        .create_task(CreateTaskParams {
            title: "Sender".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let to_id = svc
        .create_task(CreateTaskParams {
            title: "Receiver".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Set worktree but not tmux_window
    svc.update_task(
        UpdateTaskParams::for_task(to_id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into())),
    )
    .unwrap();

    let err = svc.validate_send_message(from_id, to_id).unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
    assert!(err.to_string().contains("no tmux window"));
}

#[test]
fn validate_send_message_target_not_found() {
    let db = test_db();
    let svc = task_svc(&db);

    let from_id = svc
        .create_task(CreateTaskParams {
            title: "Sender".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let err = svc.validate_send_message(from_id, TaskId(999)).unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

// -------------------------------------------------------------------------
// project_id propagation tests
// -------------------------------------------------------------------------

#[test]
fn create_task_with_explicit_project_id() {
    let db = Arc::new(Database::open_in_memory().unwrap());
    let svc = TaskService::new(db.clone() as Arc<dyn db::TaskAndEpicStore>);
    let default_id = db.get_default_project().unwrap().id;
    let other = db.create_project("Other", 1).unwrap();

    let result = svc.create_task(CreateTaskParams {
        title: "T".to_string(),
        description: String::new(),
        repo_path: "/r".to_string(),
        plan_path: None,
        epic_id: None,
        sort_order: None,
        tag: None,
        base_branch: None,
        project_id: other.id,
    });
    assert!(result.is_ok());
    let task_id = result.unwrap();
    let task = db
        .get_task(crate::models::TaskId(task_id.0))
        .unwrap()
        .unwrap();
    assert_eq!(task.project_id, other.id);
    assert_ne!(task.project_id, default_id);
}

// -------------------------------------------------------------------------
// Epic-in-epic service tests
// -------------------------------------------------------------------------

#[test]
fn create_sub_epic_links_parent() {
    let db = test_db();
    let svc = epic_svc(&db);

    let parent = svc
        .create_epic(CreateEpicParams {
            title: "Parent".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let child = svc
        .create_epic(CreateEpicParams {
            title: "Child".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: Some(parent.id),
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    assert_eq!(child.parent_epic_id, Some(parent.id));

    let fetched = svc.get_epic(child.id).unwrap();
    assert_eq!(fetched.parent_epic_id, Some(parent.id));
}

#[test]
fn list_root_epics_service() {
    let db = test_db();
    let svc = epic_svc(&db);

    let parent = svc
        .create_epic(CreateEpicParams {
            title: "Root".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    svc.create_epic(CreateEpicParams {
        title: "Sub".into(),
        description: "".into(),
        repo_path: "/repo".into(),
        sort_order: None,
        parent_epic_id: Some(parent.id),
        feed_command: None,
        feed_interval_secs: None,
        project_id: ProjectId(1),
    })
    .unwrap();

    let roots = svc.list_root_epics().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, parent.id);
}

#[test]
fn list_sub_epics_service() {
    let db = test_db();
    let svc = epic_svc(&db);

    let parent = svc
        .create_epic(CreateEpicParams {
            title: "Parent".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let child = svc
        .create_epic(CreateEpicParams {
            title: "Child".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: Some(parent.id),
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    let subs = svc.list_sub_epics(parent.id).unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].id, child.id);
}

// -- project_id in update_task --------------------------------------------

#[test]
fn update_task_project_id_moves_task() {
    let db = test_db();
    let svc = task_svc(&db);
    let d: Arc<dyn db::ProjectCrud> = db.clone();
    let other = d.create_project("Dispatch", 1).unwrap();

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).project_id(other.id))
        .unwrap();

    let db2: Arc<dyn db::TaskCrud> = db.clone();
    let task = db2.get_task(id).unwrap().unwrap();
    assert_eq!(task.project_id, other.id);
}

// -- project_id in update_epic --------------------------------------------

#[test]
fn update_epic_project_id_moves_epic() {
    let db = test_db();
    let svc = epic_svc(&db);
    let d: Arc<dyn db::ProjectCrud> = db.clone();
    let other = d.create_project("Dispatch", 1).unwrap();

    let epic = svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.update_epic(UpdateEpicParams {
        epic_id: epic.id,
        title: None,
        description: None,
        status: None,
        plan_path: None,
        sort_order: None,
        repo_path: None,
        auto_dispatch: None,
        feed_command: None,
        feed_interval_secs: None,
        project_id: Some(other.id),
    })
    .unwrap();

    let d2: Arc<dyn db::EpicCrud> = db.clone();
    let epics = d2.list_epics().unwrap();
    let updated = epics.iter().find(|e| e.id == epic.id).unwrap();
    assert_eq!(updated.project_id, other.id);
}

mod property_tests {
    use super::*;
    use proptest::prelude::*;

    /// Mirror of the `FieldUpdate ↔ Option<Option<T>>` mapping documented in
    /// CLAUDE.md and applied by `validators::build_task_patch`:
    ///   `Some(Set(v))` → `Some(Some(v))`
    ///   `Some(Clear)`  → `Some(None)`
    ///   `None`         → `None`
    fn map_field_update(fu: Option<FieldUpdate>) -> Option<Option<String>> {
        match fu {
            Some(FieldUpdate::Set(v)) => Some(Some(v)),
            Some(FieldUpdate::Clear) => Some(None),
            None => None,
        }
    }

    fn field_update_strategy() -> impl Strategy<Value = Option<FieldUpdate>> {
        prop_oneof![
            Just(None),
            Just(Some(FieldUpdate::Clear)),
            "[a-zA-Z0-9/]{0,32}".prop_map(|s| Some(FieldUpdate::Set(s))),
        ]
    }

    proptest! {
        /// `FieldUpdate` round-trips through the canonical mapping cleanly.
        #[test]
        fn field_update_roundtrip(fu in field_update_strategy()) {
            let mapped = map_field_update(fu.clone());
            let back: Option<FieldUpdate> = match mapped {
                None              => None,
                Some(None)        => Some(FieldUpdate::Clear),
                Some(Some(v))     => Some(FieldUpdate::Set(v)),
            };
            prop_assert_eq!(back, fu);
        }

        /// `build_task_patch` applies the mapping to `pr_url`, `worktree`, and
        /// `tmux_window`. For all input combinations, the resulting `TaskPatch`
        /// must carry the canonical `Option<Option<&str>>` shape.
        #[test]
        fn build_task_patch_maps_field_updates(
            pr_url in field_update_strategy(),
            worktree in field_update_strategy(),
            tmux_window in field_update_strategy(),
        ) {
            let mut params = UpdateTaskParams::for_task(TaskId(1));
            if let Some(ref u) = pr_url      { params = params.pr_url(u.clone()); }
            if let Some(ref w) = worktree    { params = params.worktree(w.clone()); }
            if let Some(ref t) = tmux_window { params = params.tmux_window(t.clone()); }

            let patch = super::super::validators::build_task_patch(&params, None, None);

            let expect = |fu: &Option<FieldUpdate>| -> Option<Option<String>> {
                fu.as_ref().map(|x| match x {
                    FieldUpdate::Set(v) => Some(v.clone()),
                    FieldUpdate::Clear  => None,
                })
            };
            prop_assert_eq!(
                patch.pr_url.map(|o| o.map(|s| s.to_string())),
                expect(&pr_url)
            );
            prop_assert_eq!(
                patch.worktree.map(|o| o.map(|s| s.to_string())),
                expect(&worktree)
            );
            prop_assert_eq!(
                patch.tmux_window.map(|o| o.map(|s| s.to_string())),
                expect(&tmux_window)
            );
        }
    }
}
