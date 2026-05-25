use super::*;

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
        let db = Arc::clone(&self.database);
        let emb_svc = Arc::clone(&self.emb_svc);
        let msg_tx = self.msg_tx.clone();
        let runner = Arc::clone(&self.runner);

        // Spawn a background task so the TUI command loop is never blocked
        // waiting for the embedding thread (which may be busy with index_repo).
        tokio::spawn(async move {
            let epic_ctx = dispatch::EpicContext::from_db(&task, &*db).await;
            let injected =
                dispatch::build_and_record_injections(&*db, &task, &emb_svc).await;
            let verify_command =
                dispatch::fetch_verify_command(&*db, &task.repo_path).await;
            tokio::task::spawn_blocking(move || {
                let id = task.id;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let injections = dispatch::LearningInjections::from(&*injected);
                    dispatch::quick_dispatch_agent(
                        &task,
                        &*runner,
                        epic_ctx.as_ref(),
                        &injections,
                        verify_command.as_deref(),
                    )
                }));
                match result {
                    Ok(Ok(r)) => {
                        let _ = msg_tx.send(Message::Task(
                            crate::tui::messages::TaskMessage::Dispatched {
                                id,
                                worktree: r.worktree_path,
                                tmux_window: r.tmux_window,
                                switch_focus: true,
                            },
                        ));
                    }
                    Ok(Err(e)) => {
                        let _ = msg_tx.send(Message::Task(
                            crate::tui::messages::TaskMessage::DispatchFailed(id),
                        ));
                        let _ = msg_tx.send(Message::System(
                            crate::tui::messages::SystemMessage::Error(
                                format!("Quick dispatch failed: {e:#}"),
                            ),
                        ));
                    }
                    Err(panic) => {
                        let detail = panic
                            .downcast_ref::<&'static str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown".to_string());
                        tracing::error!(task_id = id.0, "quick dispatch panicked: {detail}");
                        let _ = msg_tx.send(Message::Task(
                            crate::tui::messages::TaskMessage::DispatchFailed(id),
                        ));
                        let _ = msg_tx.send(Message::System(
                            crate::tui::messages::SystemMessage::Error(
                                format!("Quick dispatch panicked: {detail}"),
                            ),
                        ));
                    }
                }
            });
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

    pub(super) fn exec_dispatch_agent(&self, task: models::Task, mode: models::DispatchMode) {
        let db = Arc::clone(&self.database);
        let emb_svc = Arc::clone(&self.emb_svc);
        let msg_tx = self.msg_tx.clone();
        let runner = Arc::clone(&self.runner);

        // Spawn a background task so the TUI command loop is never blocked
        // waiting for the embedding thread (which may be busy with index_repo).
        tokio::spawn(async move {
            let epic_ctx = dispatch::EpicContext::from_db(&task, &*db).await;
            let injected =
                dispatch::build_and_record_injections(&*db, &task, &emb_svc).await;
            let verify_command =
                dispatch::fetch_verify_command(&*db, &task.repo_path).await;
            let label = mode.label();
            let id = task.id;
            tracing::info!(task_id = id.0, label, "dispatching");

            tokio::task::spawn_blocking(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let injections = dispatch::LearningInjections::from(&*injected);
                    match mode {
                        models::DispatchMode::Dispatch => dispatch::dispatch_agent(
                            &task,
                            &*runner,
                            epic_ctx.as_ref(),
                            &injections,
                            verify_command.as_deref(),
                        ),
                        models::DispatchMode::Research => dispatch::research_agent(
                            &task,
                            &*runner,
                            epic_ctx.as_ref(),
                            verify_command.as_deref(),
                        ),
                    }
                }));
                match result {
                    Ok(Ok(r)) => {
                        let _ = msg_tx.send(Message::Task(
                            crate::tui::messages::TaskMessage::Dispatched {
                                id,
                                worktree: r.worktree_path,
                                tmux_window: r.tmux_window,
                                switch_focus: false,
                            },
                        ));
                    }
                    Ok(Err(e)) => {
                        let _ = msg_tx.send(Message::Task(
                            crate::tui::messages::TaskMessage::DispatchFailed(id),
                        ));
                        let _ = msg_tx.send(Message::System(
                            crate::tui::messages::SystemMessage::Error(
                                format!("{label} failed: {e:#}"),
                            ),
                        ));
                    }
                    Err(panic) => {
                        let detail = panic
                            .downcast_ref::<&'static str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown".to_string());
                        tracing::error!(task_id = id.0, label, "dispatch panicked: {detail}");
                        let _ = msg_tx.send(Message::Task(
                            crate::tui::messages::TaskMessage::DispatchFailed(id),
                        ));
                        let _ = msg_tx.send(Message::System(
                            crate::tui::messages::SystemMessage::Error(
                                format!("{label} panicked: {detail}"),
                            ),
                        ));
                    }
                }
            });
        });
    }

    pub(super) fn exec_check_window(&self, id: TaskId, window: String) -> tokio::task::JoinHandle<()> {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            if let Ok(false) = tmux::has_window(&window, &*runner) {
                let _ = tx.send(Message::Task(
                    crate::tui::messages::TaskMessage::WindowGone(id),
                ));
            }
        })
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
            app.update(Message::RepoFilter(crate::tui::messages::RepoFilterMessage::PresetsLoaded(presets)));
        }
    }

    pub(super) async fn exec_cleanup(
        &self,
        id: TaskId,
        repo_path: String,
        worktree: String,
        tmux_window: Option<String>,
    ) {
        let shared = match self
            .database
            .has_other_tasks_with_worktree(&worktree, id)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                let _ =
                    self.msg_tx
                        .send(Message::System(crate::tui::messages::SystemMessage::Error(
                            format!("Cleanup check failed: {e:#}"),
                        )));
                return;
            }
        };

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
        let shared = match self
            .database
            .has_other_tasks_with_worktree(&worktree, id)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                let _ =
                    self.msg_tx
                        .send(Message::System(crate::tui::messages::SystemMessage::Error(
                            format!("Finish check failed: {e:#}"),
                        )));
                return;
            }
        };

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
