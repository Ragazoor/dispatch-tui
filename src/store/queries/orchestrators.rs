//! `core.allium: Orchestrator` over the shared store.

use anyhow::Result;

use crate::models::{EpicId, Orchestrator};

use super::super::Store;

#[async_trait::async_trait]
impl super::super::OrchestratorStore for Store {
    async fn list_orchestrators(&self) -> Vec<Orchestrator> {
        self.rows
            .orchestrators()
            .iter()
            .filter_map(|row| {
                crate::sync::decode::orchestrator(row)
                    .map_err(|e| tracing::warn!("{e}"))
                    .ok()
            })
            .collect()
    }

    async fn orchestrator_named(&self, name: &str) -> Option<Orchestrator> {
        let row = self
            .rows
            .orchestrators()
            .into_iter()
            .find(|row| row.name == name)?;
        crate::sync::decode::orchestrator(&row)
            .map_err(|e| tracing::warn!("{e}"))
            .ok()
    }

    async fn ensure_orchestrator(&self, epic: EpicId, host: &str) -> Result<()> {
        self.caller
            .ensure_orchestrator(epic, host.to_string())
            .await?
            .applied()
    }

    async fn start_orchestrator_session(&self, id: i64, host: &str) -> Result<()> {
        self.caller
            .start_orchestrator_session(id, host.to_string())
            .await?
            .applied()
    }

    async fn end_orchestrator_session(&self, id: i64, host: &str, died: bool) -> Result<()> {
        self.caller
            .end_orchestrator_session(id, host.to_string(), died)
            .await?
            .applied()
    }

    async fn orchestrator_turn_completed(&self, name: &str) -> Result<()> {
        self.caller
            .orchestrator_turn_completed(name.to_string())
            .await?
            .applied()
    }
}
