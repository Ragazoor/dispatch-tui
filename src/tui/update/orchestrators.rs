//! The orchestrator on the board: its keys, the ready mark and its
//! notifications (`orchestrators.allium: OrchestratorInEpicView`,
//! `OrchestratorNotifications`).

use std::collections::HashSet;

use crate::models::{descendant_epic_ids, EpicId, OrchestrationBoard, TaskStatus};

use super::super::commands::{EpicCommand, SystemCommand, TaskCommand};
use super::super::types::*;
use super::super::App;

/// Where the orchestrator key works, said when it is pressed anywhere else.
pub(in crate::tui) const OPEN_ORCHESTRATOR_NOWHERE: &str =
    "The orchestrator key works in an epic view or on a task of an orchestrated epic";

impl App {
    /// The board reload's orchestrators. An item that opened since the last
    /// reload notifies on its own machine's board, once (`EachOnItsOwnMachine`);
    /// the first reload's open items are the baseline.
    pub(in crate::tui) fn handle_orchestration_refreshed(
        &mut self,
        board: OrchestrationBoard,
    ) -> Vec<Command> {
        let mut cmds = Vec::new();
        match &mut self.board.items_seen {
            None => {
                self.board.items_seen = Some(board.open_items.iter().map(|i| i.id).collect());
            }
            Some(seen) => {
                let new: Vec<_> = board
                    .open_items
                    .iter()
                    .filter(|i| seen.insert(i.id))
                    .filter(|i| Some(&i.host) == self.local_host_id.as_ref())
                    .cloned()
                    .collect();
                if self.notifications_enabled {
                    for item in new {
                        let about = match item.task_id {
                            Some(task) => {
                                format!("{}: {}", self.board.task_label(task), item.about)
                            }
                            None => format!("the epic: {}", item.about),
                        };
                        cmds.push(Command::System(SystemCommand::SendNotification {
                            title: format!("{} waits for you", self.board.epic_label(item.epic_id)),
                            body: about,
                            urgent: true,
                        }));
                    }
                }
            }
        }
        if board != self.board.orchestration {
            self.board.orchestration = board;
            self.dirty = true;
        }
        cmds
    }

    /// `NotifyEpicFinished`: this machine's orchestrator ended on a done epic.
    pub(in crate::tui) fn handle_epic_finished(&mut self, epic: EpicId) -> Vec<Command> {
        if !self.notifications_enabled {
            return vec![];
        }
        vec![Command::System(SystemCommand::SendNotification {
            title: self.board.epic_label(epic),
            body: "The epic is finished".to_string(),
            urgent: false,
        })]
    }

    /// `covering(epic)`, the epic whose switch covers `epic`.
    pub(in crate::tui) fn covering_epic(&self, epic: Option<EpicId>) -> Option<EpicId> {
        self.board
            .orchestration
            .covering(epic, |id| {
                self.board.find_epic(id).and_then(|e| e.parent_epic_id)
            })
            .map(|o| o.epic_id)
    }

    /// Whether a task of `epic`'s subtree is not done (`has_open_work`).
    pub(in crate::tui) fn has_open_work(&self, epic: EpicId) -> bool {
        let ids = descendant_epic_ids(epic, &self.board.epics);
        self.board.tasks.iter().any(|t| {
            t.status != TaskStatus::Done && t.epic_id.is_some_and(|e| e == epic || ids.contains(&e))
        })
    }

    /// The ready key (`ReadyShowsOnTheCard`): set or clear the operator's
    /// mark on the backlog task under the cursor. The board flips its copy
    /// at once and the store write follows.
    pub(in crate::tui) fn handle_toggle_ready(&mut self) -> Vec<Command> {
        let Some(task) = self.selected_task() else {
            return vec![];
        };
        let (id, status) = (task.id, task.status);
        if status != TaskStatus::Backlog {
            self.status
                .set("Only a backlog task can be marked ready".to_string());
            return vec![];
        }
        let marks = &mut self.board.orchestration.ready_marks;
        let ready = !marks.iter().any(|m| m.task_id == id);
        if ready {
            marks.push(crate::models::ReadyMark {
                task_id: id,
                marked_by: crate::models::ReadyMarker::Operator,
                agent_name: None,
                marked_at: chrono::Utc::now(),
            });
        } else {
            marks.retain(|m| m.task_id != id);
        }
        self.dirty = true;
        vec![Command::Epic(EpicCommand::SetReady { task: id, ready })]
    }

