use crate::models::{Epic, EpicId, HookEventKind, SubStatus, Task, TaskId, TaskStatus};

use super::{
    ClaimTaskParams, CreateEpicParams, CreateTaskParams, EpicService, ListTasksFilter,
    ServiceError, TaskService, UpdateEpicParams, UpdateTaskParams, UpdateTaskResult,
};

/// Consumer-facing seam for task operations.
///
/// Mirrors the public async surface of [`TaskService`]. Callers should hold
/// `Arc<dyn TaskServiceApi>` so unit tests can inject a mock without spinning
/// up a real database. See `docs/conventions.md §"Service trait narrowing"`.
#[async_trait::async_trait]
pub trait TaskServiceApi: Send + Sync {
    async fn update_task(&self, params: UpdateTaskParams)
        -> Result<UpdateTaskResult, ServiceError>;

    /// Move a task to a different epic, or detach it (`new_epic = None`).
    /// Recalculates the status of both the previous and new epic.
    async fn move_task_to_epic(
        &self,
        task_id: TaskId,
        new_epic: Option<EpicId>,
    ) -> Result<(), ServiceError>;

    async fn cli_update_task(
        &self,
        task_id: TaskId,
        new_status: TaskStatus,
        only_if: Option<TaskStatus>,
        sub_status: Option<SubStatus>,
    ) -> Result<bool, ServiceError>;

    async fn create_task(&self, params: CreateTaskParams) -> Result<TaskId, ServiceError>;

    async fn create_task_returning(&self, params: CreateTaskParams) -> Result<Task, ServiceError>;

    async fn delete_task(&self, task_id: TaskId) -> Result<(), ServiceError>;

    async fn get_task(&self, task_id: TaskId) -> Result<Task, ServiceError>;

    async fn list_tasks(&self, filter: ListTasksFilter) -> Result<Vec<Task>, ServiceError>;

    async fn claim_task(&self, params: ClaimTaskParams) -> Result<Task, ServiceError>;

    async fn validate_wrap_up(&self, task_id: TaskId) -> Result<Task, ServiceError>;

    async fn validate_send_message(
        &self,
        from_task_id: TaskId,
        to_task_id: TaskId,
    ) -> Result<(Task, Task), ServiceError>;

    async fn record_hook_event(&self, id: TaskId, kind: HookEventKind) -> Result<(), ServiceError>;

    async fn next_backlog_task(&self, epic_id: EpicId) -> Result<Option<Task>, ServiceError>;
}

/// Consumer-facing seam for epic operations.
///
/// Mirrors the public async surface of [`EpicService`]. See
/// `docs/conventions.md §"Service trait narrowing"`.
#[async_trait::async_trait]
pub trait EpicServiceApi: Send + Sync {
    async fn create_epic(&self, params: CreateEpicParams) -> Result<Epic, ServiceError>;

    async fn get_epic(&self, epic_id: EpicId) -> Result<Epic, ServiceError>;

    async fn get_epic_with_subtasks(
        &self,
        epic_id: EpicId,
    ) -> Result<(Epic, Vec<Task>), ServiceError>;

    async fn list_epics(&self) -> Result<Vec<Epic>, ServiceError>;

    async fn list_root_epics(&self) -> Result<Vec<Epic>, ServiceError>;

    async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<Epic>, ServiceError>;

    async fn list_epics_with_progress(&self) -> Result<Vec<(Epic, usize, usize)>, ServiceError>;

    async fn update_epic(&self, params: UpdateEpicParams) -> Result<EpicId, ServiceError>;

    async fn delete_epic(&self, epic_id: EpicId) -> Result<(), ServiceError>;
}

// ---------------------------------------------------------------------------
// Production impls — delegate to the concrete structs
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl TaskServiceApi for TaskService {
    async fn update_task(
        &self,
        params: UpdateTaskParams,
    ) -> Result<UpdateTaskResult, ServiceError> {
        TaskService::update_task(self, params).await
    }

    async fn move_task_to_epic(
        &self,
        task_id: TaskId,
        new_epic: Option<EpicId>,
    ) -> Result<(), ServiceError> {
        TaskService::move_task_to_epic(self, task_id, new_epic).await
    }

    async fn cli_update_task(
        &self,
        task_id: TaskId,
        new_status: TaskStatus,
        only_if: Option<TaskStatus>,
        sub_status: Option<SubStatus>,
    ) -> Result<bool, ServiceError> {
        TaskService::cli_update_task(self, task_id, new_status, only_if, sub_status).await
    }

    async fn create_task(&self, params: CreateTaskParams) -> Result<TaskId, ServiceError> {
        TaskService::create_task(self, params).await
    }

    async fn create_task_returning(&self, params: CreateTaskParams) -> Result<Task, ServiceError> {
        TaskService::create_task_returning(self, params).await
    }

    async fn delete_task(&self, task_id: TaskId) -> Result<(), ServiceError> {
        TaskService::delete_task(self, task_id).await
    }

    async fn get_task(&self, task_id: TaskId) -> Result<Task, ServiceError> {
        TaskService::get_task(self, task_id).await
    }

    async fn list_tasks(&self, filter: ListTasksFilter) -> Result<Vec<Task>, ServiceError> {
        TaskService::list_tasks(self, filter).await
    }

    async fn claim_task(&self, params: ClaimTaskParams) -> Result<Task, ServiceError> {
        TaskService::claim_task(self, params).await
    }

    async fn validate_wrap_up(&self, task_id: TaskId) -> Result<Task, ServiceError> {
        TaskService::validate_wrap_up(self, task_id).await
    }

    async fn validate_send_message(
        &self,
        from_task_id: TaskId,
        to_task_id: TaskId,
    ) -> Result<(Task, Task), ServiceError> {
        TaskService::validate_send_message(self, from_task_id, to_task_id).await
    }

    async fn record_hook_event(&self, id: TaskId, kind: HookEventKind) -> Result<(), ServiceError> {
        TaskService::record_hook_event(self, id, kind).await
    }

    async fn next_backlog_task(&self, epic_id: EpicId) -> Result<Option<Task>, ServiceError> {
        TaskService::next_backlog_task(self, epic_id).await
    }
}

#[async_trait::async_trait]
impl EpicServiceApi for EpicService {
    async fn create_epic(&self, params: CreateEpicParams) -> Result<Epic, ServiceError> {
        EpicService::create_epic(self, params).await
    }

    async fn get_epic(&self, epic_id: EpicId) -> Result<Epic, ServiceError> {
        EpicService::get_epic(self, epic_id).await
    }

    async fn get_epic_with_subtasks(
        &self,
        epic_id: EpicId,
    ) -> Result<(Epic, Vec<Task>), ServiceError> {
        EpicService::get_epic_with_subtasks(self, epic_id).await
    }

    async fn list_epics(&self) -> Result<Vec<Epic>, ServiceError> {
        EpicService::list_epics(self).await
    }

    async fn list_root_epics(&self) -> Result<Vec<Epic>, ServiceError> {
        EpicService::list_root_epics(self).await
    }

    async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<Epic>, ServiceError> {
        EpicService::list_sub_epics(self, parent_id).await
    }

    async fn list_epics_with_progress(&self) -> Result<Vec<(Epic, usize, usize)>, ServiceError> {
        EpicService::list_epics_with_progress(self).await
    }

    async fn update_epic(&self, params: UpdateEpicParams) -> Result<EpicId, ServiceError> {
        EpicService::update_epic(self, params).await
    }

    async fn delete_epic(&self, epic_id: EpicId) -> Result<(), ServiceError> {
        EpicService::delete_epic(self, epic_id).await
    }
}
