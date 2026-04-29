use super::*;

impl TuiRuntime {
    pub(super) fn exec_insert_epic(
        &self,
        app: &mut App,
        title: String,
        description: String,
        repo_path: String,
        parent_epic_id: Option<crate::models::EpicId>,
    ) {
        match self.epic_svc.create_epic(crate::service::CreateEpicParams {
            title,
            description,
            repo_path,
            sort_order: None,
            parent_epic_id,
            feed_command: None,
            feed_interval_secs: None,
            project_id: app.active_project(),
        }) {
            Ok(epic) => {
                app.update(Message::EpicCreated(epic));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("creating epic", e)));
            }
        }
    }

    pub(super) fn exec_delete_epic(&self, app: &mut App, id: models::EpicId) {
        if let Err(e) = self.epic_svc.delete_epic(id.0) {
            app.update(Message::Error(Self::db_error("deleting epic", e)));
        }
    }

    pub(super) fn exec_persist_epic(
        &self,
        app: &mut App,
        id: models::EpicId,
        status: Option<models::TaskStatus>,
        sort_order: Option<i64>,
    ) {
        // Only call service if there's something to update
        if status.is_none() && sort_order.is_none() {
            return;
        }
        if let Err(e) = self.epic_svc.update_epic(crate::service::UpdateEpicParams {
            epic_id: id.0,
            title: None,
            description: None,
            status,
            plan_path: None,
            sort_order,
            repo_path: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: None,
        }) {
            app.update(Message::Error(Self::db_error("updating epic", e)));
        }
    }

    pub(super) fn exec_toggle_epic_auto_dispatch(
        &self,
        app: &mut App,
        id: models::EpicId,
        auto_dispatch: bool,
    ) {
        if let Err(e) = self.epic_svc.update_epic(crate::service::UpdateEpicParams {
            epic_id: id.0,
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: Some(auto_dispatch),
            feed_command: None,
            feed_interval_secs: None,
            project_id: None,
        }) {
            app.update(Message::Error(Self::db_error("toggling auto dispatch", e)));
        }
    }

    pub(super) fn exec_refresh_epics_from_db(&self, app: &mut App) {
        match self.database.list_epics() {
            Ok(epics) => {
                app.update(Message::RefreshEpics(epics));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("refreshing epics", e)));
            }
        }
    }

    pub(super) fn exec_dispatch_epic(&self, app: &mut App, epic: models::Epic) {
        let title = format!("Plan: {}", epic.title);
        let description = format!(
            "Planning subtask for epic: {}\n\n{}",
            epic.title, epic.description
        );

        // Create the planning subtask via service
        let task = match self
            .task_svc
            .create_task_returning(crate::service::CreateTaskParams {
                title: title.clone(),
                description: description.clone(),
                repo_path: epic.repo_path.clone(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: epic.project_id,
            }) {
            Ok(task) => task,
            Err(e) => {
                app.update(Message::Error(Self::db_error("creating planning task", e)));
                return;
            }
        };

        app.update(Message::TaskCreated { task: task.clone() });

        // Dispatch the planning subtask asynchronously
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        let epic_id = epic.id;
        let epic_title = epic.title.clone();
        let epic_description = epic.description.clone();

        tokio::task::spawn_blocking(move || {
            let id = task.id;
            tracing::info!(
                task_id = id.0,
                epic_id = epic_id.0,
                "dispatching epic planning agent"
            );
            match dispatch::epic_planning_agent(
                &task,
                epic_id,
                &epic_title,
                &epic_description,
                &*runner,
                &[],
            ) {
                Ok(result) => {
                    let _ = tx.send(Message::Dispatched {
                        id,
                        worktree: result.worktree_path,
                        tmux_window: result.tmux_window,
                        switch_focus: true,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::Error(format!(
                        "Epic planning dispatch failed: {e:#}"
                    )));
                }
            }
        });
    }
}
