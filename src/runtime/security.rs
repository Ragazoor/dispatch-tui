use super::*;

impl TuiRuntime {
    pub(super) fn exec_fetch_security_alerts(&self) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            tracing::info!("fetching security alerts via gh");
            match crate::github::fetch_security_alerts(&*runner) {
                Ok(alerts) => {
                    tracing::info!(count = alerts.len(), "security alerts fetched successfully");
                    let _ = tx.send(Message::SecurityAlertsLoaded(alerts));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "security alert fetch failed");
                    let _ = tx.send(Message::SecurityAlertsFetchFailed(e));
                }
            }
        });
    }

    pub(super) fn exec_persist_security_alerts(
        &self,
        app: &mut App,
        alerts: Vec<crate::models::SecurityAlert>,
    ) {
        if let Err(e) = self.database.save_security_alerts(&alerts) {
            app.update(Message::Error(Self::db_error(
                "persisting security alerts",
                e,
            )));
        }
    }
}
