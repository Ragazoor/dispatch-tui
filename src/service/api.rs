use crate::db::LearningFilter;
use crate::models::{
    Epic, EpicId, HookEventKind, Learning, LearningId, LearningVerdict, RetrievalSource, SubStatus,
    Task, TaskId, TaskStatus, Todo, TodoId,
};

use super::{
    learnings::{CreateLearningParams, LearningService, UpdateLearningParams},
    todos::{TodoService, TodoUpdate},
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

/// Consumer-facing seam for todo operations.
///
/// Mirrors the public async surface of [`TodoService`]. See
/// `docs/conventions.md §"Service trait narrowing"`.
#[async_trait::async_trait]
pub trait TodoServiceApi: Send + Sync {
    async fn list_todos(&self) -> Result<Vec<Todo>, ServiceError>;

    async fn create_todo(
        &self,
        title: String,
        linked: Option<crate::models::TodoLink>,
    ) -> Result<Todo, ServiceError>;

    async fn update_todo(&self, id: TodoId, update: TodoUpdate) -> Result<(), ServiceError>;

    async fn delete_todo(&self, id: TodoId) -> Result<(), ServiceError>;

    async fn clear_done(&self) -> Result<(), ServiceError>;
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

#[async_trait::async_trait]
impl TodoServiceApi for TodoService {
    async fn list_todos(&self) -> Result<Vec<Todo>, ServiceError> {
        TodoService::list_todos(self).await
    }

    async fn create_todo(
        &self,
        title: String,
        linked: Option<crate::models::TodoLink>,
    ) -> Result<Todo, ServiceError> {
        TodoService::create_todo(self, title, linked).await
    }

    async fn update_todo(&self, id: TodoId, update: TodoUpdate) -> Result<(), ServiceError> {
        TodoService::update_todo(self, id, update).await
    }

    async fn delete_todo(&self, id: TodoId) -> Result<(), ServiceError> {
        TodoService::delete_todo(self, id).await
    }

    async fn clear_done(&self) -> Result<(), ServiceError> {
        TodoService::clear_done(self).await
    }
}

/// Consumer-facing seam for learning operations.
///
/// Mirrors the public async surface of [`LearningService`]. Callers should hold
/// `Arc<dyn LearningServiceApi>` so unit tests can inject a mock without spinning
/// up a real database. See `docs/conventions.md §"Service trait narrowing"`.
#[async_trait::async_trait]
pub trait LearningServiceApi: Send + Sync {
    async fn create_learning(
        &self,
        params: CreateLearningParams,
    ) -> Result<LearningId, ServiceError>;

    async fn get_learning(&self, id: LearningId) -> Result<Learning, ServiceError>;

    async fn list_learnings(
        &self,
        filter: LearningFilter,
    ) -> Result<Vec<Learning>, ServiceError>;

    async fn approve_learning(&self, id: LearningId) -> Result<(), ServiceError>;

    async fn reject_learning(&self, id: LearningId) -> Result<(), ServiceError>;

    async fn archive_learning(&self, id: LearningId) -> Result<(), ServiceError>;

    async fn update_learning(&self, params: UpdateLearningParams) -> Result<(), ServiceError>;

    async fn record_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<(), ServiceError>;

    async fn apply_verdicts(
        &self,
        task_id: TaskId,
        verdicts: Vec<(LearningId, LearningVerdict)>,
    ) -> Result<(), ServiceError>;
}

#[async_trait::async_trait]
impl LearningServiceApi for LearningService {
    async fn create_learning(
        &self,
        params: CreateLearningParams,
    ) -> Result<LearningId, ServiceError> {
        LearningService::create_learning(self, params).await
    }

    async fn get_learning(&self, id: LearningId) -> Result<Learning, ServiceError> {
        LearningService::get_learning(self, id).await
    }

    async fn list_learnings(
        &self,
        filter: LearningFilter,
    ) -> Result<Vec<Learning>, ServiceError> {
        LearningService::list_learnings(self, filter).await
    }

    async fn approve_learning(&self, id: LearningId) -> Result<(), ServiceError> {
        LearningService::approve_learning(self, id).await
    }

    async fn reject_learning(&self, id: LearningId) -> Result<(), ServiceError> {
        LearningService::reject_learning(self, id).await
    }

    async fn archive_learning(&self, id: LearningId) -> Result<(), ServiceError> {
        LearningService::archive_learning(self, id).await
    }

    async fn update_learning(&self, params: UpdateLearningParams) -> Result<(), ServiceError> {
        LearningService::update_learning(self, params).await
    }

    async fn record_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<(), ServiceError> {
        LearningService::record_retrieval(self, task_id, learning_id, source).await
    }

    async fn apply_verdicts(
        &self,
        task_id: TaskId,
        verdicts: Vec<(LearningId, LearningVerdict)>,
    ) -> Result<(), ServiceError> {
        LearningService::apply_verdicts(self, task_id, verdicts).await
    }
}
