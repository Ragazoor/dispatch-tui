use super::*;

impl TuiRuntime {
    pub(super) async fn exec_insert_epic(
        &self,
        app: &mut App,
        title: String,
        description: String,
        parent_epic_id: Option<crate::models::EpicId>,
    ) {
        match self
            .epic_svc
            .create_epic(crate::service::CreateEpicParams {
                title,
                description,
                sort_order: None,
                parent_epic_id,
                feed_command: None,
                feed_interval_secs: None,
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
        if status.is_none() && sort_order.is_none() {
            return;
        }
        self.exec_patch_epic(
            app,
            crate::service::UpdateEpicParams {
                epic_id: id,
                title: None,
                description: None,
                status,
                plan_path: None,
                sort_order,
                auto_dispatch: None,
                feed_command: None,
                feed_interval_secs: None,
                group_by_repo: None,
                parent_epic_id: None,
            },
            "updating epic",
        )
        .await;
    }

    async fn exec_patch_epic(
        &self,
        app: &mut App,
        params: crate::service::UpdateEpicParams,
        context: &str,
    ) {
        if let Err(e) = self.epic_svc.update_epic(params).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error(context, e),
            )));
        }
    }

    pub(super) async fn exec_toggle_epic_auto_dispatch(
        &self,
        app: &mut App,
        id: models::EpicId,
        auto_dispatch: bool,
    ) {
        self.exec_patch_epic(
            app,
            crate::service::UpdateEpicParams {
                epic_id: id,
                title: None,
                description: None,
                status: None,
                plan_path: None,
                sort_order: None,
                auto_dispatch: Some(auto_dispatch),
                feed_command: None,
                feed_interval_secs: None,
                group_by_repo: None,
                parent_epic_id: None,
            },
            "toggling auto dispatch",
        )
        .await;
    }

    pub(super) async fn exec_toggle_epic_group_by_repo(
        &self,
        app: &mut App,
        id: models::EpicId,
        group_by_repo: bool,
    ) {
        let params = crate::service::UpdateEpicParams {
            epic_id: id,
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            group_by_repo: Some(group_by_repo),
            parent_epic_id: None,
        };
        if let Err(e) = self.epic_svc.update_epic(params).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("toggling group by repo", e),
            )));
            return;
        }
        // Apply migration only for non-feed epics (feed epics group via ingestion).
        match self.epic_svc.get_epic(id).await {
            Ok(epic) if epic.feed_command.is_none() => {
                let res = if group_by_repo {
                    self.epic_svc.regroup_epic(id).await
                } else {
                    self.epic_svc.flatten_epic(id).await
                };
                if let Err(e) = res {
                    app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                        Self::db_error("regrouping epic", e),
                    )));
                    return;
                }
            }
            Ok(_) => {}
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("loading epic after toggle", e),
                )));
                return;
            }
        }
        self.exec_refresh_epics_from_db(app).await;
    }

    pub(super) async fn exec_reparent_epic(
        &self,
        app: &mut App,
        id: models::EpicId,
        new_parent: Option<models::EpicId>,
    ) {
        self.exec_patch_epic(
            app,
            crate::service::UpdateEpicParams {
                epic_id: id,
                title: None,
                description: None,
                status: None,
                plan_path: None,
                sort_order: None,
                auto_dispatch: None,
                feed_command: None,
                feed_interval_secs: None,
                group_by_repo: None,
                parent_epic_id: Some(new_parent),
            },
            "reparenting epic",
        )
        .await;
        // Refresh so the board reflects the new hierarchy.
        self.exec_refresh_epics_from_db(app).await;
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

    pub(super) fn exec_trigger_epic_feed(
        &self,
        epic_id: models::EpicId,
        epic_title: String,
        feed_command: String,
        group_by_repo: bool,
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

            let count = items.len(); // items emitted by the feed command, not tasks inserted
            let known_paths = db.list_repo_paths().await.unwrap_or_default();
            let repo_paths = dispatch::resolve_feed_item_repo_paths(&items, &known_paths);
            let base_branches = crate::feed::resolve_base_branches(&repo_paths, &*runner);
            match crate::feed::run_feed_sync(
                &*db,
                epic_id,
                group_by_repo,
                &items,
                &repo_paths,
                &base_branches,
            )
            .await
            {
                Ok(_) => {
                    crate::feed::recalculate_epic_status_after_feed(
                        &*db,
                        epic_id,
                        "exec_trigger_epic_feed",
                    )
                    .await;
                    let _ = tx.send(Message::Feed(
                        crate::tui::messages::FeedMessage::Refreshed { epic_title, count },
                    ));
                }
                Err(e) => fail(e.to_string()),
            }
        });
    }
}
