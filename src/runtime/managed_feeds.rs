//! Runtime handlers for the managed-feed config popup.
//!
//! `PersistConfig` writes the four settings; `ProvisionAndRefresh` re-fires
//! `provision_managed_feeds_from_settings` and syncs the resulting epic tree
//! into the board, so enabling a feed provisions it without a restart. Both are
//! best-effort: a DB/provisioning failure surfaces as an error popup but never
//! wedges the TUI (matching the startup provisioning posture).

use super::*;

impl TuiRuntime {
    pub(super) async fn exec_persist_managed_feed_config(
        &self,
        app: &mut App,
        reviews_command: Option<String>,
        reviews_interval_secs: Option<i64>,
        cve_command: Option<String>,
        cve_interval_secs: Option<i64>,
    ) {
        let db = &self.database;
        let result = async {
            db.set_reviews_feed_command(reviews_command.as_deref())
                .await?;
            db.set_reviews_feed_interval_secs(reviews_interval_secs)
                .await?;
            db.set_cve_feed_command(cve_command.as_deref()).await?;
            db.set_cve_feed_interval_secs(cve_interval_secs).await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(e) = result {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("persisting managed feed config", e),
            )));
        }
    }

    pub(super) async fn exec_provision_and_refresh(&self, app: &mut App) {
        if let Err(e) = crate::service::provision_managed_feeds_from_settings(&*self.database).await
        {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("provisioning managed feeds", e),
            )));
            return;
        }
        // Surface any newly-created managed epics in the board.
        self.exec_refresh_epics_from_db(app).await;
    }
}
