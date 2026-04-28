use super::*;
use crate::models::ProjectId;

impl TuiRuntime {
    pub(super) fn exec_insert_task(
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
            epic_id: epic_id.map(|e| e.0),
            sort_order: None,
            tag: draft.tag,
            base_branch: Some(draft.base_branch),
            project_id: app.active_project(),
        };
        if let Some(task) = self.create_task(app, params) {
            app.update(Message::TaskCreated { task });
        }
    }

    pub(super) fn exec_quick_dispatch(
        &self,
        app: &mut App,
        draft: tui::TaskDraft,
        epic_id: Option<models::EpicId>,
    ) {
        use crate::service::CreateTaskParams;
        let repo_path = draft.repo_path.clone();
        let Some(task) = self.create_task(
            app,
            CreateTaskParams {
                title: draft.title,
                description: draft.description,
                repo_path: draft.repo_path,
                plan_path: None,
                epic_id: epic_id.map(|e| e.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: app.active_project(),
            },
        ) else {
            return;
        };
        app.update(Message::TaskCreated { task: task.clone() });
        app.update(Message::MarkDispatching(task.id));
        let expanded = models::expand_tilde(&repo_path);
        let _ = self.database.save_repo_path(&expanded);
        let paths = self.database.list_repo_paths().unwrap_or_default();
        app.update(Message::RepoPathsUpdated(paths));
        let epic_ctx = dispatch::EpicContext::from_db(&task, &*self.database);
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            let id = task.id;
            match dispatch::quick_dispatch_agent(&task, &*runner, epic_ctx.as_ref()) {
                Ok(result) => {
                    let _ = tx.send(Message::Dispatched {
                        id,
                        worktree: result.worktree_path,
                        tmux_window: result.tmux_window,
                        switch_focus: true,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::DispatchFailed(id));
                    let _ = tx.send(Message::Error(format!("Quick dispatch failed: {e:#}")));
                }
            }
        });
    }

    pub(super) fn exec_persist_task(&self, app: &mut App, task: models::Task) {
        use crate::service::UpdateTaskParams;
        let mut p = UpdateTaskParams::for_task(task.id.0)
            .status(task.status)
            .sub_status(task.sub_status)
            .pr_url(option_to_field_update(task.pr_url.clone()))
            .worktree(option_to_field_update(task.worktree.clone()))
            .tmux_window(option_to_field_update(task.tmux_window.clone()));
        if let Some(so) = task.sort_order {
            p = p.sort_order(so);
        }
        if let Err(e) = self.task_svc.update_task(p) {
            app.update(Message::Error(Self::db_error("persisting task", e)));
        }
    }

    pub(super) fn exec_patch_sub_status(
        &self,
        app: &mut App,
        id: models::TaskId,
        sub_status: models::SubStatus,
    ) {
        use crate::service::UpdateTaskParams;
        if let Err(e) = self
            .task_svc
            .update_task(UpdateTaskParams::for_task(id.0).sub_status(sub_status))
        {
            app.update(Message::Error(Self::db_error("patching sub_status", e)));
        }
    }

    pub(super) fn exec_delete_task(&self, app: &mut App, id: TaskId) {
        if let Err(e) = self.task_svc.delete_task(id.0) {
            app.update(Message::Error(Self::db_error("deleting task", e)));
        }
    }

    pub(super) fn exec_dispatch_agent(&self, task: models::Task, mode: models::DispatchMode) {
        let epic_ctx = dispatch::EpicContext::from_db(&task, &*self.database);
        let label = match mode {
            models::DispatchMode::Dispatch => "Dispatch",
            models::DispatchMode::Brainstorm => "Brainstorm",
            models::DispatchMode::Plan => "Plan",
        };
        self.spawn_dispatch(
            task,
            move |t, r| match mode {
                models::DispatchMode::Dispatch => dispatch::dispatch_agent(t, r, epic_ctx.as_ref()),
                models::DispatchMode::Brainstorm => {
                    dispatch::brainstorm_agent(t, r, epic_ctx.as_ref())
                }
                models::DispatchMode::Plan => dispatch::plan_agent(t, r, epic_ctx.as_ref()),
            },
            label,
        );
    }

    pub(super) fn exec_capture_tmux(&self, id: TaskId, window: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            if let Ok(false) = tmux::has_window(&window, &*runner) {
                let _ = tx.send(Message::WindowGone(id));
                return;
            }

            // Activity timestamp for staleness detection (fall back to 0 on error
            // so we never falsely mark an agent as stale).
            let activity_ts = tmux::window_activity(&window, &*runner).unwrap_or(0);

            match tmux::capture_pane(&window, 5, &*runner) {
                Ok(output) => {
                    let _ = tx.send(Message::TmuxOutput {
                        id,
                        output,
                        activity_ts,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::Error(format!(
                        "tmux capture failed for window {window}: {e}"
                    )));
                }
            }
        });
    }

    pub(super) fn exec_save_repo_path(&self, app: &mut App, path: String) {
        let path = models::expand_tilde(&path);
        if let Err(e) = self.database.save_repo_path(&path) {
            app.update(Message::Error(Self::db_error("saving repo path", e)));
        }
        match self.database.list_repo_paths() {
            Ok(paths) => {
                app.update(Message::RepoPathsUpdated(paths));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("listing repo paths", e)));
            }
        }
    }

    pub(super) fn exec_refresh_from_db(&self, app: &mut App) -> Vec<Command> {
        let mut cmds = Vec::new();
        match self.database.list_all() {
            Ok(tasks) => {
                cmds = app.update(Message::RefreshTasks(tasks));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("refreshing tasks", e)));
            }
        }
        self.exec_refresh_epics_from_db(app);
        self.exec_refresh_usage_from_db(app);
        cmds
    }

    pub(super) fn exec_delete_repo_path(&self, app: &mut App, path: &str) {
        if let Err(e) = self.database.delete_repo_path(path) {
            app.update(Message::Error(Self::db_error("deleting repo path", e)));
            return;
        }
        match self.database.list_repo_paths() {
            Ok(paths) => {
                app.update(Message::RepoPathsUpdated(paths));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("listing repo paths", e)));
            }
        }
        // Refresh presets since delete_repo_path cleans them
        if let Ok(raw) = self.database.list_filter_presets() {
            let known: HashSet<String> = app.repo_paths().iter().cloned().collect();
            let presets = parse_raw_presets(raw, Some(&known));
            app.update(Message::FilterPresetsLoaded(presets));
        }
    }

    pub(super) fn exec_cleanup(
        &self,
        id: TaskId,
        repo_path: String,
        worktree: String,
        tmux_window: Option<String>,
    ) {
        let shared = self
            .database
            .has_other_tasks_with_worktree(&worktree, id)
            .unwrap_or(false);

        if shared {
            // Other active tasks share this worktree — just detach this task
            tracing::info!(task_id = id.0, "worktree shared, detaching only");
            if let Err(e) = self.task_svc.update_task(
                crate::service::UpdateTaskParams::for_task(id.0)
                    .worktree(FieldUpdate::Clear)
                    .tmux_window(FieldUpdate::Clear),
            ) {
                let _ = self
                    .msg_tx
                    .send(Message::Error(format!("Detach failed: {e:#}")));
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
                let _ = tx.send(Message::Error(format!("Cleanup failed: {e:#}")));
            }
        });
    }

    pub(super) fn exec_finish(
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
            .unwrap_or(false);

        if shared {
            tracing::info!(
                task_id = id.0,
                "worktree shared, detaching only (no rebase)"
            );
            if let Err(e) = self.task_svc.update_task(
                crate::service::UpdateTaskParams::for_task(id.0)
                    .worktree(FieldUpdate::Clear)
                    .tmux_window(FieldUpdate::Clear),
            ) {
                let _ = self
                    .msg_tx
                    .send(Message::Error(format!("Detach failed: {e:#}")));
            }
            let _ = self.msg_tx.send(Message::FinishComplete(id));
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
                    let _ = tx.send(Message::FinishComplete(id));
                }
                Err(e) => {
                    let is_conflict = matches!(e, dispatch::FinishError::RebaseConflict(_));
                    let _ = tx.send(Message::FinishFailed {
                        id,
                        error: e.to_string(),
                        is_conflict,
                    });
                }
            }
        });
    }

    fn exec_refresh_projects_from_db(&self, app: &mut App) {
        match self.database.list_projects() {
            Ok(projects) => {
                app.update(Message::ProjectsUpdated(projects));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("refreshing projects", e)));
            }
        }
    }

    pub(super) fn exec_create_project(&self, app: &mut App, name: String) {
        let max_order = app
            .projects()
            .iter()
            .map(|p| p.sort_order)
            .max()
            .unwrap_or(0);
        match self.database.create_project(&name, max_order + 1) {
            Ok(project) => {
                app.update(Message::StatusInfo(format!(
                    "Created project \"{}\"",
                    project.name
                )));
                self.exec_refresh_projects_from_db(app);
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("creating project", e)));
            }
        }
    }

    pub(super) fn exec_rename_project(&self, app: &mut App, id: ProjectId, name: String) {
        match self.database.rename_project(id, &name) {
            Ok(()) => self.exec_refresh_projects_from_db(app),
            Err(e) => {
                app.update(Message::Error(Self::db_error("renaming project", e)));
            }
        }
    }

    pub(super) fn exec_delete_project(&self, app: &mut App, id: ProjectId) {
        let Some(default_id) = app.projects().iter().find(|p| p.is_default).map(|p| p.id) else {
            app.update(Message::Error("No default project found".to_string()));
            return;
        };
        if let Err(e) = self.database.delete_project_and_move_items(id, default_id) {
            app.update(Message::Error(Self::db_error("deleting project", e)));
            return;
        }
        if app.active_project() == id {
            app.update(Message::SelectProject(default_id));
        }
        self.exec_refresh_projects_from_db(app);
    }

    pub(super) fn exec_reorder_project(&self, app: &mut App, id: ProjectId, delta: i8) {
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
        let _ = self.database.reorder_project(id, neighbor_order);
        let _ = self
            .database
            .reorder_project(projects[neighbor_idx].id, current_order);
        self.exec_refresh_projects_from_db(app);
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
                    let _ = tx.send(Message::Resumed {
                        id,
                        tmux_window: result.tmux_window,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::Error(format!("Resume failed: {e:#}")));
                }
            }
        });
    }
}
