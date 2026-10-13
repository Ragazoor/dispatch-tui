//! PR-related message handlers: review state polling.

use crate::models::{SubStatus, TaskId, TaskStatus, TmuxWindow};

use super::super::types::*;
use super::super::App;
use crate::tui::{PR_POLL_BACKOFF_MAX, PR_POLL_INTERVAL, PR_POLL_PERMANENT_FAILURE_THRESHOLD};
use std::time::{Duration, Instant};

/// How long to wait before the nth consecutive transient failure is retried:
/// `PR_POLL_INTERVAL` doubled per failure, capped at `PR_POLL_BACKOFF_MAX`.
///
/// The cap is what keeps a long GitHub outage from pushing a task's next
/// attempt past the point anyone would still care, while the doubling is what
/// stops a spent rate limit costing one call per task per tick.
fn transient_backoff(consecutive_transient_failures: u32) -> Duration {
    crate::backoff::exponential(
        PR_POLL_INTERVAL,
        PR_POLL_BACKOFF_MAX,
        consecutive_transient_failures,
    )
}

impl App {
    /// The desktop notification, when notifications are enabled, for a PR event
    /// the module has just recorded. The task write itself is the module's
    /// (`store-split.allium: ReportPrStatus`); the board's in-memory task moves
    /// when the store's row arrives, so nothing here mutates it.
    fn pr_notification(&self, title: &str, body: String) -> Option<Command> {
        self.notifications_enabled.then(|| {
            Command::System(crate::tui::commands::SystemCommand::SendNotification {
                title: title.to_string(),
                body,
                urgent: false,
            })
        })
    }

    /// The module accepted a merged PR and moved the task to Done. What is left
    /// is host work: kill the window the task had (read before the report, since
    /// the module cleared it), tell the user and free the split pane. The task
    /// keeps its worktree.
    ///
    /// Acts on a Review task, or a Done one: the store's row can reach the board
    /// before this message does.
    pub(in crate::tui) fn handle_pr_merged(
        &mut self,
        id: TaskId,
        window: Option<TmuxWindow>,
    ) -> Vec<Command> {
        let mut cmds = Vec::new();
        self.clear_pr_poll_failures(id);

        if let Some(task) = self.find_task_mut(id) {
            if !matches!(task.status, TaskStatus::Review | TaskStatus::Done) {
                return cmds;
            }

            let pr_label = task.url.as_ref().map_or("PR".to_string(), |u| u.label());
            let task_title = task.title.clone();

            if let Some(window) = window {
                cmds.push(Command::Task(
                    crate::tui::commands::TaskCommand::KillTmuxWindow { window },
                ));
            }

            self.clear_agent_tracking(id);
            self.sync_board_selection();
            self.status.set(format!(
                "{pr_label} merged \u{2014} task #{id} moved to Done"
            ));

            cmds.extend(
                self.pr_notification("PR merged", format!("{pr_label} merged: {task_title}")),
            );
        }

        cmds.extend(self.maybe_respawn_split_pane(id));

        cmds
    }

    /// A closed-without-merge PR is NOT a terminal event: the module only
    /// sets `sub_status` to `pr_closed`, so the user notices and decides what
    /// to do (reopen the PR, push a new one, move it to Done and delete it).
    /// No tmux/worktree teardown, unlike `PrMerged`.
    ///
    /// `PollPrStatus` re-reports this every tick the PR stays closed; the runtime
    /// sends this message only for the report that changed the task (it read the
    /// sub-status before posting), so no guard on the board's own copy, which the
    /// store's row may already have moved.
    pub(in crate::tui) fn handle_pr_closed(&mut self, id: TaskId) -> Vec<Command> {
        self.clear_pr_poll_failures(id);

        let Some(task) = self.find_task_mut(id) else {
            return vec![];
        };
        if task.status != TaskStatus::Review {
            return vec![];
        }
        let pr_label = task.url.as_ref().map_or("PR".to_string(), |u| u.label());
        let task_title = task.title.clone();

        self.status.set(format!(
            "{pr_label} closed \u{2014} task #{id} marked \"PR closed\""
        ));

        self.pr_notification("PR closed", format!("{pr_label} closed: {task_title}"))
            .into_iter()
            .collect()
    }

