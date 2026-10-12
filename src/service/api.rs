//! Consumer-facing service seams (`*ServiceApi` traits).
//!
//! Each seam is declared **once**, as a `macro_rules!` "spec" macro holding the
//! doc comments and the signature list ([`crate::task_service_api!`],
//! [`crate::epic_service_api!`], [`crate::learning_service_api!`],
//! [`crate::work_package_service_api!`]). A spec macro takes the name of an
//! *emitter* macro and replays its signature list into it, so every artefact
//! derived from the surface is generated from the same tokens and cannot drift:
//!
//! | Emitter | Generates |
//! |---------|-----------|
//! | [`crate::service_api_trait!`] | the `#[async_trait]` trait declaration |
//! | [`crate::service_api_delegate!`] | the production impl, delegating via UFCS to the concrete service |
//! | [`crate::service_api_stub_trait!`] | a test-only stub trait whose methods all default to `panic!` |
//! | [`crate::service_api_stub_bridge!`] | `impl <Api> for <MockType>`, forwarding to that stub trait |
//!
//! Adding a method to a seam therefore means editing exactly one signature
//! list. Test mocks implement the *stub* trait and override only the methods
//! they exercise, so a new method never breaks an unrelated mock.
//!
//! Types in the signature lists are fully qualified (`$crate::…`) because
//! `macro_rules!` resolves type paths at the *call site*: mocks living in other
//! modules invoke the same spec macro.

// The signature lists below name their types through `$crate::…` paths, so the
// only imports this module needs are the concrete services the production impls
// delegate to.
use super::{learnings::LearningService, EpicService, TaskService, WorkPackageService};

// ---------------------------------------------------------------------------
// Emitters — consume a spec macro's signature list, emit one artefact each
// ---------------------------------------------------------------------------

