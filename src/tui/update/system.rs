//! System / status / editor / quick-dispatch / misc handlers.

use crate::models::{EpicId, Task, TaskId, DEFAULT_BASE_BRANCH, DEFAULT_QUICK_TASK_TITLE};

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_error(&mut self, msg: String) -> Vec<Command> {
        self.status.error_popup = Some(msg);
        vec![]
    }

    pub(in crate::tui) fn handle_task_edited(&mut self, edit: TaskEdit) -> Vec<Command> {
        if let Some(t) = self.find_task_mut(edit.id) {
            t.title = edit.title;
            t.description = edit.description;
            t.repo_path = edit.repo_path;
            // Not `set_local_status`: the editor owns sub_status separately (it
            // is not part of TaskEdit, so the current one is kept deliberately)
            // and only the leaving-Running clear applies here. The editor may
            // set any status from any other, Running -> Review included.
            if crate::models::clears_pending_stop(t.status, edit.status) {
                t.stop_pending = false;
            }
            t.status = edit.status;
            t.plan_path = edit.plan_path;
            t.tag = edit.tag;
            if let Some(bb) = edit.base_branch {
                t.base_branch = bb;
            }
            t.wrap_up_mode = edit.wrap_up_mode;
            t.model = edit.model;
            t.url = edit.url;
            t.phoenix = edit.phoenix;
            t.updated_at = chrono::Utc::now();
        }
        self.sync_board_selection();
        vec![]
    }

    pub(in crate::tui) fn handle_repo_paths_updated(&mut self, paths: Vec<String>) -> Vec<Command> {
        self.broken_repo_paths = paths
            .iter()
            .filter(|p| !std::path::Path::new(p).is_dir())
            .cloned()
            .collect();
        self.board.repo_paths = paths;
        // The cursor indexes the repo list directly; keep it on a real row.
        self.input.repo_cursor = self
            .input
            .repo_cursor
            .min(self.board.repo_paths.len().saturating_sub(1));
        vec![]
    }

    pub(in crate::tui) fn handle_base_branches_updated(
        &mut self,
        map: std::collections::HashMap<String, Vec<String>>,
    ) -> Vec<Command> {
        self.board.repo_base_branches = map;
        vec![]
    }

    pub(in crate::tui) fn handle_quick_dispatch(
        &mut self,
        repo_path: String,
        epic_id: Option<EpicId>,
    ) -> Vec<Command> {
        vec![Command::Task(
            crate::tui::commands::TaskCommand::QuickDispatch {
                draft: TaskDraft {
                    title: DEFAULT_QUICK_TASK_TITLE.to_string(),
                    description: String::new(),
                    repo_path,
                    tag: None,
                    base_branch: DEFAULT_BASE_BRANCH.to_string(),
                    wrap_up_mode: None,
                    model: None,
                    // Quick dispatch runs no tag picker, so there is nowhere
                    // for the p key that arms phoenix to live — and a quick
                    // task is a one-off by construction anyway. The agent can
                    // arm the flag through update_tasks once it knows the work.
                    phoenix: false,
                    work_package: self.current_package_id(),
                },
                epic_id,
            },
        )]
    }

    pub(in crate::tui) fn handle_dismiss_error(&mut self) -> Vec<Command> {
        self.status.error_popup = None;
        vec![]
    }

    pub(in crate::tui) fn handle_status_info(&mut self, msg: String) -> Vec<Command> {
        self.status.set(msg);
        vec![]
    }

    pub(in crate::tui) fn handle_toggle_help(&mut self) -> Vec<Command> {
        if self.input.mode == InputMode::Help {
            self.input.mode = InputMode::Normal;
        } else {
            self.interaction.help_scroll = 0;
            self.input.mode = InputMode::Help;
        }
        vec![]
    }

    pub(in crate::tui) fn exit_split_if_active(&mut self) -> Vec<Command> {
        if !self.board.split.active {
            return vec![];
        }
        // Cloned, never taken: the pane id is what a re-attempt needs. Exiting
        // can be refused — `dispatch::restore_pinned_pane` will not assign a
        // name a live window already holds, unless that window is the one the
        // pane was pinned from (split-pane.allium:
        // RefuseExitSplitModeOntoLiveWindow) — and the refusal comes back as an
        // error message, not a state change. `SplitMessage::PaneClosed` is the
        // only confirmation that tmux really did it, and `handle_split_pane_closed`
        // resets the whole `SplitState` when it arrives. Consuming the id here
        // instead left a refused exit with split mode active and no pane id, a
        // state nothing can leave: a second [s] emits no command, a swap bails,
        // and the liveness poll that would raise `PaneClosed` is never issued.
        let pane_id = match self.board.split.right_pane_id.clone() {
            Some(id) => id,
            None => return vec![],
        };
        let restore_window = self
            .board
            .split
            .pinned_task_id
            .and_then(|id| self.board.find_task(id))
            .and_then(|t| t.tmux_window.clone());
        vec![Command::Split(crate::tui::commands::SplitCommand::Exit {
            pane_id,
            restore_window,
        })]
    }

    /// Emit the creation of the drafted task, plus the two settings writes that
    /// record where it was created.
    ///
    /// The repo path is read off the draft rather than passed in: every caller
    /// was passing exactly `draft.repo_path`, and a parameter that can only
    /// hold one value is a chance to pass the wrong one.
    pub(in crate::tui) fn finish_task_creation(&mut self) -> Vec<Command> {
        let mut draft = self.input.task_draft.take().unwrap_or_default();
        let base_branch = draft.base_branch.clone();
        let repo_path = draft.repo_path.clone();
        self.input.mode = InputMode::Normal;
        self.status.clear();
        let epic_id = match self.view().effective_view_mode() {
            BoardViewMode::Epic { epic_id, .. } => Some(epic_id),
            BoardViewMode::Board(_) => None,
            // A package only links: a new task in a package view is a plain
            // main-board task, linked to the package once created
            // (`work-packages.allium: CreateTaskInPackageView`).
            BoardViewMode::Package { package_id, .. } => {
                draft.work_package = Some(package_id);
                None
            }
        };
        vec![
            Command::Task(crate::tui::commands::TaskCommand::Insert { draft, epic_id }),
            Command::Settings(crate::tui::commands::SettingsCommand::SaveRepoPath(
                repo_path.clone(),
            )),
            Command::Settings(crate::tui::commands::SettingsCommand::SaveBaseBranch(
                repo_path,
                base_branch,
            )),
        ]
    }

    /// A dispatch ask ended without writing a request — refused, unreachable,
    /// or given up before asking (a failed repo-trust grant). Drain the pending
    /// entry and touch nothing else (`dispatch.allium: LostClaimReported`): a
    /// refused request holds nothing, and no client releases a claim.
    pub(in crate::tui) fn handle_dispatch_abandoned(&mut self, id: TaskId) -> Vec<Command> {
        self.unmark_dispatching(id);
        vec![]
    }

    pub(in crate::tui) fn handle_mark_dispatching(&mut self, id: TaskId) -> Vec<Command> {
        self.mark_dispatching(id);
        vec![]
    }

    /// This host's dispatch request rows, as the latest board reload read
    /// them (store-split M4).
    ///
    /// - DispatchingFeedback's OPEN membership is rebuilt from the open rows,
    ///   and an open row supersedes the task's pending entry (`SpansTheClaim`).
    /// - A row that reached failed is raised once as an error popup carrying
    ///   its reason (`FailedRequestReported`) — every reason the same way.
    /// - A failed `epic_chain` row for a task this board still holds raises
    ///   the chain's AutoDispatchFailed (`epics.allium:
    ///   ReportChainedDispatchFailure`).
    ///
    /// The FIRST delivery is the baseline: rows already failed in it are
    /// history and raise nothing, so a restart does not re-raise every failure
    /// the store keeps. Failures after it — this board's own startup sweep
    /// ("host restarted") among them — are reported.
    pub(in crate::tui) fn handle_dispatch_requests_refreshed(
        &mut self,
        rows: Vec<crate::models::DispatchRequest>,
    ) -> Vec<Command> {
        use crate::models::{DispatchOrigin, DispatchRequestState};

        let baseline = !self.dispatch_requests.baselined;
        self.dispatch_requests.baselined = true;

        // The subscription delivers only this host's rows
        // (`sync.allium: SubscribeToThisHostsDispatchRequests`).
        let mut open = std::collections::HashMap::new();
        let mut newly_failed = Vec::new();
        for row in rows {
            if row.is_open() {
                open.insert(row.task_id, row.id);
                continue;
            }
            if row.state == DispatchRequestState::Failed
                && self.dispatch_requests.reported.insert(row.id)
                && !baseline
            {
                newly_failed.push(row);
            }
        }
        for task in open.keys() {
            // The ask is answered and its row is in the subscription: the
            // open row carries the membership from here on.
            self.dispatching.remove(task);
        }
        let changed = open != self.dispatch_requests.open;
        self.dispatch_requests.open = open;
        if changed || !newly_failed.is_empty() {
            self.refresh_dispatching_status();
            self.dirty = true;
        }

        let mut cmds = Vec::new();
        if newly_failed.is_empty() {
            return cmds;
        }
        let reason_of = |row: &crate::models::DispatchRequest| {
            row.reason
                .clone()
                .unwrap_or_else(|| "no reason recorded".to_string())
        };
        self.status.error_popup = Some(match newly_failed.as_slice() {
            [row] => format!(
                "Dispatch of task #{} failed: {}",
                row.task_id.0,
                reason_of(row)
            ),
            rows => {
                let each: Vec<String> = rows
                    .iter()
                    .map(|r| format!("#{}: {}", r.task_id.0, reason_of(r)))
                    .collect();
                format!("{} dispatches failed — {}", rows.len(), each.join("; "))
            }
        });
        for row in &newly_failed {
            if row.origin != DispatchOrigin::EpicChain {
                continue;
            }
            // `requires: exists request.task` — a subtask deleted under its
            // request has no card to mark.
            let Some(epic_id) = self.board.find_task(row.task_id).and_then(|t| t.epic_id) else {
                continue;
            };
            cmds.extend(self.handle_auto_dispatch_failed(row.task_id, epic_id, reason_of(row)));
        }
        cmds
    }

    /// An epic's auto-dispatch chain requested a subtask and the request
    /// failed, so the claim was released back to backlog and the epic has
    /// stopped progressing (`ReportChainedDispatchFailure` and
    /// `SurfaceAutoDispatchFailure` in docs/specs/epics.allium).
    ///
    /// Three surfaces, because they fail at different distances from the board:
    /// the status message reaches an operator who is looking at it, the
    /// notification one who is not, and the marker is what is still there an
    /// hour later. Only the marker is durable; see
    /// [`crate::tui::types::AgentTracking::auto_dispatch_failed`].
    ///
    /// No claim is released here: the module released it when the row failed.
    pub(in crate::tui) fn handle_auto_dispatch_failed(
        &mut self,
        task_id: TaskId,
        epic_id: crate::models::EpicId,
        reason: String,
    ) -> Vec<Command> {
        self.status.set(format!(
            "Auto-dispatch of #{} failed — epic #{} stalled: {reason}",
            task_id.0, epic_id.0
        ));
        self.agents.auto_dispatch_failed.insert(task_id, reason);

        if self.notifications_enabled {
            let title = self.board.find_task(task_id).map_or_else(
                || crate::models::task_heading(task_id, None),
                Task::notification_title,
            );
            vec![Command::System(
                crate::tui::commands::SystemCommand::SendNotification {
                    title,
                    body: "Epic auto-dispatch failed — chain stopped".to_string(),
                    // Urgent, unlike a task becoming reviewable: nothing further
                    // happens on this epic until a human acts.
                    urgent: true,
                },
            )]
        } else {
            vec![]
        }
    }

    /// Whether the subtask's last chained dispatch failed and left the epic
    /// stalled (`AutoDispatchFailureIndicator` in docs/specs/epics.allium).
    pub fn auto_dispatch_failed(&self, id: TaskId) -> bool {
        self.agents.auto_dispatch_failed.contains_key(&id)
    }

    pub(in crate::tui) fn handle_description_editor_result(
        &mut self,
        value: String,
    ) -> Vec<Command> {
        match self.input.mode {
            InputMode::InputDescription => {
                super::forms::submit_description(&mut self.input, &mut self.status, value)
            }
            InputMode::InputEpicDescription => {
                super::epic_input::apply_epic_description(&mut self.input, value);
                self.begin_placement(PlacementFor::Epic)
            }
            _ => vec![],
        }
    }

    /// Router for editor results that come back from a pop-out editor. Each
    /// `EditKind` is finalized by an `EditorCommand::FinalizeResult` command
    /// dispatched to the runtime, except the `Description` variant which
    /// threads straight through the existing description-flow messages.
    pub(in crate::tui) fn handle_editor_result(
        &mut self,
        kind: EditKind,
        outcome: EditorOutcome,
    ) -> Vec<Command> {
        use crate::tui::commands::EditorCommand;
        use crate::tui::messages::EditorMessage;
        match (&kind, &outcome) {
            (EditKind::Description { .. }, EditorOutcome::Saved(text)) => {
                let text = crate::editor::parse_description_editor_output(text);
                self.update(Message::Editor(EditorMessage::DescriptionResult(text)))
            }
            (EditKind::Description { .. }, EditorOutcome::Cancelled) => self.update(
                Message::Input(crate::tui::messages::InputMessage::CancelInput),
            ),
            _ => vec![Command::Editor(EditorCommand::FinalizeResult {
                kind,
                outcome,
            })],
        }
    }

    pub(in crate::tui) fn handle_open_in_browser(&self, url: String) -> Vec<Command> {
        vec![Command::System(
            crate::tui::commands::SystemCommand::OpenInBrowser { url },
        )]
    }
}
