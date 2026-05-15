use super::*;

impl TuiRuntime {
    pub(super) async fn exec_insert_epic(
        &self,
        app: &mut App,
        title: String,
        description: String,
        repo_path: String,
        parent_epic_id: Option<crate::models::EpicId>,
    ) {
        match self
            .epic_svc
            .create_epic(crate::service::CreateEpicParams {
                title,
                description,
                repo_path,
                sort_order: None,
                parent_epic_id,
                feed_command: None,
                feed_interval_secs: None,
                project_id: app.active_project(),
            })
            .await
        {
            Ok(epic) => {
                app.update(Message::Epic(crate::tui::messages::EpicMessage::Created(
                    epic,
                )));
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("creating epic", e),
                )));
            }
        }
    }

    pub(super) async fn exec_delete_epic(&self, app: &mut App, id: models::EpicId) {
        if let Err(e) = self.epic_svc.delete_epic(id).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("deleting epic", e),
            )));
        }
    }

    pub(super) async fn exec_persist_epic(
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
        if let Err(e) = self
            .epic_svc
            .update_epic(crate::service::UpdateEpicParams {
                epic_id: id,
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
            })
            .await
        {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("updating epic", e),
            )));
        }
    }

    pub(super) async fn exec_toggle_epic_auto_dispatch(
        &self,
        app: &mut App,
        id: models::EpicId,
        auto_dispatch: bool,
    ) {
        if let Err(e) = self
            .epic_svc
            .update_epic(crate::service::UpdateEpicParams {
                epic_id: id,
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
            })
            .await
        {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("toggling auto dispatch", e),
            )));
        }
    }

    pub(super) async fn exec_refresh_epics_from_db(&self, app: &mut App) {
        match self.database.list_epics().await {
            Ok(epics) => {
                app.update(Message::Epic(crate::tui::messages::EpicMessage::Refresh(
                    epics,
                )));
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("refreshing epics", e),
                )));
            }
        }
    }

    pub(super) async fn exec_dispatch_epic(&self, app: &mut App, epic: models::Epic) {
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
                epic_id: Some(epic.id),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: epic.project_id,
            })
            .await
        {
            Ok(task) => task,
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("creating planning task", e),
                )));
                return;
            }
        };

        app.update(Message::Task(crate::tui::messages::TaskMessage::Created {
            task: task.clone(),
        }));

        let project_ctx = dispatch::ProjectContext::from_db(&task, &*self.database).await;
        let verify_command = dispatch::fetch_verify_command(&*self.database, &task.repo_path).await;

        // Dispatch the planning subtask asynchronously
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        let epic_id = epic.id;
        let epic_title = epic.title.clone();

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
                &project_ctx,
                &*runner,
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
                    let _ = tx.send(Message::System(crate::tui::messages::SystemMessage::Error(
                        format!("Epic planning dispatch failed: {e:#}"),
                    )));
                }
            }
        });
    }

    pub(super) fn exec_trigger_epic_feed(
        &self,
        epic_id: models::EpicId,
        epic_title: String,
        feed_command: String,
    ) {
        let db = self.database.clone();
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::spawn(async move {
            let fail = |error: String| {
                let _ = tx.send(Message::Feed(crate::tui::messages::FeedMessage::Failed {
                    epic_title: epic_title.clone(),
                    error,
                }));
            };

            let output = match tokio::process::Command::new("sh")
                .args(["-c", &feed_command])
                .output()
                .await
            {
                Ok(o) => o,
                Err(e) => return fail(e.to_string()),
            };

            if !output.status.success() {
                return fail(String::from_utf8_lossy(&output.stderr).into_owned());
            }

            let items: Vec<models::FeedItem> = match serde_json::from_slice(&output.stdout) {
                Ok(i) => i,
                Err(e) => return fail(e.to_string()),
            };

            let count = items.len();
            let known_paths = db.list_repo_paths().await.unwrap_or_default();
            let repo_paths = dispatch::resolve_feed_item_repo_paths(&items, &known_paths);
            let base_branches = crate::feed::resolve_base_branches(&repo_paths, &*runner);
            match db
                .upsert_feed_tasks(epic_id, &items, &repo_paths, &base_branches)
                .await
            {
                Ok(()) => {
                    let _ = tx.send(Message::Feed(
                        crate::tui::messages::FeedMessage::Refreshed { epic_title, count },
                    ));
                }
                Err(e) => fail(e.to_string()),
            }
        });
    }
}