    /// Record a failed PR read, and give up on the task once permanent failures
    /// reach the threshold.
    ///
    /// The counting is host pacing (`store-split.allium`, P3): this machine's
    /// own `gh` calls and its backoff, session-scoped on purpose. Giving up
    /// posts a `ReportUnreadable` and the module marks the task
    /// `pr_unreachable`.
    ///
    /// The log line is the point of the split. Before this, every failed attempt
    /// warned, so five PRs the `gh` account could not read produced 63,000
    /// identical lines over five months and told the user nothing. Now a
    /// permanent failure warns once, on the transition into giving up, and a
    /// transient one only widens the backoff at debug level.
    pub(in crate::tui) fn handle_pr_check_failed(
        &mut self,
        id: TaskId,
        permanent: bool,
        error: String,
    ) -> Vec<Command> {
        let state = self.agents.pr_poll.entry(id).or_default();

        if !permanent {
            // Transient: the permanent counter is deliberately left alone — a
            // network blip must never accumulate towards stranding a task —
            // and only the deadline moves.
            state.consecutive_transient_failures += 1;
            let wait = transient_backoff(state.consecutive_transient_failures);
            state.next_poll_at = Some(Instant::now() + wait);
            tracing::debug!(
                task_id = id.0,
                retry_in_s = wait.as_secs(),
                "PR status check failed transiently, backing off: {error}"
            );
            return vec![];
        }

        state.consecutive_permanent_failures += 1;
        if state.consecutive_permanent_failures < PR_POLL_PERMANENT_FAILURE_THRESHOLD {
            tracing::debug!(
                task_id = id.0,
                failures = state.consecutive_permanent_failures,
                "PR status check failed permanently, below give-up threshold: {error}"
            );
            return vec![];
        }
        // `gave_up` is derived from this same counter (see PrPollState::gave_up),
        // so the only way to tell "just now crossed the threshold" from "already
        // given up, this is a late in-flight result" is the exact value: the
        // counter increments by one per permanent failure, so it passes through
        // the threshold exactly once.
        if state.consecutive_permanent_failures > PR_POLL_PERMANENT_FAILURE_THRESHOLD {
            return vec![];
        }

        tracing::warn!(
            task_id = id.0,
            failures = PR_POLL_PERMANENT_FAILURE_THRESHOLD,
            "giving up on PR status polling for this task: {error}"
        );

        let Some(task) = self.find_task_mut(id) else {
            return vec![];
        };
        if task.status != TaskStatus::Review || task.sub_status == SubStatus::PrUnreachable {
            return vec![];
        }
        let pr_label = task.url.as_ref().map_or("PR".to_string(), |u| u.label());
        let task_title = task.title.clone();

        self.status.set(format!(
            "{pr_label} unreadable \u{2014} task #{id} marked \"PR unreachable\""
        ));

        let mut cmds = vec![Command::Pr(
            crate::tui::commands::PrCommand::ReportUnreadable { id, reason: error },
        )];
        cmds.extend(self.pr_notification(
            "PR unreachable",
            format!("{pr_label} could not be read: {task_title}"),
        ));
        cmds
    }

    /// Clear every failure record for a task whose PR was just read
    /// successfully. This is the only path out of `gave_up`.
    fn clear_pr_poll_failures(&mut self, id: TaskId) {
        if let Some(state) = self.agents.pr_poll.get_mut(&id) {
            *state = crate::tui::types::PrPollState::default();
        }
    }

    /// An open PR was read and the module set the review sub-status
    /// (`ReportPrStatus`). Nothing is decided or written here.
    pub(in crate::tui) fn handle_pr_review_state(&mut self, id: TaskId) -> Vec<Command> {
        self.clear_pr_poll_failures(id);
        vec![]
    }
}
