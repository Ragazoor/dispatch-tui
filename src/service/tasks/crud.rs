//! `TaskService` — CRUD and lifecycle operations for tasks.
//!
//! Pure parameter shapes live in `params.rs`; the patch builder lives in
//! `validators.rs`. Methods that read or mutate DB state are kept here
//! because they need `&self`.

use std::sync::Arc;

use crate::models::{
    classify_agent_activity, EpicId, HookEventKind, NotificationWrite, SubagentEvent, Task, TaskId,
    TaskStatus, TaskUrl, WrapUpBlock, DEFAULT_BASE_BRANCH,
};
use crate::service::ServiceError;
use crate::store::{self, CreateTaskRequest, TaskPatch};

use super::params::{CreateTaskParams, ListTasksFilter, UpdateTaskParams};
use super::validators::build_task_patch;
use crate::service::UrlUpdate;

/// `ClosePrOnDone` (`docs/specs/pr-workflow.allium`): the PR url to close,
/// exactly when `prior`/`next` is a genuine transition into Done and `url` is
/// pr-typed.
///
/// Deliberately fires for every route into Done, including `PrMerged`: there
/// is no cheap signal here distinguishing "entered Done because this task's
/// own PR was just merged" from any other route, and closing an
/// already-merged PR is a no-op `gh` itself refuses — see
/// `TaskService::spawn_close_attached_pr` for how that outcome is logged.
fn pr_url_to_close(prior: TaskStatus, next: TaskStatus, url: Option<&TaskUrl>) -> Option<String> {
    if next != TaskStatus::Done || prior == TaskStatus::Done {
        return None;
    }
    url.filter(|u| u.is_pr()).map(|u| u.url.clone())
}

/// Substrings of `gh pr close`'s stderr meaning the PR was already in a state
/// `gh` refuses to change — expected, not a fault, so
/// `TaskService::spawn_close_attached_pr` logs these at debug rather than
/// warn. The list is its own (deliberately not `dispatch::PERMANENT_GH_FAILURE_MARKERS`
/// — "expected" means something different for a close than for a view), but
/// the matching itself reuses `dispatch::matches_any_marker`, the same
/// primitive `classify_gh_failure` uses.
const EXPECTED_GH_CLOSE_FAILURE_MARKERS: &[&str] = &[
    "already merged",
    "already closed",
    "Pull request is not open",
];

/// The name part of a native `SendMessage` `to` value. Strips a
/// disambiguating `" [ref]"` suffix — `ListAgents`/`SendMessage` append one
/// when more than one live agent answers to a name.
fn peer_message_target_name(to: &str) -> &str {
    to.split(' ').next().unwrap_or(to)
}

/// Parse a native `SendMessage` `to` value as the `task-<id>` name dispatch
/// gave every session before it named them after their agents
/// (`session_name_flag` in `src/dispatch/agents.rs`), delegating to
/// [`TmuxWindow::task_id`](crate::models::TmuxWindow::task_id), the same parser
/// tmux window names go through. Still read for a session started before
/// agents and not resumed since, which keeps the name it began with. `None`
/// for any value that doesn't match this shape.
fn parse_peer_message_target_name(to: &str) -> Option<TaskId> {
    crate::models::TmuxWindow::parse(peer_message_target_name(to))?.task_id()
}

