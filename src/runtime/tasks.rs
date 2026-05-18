use super::*;
use crate::models::ProjectId;

impl TuiRuntime {
    pub(super) async fn exec_insert_task(
        &self,
        app: &mut App,
        draft: tui::TaskDraft,
        epic_id: Option<models::EpicId>,
    ) {
        use crate::service::CreateTaskParams;
        let params = CreateTaskParams {
            title: draft.title,
            description: draft.description,
            repo_path: draft.repo_path,
            plan_path: None,
            epic_id,
            sort_order: None,
            tag: draft.tag,
            base_branch: Some(draft.base_branch),
            project_id: app.active_project(),
            wrap_up_mode: draft.wrap_up_mode,
        };
        if let Some(task) = self.create_task(app, params).await {
            app.update(Message::Task(crate::tui::messages::TaskMessage::Created {
                task,
            }));
        }
    }

    pub(super) async fn exec_quick_dispatch(
        &self,
        app: &mut App,
        draft: tui::TaskDraft,
        epic_id: Option<models::EpicId>,
    ) {
        use crate::service::CreateTaskParams;
        let repo_path = draft.repo_path.clone();
        let expanded = models::expand_tilde(&repo_path);
        // detect_default_branch falls back to "main" when origin/HEAD is
        // unavailable, so dispatch doesn't fail on repos whose default isn't main.
        let base_branch = crate::git::detect_default_branch(&expanded, &*self.runner);
        let Some(task) = self
            .create_task(
                app,
                CreateTaskParams {
                    title: draft.title,
                    description: draft.description,
                    repo_path: draft.repo_path,
                    plan_path: None,
                    epic_id,
                    sort_order: None,
                    tag: None,
                    base_branch: Some(base_branch),
                    project_id: app.active_project(),
                    wrap_up_mode: None,
                },
            )
            .await
        else {
            return;
        };
        app.update(Message::Task(crate::tui::messages::TaskMessage::Created {
            task: task.clone(),
        }));
        app.update(Message::Task(
            crate::tui::messages::TaskMessage::MarkDispatching(task.id),
        ));
        let _ = self.database.save_repo_path(&expanded).await;
        let paths = self.database.list_repo_paths().await.unwrap_or_default();
        app.update(Message::RepoPathsUpdated(paths));
        let epic_ctx = dispatch::EpicContext::from_db(&task, &*self.database).await;
        let project_ctx = dispatch::ProjectContext::from_db(&task, &*self.database).await;
        let (procedural, tiered) = dispatch::build_and_record_injections(
            &*self.database,
            &task,
            &crate::service::embeddings::EmbeddingService::new_noop(),
        )
        .await;
        let verify_command = dispatch::fetch_verify_command(&*self.database, &task.repo_path).await;
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            let id = task.id;
            let injections = dispatch::LearningInjections {
                procedural: procedural.iter().collect(),
                tiered: tiered.iter().collect(),
            };
            match dispatch::quick_dispatch_agent(
                &task,
                &*runner,
                epic_ctx.as_ref(),
                Some(&project_ctx),
                &injections,
                verify_command.as_deref(),
            ) {
                Ok(result) => {
                    let _ = tx.send(Message::Task(
                        crate::tui::messages::TaskMessage::Dispatched {
                            id,
                            worktree: result.worktree_path,
                            tmux_window: result.tmux_window,
                            switch_focus: true,
                        },
                    ));
                }
                Err(e) => {
                    let _ = tx.send(Message::Task(
                        crate::tui::messages::TaskMessage::DispatchFailed(id),
                    ));
                    let _ = tx.send(Message::System(crate::tui::messages::SystemMessage::Error(
                        format!("Quick dispatch failed: {e:#}"),
                    )));
                }
            }
        });
    }

    pub(super) async fn exec_persist_task(&self, app: &mut App, task: models::Task) {
        use crate::service::UpdateTaskParams;
        // `last_pre_tool_use_at` is intentionally omitted: hooks own that
        // column. Writing it here would let a stale in-memory snapshot
        // (e.g. from a tick reclassification or sort_order swap) overwrite
        // a fresher hook write, flipping the task to Stale on the next tick.
        // Backlog→Running seeds go through `SeedActivity` instead.
        let mut p = UpdateTaskParams::for_task(task.id)
            .status(task.status)
            .sub_status(task.sub_status)
            .pr_url(option_to_field_update(task.pr_url.clone()))
            .worktree(option_to_field_update(task.worktree.clone()))
            .tmux_window(option_to_field_update(task.tmux_window.clone()));
        if let Some(so) = task.sort_order {
            p = p.sort_order(so);
        }
        if let Err(e) = self.task_svc.update_task(p).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("persisting task", e),
            )));
        }
    }

    /// Write `last_pre_tool_use_at` for a freshly running task. Used after
    /// Backlog→Running transitions so the tick classifier sees a recent
    /// activity stamp through the ACTIVE_THRESHOLD window before the agent's
    /// first PreToolUse hook fires.
    pub(super) async fn exec_seed_activity(
        &self,
        app: &mut App,
        id: models::TaskId,
        at: chrono::DateTime<chrono::Utc>,
    ) {
        use crate::service::UpdateTaskParams;
        if let Err(e) = self
            .task_svc
            .update_task(UpdateTaskParams::for_task(id).last_pre_tool_use_at(Some(at)))
            .await
        {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("seeding activity timestamp", e),
            )));
        }
    }

    pub(super) async fn exec_patch_sub_status(
        &self,
        app: &mut App,
        id: models::TaskId,
        sub_status: models::SubStatus,
    ) {
        use crate::service::UpdateTaskParams;
        if let Err(e) = self
            .task_svc
            .update_task(UpdateTaskParams::for_task(id).sub_status(sub_status))
            .await
        {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("patching sub_status", e),
            )));
        }
    }

    pub(super) async fn exec_delete_task(&self, app: &mut App, id: TaskId) {
        if let Err(e) = self.task_svc.delete_task(id).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("deleting task", e),
            )));
        }
    }

    pub(super) async fn exec_dispatch_agent(&self, task: models::Task, mode: models::DispatchMode) {
        let epic_ctx = dispatch::EpicContext::from_db(&task, &*self.database).await;
        let project_ctx = dispatch::ProjectContext::from_db(&task, &*self.database).await;
        let (procedural, tiered) = dispatch::build_and_record_injections(
            &*self.database,
            &task,
            &crate::service::embeddings::EmbeddingService::new_noop(),
        )
        .await;
        let verify_command = dispatch::fetch_verify_command(&*self.database, &task.repo_path).await;
        let label = mode.label();
        self.spawn_dispatch(
            task,
            move |t, r| {
                let injections = dispatch::LearningInjections {
                    procedural: procedural.iter().collect(),
                    tiered: tiered.iter().collect(),
                };
                match mode {
                    models::DispatchMode::Dispatch => dispatch::dispatch_agent(
                        t,
                        r,
                        epic_ctx.as_ref(),
                        Some(&project_ctx),
                        &injections,
                        verify_command.as_deref(),
                    ),
                    models::DispatchMode::Research => dispatch::research_agent(
                        t,
                        r,
                        epic_ctx.as_ref(),
                        Some(&project_ctx),
                        verify_command.as_deref(),
                    ),
                }
            },
            label,
        );
    }

    pub(super) fn exec_capture_tmux(&self, id: TaskId, window: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            if let Ok(false) = tmux::has_window(&window, &*runner) {
                let _ = tx.send(Message::Task(
                    crate::tui::messages::TaskMessage::WindowGone(id),
                ));
                return;
            }

            // Activity timestamp for staleness detection (fall back to 0 on error
            // so we never falsely mark an agent as stale).
            let activity_ts = tmux::window_activity(&window, &*runner).unwrap_or(0);

            match tmux::capture_pane(&window, 5, &*runner) {
                Ok(output) => {
                    let _ = tx.send(Message::Task(
                        crate::tui::messages::TaskMessage::TmuxOutput {
                            id,
                            output,
                            activity_ts,
                        },
                    ));
                }
                Err(e) => {
                    let _ = tx.send(Message::System(crate::tui::messages::SystemMessage::Error(
                        format!("tmux capture failed for window {window}: {e}"),
                    )));
                }
            }
        });
    }

    pub(super) async fn exec_save_repo_path(&self, app: &mut App, path: String) {
        let path = models::expand_tilde(&path);
        if let Err(e) = self.database.save_repo_path(&path).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("saving repo path", e),
            )));
        }
        match self.database.list_repo_paths().await {
            Ok(paths) => {
                app.update(Message::RepoPathsUpdated(paths));
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("listing repo paths", e),
                )));
            }
        }
    }

    /// Reload a single task from the DB and splice it into the app state.
    /// Falls back to a full refresh if the task is gone (e.g. deleted while
    /// the event was in flight); returns silently on DB errors so the runtime
    /// keeps draining notifications.
    pub(super) async fn exec_refresh_task(&self, app: &mut App, task_id: TaskId) -> Vec<Command> {
        match self.database.get_task(task_id).await {
            Ok(Some(task)) => app.update(Message::Task(
                crate::tui::messages::TaskMessage::Updated(task),
            )),
            Ok(None) => self.exec_refresh_from_db(app).await,
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("refreshing task", e),
                )));
                vec![]
            }
        }
    }

    /// Reload a single epic plus its tasks (feed-sync changes appear here as
    /// a batch update) and splice both into the app state. Falls back to a
    /// full refresh if the epic is gone.
    pub(super) async fn exec_refresh_epic(
        &self,
        app: &mut App,
        epic_id: models::EpicId,
    ) -> Vec<Command> {
        let epic_result = self.database.get_epic(epic_id).await;
        let epic = match epic_result {
            Ok(Some(e)) => e,
            Ok(None) => return self.exec_refresh_from_db(app).await,
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("refreshing epic", e),
                )));
                return vec![];
            }
        };
        let mut cmds = app.update(Message::Epic(crate::tui::messages::EpicMessage::Updated(
            epic,
        )));
        // Feed-sync upserts whole batches under the epic — reload the
        // epic's tasks so card lists reflect the new rows in one shot.
        match self.database.list_tasks_for_epic(epic_id).await {
            Ok(tasks) => {
                for task in tasks {
                    cmds.extend(app.update(Message::Task(
                        crate::tui::messages::TaskMessage::Updated(task),
                    )));
                }
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("listing epic tasks", e),
                )));
            }
        }
        cmds
    }

    pub(super) async fn exec_refresh_from_db(&self, app: &mut App) -> Vec<Command> {
        let mut cmds = Vec::new();
        match self.database.list_all().await {
            Ok(tasks) => {
                cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
                    tasks,
                )));
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("refreshing tasks", e),
                )));
            }
        }
        self.exec_refresh_epics_from_db(app).await;
        self.exec_refresh_usage_from_db(app).await;
        self.exec_refresh_needs_review_count(app).await;
        cmds
    }

    pub(super) async fn exec_delete_repo_path(&self, app: &mut App, path: &str) {
        if let Err(e) = self.database.delete_repo_path(path).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("deleting repo path", e),
            )));
            return;
        }
        match self.database.list_repo_paths().await {
            Ok(paths) => {
                app.update(Message::RepoPathsUpdated(paths));
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("listing repo paths", e),
                )));
            }
        }
        // Refresh presets since delete_repo_path cleans them
        if let Ok(raw) = self.database.list_filter_presets().await {
            let known: HashSet<String> = app.repo_paths().iter().cloned().collect();
            let presets = parse_raw_presets(raw, Some(&known));
            app.update(Message::FilterPresetsLoaded(presets));
        }
    }

    pub(super) async fn exec_cleanup(
        &self,
        id: TaskId,
        repo_path: String,
        worktree: String,
        tmux_window: Option<String>,
    ) {
        let shared = self
            .database
            .has_other_tasks_with_worktree(&worktree, id)
            .await
            .unwrap_or(false);

        if shared {
            // Other active tasks share this worktree — just detach this task
            tracing::info!(task_id = id.0, "worktree shared, detaching only");
            if let Err(e) = self
                .task_svc
                .update_task(
                    crate::service::UpdateTaskParams::for_task(id)
                        .worktree(FieldUpdate::Clear)
                        .tmux_window(FieldUpdate::Clear),
                )
                .await
            {
                let _ =
                    self.msg_tx
                        .send(Message::System(crate::tui::messages::SystemMessage::Error(
                            format!("Detach failed: {e:#}"),
                        )));
            }
            return;
        }

        // No other active tasks — full cleanup
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            if let Err(e) =
                dispatch::cleanup_task(&repo_path, &worktree, tmux_window.as_deref(), &*runner)
            {
                let _ = tx.send(Message::System(crate::tui::messages::SystemMessage::Error(
                    format!("Cleanup failed: {e:#}"),
                )));
            }
        });
    }

    pub(super) async fn exec_finish(
        &self,
        id: TaskId,
        repo_path: String,
        branch: String,
        base_branch: String,
        worktree: String,
        tmux_window: Option<String>,
    ) {
        let shared = self
            .database
            .has_other_tasks_with_worktree(&worktree, id)
            .await
            .unwrap_or(false);

        if shared {
            tracing::info!(
                task_id = id.0,
                "worktree shared, detaching only (no rebase)"
            );
            if let Err(e) = self
                .task_svc
                .update_task(
                    crate::service::UpdateTaskParams::for_task(id)
                        .worktree(FieldUpdate::Clear)
                        .tmux_window(FieldUpdate::Clear),
                )
                .await
            {
                let _ =
                    self.msg_tx
                        .send(Message::System(crate::tui::messages::SystemMessage::Error(
                            format!("Detach failed: {e:#}"),
                        )));
            }
            let _ = self.msg_tx.send(Message::Task(
                crate::tui::messages::TaskMessage::FinishComplete(id),
            ));
            return;
        }

        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            match dispatch::finish_task(
                &repo_path,
                &worktree,
                &branch,
                &base_branch,
                tmux_window.as_deref(),
                &*runner,
            ) {
                Ok(()) => {
                    let _ = tx.send(Message::Task(
                        crate::tui::messages::TaskMessage::FinishComplete(id),
                    ));
                }
                Err(e) => {
                    let is_conflict = matches!(e, dispatch::FinishError::RebaseConflict(_));
                    let _ = tx.send(Message::Task(
                        crate::tui::messages::TaskMessage::FinishFailed {
                            id,
                            error: e.to_string(),
                            is_conflict,
                        },
                    ));
                }
            }
        });
    }

    pub(super) async fn exec_refresh_projects_from_db(&self, app: &mut App) {
        match self.database.list_projects().await {
            Ok(projects) => {
                app.update(Message::ProjectsUpdated(projects));
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("refreshing projects", e),
                )));
            }
        }
    }

    pub(super) async fn exec_create_project(&self, app: &mut App, name: String) {
        let max_order = app
            .projects()
            .iter()
            .map(|p| p.sort_order)
            .max()
            .unwrap_or(0);
        match self.database.create_project(&name, max_order + 1).await {
            Ok(project) => {
                app.update(Message::System(
                    crate::tui::messages::SystemMessage::StatusInfo(format!(
                        "Created project \"{}\"",
                        project.name
                    )),
                ));
                self.exec_refresh_projects_from_db(app).await;
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("creating project", e),
                )));
            }
        }
    }

    pub(super) async fn exec_rename_project(&self, app: &mut App, id: ProjectId, name: String) {
        match self.database.rename_project(id, &name).await {
            Ok(()) => self.exec_refresh_projects_from_db(app).await,
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("renaming project", e),
                )));
            }
        }
    }

    pub(super) async fn exec_delete_project(&self, app: &mut App, id: ProjectId) {
        let Some(default_id) = app.projects().iter().find(|p| p.is_default).map(|p| p.id) else {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                "No default project found".to_string(),
            )));
            return;
        };
        if let Err(e) = self
            .database
            .delete_project_and_move_items(id, default_id)
            .await
        {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("deleting project", e),
            )));
            return;
        }
        if app.active_project() == id {
            app.update(Message::SelectProject(default_id));
        }
        self.exec_refresh_projects_from_db(app).await;
    }

    pub(super) async fn exec_reorder_project(&self, app: &mut App, id: ProjectId, delta: i8) {
        let projects = app.projects().to_vec();
        let Some(idx) = projects.iter().position(|p| p.id == id) else {
            return;
        };
        let neighbor_idx = if delta > 0 {
            if idx + 1 >= projects.len() {
                return;
            }
            idx + 1
        } else {
            if idx == 0 {
                return;
            }
            idx - 1
        };
        let current_order = projects[idx].sort_order;
        let neighbor_order = projects[neighbor_idx].sort_order;
        // Swap sort_order values — let _ is intentional: partial failure is non-critical,
        // exec_refresh_projects_from_db below will reflect whatever state the DB is in.
        let _ = self.database.reorder_project(id, neighbor_order).await;
        let _ = self
            .database
            .reorder_project(projects[neighbor_idx].id, current_order)
            .await;
        self.exec_refresh_projects_from_db(app).await;
        app.update(Message::FollowProject(id));
    }

    pub(super) fn exec_resume(&self, task: models::Task) {
        let tx = self.msg_tx.clone();
        let id = task.id;
        let worktree_path = task.worktree.clone().unwrap_or_default();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            tracing::info!(task_id = id.0, "resuming task");
            match dispatch::resume_agent(id, &worktree_path, &*runner) {
                Ok(result) => {
                    let _ = tx.send(Message::Task(crate::tui::messages::TaskMessage::Resumed {
                        id,
                        tmux_window: result.tmux_window,
                    }));
                }
                Err(e) => {
                    let _ = tx.send(Message::System(crate::tui::messages::SystemMessage::Error(
                        format!("Resume failed: {e:#}"),
                    )));
                }
            }
        });
    }
}
