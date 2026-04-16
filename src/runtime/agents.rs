use super::*;

impl TuiRuntime {
    pub(super) fn exec_dispatch_fix_agent(&self, req: tui::FixAgentRequest) {
        // repo is already resolved to a local path by the TUI
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            let github_repo = req.github_repo.clone();
            let number = req.number;
            let kind = req.kind;
            match dispatch::dispatch_fix_agent(req, &*runner) {
                Ok(result) => {
                    let _ = tx.send(Message::FixAgentDispatched {
                        github_repo,
                        number,
                        kind,
                        tmux_window: result.tmux_window,
                        worktree: result.worktree_path,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::FixAgentFailed {
                        github_repo,
                        number,
                        kind,
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    pub(super) fn exec_dispatch_review_agent(&self, req: ReviewAgentRequest) {
        // repo is already resolved to a local path by the TUI
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            match crate::dispatch::dispatch_review_agent(&req, &*runner) {
                Ok(result) => {
                    let _ = tx.send(Message::ReviewAgentDispatched {
                        github_repo: req.github_repo,
                        number: req.number,
                        tmux_window: result.tmux_window,
                        worktree: result.worktree_path,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::ReviewAgentFailed {
                        github_repo: req.github_repo,
                        number: req.number,
                        error: format!("{e:#}"),
                    });
                }
            }
        });
    }
}
