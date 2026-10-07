use std::sync::Arc;

use super::{CreateTaskParams, ListTasksFilter, TaskService, UpdateTaskParams};
use crate::models::{
    EpicId, HookEventKind, NotificationKind, SubStatus, SubagentEvent, TaskId, TaskStatus, TaskTag,
};
use crate::service::epics::{CreateEpicParams, EpicService, UpdateEpicParams};
use crate::service::{FieldUpdate, ServiceError};
use crate::store::{self, Database, EpicRead, TaskRead};

async fn test_db() -> Arc<dyn store::TaskStore> {
    Arc::new(Database::open_in_memory().await.unwrap())
}

/// A SQLite-only handle, for a test whose subject is a behaviour the shared
/// store does not share (it refuses to delete a task that is not done, and
/// treats a patch on a missing id as a no-op).
async fn test_db_unattached() -> Arc<dyn store::TaskStore> {
    Arc::new(Database::open_in_memory().await.unwrap())
}

fn task_svc(db: &Arc<dyn store::TaskStore>) -> TaskService {
    task_svc_with_runner(db, crate::process::MockProcessRunner::unused())
}

/// A `TaskService` whose clock the caller drives. Hook-event ordering is
/// compared at sub-second resolution (`stop_pending_at`), so two wall-clock
/// reads in one test can tie; advance this instead.
fn task_svc_with_fixed_clock(
    db: &Arc<dyn store::TaskStore>,
) -> (TaskService, crate::service::FixedClock) {
    let clock = crate::service::FixedClock::new(chrono::Utc::now());
    (task_svc(db).with_clock(Arc::new(clock.clone())), clock)
}

fn epic_svc(db: &Arc<dyn store::TaskStore>) -> EpicService {
    let shared: Arc<dyn store::TaskAndEpicStore> = db.clone();
    let local: Arc<dyn store::LearningStore> = db.clone();
    EpicService::new(shared, local)
}

/// Construct a `TaskService` with a caller-supplied `ProcessRunner` (e.g. a
/// `MockProcessRunner`). Used by watch/finish notification tests to assert
/// on tmux/file-system side effects deterministically.
fn task_svc_with_runner(
    db: &Arc<dyn store::TaskStore>,
    runner: Arc<dyn crate::process::ProcessRunner>,
) -> TaskService {
    TaskService::new(db.clone(), runner)
}

fn make_task_params(repo_path: &str) -> CreateTaskParams {
    CreateTaskParams {
        title: "T".into(),
        description: "".into(),
        repo_path: repo_path.to_string(),
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

/// `make_task_params` with the base branch named, so creating the task makes
/// no `git symbolic-ref` probe of its own (`TaskService::create_task_returning`
/// resolves an omitted one from the repo). For a test whose subject is what
/// some LATER operation runs, and which therefore asserts on `recorded_calls`.
fn make_task_params_on_branch(repo_path: &str, base_branch: &str) -> CreateTaskParams {
    CreateTaskParams {
        base_branch: Some(base_branch.to_string()),
        ..make_task_params(repo_path)
    }
}

/// Helper: create a root epic with the given title.
async fn make_epic(svc: &EpicService, title: &str) -> crate::models::Epic {
    svc.create_epic(CreateEpicParams {
        title: title.into(),
        description: "".into(),
        sort_order: None,
        parent_epic_id: None,
        feed_command: None,
        feed_interval_secs: None,
    })
    .await
    .unwrap()
}

/// Helper: create a backlog task in the given (optional) epic.
async fn make_task(svc: &TaskService, epic_id: Option<EpicId>) -> TaskId {
    svc.create_task(CreateTaskParams {
        title: "T".into(),
        description: "".into(),
        repo_path: "/repo".to_string(),
        plan_path: None,
        epic_id,
        sort_order: None,
        tag: None,
        base_branch: None,
        wrap_up_mode: None,
        auto_run_plan: false,
        phoenix: false,
    })
    .await
    .unwrap()
}

mod crud;
mod dispatch;
mod property_tests;
mod validators;
mod watchers;
mod wrap_up;
