use super::*;

impl TuiRuntime {
    pub(super) fn exec_send_notification(
        &self,
        title: &str,
        body: &str,
        urgent: bool,
    ) -> tokio::task::JoinHandle<()> {
        let runner = self.runner.clone();
        let urgency = if urgent { "critical" } else { "normal" };
        let title = title.to_owned();
        let body = body.to_owned();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = runner.run("notify-send", &["-u", urgency, &title, &body]) {
                tracing::warn!("notify-send failed: {e}");
            }
        })
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

    pub(super) fn exec_open_in_browser(&self, url: String) -> tokio::task::JoinHandle<()> {
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = runner.run("xdg-open", &[&url]) {
                tracing::warn!("Failed to open browser: {e}");
            }
        })
    }
}
