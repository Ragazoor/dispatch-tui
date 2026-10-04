use super::poll_ownership::{decide_poll_action, PollAction};
use super::*;

impl TuiRuntime {
    pub(super) fn exec_check_pr_status(
        &self,
        id: TaskId,
        url: String,
    ) -> tokio::task::JoinHandle<()> {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            let result = dispatch::check_pr_status(&url, &*runner);
            send_pr_check_result(&tx, id, result);
        })
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
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::spawn(async move {
            let owner = match board_reads
                .poll_owner(crate::models::PollScopeId::Task(id))
                .await
            {
                Ok(owner) => owner,
                // A failed read is "cannot tell", and skipping is the side to
                // err on: it costs a delayed poll, never a duplicated one.
                Err(e) => {
                    tracing::debug!(
                        task_id = id.0,
                        "failed to read PR-poll ownership, skipping this tick: {e:#}"
                    );
                    return;
                }
            };
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
            send_pr_check_result(&tx, id, result);
        })
    }
}

/// Turn a `check_pr_status` outcome into the `Message::Pr` variant it maps to
/// and send it. Shared by [`TuiRuntime::exec_check_pr_status`] (unconditional)
/// and [`TuiRuntime::exec_check_status_if_owned`] (after the ownership check).
fn send_pr_check_result(
    tx: &tokio::sync::mpsc::UnboundedSender<Message>,
    id: TaskId,
    result: Result<dispatch::PrStatus, dispatch::PrCheckFailure>,
) {
    match result {
        Ok(status) => match status.state {
            dispatch::PrState::Merged => {
                let _ = tx.send(Message::Pr(crate::tui::messages::PrMessage::Merged(id)));
            }
            dispatch::PrState::Closed => {
                let _ = tx.send(Message::Pr(crate::tui::messages::PrMessage::Closed(id)));
            }
            dispatch::PrState::Open => {
                let _ = tx.send(Message::Pr(crate::tui::messages::PrMessage::ReviewState {
                    id,
                    review_decision: status.review_decision,
                }));
            }
        },
        // Deliberately NOT logged here. This ran once per task per
        // PR_POLL_INTERVAL, so a permanently unreadable PR warned every 30
        // seconds — 63,000 identical lines from five tasks over five
        // months. The failure now travels to the update loop, which counts
        // it and warns once, on the transition into giving up
        // (pr-workflow.allium: PrPollGaveUp).
        Err(failure) => {
            let _ = tx.send(Message::Pr(crate::tui::messages::PrMessage::CheckFailed {
                id,
                permanent: failure.is_permanent(),
                error: failure.to_string(),
            }));
        }
    }
}
