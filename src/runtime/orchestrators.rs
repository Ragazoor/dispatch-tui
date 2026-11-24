//! The board's orchestrator commands (`orchestrators.allium:
//! OrchestratorInEpicView`).

use crate::models::{EpicId, ReadyMarker, TaskId};
use crate::tui::messages::SystemMessage;
use crate::tui::{App, Message};

use super::TuiRuntime;

impl TuiRuntime {
    /// The ready key's write, as the operator.
    pub(super) async fn exec_set_ready(&self, app: &mut App, task: TaskId, ready: bool) {
        let result = if ready {
            self.task_svc
                .mark_task_ready(task, ReadyMarker::Operator, None)
                .await
        } else {
            self.task_svc.clear_ready_mark(task).await
        };
        if let Err(e) = result {
            app.update(Message::System(SystemMessage::Error(format!(
                "Could not change the ready mark: {e}"
            ))));
            // Puts back the board's own flip of the mark.
            drop(self.spawn_refresh_from_db());
        }
    }

    /// "Start an orchestrator for this epic?" answered y.
    pub(super) async fn exec_switch_orchestrator_on(&self, app: &mut App, epic: EpicId) {
        match self.epic_svc.switch_orchestrator_on(epic, None, None).await {
            Ok(()) => {
                app.update(Message::System(SystemMessage::StatusInfo(
                    "Orchestrator switched on".to_string(),
                )));
            }
            Err(e) => {
                app.update(Message::System(SystemMessage::Error(format!(
                    "Could not switch the orchestrator on: {e}"
                ))));
            }
        }
        drop(self.spawn_refresh_from_db());
    }

    /// `OperatorRestartsADownOrchestrator`, then the session and its window
    /// now rather than at the runner's next check.
    pub(super) async fn exec_restart_orchestrator(&self, app: &mut App, epic: EpicId, id: i64) {
        if let Err(e) = self.database.restart_orchestrator(id, &self.host_id).await {
            app.update(Message::System(SystemMessage::Error(format!(
                "Could not start the orchestrator: {e:#}"
            ))));
            return;
        }
        self.orchestrator_opener.open_after_start(epic);
    }

    /// "Resolve item #N?" answered y (`WaitingItemResolved`).
    pub(super) async fn exec_resolve_waiting_item(&self, app: &mut App, item: i64) {
        if let Err(e) = self.database.resolve_waiting_item(item).await {
            app.update(Message::System(SystemMessage::Error(format!(
                "Could not resolve item #{item}: {e:#}"
            ))));
            // Puts the item the board dropped back.
            drop(self.spawn_refresh_from_db());
        }
    }
}
