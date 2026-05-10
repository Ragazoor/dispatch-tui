use super::*;

impl TuiRuntime {
    pub(super) fn exec_send_notification(&self, title: &str, body: &str, urgent: bool) {
        let urgency = if urgent { "critical" } else { "normal" };
        if let Err(e) = self
            .runner
            .run("notify-send", &["-u", urgency, title, body])
        {
            tracing::warn!("notify-send failed: {e}");
        }
    }

    pub(super) async fn exec_persist_setting(&self, app: &mut App, key: &str, value: bool) {
        if let Err(e) = self.database.set_setting_bool(key, value).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("persisting setting", e),
            )));
        }
    }

    pub(super) async fn exec_persist_string_setting(&self, app: &mut App, key: &str, value: &str) {
        if let Err(e) = self.database.set_setting_string(key, value).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("persisting setting", e),
            )));
        }
    }

    pub(super) async fn exec_persist_filter_preset(
        &self,
        app: &mut App,
        name: &str,
        repo_paths: &[String],
        mode: &str,
    ) {
        if let Err(e) = self
            .database
            .save_filter_preset(name, repo_paths, mode)
            .await
        {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("saving filter preset", e),
            )));
        }
    }

    pub(super) async fn exec_delete_filter_preset(&self, app: &mut App, name: &str) {
        if let Err(e) = self.database.delete_filter_preset(name).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("deleting filter preset", e),
            )));
        }
    }

    pub(super) async fn exec_refresh_usage_from_db(&self, app: &mut App) {
        match self.database.get_all_usage().await {
            Ok(usage) => {
                app.update(Message::RefreshUsage(usage));
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("refreshing usage", e),
                )));
            }
        }
    }

    pub(super) fn exec_open_in_browser(&self, url: String) {
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = runner.run("xdg-open", &[&url]) {
                tracing::warn!("Failed to open browser: {e}");
            }
        });
    }

    pub(super) async fn exec_save_tips_state(
        &self,
        seen_up_to: u32,
        show_mode: crate::models::TipsShowMode,
    ) {
        if let Err(e) = self.database.save_tips_state(seen_up_to, show_mode).await {
            tracing::warn!("Failed to save tips state: {e:#}");
        }
    }
}
