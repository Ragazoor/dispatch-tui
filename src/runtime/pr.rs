use super::*;

impl TuiRuntime {
    pub(super) fn exec_check_pr_status(
        &self,
        id: TaskId,
        pr_url: String,
    ) -> tokio::task::JoinHandle<()> {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || match dispatch::check_pr_status(&pr_url, &*runner) {
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
            Err(e) => {
                tracing::warn!(task_id = id.0, "PR status check failed: {e}");
            }
        })
    }
}
