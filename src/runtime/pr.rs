use super::*;

impl TuiRuntime {
    pub(super) fn exec_check_pr_status(&self, id: TaskId, pr_url: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            match dispatch::check_pr_status(&pr_url, &*runner) {
                Ok(status) => {
                    if status.state == dispatch::PrState::Merged {
                        let _ = tx.send(Message::Pr(crate::tui::messages::PrMessage::Merged(id)));
                    } else if status.state == dispatch::PrState::Open {
                        let _ =
                            tx.send(Message::Pr(crate::tui::messages::PrMessage::ReviewState {
                                id,
                                review_decision: status.review_decision,
                            }));
                    }
                    // Closed PRs: no message
                }
                Err(e) => {
                    tracing::warn!(task_id = id.0, "PR status check failed: {e}");
                }
            }
        });
    }

    pub(super) fn exec_merge_pr(&self, id: TaskId, pr_url: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || match dispatch::merge_pr(&pr_url, &*runner) {
            Ok(()) => {
                let _ = tx.send(Message::Pr(crate::tui::messages::PrMessage::Merged(id)));
            }
            Err(e) => {
                let _ = tx.send(Message::Pr(crate::tui::messages::PrMessage::MergeFailed {
                    id,
                    error: e.to_string(),
                }));
            }
        });
    }
}