    /// The orchestrator key (`OrchestratorKey`).
    pub(in crate::tui) fn handle_open_orchestrator(&mut self) -> Vec<Command> {
        let from_task = self
            .selected_task()
            .and_then(|t| self.covering_epic(t.epic_id));
        let target = match (from_task, self.current_epic_id()) {
            (Some(epic), _) => epic,
            (None, None) => {
                self.status.set(OPEN_ORCHESTRATOR_NOWHERE.to_string());
                return vec![];
            }
            (None, Some(view)) => match self.covering_epic(Some(view)) {
                Some(epic) => epic,
                None => match self.orchestrated_below(view).as_slice() {
                    [] => {
                        self.input.mode = InputMode::ConfirmStartOrchestrator(view);
                        self.status.set(start_orchestrator_prompt(self.key_table));
                        return vec![];
                    }
                    [one] => *one,
                    several => {
                        let names: Vec<String> =
                            several.iter().map(|e| self.board.epic_label(*e)).collect();
                        self.status.set(format!(
                            "Epics below have orchestrators of their own: {}. Open one from its epic",
                            names.join(", ")
                        ));
                        return vec![];
                    }
                },
            },
        };
        let title = self.board.epic_label(target);
        let host = self.local_host_id.clone().unwrap_or_default();
        let Some(o) = self.board.orchestration.orchestrator_on(target, &host) else {
            self.status.set(format!(
                "The orchestrator of {title} has not started on this machine yet"
            ));
            return vec![];
        };
        if let Some(window) = o.tmux_window.clone() {
            return vec![Command::Task(TaskCommand::JumpToTmux { window })];
        }
        if !self.has_open_work(target) {
            self.status.set(format!(
                "{title} has no open work, so its orchestrator rests"
            ));
            return vec![];
        }
        let id = o.id;
        self.status
            .set(format!("Starting the orchestrator of {title}\u{2026}"));
        vec![Command::Epic(EpicCommand::RestartOrchestrator {
            epic: target,
            id,
        })]
    }

    /// The switched-on epics strictly below `epic`, by id.
    fn orchestrated_below(&self, epic: EpicId) -> Vec<EpicId> {
        let ids: HashSet<EpicId> = descendant_epic_ids(epic, &self.board.epics)
            .into_iter()
            .filter(|e| *e != epic)
            .collect();
        let mut below: Vec<EpicId> = self
            .board
            .orchestration
            .orchestrations
            .iter()
            .map(|o| o.epic_id)
            .filter(|e| ids.contains(e))
            .collect();
        below.sort_unstable();
        below
    }

    /// y / anything else on "Start an orchestrator for this epic?".
    pub(in crate::tui) fn handle_key_confirm_start_orchestrator(
        &mut self,
        label: &str,
        yes: bool,
        epic: EpicId,
    ) -> Vec<Command> {
        self.confirm_dialog(label, yes, "confirm_start_orchestrator", |_| {
            vec![Command::Epic(EpicCommand::SwitchOrchestratorOn(epic))]
        })
    }

    /// The resolve key (`ResolveKey`): ask about the oldest open item of the
    /// orchestrator that covers the epic view.
    pub(in crate::tui) fn handle_resolve_item_key(&mut self) -> Vec<Command> {
        let Some(view) = self.current_epic_id() else {
            return vec![];
        };
        let Some(epic) = self.covering_epic(Some(view)) else {
            self.status
                .set("No orchestrator covers this epic".to_string());
            return vec![];
        };
        let Some(item) = self
            .board
            .orchestration
            .open_items_of(epic)
            .first()
            .copied()
        else {
            self.status.set(format!(
                "Nothing waits for you in {}",
                self.board.epic_label(epic)
            ));
            return vec![];
        };
        let prompt = resolve_item_prompt(self.key_table, item);
        self.input.mode = InputMode::ConfirmResolveItem(item.id);
        self.status.set(prompt);
        vec![]
    }

    /// y / anything else on "Resolve item #N?".
    pub(in crate::tui) fn handle_key_confirm_resolve_item(
        &mut self,
        label: &str,
        yes: bool,
        item: i64,
    ) -> Vec<Command> {
        self.confirm_dialog(label, yes, "confirm_resolve_item", |app| {
            app.board.orchestration.open_items.retain(|i| i.id != item);
            app.dirty = true;
            vec![Command::Epic(EpicCommand::ResolveWaitingItem(item))]
        })
    }
}

/// "Start an orchestrator for this epic? [y/n]".
pub(in crate::tui) fn start_orchestrator_prompt(
    table: &'static [crate::keybindings::KeyBinding],
) -> String {
    let keys = crate::keybindings::yes_no_hint(
        table,
        crate::keybindings::KeyNamespace::BoardConfirmStartOrchestrator,
    );
    format!("Start an orchestrator for this epic? {keys}")
}

/// "Resolve item #N: <about>? [y/n]".
pub(in crate::tui) fn resolve_item_prompt(
    table: &'static [crate::keybindings::KeyBinding],
    item: &crate::models::WaitingItem,
) -> String {
    let keys = crate::keybindings::yes_no_hint(
        table,
        crate::keybindings::KeyNamespace::BoardConfirmResolveItem,
    );
    format!("Resolve item #{}: {}? {keys}", item.id, item.about)
}
