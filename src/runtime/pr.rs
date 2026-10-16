use super::*;
use crate::models::{decide_poll_action, PollAction};

impl TuiRuntime {
    pub(super) fn exec_check_pr_status(
        &self,
        id: TaskId,
        url: String,
    ) -> tokio::task::JoinHandle<()> {
        let reporter = self.pr_reporter();
        let runner = self.runner.clone();

        tokio::spawn(async move {
            let Ok(result) =
                tokio::task::spawn_blocking(move || dispatch::check_pr_status(&url, &*runner))
                    .await
            else {
                return;
            };
            reporter.report(id, result).await;
        })
    }

    /// `PrCommand::ReportUnreadable`: the poll gave up on this task's PR
    /// (`store-split.allium: ReportPrUnreadable`). The module marks the task
    /// `pr_unreachable`; nothing is decided here.
    pub(super) fn exec_report_pr_unreadable(
        &self,
        id: TaskId,
        reason: String,
    ) -> tokio::task::JoinHandle<()> {
        let task_svc = self.task_svc.clone();
        tokio::spawn(async move {
            if let Err(e) = task_svc.report_pr_unreadable(id, reason).await {
                tracing::warn!(task_id = id.0, "could not report an unreadable PR: {e:#}");
            }
        })
    }

    fn pr_reporter(&self) -> PrReporter {
        PrReporter {
            task_svc: self.task_svc.clone(),
            database: self.database.clone(),
            tx: self.msg_tx.clone(),
        }
    }

    /// `PrCommand::CheckStatusIfOwned` — a host-less review task's PR check
    /// (`pr-workflow.allium: PollPrStatus`, "Host scoping"): reads
    /// `core/PollOwner` for the task before doing anything else, claims it if
    /// absent, and only polls when this host owns the row afterward.
    ///
    /// The predict-then-verify shape the design doc calls for: a host that
    /// sees no owner claims it and proceeds OPTIMISTICALLY in the same call —
    /// it does not wait for its own claim to round-trip back through the
    /// subscription before acting. A genuine simultaneous claim by two hosts
    /// is resolved by whichever reducer call the store serialises first; the
    /// loser's NEXT tick sees the winner's row and stands down. A single
    /// doubled poll at that exact instant costs one extra `gh` call — the
    /// same order of cost `PollPrStatus`'s own transient-failure tolerance
    /// already accepts.
    pub(super) fn exec_check_status_if_owned(
        &self,
        id: TaskId,
        url: String,
    ) -> tokio::task::JoinHandle<()> {
        let board_reads = self.board_reads.clone();
        let database = self.database.clone();
        let host_id = self.host_id.clone();
        let reporter = self.pr_reporter();
        let runner = self.runner.clone();

        tokio::spawn(async move {
            let owner = board_reads
                .poll_owner(crate::models::PollScopeId::Task(id))
                .await;
            match decide_poll_action(owner.as_deref(), &host_id) {
                PollAction::Skip => return,
                // A failed claim call does not block the poll below — the
                // claim is best-effort, and the worst case of proceeding
                // anyway is the same harmless double-poll a genuine race
                // would cause.
                PollAction::ClaimAndProceed => {
                    if let Err(e) = database
                        .claim_poll_owner(crate::models::PollScopeId::Task(id))
                        .await
                    {
                        tracing::debug!(
                            task_id = id.0,
                            "failed to claim PR-poll ownership, polling anyway: {e:#}"
                        );
                    }
                }
                PollAction::Proceed => {}
            }

            let Ok(result) =
                tokio::task::spawn_blocking(move || dispatch::check_pr_status(&url, &*runner))
                    .await
            else {
                return;
            };
            reporter.report(id, result).await;
        })
    }
}

/// What turns a `check_pr_status` outcome into a report to the module and the
/// `Message::Pr` the board's own reaction needs. Shared by
/// [`TuiRuntime::exec_check_pr_status`] (unconditional) and
/// [`TuiRuntime::exec_check_status_if_owned`] (after the ownership check).
///
/// The host interprets nothing (`store-split.allium: ReportPrStatus`): the raw
/// strings go to the module, which writes the task. The message sent after an
/// accepted report only picks the host's own effects (kill the window, notify,
/// clear the failure counters).
struct PrReporter {
    task_svc: Arc<dyn crate::service::TaskServiceApi>,
    database: Arc<dyn store::TaskReadStore>,
    tx: tokio::sync::mpsc::UnboundedSender<Message>,
}

impl PrReporter {
    async fn report(
        &self,
        id: TaskId,
        result: Result<dispatch::RawPrStatus, dispatch::PrCheckFailure>,
    ) {
        use crate::tui::messages::PrMessage;
        let message = match result {
            Ok(status) => self.report_raw_status(id, status).await,
            // Deliberately NOT logged here. This ran once per task per
            // PR_POLL_INTERVAL, so a permanently unreadable PR warned every 30
            // seconds — 63,000 identical lines from five tasks over five
            // months. The failure now travels to the update loop, which counts
            // it and warns once, on the transition into giving up
            // (pr-workflow.allium: PrPollGaveUp).
            Err(failure) => PrMessage::CheckFailed {
                id,
                permanent: failure.is_permanent(),
                error: failure.to_string(),
            },
        };
        let _ = self.tx.send(Message::Pr(message));
    }

    /// Post a raw PR status to the module and pick the host's follow-up message.
    async fn report_raw_status(
        &self,
        id: TaskId,
        status: dispatch::RawPrStatus,
    ) -> crate::tui::messages::PrMessage {
        use crate::tui::messages::PrMessage;
        // Read BEFORE posting: a merge clears the window in the store,
        // and the host still has to kill it.
        // Only a merge or a close has host effects, so only they pay
        // for the read; an open PR goes straight to the post.
        let effectful = matches!(status.state.to_uppercase().as_str(), "MERGED" | "CLOSED");
        let before = if effectful {
            self.database.get_task(id).await
        } else {
            None
        };
        let window = before.as_ref().and_then(|task| task.tmux_window.clone());
        // A closed PR the task already shows as closed changed nothing:
        // no second notice. Decided here, from the row read before the
        // post, because the store's row can reach the board before this
        // message does.
        let already_closed = before
            .as_ref()
            .is_some_and(|task| task.sub_status == crate::models::SubStatus::PrClosed);
        match self
            .task_svc
            .report_pr_status(id, status.state.clone(), status.review_decision.clone())
            .await
        {
            Ok(true) => match status.state.to_uppercase().as_str() {
                "MERGED" => PrMessage::Merged { id, window },
                "CLOSED" if !already_closed => PrMessage::Closed(id),
                _ => PrMessage::ReviewState(id),
            },
            // A state or review decision the module does not know stays
            // unknown however often it is retried.
            Ok(false) => PrMessage::CheckFailed {
                id,
                permanent: true,
                error: format!(
                    "the store does not know PR state {:?} / review decision {:?}",
                    status.state, status.review_decision
                ),
            },
            Err(e) => PrMessage::CheckFailed {
                id,
                permanent: false,
                error: format!("could not report the PR status: {e:#}"),
            },
        }
    }
}