/// Result of [`TaskService::update_task`]. Carries the updated task id plus
/// presentation-relevant transition flags so MCP handlers can format their
/// response without re-reading the DB.
#[derive(Debug, Clone)]
pub struct UpdateTaskResult {
    pub task_id: TaskId,
    /// `true` when the same call set a PR-typed `url` on a task that
    /// previously had no url AND moved its status to Review.
    pub was_pr_finalisation: bool,
    /// Whether this call wrote `completed_at`, and to what.
    ///
    /// `None` means this call's patch didn't touch `completed_at` at all (the
    /// in-memory value the caller already holds is still current). `Some(v)`
    /// means the patch wrote `completed_at`, where `v` is exactly what was
    /// written — `Some(None)` for a clear, `Some(Some(t))` for a set to `t`.
    /// Callers that hold their own in-memory copy of the task (the TUI's
    /// `App.board.tasks`) use this to learn a value they could not have
    /// computed themselves: `completed_at_for_status_transition` runs inside
    /// this method, not at the call site.
    pub completed_at_after_write: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

pub struct TaskService {
    /// The wide bundle, not the `TaskAndEpicStore` the CRUD methods alone would
    /// need: [`serve_dispatch_request`](Self::serve_dispatch_request) runs the
    /// dispatch prologue, which
    /// reads the whole [`TaskReadStore`](store::TaskReadStore) surface (epic
    /// banner, learning injections and their retrieval records). `TaskStore` is
    /// the whole store plus that read bundle, so it is still the narrowest
    /// handle covering what this service actually calls — and one
    /// handle is what keeps the prologue's reads and the service's writes on
    /// the same database by construction.
    pub db: Arc<dyn store::TaskStore>,
    pub(super) clock: Arc<dyn crate::clock::Clock>,
    pub(super) runner: Arc<dyn crate::process::ProcessRunner>,
    /// Handles for in-flight `ClosePrOnDone` closes (`spawn_close_attached_pr`),
    /// so a test can wait for a fire-and-forget close to actually finish —
    /// deterministically, rather than sleeping — before asserting on
    /// `MockProcessRunner::recorded_calls()`. See `wait_for_background_pr_closes`.
    /// Unused outside `cfg(test)`: production never reads this back, since the
    /// whole point of spawning is that nothing waits on it.
    #[cfg(test)]
    background_pr_closes: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl TaskService {
    /// Construct a `TaskService` with an explicitly chosen process runner.
    ///
    /// Required, not defaulted: `TaskService` really does shell out (see
    /// `wrap_up.rs`), so a default would let a test silently touch the host.
    /// Tests pass `MockProcessRunner::unused`;
    /// production says so by name via
    /// [`new_with_real_runner`](Self::new_with_real_runner).
    pub fn new(
        db: Arc<dyn store::TaskStore>,
        runner: Arc<dyn crate::process::ProcessRunner>,
    ) -> Self {
        Self {
            db,
            clock: Arc::new(crate::clock::SystemClock),
            runner,
            #[cfg(test)]
            background_pr_closes: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Construct a `TaskService` that shells out for real. Named so that the
    /// non-hermetic choice is visible at the call site; see [`new`](Self::new).
    pub fn new_with_real_runner(db: Arc<dyn store::TaskStore>) -> Self {
        // `RealProcessRunner::default()` names no `~/.claude.json`, so an agent
        // dispatched through a service built here would carry no caller
        // identity. No caller does that today (pr-gate, hooks, plan attach);
        // one that needs to must hand the runner the path, as TUI startup does.
        Self::new(db, Arc::new(crate::process::RealProcessRunner::default()))
    }

    /// Override the clock used for timestamping. Tests inject a
    /// [`FixedClock`](crate::clock::FixedClock) so timestamp-dependent flows
    /// (hook-event ordering) are deterministic without sleeping.
    ///
    /// Unlike the runner, this stays an optional builder on purpose:
    /// `SystemClock` only reads the wall clock, so an un-injected default
    /// costs determinism, never a real side effect.
    pub fn with_clock(mut self, clock: Arc<dyn crate::clock::Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Updates a task. Used by MCP handlers and internal dispatch flows.
    ///
    /// Supports the full `UpdateTaskParams` builder (title, description,
    /// repo_path, pr_url, worktree, tmux_window, base_branch, sub_status,
    /// epic_id, sort_order, tag, status). Epic recalculation is the store's:
    /// `patch_task` and `set_task_epic` recalculate the epics they touch.
    ///
    /// This is the only status-writing path on the service: it can transition
    /// to any status including `Done`. The restriction against agents
    /// completing their own tasks lives at the MCP handler layer, not here —
    /// the TUI needs the unrestricted surface.
    pub async fn update_task(
        &self,
        params: UpdateTaskParams,
    ) -> Result<UpdateTaskResult, ServiceError> {
        if !params.has_any_field() {
            return Err(ServiceError::Validation(
                "At least one field must be provided".into(),
            ));
        }

        let task_id = params.task_id;
        // The shared store's `patch_task` is a deliberate no-op on a missing
        // id (hooks race with deletes), so the not-found answer is this
        // service's to give — see `UpdateTaskViaMcp` in mcp-task-tools.allium.
        if !self.db.task_exists(task_id).await {
            return Err(Self::task_not_found(task_id));
        }
        let expanded_repo_path = params.repo_path.as_deref().map(crate::models::expand_tilde);

        // The sub-status goes through as given: the store's `patch_task`
        // refuses one that is illegal for the status the row will hold
        // (`UpdateTaskViaMcp`, store-split M2).
        let mut patch = build_task_patch(&params, expanded_repo_path.as_deref(), params.sub_status);

        // Snapshot the task before the patch, whenever `status` changes: the
        // PR-finalisation check, ClosePrOnDone and the Done-transition echo
        // below all read the prior row. Nothing else here needs it.
        let is_pr_url_set = matches!(
            params.url.as_ref(),
            Some(UrlUpdate::Set(u)) if u.is_pr()
        );
        let prior = if params.status.is_some() {
            self.db.get_task(task_id).await
        } else {
            None
        };
        let was_pr_finalisation = params.status == Some(TaskStatus::Review)
            && is_pr_url_set
            && prior.as_ref().is_some_and(|t| t.url.is_none());

        // A transition into Done is stamped by the module's `patch_task`
        // (`completed_at`, and the transition rules that go with it,
        // store-split.allium X2), not here. The stamp must win over anything the
        // caller's params already set for completed_at —
        // exec_persist_task (src/runtime/tasks.rs) forwards whatever is
        // sitting on the in-memory Task struct alongside a status change, and a
        // stale snapshot must not overwrite a fresh completion — and the module
        // lets a completion the patch names win, so the caller's is dropped
        // from the patch rather than overwritten. Not a computation of the
        // stamp: the module still decides it.
        let entering_done = params.status == Some(TaskStatus::Done)
            && prior.as_ref().is_some_and(|p| p.status != TaskStatus::Done);
        if entering_done {
            patch.completed_at = None;
        }

        // ORDER OF THE WRITES (mcp-task-tools.allium: UpdateTaskViaMcp). An
        // explicit relink goes BEFORE the patch when the call also changes the
        // repo: the store routes the relink by the repo the task has now, and
        // the patch's repo change then re-routes it within that subtree, so the
        // call ends where routing by the new repo puts it. Patch-first would
        // let the patch's re-route delete an explicitly named sub-epic before
        // the relink lands in it. Otherwise the patch goes first, so a patch
        // the store refuses (an illegal sub_status) leaves the task where it
        // was.
        let relink_first = params.repo_path.is_some();
        if relink_first {
            if let Some(epic_id) = params.epic_id {
                self.db.set_task_epic_id(task_id, Some(epic_id)).await?;
            }
        }
        self.db.patch_task(task_id, &patch).await?;
        if !relink_first {
            if let Some(epic_id) = params.epic_id {
                self.db.set_task_epic_id(task_id, Some(epic_id)).await?;
            }
        }

        // ClosePrOnDone (docs/specs/pr-workflow.allium): resolved from the
        // final url this call leaves the task with — an explicit `params.url`
        // wins, otherwise the prior row's url carries over unchanged — not
        // just `prior`'s, so a call that attaches a pr-typed url and moves to
        // Done in the same write is still caught.
        let final_url = match params.url.as_ref() {
            Some(u) => u.as_option(),
            None => prior.as_ref().and_then(|t| t.url.as_ref()),
        };
        self.close_pr_on_done(task_id, prior.as_ref(), params.status, final_url);

        // No epic recalculation, no stop_pending clear and no phoenix respawn
        // here: `patch_task` and `set_task_epic` do all three in their own
        // transactions (store-split.allium X2, X3).

        // What this call wrote to completed_at. A stamp the module chose is
        // read back rather than computed, so the TUI's in-memory copy agrees
        // with the store. See `UpdateTaskResult`.
        let completed_at_after_write = if entering_done {
            self.db.get_task(task_id).await.map(|t| t.completed_at)
        } else {
            patch.completed_at
        };

        Ok(UpdateTaskResult {
            task_id,
            was_pr_finalisation,
            completed_at_after_write,
        })
    }

    /// `ClosePrOnDone` (`docs/specs/pr-workflow.allium`): one call site per
    /// status-writing method — derive from (prior, next), then act — rather than each
    /// caller separately computing `pr_url_to_close` and separately guarding
    /// the close.
    ///
    /// Synchronous: the close itself never runs on this call's stack, so there
    /// is nothing here to await. See `spawn_close_attached_pr`.
    fn close_pr_on_done(
        &self,
        task_id: TaskId,
        prior: Option<&Task>,
        new_status: Option<TaskStatus>,
        url: Option<&TaskUrl>,
    ) {
        let Some(new_status) = new_status else {
            return;
        };
        let Some(prior) = prior else { return };
        if let Some(pr_url) = pr_url_to_close(prior.status, new_status, url) {
            self.spawn_close_attached_pr(task_id, pr_url);
        }
    }

    /// Spawn `gh pr close <url>` for a task that just entered Done carrying a
    /// pr-typed url, detached from the caller.
    ///
    /// Called only after the Done write has already persisted, and never
    /// awaited by the caller — deliberately: `update_task` and a served close run
    /// on the TUI's synchronous command-drain loop
    /// (`src/runtime/event_loop.rs::run_loop`), which does not redraw or read input
    /// again until every queued command's future resolves. Awaiting an actual
    /// `gh` network round trip inline there — bounded by
    /// `SUBPROCESS_TIMEOUT` (120s) — would freeze the whole TUI on the
    /// ordinary case of finishing a PR-backed task, not just an edge case.
    /// Detaching keeps the Done write's own latency exactly what it was
    /// before this feature existed.
    ///
    /// Never surfaces an error to anything: there is nothing to retry and no
    /// give-up state, unlike `PollPrStatus` — either the close took or it
    /// didn't, and the task is already Done either way. An expected failure
    /// (the PR was already merged or already closed — the outcome for a task
    /// entering Done via `PrMerged` itself, which this rule does not
    /// distinguish from any other route in) logs at debug; anything else
    /// warns.
    pub(super) fn spawn_close_attached_pr(&self, task_id: TaskId, url: String) {
        let runner = self.runner.clone();
        let close = async move {
            let joined =
                tokio::task::spawn_blocking(move || crate::dispatch::close_pr(&url, &*runner))
                    .await;
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let message = format!("{e:#}");
                    if crate::dispatch::matches_any_marker(
                        &message,
                        EXPECTED_GH_CLOSE_FAILURE_MARKERS,
                    ) {
                        tracing::debug!(
                            task_id = task_id.0,
                            "gh pr close: PR was already merged or closed: {message}"
                        );
                    } else {
                        tracing::warn!(task_id = task_id.0, "gh pr close failed: {message}");
                    }
                }
                Err(e) => tracing::warn!(task_id = task_id.0, "gh pr close: worker died: {e}"),
            }
        };
        #[cfg(test)]
        {
            let handle = tokio::spawn(close);
            // test-only bookkeeping — panics on poisoned mutex (programming error)
            self.background_pr_closes.lock().unwrap().push(handle);
        }
        #[cfg(not(test))]
        {
            tokio::spawn(close);
        }
    }

    /// Wait for every `ClosePrOnDone` close spawned so far
    /// (`spawn_close_attached_pr`) to actually finish, so a test can assert on
    /// `MockProcessRunner::recorded_calls()` deterministically instead of
    /// sleeping. Test-only: production never calls this, since the whole
    /// point of `spawn_close_attached_pr` is that nothing waits on it.
    #[cfg(test)]
    #[allow(clippy::unwrap_used)] // test helper — panics on poisoned mutex (programming error)
    pub(crate) async fn wait_for_background_pr_closes(&self) {
        let handles: Vec<_> = self
            .background_pr_closes
            .lock()
            .unwrap()
            .drain(..)
            .collect();
        for handle in handles {
            let _ = handle.await;
        }
    }

    /// Move a task to a different epic, or detach it to standalone when
    /// `new_epic` is `None`. Validates that a chosen target epic exists, then
    /// hands the move to the store's `set_task_epic`, which routes a task moved
    /// onto a `group_by_repo` non-feed root into its per-repo sub-epic and
    /// recalculates both the previous epic and the new one.
    pub async fn move_task_to_epic(
        &self,
        task_id: TaskId,
        new_epic: Option<EpicId>,
    ) -> Result<(), ServiceError> {
        // A chosen target must exist; a null target detaches the task.
        // Validate against the ORIGINAL requested epic (the store may route
        // into a sub-epic, but the caller's intent is this epic).
        // Detach (`None`) is never checked: there is no target to exist.
        if let Some(epic_id) = new_epic {
            crate::service::require_epic_accepting_work(&*self.db, epic_id).await?;
        }

        if !self.db.task_exists(task_id).await {
            return Err(Self::task_not_found(task_id));
        }

        // The store routes a task dropped onto a group_by_repo (non-feed) root
        // into its per-repo sub-epic, and recalculates both epics it touched.
        self.db.set_task_epic_id(task_id, new_epic).await?;
        Ok(())
    }

    /// Expand `~` in a repo path and canonicalise a plan path, the
    /// operator-supplied-path pre-processing every task creation does.
    fn normalize_repo_and_plan(
        repo_path: &str,
        plan_path: Option<&str>,
    ) -> (String, Option<String>) {
        let repo_path = crate::models::expand_tilde(repo_path);
        let plan = plan_path.map(|p| {
            std::fs::canonicalize(p)
                .map(|abs| abs.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.to_string())
        });
        (repo_path, plan)
    }

    /// Attach a plan file (by absolute path) to a task. Used by the `plan`
    /// CLI subcommand so plan attachment routes through the service like its
    /// siblings, rather than writing to the DB directly. Plan attachment
    /// carries no epic-status invariant, so no recalculation is needed.
    pub async fn attach_plan(&self, task_id: TaskId, plan_path: &str) -> Result<(), ServiceError> {
        // Verify the task exists so a bad id surfaces as NotFound rather than
        // a silent no-op.
        self.get_task(task_id).await?;
        self.db
            .patch_task(task_id, &TaskPatch::new().plan_path(Some(plan_path)))
            .await
            .map_err(ServiceError::from)
    }

    pub async fn create_task(&self, params: CreateTaskParams) -> Result<TaskId, ServiceError> {
        Ok(self.create_task_returning(params).await?.id)
    }

    /// [`Self::create_task`], and mark the task unseen for every follower of its
    /// epic and ancestors (`mcp-task-tools.allium: CreateTaskViaMcp`'s
    /// `signal`).
    pub async fn create_task_signalled(
        &self,
        params: CreateTaskParams,
    ) -> Result<TaskId, ServiceError> {
        Ok(self.create_task_inner(params, true).await?.id)
    }

    /// Create a task and return the full Task object (used by TUI).
    pub async fn create_task_returning(
        &self,
        params: CreateTaskParams,
    ) -> Result<Task, ServiceError> {
        self.create_task_inner(params, false).await
    }

    async fn create_task_inner(
        &self,
        params: CreateTaskParams,
        signal: bool,
    ) -> Result<Task, ServiceError> {
        let (repo_path, plan) =
            Self::normalize_repo_and_plan(&params.repo_path, params.plan_path.as_deref());

        // DefaultBaseBranchIsDetectedNotAssumed (docs/specs/dispatch.allium):
        // an omitted base branch is resolved from the repository, never taken
        // as the literal DEFAULT_BASE_BRANCH. That literal is
        // `detect_default_branch`'s own last resort, for a repo that names no
        // default at all — reaching for it directly is what produced tasks in
        // "master" repos claiming "main", which could not be dispatched at all
        // because no such ref existed to cut a worktree from.
        //
        // Resolved here rather than in each caller so the one chokepoint holds
        // every creation path, for the same reason the epic-existence guard
        // below lives here: the TUI's own create path is held to it too. It
        // passes an explicit branch from its picker, so this is a no-op there.
        let base_branch = match params.base_branch.as_deref() {
            // A named branch is honoured verbatim, including one the repo does
            // not have yet — naming a branch that does not exist is legitimate,
            // and provisioning is where an unusable one gets caught
            // (dispatch.allium: UnresolvableBaseIsRefusedByName).
            Some(named) => named.to_string(),
            // Nothing to ask when the task is not pointed at a repo.
            None if repo_path.is_empty() => DEFAULT_BASE_BRANCH.to_string(),
            None => {
                crate::git::detect_default_branch_async(repo_path.clone(), Arc::clone(&self.runner))
                    .await
            }
        };

        // The named epic must exist, checked before the insert so a refused
        // target leaves no task behind rather than a loose one the caller
        // never asked for. Checked on the epic the caller NAMED, not on
        // whatever sub-epic the store's `create_task` routes it into — routing
        // only ever substitutes a `RepoGroup` sub-epic, which cannot outlive
        // its parent, so there is no existence gap to close there.
        //
        // Same `requires` as CreateTaskViaMcp: `if epic_id != null:
        // core/Epic.exists(epic_id)`. Left to the store's own insert it
        // would surface as ServiceError::Internal with a bare "Failed to insert
        // task", which reads as a dispatch fault rather than the bad argument
        // it is.
        if let Some(epic_id) = params.epic_id {
            crate::service::require_epic_accepting_work(&*self.db, epic_id).await?;
        }

        // Repo-grouping: the store places a task assigned to a group_by_repo
        // (non-feed) epic into its per-repo sub-epic, in the same transaction.
        let request = CreateTaskRequest {
            title: &params.title,
            description: &params.description,
            repo_path: &repo_path,
            plan: plan.as_deref(),
            status: TaskStatus::Backlog,
            base_branch: &base_branch,
            epic_id: params.epic_id,
            sort_order: params.sort_order,
            tag: params.tag,
            wrap_up_mode: params.wrap_up_mode,
            model: params.model,
            auto_run_plan: params.auto_run_plan,
            phoenix: params.phoenix,
            schedule: params.schedule.as_deref(),
        };
        let task_id = if signal {
            self.db.create_task_signalled(request).await?
        } else {
            self.db.create_task(request).await?
        };

        self.get_task(task_id).await
    }

    /// The delete pre-check (`DeleteTask`, tasks.allium): the store's TRUE row
    /// must be done. Run before any teardown.
    pub async fn ensure_deletable(&self, task_id: TaskId) -> Result<(), ServiceError> {
        let task = self.get_task(task_id).await?;
        if task.status != TaskStatus::Done {
            return Err(ServiceError::Validation(format!(
                "cannot delete task {}: it is {}, not done",
                task_id.0,
                task.status.as_str()
            )));
        }
        Ok(())
    }

    pub async fn delete_task(&self, task_id: TaskId) -> Result<(), ServiceError> {
        self.get_task(task_id).await?;
        self.db
            .delete_task(task_id)
            .await
            .map_err(ServiceError::from)
    }

    /// `tasks.allium: BatchDelete`'s service-layer entry point: performs every
    /// task and epic delete together in ONE atomic call. No status check here:
    /// the board's own `all_pass` check (src/tui/update/selection.rs) is the
    /// responsive, explain-why pass, the runtime's `pass_delete_guard` re-reads
    /// the store before any teardown or purge, and the reducer is the final
    /// authority.
    pub async fn batch_delete(
        &self,
        task_ids: &[TaskId],
        epic_ids: &[EpicId],
    ) -> Result<(), ServiceError> {
        self.db
            .batch_delete(task_ids, epic_ids)
            .await
            .map_err(ServiceError::from)
    }

    pub async fn get_task(&self, task_id: TaskId) -> Result<Task, ServiceError> {
        self.db
            .get_task(task_id)
            .await
            .ok_or_else(|| ServiceError::NotFound(format!("Task {} not found", task_id.0)))
    }

    /// Batch-update `sub_status` for many tasks in a single transaction.
    ///
    /// `sub_status` is derived board state (activity classification), not a
    /// status/epic-linkage change, so it carries no epic recalculation
    /// obligation — but it still routes through the service so consumers
    /// (the tick) never touch the DB write surface directly.
    pub async fn batch_patch_sub_status(
        &self,
        updates: &[(TaskId, crate::models::SubStatus)],
    ) -> Result<(), ServiceError> {
        self.db
            .batch_patch_sub_status(updates)
            .await
            .map_err(ServiceError::from)
    }

    pub async fn list_tasks(&self, filter: ListTasksFilter) -> Result<Vec<Task>, ServiceError> {
        let tasks = self.db.list_all().await;

        let epic_scope: Option<std::collections::HashSet<EpicId>> = match filter.epic_id {
            Some(eid) if filter.recursive => Some(crate::models::descendant_epic_ids(
                eid,
                &self.db.list_epics().await,
            )),
            Some(eid) => Some(std::iter::once(eid).collect()),
            None => None,
        };

        let filtered: Vec<_> = tasks
            .into_iter()
            .filter(|t| match &filter.statuses {
                Some(statuses) => statuses.contains(&t.status),
                None => true,
            })
            .filter(|t| match &epic_scope {
                Some(scope) => t.epic_id.is_some_and(|eid| scope.contains(&eid)),
                None => true,
            })
            .filter(|t| match &filter.repo_paths {
                Some(paths) => paths.iter().any(|p| p == &t.repo_path),
                None => true,
            })
            .filter(|t| match &filter.base_branch {
                Some(branch) => &t.base_branch == branch,
                None => true,
            })
            .filter(|t| match filter.exclude_task_id {
                Some(excluded) => t.id != excluded,
                None => true,
            })
            .filter(|t| match &filter.query {
                Some(q) => crate::models::text_query_matches(q, &t.title, &t.description),
                None => true,
            })
            .collect();

        Ok(filtered)
    }

    /// Gate every wrap-up path.
    ///
    /// The decision itself is [`Task::wrap_up_block`]; what is here is the
    /// wording, because each block has its own remedy and only the caller-facing
    /// layer should be choosing sentences. The review-tag refusal names the
    /// retag rather than stopping at "no": an agent that took a review task over
    /// and did real work on it needs somewhere to put that work, and without the
    /// escape hatch it would leave it uncommitted. See
    /// `ReviewTasksAreNotWrappedUp` in `docs/specs/mcp-task-tools.allium`.
    pub async fn validate_wrap_up(&self, task_id: TaskId) -> Result<Task, ServiceError> {
        let task = self.get_task(task_id).await?;

        match task.wrap_up_block() {
            None => Ok(task),
            Some(block) => Err(ServiceError::Validation(wrap_up_block_message(
                task_id, block,
            ))),
        }
    }

    /// Record a Claude Code hook event for a task.
    ///
    /// `Stop` transitions Running → Review and clears both timestamps.
    /// `PreToolUse`/`Notification` stamp their timestamp and reclassify
    /// `sub_status`; both are no-ops on non-Running tasks. `UserPromptSubmit`
    /// additionally resumes a Review task straight to Running — it is the
    /// earliest signal that the human has continued the conversation — and
    /// voids the deferred `Stop` that prompt supersedes; it is a no-op outside
    /// {Running, Review}.
    pub async fn record_hook_event(
        &self,
        id: TaskId,
        kind: HookEventKind,
    ) -> Result<(), ServiceError> {
        let now = self.clock.now();
        // Exhaustive on purpose: every HookEventKind is handled here, so a new
        // variant is a compile error rather than a silent no-op.
        //
        // Three of the four arms take no snapshot of the row at all. Each
        // decides its branch against the row's committed state, inside the
        // statement that applies it — another hook process (every Claude Code
        // hook is one) can invalidate a snapshot between the read and the
        // write. For Stop that is the flip-or-defer choice; for
        // UserPromptSubmit, whether the deferred Stop it would void is one this
        // prompt actually supersedes; for Notification, whether an idle_prompt
        // is a real block or the agent waiting on its own background work. See
        // HookStop / HookUserPromptSubmit / HookNotification in
        // agent-health.allium.
        match kind {
            HookEventKind::Stop => {
                self.db.try_record_stop(id, now).await?;
            }
            HookEventKind::UserPromptSubmit => {
                self.db.record_user_prompt_submit(id, now).await?;
            }
            HookEventKind::Notification(notification_kind) => {
                self.db
                    .record_notification(id, NotificationWrite::from_kind(notification_kind), now)
                    .await?;
            }
            HookEventKind::PreToolUse => {
                // The one arm that does need the row, and the only one that
                // pays for the read. Classifying from a snapshot is sound here
                // where it is not above: this sub_status is re-derived from
                // fresh state by ClassifyAgentActivity on the very next tick,
                // so a stale read self-corrects within a tick. The
                // `status = running` guard, which cannot self-correct, rides in
                // the write.
                let task =
                    self.db.get_task(id).await.ok_or_else(|| {
                        ServiceError::NotFound(format!("Task {} not found", id.0))
                    })?;
                let activity = classify_agent_activity(
                    Some(now),
                    task.last_notification_at,
                    task.live_subagents,
                    now,
                );
                self.db
                    .record_pre_tool_use(id, activity.to_sub_status(), now)
                    .await?;
            }
        }
        // Only Stop and UserPromptSubmit move task.status, and the module
        // recalculates their epic in the same transaction (store-split.allium
        // X3).
        Ok(())
    }

    /// Record an observed native Claude Code `SendMessage` call from
    /// `sender_id`'s agent, addressed to session `target_name` (task #4098).
    /// Called from the `dispatch hook <id> peer-message` CLI subcommand, which
    /// the Claude Code hook pipeline invokes on `PostToolUse` for the
    /// `SendMessage` tool — dispatch never performs the delivery itself, only
    /// observes it to drive the TUI flash and reconstruct an audit trail.
    ///
    /// Stamps `last_peer_message_sent_at` on the sender's own row
    /// unconditionally. `target_name` is resolved against the agent names first (a
    /// session is named after its agent, `AgentSessionIsNamedAfterTheAgent`),
    /// then against the `task-<id>` name a session started before agents still
    /// carries; when it resolves to a task that still
    /// exists, that task's `last_peer_message_received_at` is stamped too. A
    /// name that doesn't parse, or a task id that no longer exists, is not an
    /// error — the sender addressed a session outside dispatch's own fleet
    /// (or one that's since been deleted), which is not dispatch's concern.
    /// Only a missing *sender* is an error, matching `record_hook_event`'s
    /// contract.
    pub async fn record_peer_message_sent(
        &self,
        sender_id: TaskId,
        target_name: &str,
    ) -> Result<(), ServiceError> {
        if !self.db.task_exists(sender_id).await {
            return Err(Self::task_not_found(sender_id));
        }

        let now = self.clock.now();
        self.db
            .patch_task(
                sender_id,
                &TaskPatch::new().last_peer_message_sent_at(Some(now)),
            )
            .await?;

        let by_agent_name = {
            let name = peer_message_target_name(target_name);
            self.db
                .list_agents()
                .await
                .into_iter()
                .find(|a| a.name == name)
                .map(|a| a.task_id)
        };
        if let Some(target_id) =
            by_agent_name.or_else(|| parse_peer_message_target_name(target_name))
        {
            // No existence pre-check: patch_task itself is the only
            // authoritative answer to "does this row still exist", and
            // checking first would leave a race (the target could be
            // deleted between the check and the write) that this direct
            // attempt does not have. A failure here — an unresolvable
            // target is already handled above; this is specifically "it
            // resolved but the row is gone by the time we got here" — is
            // not this call's concern either: the sender's own stamp above
            // already landed, and a missed *target* stamp is a missed TUI
            // flash, not a lost message (the message itself travels over
            // Claude Code's own native delivery, not through this call).
            if let Err(e) = self
                .db
                .patch_task(
                    target_id,
                    &TaskPatch::new().last_peer_message_received_at(Some(now)),
                )
                .await
            {
                tracing::debug!(
                    target_id = target_id.0,
                    "peer-message target vanished before its stamp landed: {e}"
                );
            }
        }

        Ok(())
    }

    /// Record a subagent lifecycle event and, when it drains the last
    /// subagent for a task carrying a deferred Stop, apply that Stop.
    ///
    /// A `Start` is handled on its own and returns before the drain path
    /// exists: it inserts its row *before* the recount, so the resulting count
    /// can never be zero and it can never drain. Keeping that as control flow
    /// rather than a condition means the drain predicate does not have to encode
    /// the invariant — nor silently depend on it.
    ///
    /// See `HookSubagentStart` / `HookSubagentStop` in
    /// `docs/specs/agent-health.allium`.
    pub async fn record_subagent_event(
        &self,
        id: TaskId,
        event: SubagentEvent,
    ) -> Result<(), ServiceError> {
        if !self.db.task_exists(id).await {
            return Err(Self::task_not_found(id));
        }

        // Whether a deferred Stop applied is decided inside the same
        // transaction that recomputes the count, so there is nothing to check
        // out here — and no window in which the count has reached zero but the
        // flip has not landed. The flip recalculates its epic there too
        // (store-split.allium X3).
        match event {
            SubagentEvent::Start {
                agent_id,
                session_id,
            } => {
                // A Start can never drain: it only ever raises the count.
                self.db
                    .subagent_start(id, &agent_id, &session_id, self.clock.now())
                    .await?;
            }
            SubagentEvent::Stop {
                agent_id,
                session_id,
            } => {
                self.db.subagent_stop(id, &agent_id, &session_id).await?;
            }
            SubagentEvent::Clear => {
                self.db.subagent_clear(id).await?;
            }
        }
        Ok(())
    }

    /// The service's one spelling of "that task id doesn't exist". Callers that
    /// need the row use [`get_task`](Self::get_task), which produces the same
    /// error; this is for the existence-only checks, which have no row to fail on.
    fn task_not_found(id: TaskId) -> ServiceError {
        ServiceError::NotFound(format!("Task {} not found", id.0))
    }

    /// Clear a task's subagent entries and void any pending Stop **without**
    /// the drain path.
    ///
    /// Three callers: crash and dispatch-claim already own the resulting status
    /// (running the drain path there would leave a task Crashed-and-in-Review
    /// or freshly-dispatched-and-in-Review), and `SessionStart` — where a Stop
    /// deferred by the *previous* turn is stale by definition and must be
    /// voided, not applied. See `ClearSubagentsOnSessionStart` in
    /// `docs/specs/agent-health.allium`.
    ///
    /// `NotFound` for an unknown task, matching `record_subagent_event`: the
    /// hook CLI turns that into a silent skip rather than a failed tool call.
    /// Existence is all this needs, so it does not materialise the row.
    ///
    /// Entries and `stop_pending` go in one writer round trip. Not just for the
    /// saved trip: a partial clear-then-patch would briefly leave
    /// `live_subagents = 0` + `stop_pending` + Running, so a `SubagentStart`
    /// landing in that window would be counted against a task whose bit is
    /// about to be wiped. The single transaction rules that out.
    pub async fn clear_subagents_no_drain(&self, id: TaskId) -> Result<(), ServiceError> {
        if !self.db.task_exists(id).await {
            return Err(Self::task_not_found(id));
        }
        self.db.subagent_clear_and_void_pending_stop(id).await?;
        Ok(())
    }

    /// Mark that the PR-learnings reminder has been shown for this task.
    ///
    /// Returns `true` if this call set the flag (first `gh pr create` →
    /// caller should block), `false` if it was already set or the task does
    /// not exist (caller should allow the PR). One-time reminder; no epic
    /// recalculation is involved.
    pub async fn mark_pr_learnings_gate_shown(&self, id: TaskId) -> Result<bool, ServiceError> {
        Ok(self.db.mark_pr_learnings_gate_shown(id).await?)
    }
}

/// The caller-facing sentence for each [`WrapUpBlock`].
///
/// Free-standing so `get_task` can render the same wording it would hit at
/// `wrap_up` — the block is reported where the `/wrap-up` skill first reads the
/// task, rather than only at the last call of its sequence, so an agent does
/// not run a retro and a commit before learning none of it was wanted.
pub fn wrap_up_block_message(task_id: TaskId, block: WrapUpBlock) -> String {
    match block {
        WrapUpBlock::NotDispatched => format!(
            "Task {} cannot be wrapped up. Requires Running or Review status with a worktree.",
            task_id.0
        ),
        WrapUpBlock::ReviewTag(tag) => format!(
            "Task {} is tagged `{}`, so it is not wrapped up. A review task ends when its PR \
             merges — the feed that created it deletes the card on the next poll — or when you \
             hand it back to the user. If it became real code work, retag it \
             (update_tasks(task_id={}, tag=\"chore\")) and call wrap_up again.",
            task_id.0,
            tag.as_str(),
            task_id.0,
        ),
    }
}
