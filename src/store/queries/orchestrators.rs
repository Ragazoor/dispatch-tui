//! `core.allium: Orchestrator` over the shared store.

use anyhow::Result;

use crate::models::{EpicId, EpicNote, Orchestrator, OrchestratorLogEntry, TaskId, WaitingItem};

use super::super::Store;

/// Every row that decodes; a row that does not is logged and left out.
fn decoded<R, T, E: std::fmt::Display>(
    rows: Vec<R>,
    decode: impl Fn(&R) -> Result<T, E>,
) -> Vec<T> {
    rows.iter()
        .filter_map(|row| decode(row).map_err(|e| tracing::warn!("{e}")).ok())
        .collect()
}

/// The one id an applied create read back.
fn one_id(ids: &[i64], what: &str) -> Result<i64> {
    match ids {
        [id] => Ok(*id),
        _ => anyhow::bail!(
            "the shared store wrote the {what} but it is not in this board's subscription"
        ),
    }
}

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

    async fn start_orchestrator_session(&self, id: i64, host: &str, resumed: bool) -> Result<()> {
        self.caller
            .start_orchestrator_session(id, host.to_string(), resumed)
            .await?
            .applied()
    }

    async fn orchestrator_log_of_epic(&self, epic: EpicId) -> Vec<OrchestratorLogEntry> {
        let rows = self.rows.orchestrator_log();
        let mine = rows.into_iter().filter(|e| e.epic_id == epic.0).collect();
        decoded(mine, crate::sync::decode::orchestrator_log_entry)
    }

    async fn orchestrator_log_of_task(&self, task: TaskId) -> Vec<OrchestratorLogEntry> {
        let rows = self.rows.orchestrator_log();
        let mine = rows.into_iter().filter(|e| e.task_id == task.0).collect();
        decoded(mine, crate::sync::decode::orchestrator_log_entry)
    }

    async fn list_epic_notes(&self) -> Vec<EpicNote> {
        decoded(self.rows.epic_notes(), crate::sync::decode::epic_note)
    }

    async fn list_waiting_items(&self) -> Vec<WaitingItem> {
        decoded(self.rows.waiting_items(), crate::sync::decode::waiting_item)
    }

    async fn record_orchestrator_answer(
        &self,
        name: &str,
        task: TaskId,
        question: &str,
        answer: &str,
        reason: &str,
    ) -> Result<()> {
        self.caller
            .record_orchestrator_answer(
                name.to_string(),
                task,
                question.to_string(),
                answer.to_string(),
                reason.to_string(),
            )
            .await?
            .applied()
    }

    async fn record_orchestrator_trouble(
        &self,
        name: &str,
        task: TaskId,
        trouble: &str,
        text: &str,
    ) -> Result<()> {
        self.caller
            .record_orchestrator_trouble(
                name.to_string(),
                task,
                trouble.to_string(),
                text.to_string(),
            )
            .await?
            .applied()
    }

    async fn record_orchestrator_summary(&self, name: &str, text: &str) -> Result<()> {
        self.caller
            .record_orchestrator_summary(name.to_string(), text.to_string())
            .await?
            .applied()
    }

    async fn raise_waiting_item(
        &self,
        name: &str,
        task: Option<TaskId>,
        about: &str,
        tried: &str,
        recommendation: &str,
    ) -> Result<i64> {
        let ids = self
            .caller
            .raise_waiting_item(
                name.to_string(),
                task.map_or(0, |t| t.0),
                about.to_string(),
                tried.to_string(),
                recommendation.to_string(),
            )
            .await?
            .into_result()?;
        one_id(&ids, "waiting item")
    }

    async fn raise_orchestrator_down_item(&self, id: i64, host: &str) -> Result<()> {
        self.caller
            .raise_orchestrator_down_item(id, host.to_string())
            .await?
            .applied()
    }

    async fn resolve_waiting_item(&self, id: i64) -> Result<()> {
        self.caller.resolve_waiting_item(id).await?.applied()
    }

    async fn write_epic_note(&self, name: &str, text: &str) -> Result<i64> {
        let ids = self
            .caller
            .write_epic_note(name.to_string(), text.to_string())
            .await?
            .into_result()?;
        one_id(&ids, "epic note")
    }

    async fn change_epic_note(
        &self,
        name: &str,
        id: i64,
        text: Option<&str>,
        done: Option<bool>,
    ) -> Result<()> {
        self.caller
            .change_epic_note(name.to_string(), id, text.map(str::to_string), done)
            .await?
            .applied()
    }

    async fn remove_epic_note(&self, name: &str, id: i64) -> Result<()> {
        self.caller
            .remove_epic_note(name.to_string(), id)
            .await?
            .applied()
    }

    async fn end_orchestrator_session(&self, id: i64, host: &str, died: bool) -> Result<()> {
        self.caller
            .end_orchestrator_session(id, host.to_string(), died)
            .await?
            .applied()
    }

    async fn restart_orchestrator(&self, id: i64, host: &str) -> Result<()> {
        self.caller
            .restart_orchestrator(id, host.to_string())
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