/// Emit the `#[async_trait]` trait declaration for a service seam.
///
/// Invoke through a spec macro: `task_service_api!(service_api_trait);`
#[macro_export]
macro_rules! service_api_trait {
    (
        []
        $(#[$trait_attr:meta])*
        trait $Api:ident for $Concrete:ident, stub $Stub:ident;
        $(
            $(#[$method_attr:meta])*
            async fn $method:ident(&self $(, $arg:ident : $arg_ty:ty)*) -> $ret:ty;
        )*
    ) => {
        $(#[$trait_attr])*
        #[async_trait::async_trait]
        pub trait $Api: Send + Sync {
            $(
                $(#[$method_attr])*
                async fn $method(&self $(, $arg: $arg_ty)*) -> $ret;
            )*
        }
    };
}

/// Emit the production impl, delegating each method via UFCS to the concrete
/// service so the inherent methods are not shadowed.
///
/// Invoke through a spec macro: `task_service_api!(service_api_delegate);`
#[macro_export]
macro_rules! service_api_delegate {
    (
        []
        $(#[$trait_attr:meta])*
        trait $Api:ident for $Concrete:ident, stub $Stub:ident;
        $(
            $(#[$method_attr:meta])*
            async fn $method:ident(&self $(, $arg:ident : $arg_ty:ty)*) -> $ret:ty;
        )*
    ) => {
        #[async_trait::async_trait]
        impl $Api for $Concrete {
            $(
                async fn $method(&self $(, $arg: $arg_ty)*) -> $ret {
                    $Concrete::$method(self $(, $arg)*).await
                }
            )*
        }
    };
}

/// Emit a test-only stub trait mirroring the seam, where every method defaults
/// to `panic!`. Mocks implement this trait and override only what they
/// exercise; unmocked calls fail loudly instead of silently succeeding.
///
/// Invoke through a spec macro: `task_service_api!(service_api_stub_trait);`
#[macro_export]
macro_rules! service_api_stub_trait {
    (
        []
        $(#[$trait_attr:meta])*
        trait $Api:ident for $Concrete:ident, stub $Stub:ident;
        $(
            $(#[$method_attr:meta])*
            async fn $method:ident(&self $(, $arg:ident : $arg_ty:ty)*) -> $ret:ty;
        )*
    ) => {
        /// Test-only mirror of the service seam with panicking defaults.
        ///
        /// Implement this instead of the `*ServiceApi` trait in mocks, override
        /// the methods the test exercises, then bridge the mock onto the real
        /// seam with `service_api_stub_bridge`.
        #[async_trait::async_trait]
        pub trait $Stub: Send + Sync {
            $(
                async fn $method(&self $(, _: $arg_ty)*) -> $ret {
                    panic!(concat!(
                        stringify!($Stub),
                        "::",
                        stringify!($method),
                        " is not mocked — override it in the mock impl"
                    ))
                }
            )*
        }
    };
}

/// Emit `impl <Api> for <MockType>`, forwarding every method to the mock's
/// stub-trait impl.
///
/// Invoke through a spec macro:
/// `task_service_api!(service_api_stub_bridge, MockTaskService);`
#[macro_export]
macro_rules! service_api_stub_bridge {
    (
        [$Mock:ident]
        $(#[$trait_attr:meta])*
        trait $Api:ident for $Concrete:ident, stub $Stub:ident;
        $(
            $(#[$method_attr:meta])*
            async fn $method:ident(&self $(, $arg:ident : $arg_ty:ty)*) -> $ret:ty;
        )*
    ) => {
        #[async_trait::async_trait]
        impl $crate::service::api::$Api for $Mock {
            $(
                async fn $method(&self $(, $arg: $arg_ty)*) -> $ret {
                    <$Mock as $crate::service::api::$Stub>::$method(self $(, $arg)*).await
                }
            )*
        }
    };
}

// ---------------------------------------------------------------------------
// Specs — the single source of truth for each seam's surface
// ---------------------------------------------------------------------------

/// The `TaskServiceApi` surface. Pass an emitter macro name; see the module
/// docs for the available emitters.
#[macro_export]
macro_rules! task_service_api {
    ($emit:ident $(, $extra:tt)*) => {
        $crate::$emit! {
            [$($extra)*]

            /// Consumer-facing seam for task operations.
            ///
            /// Mirrors the public async surface of [`TaskService`]. Callers should hold
            /// `Arc<dyn TaskServiceApi>` so unit tests can inject a mock without spinning
            /// up a real database. See `docs/conventions.md §"Service trait narrowing"`.
            trait TaskServiceApi for TaskService, stub TaskServiceApiStub;

            async fn update_task(
                &self,
                params: $crate::service::UpdateTaskParams
            ) -> Result<$crate::service::UpdateTaskResult, $crate::service::ServiceError>;

            /// Move a task to a different epic, or detach it (`new_epic = None`).
            /// Recalculates the status of both the previous and new epic.
            async fn move_task_to_epic(
                &self,
                task_id: $crate::models::TaskId,
                new_epic: Option<$crate::models::EpicId>
            ) -> Result<(), $crate::service::ServiceError>;

            async fn create_task(
                &self,
                params: $crate::service::CreateTaskParams
            ) -> Result<$crate::models::TaskId, $crate::service::ServiceError>;

            async fn create_task_returning(
                &self,
                params: $crate::service::CreateTaskParams
            ) -> Result<$crate::models::Task, $crate::service::ServiceError>;

            /// The delete pre-check: the store's true row must be done. See
            /// `TaskService::ensure_deletable`.
            async fn ensure_deletable(
                &self,
                task_id: $crate::models::TaskId
            ) -> Result<(), $crate::service::ServiceError>;

            async fn delete_task(
                &self,
                task_id: $crate::models::TaskId
            ) -> Result<(), $crate::service::ServiceError>;

            /// `tasks.allium: BatchDelete`'s atomic call — every task and epic
            /// in the selection, deleted together in one operation. See
            /// `TaskService::batch_delete`'s doc comment.
            async fn batch_delete(
                &self,
                task_ids: &[$crate::models::TaskId],
                epic_ids: &[$crate::models::EpicId]
            ) -> Result<(), $crate::service::ServiceError>;

            /// Batch-update `sub_status` for many tasks in one transaction (tick-driven
            /// activity reclassification). Carries no epic-recalc obligation.
            async fn batch_patch_sub_status(
                &self,
                updates: &[($crate::models::TaskId, $crate::models::SubStatus)]
            ) -> Result<(), $crate::service::ServiceError>;

            async fn get_task(
                &self,
                task_id: $crate::models::TaskId
            ) -> Result<$crate::models::Task, $crate::service::ServiceError>;

            async fn list_tasks(
                &self,
                filter: $crate::service::ListTasksFilter
            ) -> Result<Vec<$crate::models::Task>, $crate::service::ServiceError>;

            async fn validate_wrap_up(
                &self,
                task_id: $crate::models::TaskId
            ) -> Result<$crate::models::Task, $crate::service::ServiceError>;

            /// Apply a session close as one patch and return the tmux window it
            /// cleared. `Err` means exactly "the terminal write did not land",
            /// which is what makes it safe to gate the teardown and the epic
            /// chain on — see `ExitSession` in `docs/specs/pr-workflow.allium`.
            /// Do not replace call sites with a generic `update_task`.
            async fn close_session(
                &self,
                task_id: $crate::models::TaskId,
                outcome: $crate::service::CloseSessionOutcome
            ) -> Result<$crate::service::ClosedSession, $crate::service::ServiceError>;

            async fn record_hook_event(
                &self,
                id: $crate::models::TaskId,
                kind: $crate::models::HookEventKind
            ) -> Result<(), $crate::service::ServiceError>;

            /// Mark the PR-learnings gate as shown, answering whether this
            /// was the first attempt for the task — the one that gets the
            /// reminder. The write carries its own condition, so a task that
            /// does not exist answers `false` rather than failing. See
            /// `PrLearningsGate` in `docs/specs/pr-workflow.allium`.
            async fn mark_pr_learnings_gate_shown(
                &self,
                id: $crate::models::TaskId
            ) -> Result<bool, $crate::service::ServiceError>;

            /// Stamp an observed native `SendMessage` tool call on the
            /// sender's row, and on the target's when the name resolves to a
            /// task that still exists. Only a missing *sender* is an error.
            /// See `HookPeerMessageSent` in `docs/specs/agent-health.allium`.
            async fn record_peer_message_sent(
                &self,
                sender_id: $crate::models::TaskId,
                target_name: &str
            ) -> Result<(), $crate::service::ServiceError>;

            /// Record a subagent lifecycle event and, when it drains the last
            /// subagent for a task carrying a deferred Stop, apply that Stop.
            /// See `HookSubagentStart` / `HookSubagentStop` in
            /// `docs/specs/agent-health.allium`.
            async fn record_subagent_event(
                &self,
                id: $crate::models::TaskId,
                event: $crate::models::SubagentEvent
            ) -> Result<(), $crate::service::ServiceError>;

            /// Clear a task's subagent entries and void any pending Stop
            /// **without** the drain path — for callers that already own the
            /// resulting status (crash, dispatch) and for `SessionStart`, where
            /// the deferred Stop belongs to a finished turn. See
            /// `record_subagent_event` for the draining twin.
            async fn clear_subagents_no_drain(
                &self,
                id: $crate::models::TaskId
            ) -> Result<(), $crate::service::ServiceError>;

            /// `store-split.allium: RequestDispatch` — the one call every
            /// launch path makes (D6): claim the task and write a `requested`
            /// dispatch request naming THIS host, in one module transaction.
            /// Provisions nothing; this host's board serves the row
            /// ([`Self::serve_dispatch_request`]). `mode` `None` lets the module
            /// derive it (D3).
            ///
            /// A refusal is an ordinary `RequestDispatchAnswer::Refused`, not an
            /// `Err`: it is how a caller learns another entry point claimed the
            /// task first (`DispatchClaimExclusive` in
            /// `docs/specs/dispatch.allium`). Do not add a "read the status,
            /// then provision" path beside it.
            async fn request_dispatch(
                &self,
                task_id: $crate::models::TaskId,
                origin: $crate::models::DispatchOrigin,
                mode: Option<$crate::models::DispatchMode>
            ) -> Result<$crate::store::RequestDispatchAnswer, $crate::service::ServiceError>;

            /// `HostClaimsDispatch` -> `ProvisionDispatch` for one delivered
            /// row: claim it, provision with the row's mode under the row's
            /// deadline, report dispatched or failed. THE dispatch seam — the
            /// only place the board provisions a dispatch. Never `Err`.
            async fn serve_dispatch_request(
                &self,
                request: $crate::models::DispatchRequest,
                emb_svc: std::sync::Arc<$crate::embeddings::EmbeddingService>
            ) -> $crate::service::ServedDispatch;

            /// `HostRestartFailsInterrupted`: fail the rows this host left
            /// `provisioning`, releasing their claims. Once, at startup.
            async fn fail_interrupted_dispatch_requests(
                &self
            ) -> Result<(), $crate::service::ServiceError>;

            /// `store-split.allium: RequestWrapUp`: write a wrap_up row naming
            /// THIS host, after the caller's host-local checks. `done` and `pr`
            /// come back with a row already done, whose id is the exit token; a
            /// `rebase` row is served by this host's board
            /// ([`Self::serve_session_request`]). A refusal is an ordinary
            /// `SessionRequestAnswer::Refused` carrying the module's reason.
            async fn request_wrap_up(
                &self,
                task_id: $crate::models::TaskId,
                action: $crate::models::WrapUpMode
            ) -> Result<$crate::store::SessionRequestAnswer, $crate::service::ServiceError>;

            /// `store-split.allium: RequestExitSession`: the token check, the
            /// close, the epic chain and the exit_session row in one module
            /// transaction — whole or absent, the token valid after a refusal.
            /// Tears nothing down.
            async fn request_exit_session(
                &self,
                task_id: $crate::models::TaskId,
                token: $crate::models::SessionRequestId,
                action: $crate::models::WrapUpMode,
                pr_url: Option<String>
            ) -> Result<$crate::store::SessionRequestAnswer, $crate::service::ServiceError>;

            /// `HostClaimsSessionRequest` -> `WrapUpRebase` /
            /// `ExitSessionTeardown` for one delivered row: claim it, do the
            /// host work under the row's deadline, report done or failed. THE
            /// session seam — the only place the board rebases for a wrap-up or
            /// tears a closed session's window down. Never `Err`.
            async fn serve_session_request(
                &self,
                request: $crate::models::SessionRequest
            ) -> $crate::service::ServedSession;

            /// `KillWindowOfFailedExitSession`: best-effort kill of the window a
            /// failed exit_session row names.
            async fn tear_down_failed_exit_window(
                &self,
                request: &$crate::models::SessionRequest
            ) -> ();

            /// `HostRestartFailsInterruptedSessionRequests`: fail the rows this
            /// host left `running`. Once, at startup.
            async fn fail_interrupted_session_requests(
                &self
            ) -> Result<(), $crate::service::ServiceError>;

            /// The checks `wrap_up` makes before it acts: the worktree is
            /// committed and the repo's verify command, if any, passes in it.
            /// `Err(Validation)` is the refusal message.
            /// `WrapUpRefusesUnfinishedWork` in
            /// `docs/specs/mcp-task-tools.allium`.
            async fn check_wrap_up_ready(
                &self,
                task: $crate::models::Task
            ) -> Result<$crate::service::WrapUpChecks, $crate::service::ServiceError>;

            /// Kill the tmux window a closed session left behind. Keeps the
            /// only tmux call the MCP layer needs behind the service boundary
            /// — see `ExitSession` in `docs/specs/pr-workflow.allium`.
            async fn kill_session_window(&self, window: $crate::models::TmuxWindow) -> ();
        }
    };
}

/// The `EpicServiceApi` surface. Pass an emitter macro name; see the module
/// docs for the available emitters.
#[macro_export]
macro_rules! epic_service_api {
    ($emit:ident $(, $extra:tt)*) => {
        $crate::$emit! {
            [$($extra)*]

            /// Consumer-facing seam for epic operations.
            ///
            /// Mirrors the public async surface of [`EpicService`]. See
            /// `docs/conventions.md §"Service trait narrowing"`.
            trait EpicServiceApi for EpicService, stub EpicServiceApiStub;

            async fn create_epic(
                &self,
                params: $crate::service::CreateEpicParams
            ) -> Result<$crate::models::Epic, $crate::service::ServiceError>;

            async fn get_epic(
                &self,
                epic_id: $crate::models::EpicId
            ) -> Result<$crate::models::Epic, $crate::service::ServiceError>;

            async fn get_epic_with_subtasks(
                &self,
                epic_id: $crate::models::EpicId
            ) -> Result<($crate::models::Epic, Vec<$crate::models::Task>), $crate::service::ServiceError>;

            async fn get_epic_with_progress(
                &self,
                epic_id: $crate::models::EpicId
            ) -> Result<($crate::models::Epic, usize, usize), $crate::service::ServiceError>;

            async fn list_epics(
                &self
            ) -> Result<Vec<$crate::models::Epic>, $crate::service::ServiceError>;

            async fn list_root_epics(
                &self
            ) -> Result<Vec<$crate::models::Epic>, $crate::service::ServiceError>;

            async fn list_sub_epics(
                &self,
                parent_id: $crate::models::EpicId
            ) -> Result<Vec<$crate::models::Epic>, $crate::service::ServiceError>;

            async fn list_epics_with_progress(
                &self
            ) -> Result<Vec<($crate::models::Epic, usize, usize)>, $crate::service::ServiceError>;

            async fn list_epics_with_progress_under(
                &self,
                parent: Option<$crate::models::EpicId>,
                recursive: bool
            ) -> Result<Vec<($crate::models::Epic, usize, usize)>, $crate::service::ServiceError>;

            async fn update_epic(
                &self,
                params: $crate::service::UpdateEpicParams
            ) -> Result<$crate::service::UpdateEpicResult, $crate::service::ServiceError>;

            /// The delete pre-check: every subtree task must be done in the
            /// store. See `EpicService::ensure_deletable`.
            async fn ensure_deletable(
                &self,
                epic_id: $crate::models::EpicId
            ) -> Result<(), $crate::service::ServiceError>;

            async fn delete_epic(
                &self,
                epic_id: $crate::models::EpicId
            ) -> Result<(), $crate::service::ServiceError>;

            async fn regroup_epic(
                &self,
                root: $crate::models::EpicId
            ) -> Result<(), $crate::service::ServiceError>;

            async fn flatten_epic(
                &self,
                root: $crate::models::EpicId
            ) -> Result<(), $crate::service::ServiceError>;

            /// Materialise the managed feed-epic tree from already-read settings. The
            /// caller reads the four settings via its read-only handle
            /// ([`crate::service::read_managed_feed_settings`]) and passes them in; the
            /// epic writes happen here, behind the service boundary.
            async fn provision_managed_feeds(
                &self,
                settings: $crate::service::ManagedFeedSettings
            ) -> Result<(), $crate::service::ServiceError>;
        }
    };
}

/// The `WorkPackageServiceApi` surface. Pass an emitter macro name; see the
/// module docs for the available emitters.
#[macro_export]
macro_rules! work_package_service_api {
    ($emit:ident $(, $extra:tt)*) => {
        $crate::$emit! {
            [$($extra)*]

            /// Consumer-facing seam for work-package operations
            /// (`docs/specs/work-packages.allium`).
            ///
            /// Mirrors the public async surface of [`WorkPackageService`].
            trait WorkPackageServiceApi for WorkPackageService, stub WorkPackageServiceApiStub;

            async fn create_work_package(
                &self,
                params: $crate::service::CreateWorkPackageParams
            ) -> Result<$crate::models::WorkPackageId, $crate::service::ServiceError>;

            /// Every package, or only `id`, each with its resolved links.
            async fn list_work_packages(
                &self,
                id: Option<$crate::models::WorkPackageId>
            ) -> Result<Vec<$crate::service::WorkPackageView>, $crate::service::ServiceError>;

            /// Title/description, then unlinks, then links — all validated
            /// before the first write.
            async fn update_work_package(
                &self,
                params: $crate::service::UpdateWorkPackageParams
            ) -> Result<(), $crate::service::ServiceError>;

            async fn delete_work_package(
                &self,
                id: $crate::models::WorkPackageId
            ) -> Result<(), $crate::service::ServiceError>;
        }
    };
}

/// The `LearningServiceApi` surface. Pass an emitter macro name; see the module
/// docs for the available emitters.
#[macro_export]
macro_rules! learning_service_api {
    ($emit:ident $(, $extra:tt)*) => {
        $crate::$emit! {
            [$($extra)*]

            /// Consumer-facing seam for learning operations.
            ///
            /// Mirrors the public async surface of [`LearningService`]. Callers should hold
            /// `Arc<dyn LearningServiceApi>` so unit tests can inject a mock without spinning
            /// up a real database. See `docs/conventions.md §"Service trait narrowing"`.
            trait LearningServiceApi for LearningService, stub LearningServiceApiStub;

            async fn create_learning(
                &self,
                params: $crate::service::CreateLearningParams
            ) -> Result<$crate::models::LearningId, $crate::service::ServiceError>;

            async fn get_learning(
                &self,
                id: $crate::models::LearningId
            ) -> Result<$crate::models::Learning, $crate::service::ServiceError>;

            async fn list_learnings(
                &self,
                filter: $crate::store::LearningFilter
            ) -> Result<Vec<$crate::models::Learning>, $crate::service::ServiceError>;

            async fn record_retrieval(
                &self,
                task_id: $crate::models::TaskId,
                learning_id: $crate::models::LearningId,
                source: $crate::models::RetrievalSource
            ) -> Result<(), $crate::service::ServiceError>;

            async fn apply_verdicts(
                &self,
                task_id: $crate::models::TaskId,
                verdicts: Vec<($crate::models::LearningId, $crate::models::LearningVerdict)>
            ) -> Result<(), $crate::service::ServiceError>;

            async fn archive_stale_learnings(
                &self,
                cutoff: chrono::DateTime<chrono::Utc>
            ) -> Result<u64, $crate::service::ServiceError>;

            async fn delete_learning(
                &self,
                id: $crate::models::LearningId
            ) -> Result<(), $crate::service::ServiceError>;

            async fn query_learnings(
                &self,
                params: $crate::service::QueryLearningsParams
            ) -> Result<Vec<$crate::models::Learning>, $crate::service::ServiceError>;
        }
    };
}

// ---------------------------------------------------------------------------
// Trait declarations
// ---------------------------------------------------------------------------

task_service_api!(service_api_trait);
epic_service_api!(service_api_trait);
learning_service_api!(service_api_trait);
work_package_service_api!(service_api_trait);

// ---------------------------------------------------------------------------
// Production impls — delegate to the concrete structs
// ---------------------------------------------------------------------------

task_service_api!(service_api_delegate);
epic_service_api!(service_api_delegate);
learning_service_api!(service_api_delegate);
work_package_service_api!(service_api_delegate);

// ---------------------------------------------------------------------------
// Test-only stub traits — mocks override only what they exercise
// ---------------------------------------------------------------------------

#[cfg(test)]
task_service_api!(service_api_stub_trait);
#[cfg(test)]
learning_service_api!(service_api_stub_trait);
#[cfg(test)]
work_package_service_api!(service_api_stub_trait);

/// No-op [`LearningServiceApi`] for tests that construct a [`TuiRuntime`] or
/// [`McpState`] but never exercise learning operations. Overrides nothing, so
/// every method keeps `LearningServiceApiStub`'s panicking default — accidental
/// learning-service calls in non-learning tests are caught.
#[cfg(test)]
pub struct MockLearningService;

#[cfg(test)]
impl LearningServiceApiStub for MockLearningService {}

#[cfg(test)]
learning_service_api!(service_api_stub_bridge, MockLearningService);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Task, TaskId};
    use crate::service::{
        CreateEpicParams, CreateLearningParams, CreateTaskParams, ListTasksFilter, ServiceError,
    };
    use crate::store::Store;
    use std::sync::Arc;

    /// `TaskStore` because `TaskService` takes it; upcasts for `EpicService`.
    async fn store() -> Arc<dyn crate::store::TaskStore> {
        Arc::new(Store::open_in_memory().unwrap())
    }

    // -----------------------------------------------------------------------
    // Delegation guards — every `*Api` trait must reach the concrete service
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn task_service_api_delegates_to_task_service() {
        let svc: Arc<dyn TaskServiceApi> = Arc::new(TaskService::new(
            store().await,
            crate::process::MockProcessRunner::unused(),
        ));

        let id = svc
            .create_task(CreateTaskParams::fixture("delegated", "/repo".to_string()))
            .await
            .unwrap();

        assert_eq!(svc.get_task(id).await.unwrap().title, "delegated");
        assert_eq!(
            svc.list_tasks(ListTasksFilter::default())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn epic_service_api_delegates_to_epic_service() {
        let store = store().await;
        let svc: Arc<dyn EpicServiceApi> = Arc::new(EpicService::new(store.clone()));

        let epic = svc
            .create_epic(CreateEpicParams::fixture("delegated epic"))
            .await
            .unwrap();

        assert_eq!(svc.get_epic(epic.id).await.unwrap().title, "delegated epic");
        assert_eq!(svc.list_epics().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn work_package_service_api_delegates_to_work_package_service() {
        let store = store().await;
        let svc: Arc<dyn WorkPackageServiceApi> = Arc::new(WorkPackageService::new(store));

        let id = svc
            .create_work_package(crate::service::CreateWorkPackageParams {
                title: "delegated package".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();

        let views = svc.list_work_packages(Some(id)).await.unwrap();
        assert_eq!(views[0].package.title, "delegated package");
    }

    #[tokio::test]
    async fn learning_service_api_delegates_to_learning_service() {
        let db: Arc<dyn crate::store::TaskStore> = Arc::new(Store::open_in_memory().unwrap());
        let svc: Arc<dyn LearningServiceApi> = Arc::new(LearningService::new(
            db,
            crate::embeddings::EmbeddingService::new_test(),
        ));

        let id = svc
            .create_learning(CreateLearningParams {
                kind: crate::models::LearningKind::Convention,
                summary: "delegated learning".to_string(),
                detail: None,
                scope: crate::models::LearningScope::Repo,
                scope_ref: Some("/repo".to_string()),
                tags: vec![],
                source_task_id: None,
            })
            .await
            .unwrap();

        assert_eq!(
            svc.get_learning(id).await.unwrap().summary,
            "delegated learning"
        );
    }

    // -----------------------------------------------------------------------
    // Stub-trait machinery — mocks declare only the methods they exercise
    // -----------------------------------------------------------------------

    /// A partial mock: overrides one method, inherits panicking defaults for
    /// the rest. Adding a method to `TaskServiceApi` must not break this.
    struct PartialTaskStub;

    #[async_trait::async_trait]
    impl TaskServiceApiStub for PartialTaskStub {
        async fn list_tasks(&self, _: ListTasksFilter) -> Result<Vec<Task>, ServiceError> {
            Ok(vec![])
        }
    }

    crate::task_service_api!(service_api_stub_bridge, PartialTaskStub);

    #[tokio::test]
    async fn stub_override_is_reached_through_the_api_trait() {
        let svc: Arc<dyn TaskServiceApi> = Arc::new(PartialTaskStub);

        assert!(svc
            .list_tasks(ListTasksFilter::default())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    #[should_panic(expected = "not mocked")]
    async fn stub_default_panics_for_a_method_the_mock_did_not_override() {
        let svc: Arc<dyn TaskServiceApi> = Arc::new(PartialTaskStub);

        let _ = svc.get_task(TaskId(1)).await;
    }
}
