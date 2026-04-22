use super::*;

impl TuiRuntime {
    pub(super) fn exec_create_pr(
        &self,
        id: TaskId,
        repo_path: String,
        branch: String,
        base_branch: String,
        title: String,
        description: String,
    ) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            match dispatch::create_pr(
                &repo_path,
                &branch,
                &title,
                &description,
                &base_branch,
                &*runner,
            ) {
                Ok(result) => {
                    let _ = tx.send(Message::PrCreated {
                        id,
                        pr_url: result.pr_url,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::PrFailed {
                        id,
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    pub(super) fn exec_check_pr_status(&self, id: TaskId, pr_url: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            match dispatch::check_pr_status(&pr_url, &*runner) {
                Ok(status) => {
                    if status.state == dispatch::PrState::Merged {
                        let _ = tx.send(Message::PrMerged(id));
                    } else if status.state == dispatch::PrState::Open {
                        let _ = tx.send(Message::PrReviewState {
                            id,
                            review_decision: status.review_decision,
                        });
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
                let _ = tx.send(Message::PrMerged(id));
            }
            Err(e) => {
                let _ = tx.send(Message::MergePrFailed {
                    id,
                    error: e.to_string(),
                });
            }
        });
    }

    pub(super) fn exec_fetch_prs(&self, kind: PrListKind) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        let queries = if kind == PrListKind::Bot {
            self.load_dependabot_queries()
        } else {
            self.load_github_queries(kind.settings_key())
        };

        if queries.is_empty() && kind == PrListKind::Bot {
            let _ = tx.send(Message::PrsFetchFailed(
                kind,
                "Bot queries not configured — press [e] to add repos".to_string(),
            ));
            return;
        }

        tokio::task::spawn_blocking(move || {
            tracing::info!(kind = kind.label(), "fetching PRs via gh");
            match crate::github::fetch_prs(&*runner, &queries) {
                Ok(prs) => {
                    tracing::info!(
                        kind = kind.label(),
                        count = prs.len(),
                        "PRs fetched successfully"
                    );
                    let _ = tx.send(Message::PrsLoaded(kind, prs));
                }
                Err(e) => {
                    tracing::warn!(kind = kind.label(), error = %e, "PR fetch failed");
                    let _ = tx.send(Message::PrsFetchFailed(kind, e));
                }
            }
        });
    }

    pub(super) fn exec_persist_prs(
        &self,
        app: &mut App,
        kind: PrListKind,
        prs: Vec<crate::models::ReviewPr>,
    ) {
        let result = self.database.save_prs(kind.to_pr_kind(), &prs);
        if let Err(e) = result {
            app.update(Message::Error(Self::db_error(
                &format!("persisting {} PRs", kind.label()),
                e,
            )));
        }
    }

    pub(super) fn exec_approve_bot_pr(&self, url: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            tracing::info!(url, "approving PR");
            match runner.run("gh", &["pr", "review", "--approve", &url]) {
                Ok(output) if output.status.success() => {
                    let _ = tx.send(Message::RefreshBotPrs);
                    let _ = tx.send(Message::StatusInfo(format!("Approved PR {url}")));
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::warn!(url, error = %stderr, "failed to approve PR");
                    let _ = tx.send(Message::StatusInfo(format!(
                        "Failed to approve PR: {stderr}"
                    )));
                }
                Err(e) => {
                    tracing::warn!(url, error = %e, "failed to run gh");
                    let _ = tx.send(Message::StatusInfo(format!("Failed to run gh: {e}")));
                }
            }
        });
    }

    pub(super) fn exec_approve_review_pr(&self, url: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            tracing::info!(url, "approving review PR");
            match runner.run("gh", &["pr", "review", "--approve", &url]) {
                Ok(output) if output.status.success() => {
                    let _ = tx.send(Message::RefreshReviewPrs);
                    let _ = tx.send(Message::StatusInfo(format!("Approved PR {url}")));
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::warn!(url, error = %stderr, "failed to approve review PR");
                    let _ = tx.send(Message::StatusInfo(format!(
                        "Failed to approve PR: {stderr}"
                    )));
                }
                Err(e) => {
                    tracing::warn!(url, error = %e, "failed to run gh");
                    let _ = tx.send(Message::StatusInfo(format!("Failed to approve PR: {e}")));
                }
            }
        });
    }

    pub(super) fn exec_merge_review_pr(&self, url: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            tracing::info!(url, "merging review PR");
            match runner.run("gh", &["pr", "merge", "--squash", &url]) {
                Ok(output) if output.status.success() => {
                    let _ = tx.send(Message::RefreshReviewPrs);
                    let _ = tx.send(Message::StatusInfo(format!("Merged PR {url}")));
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::warn!(url, error = %stderr, "failed to merge review PR");
                    let _ = tx.send(Message::StatusInfo(format!("Failed to merge PR: {stderr}")));
                }
                Err(e) => {
                    tracing::warn!(url, error = %e, "failed to run gh");
                    let _ = tx.send(Message::StatusInfo(format!("Failed to merge PR: {e}")));
                }
            }
        });
    }

    pub(super) fn exec_persist_review_agent(
        &self,
        app: &mut App,
        pr_kind: db::PrKind,
        github_repo: String,
        number: i64,
        tmux_window: String,
        worktree: String,
    ) -> Vec<Command> {
        if let Err(e) =
            self.database
                .set_pr_agent(pr_kind, &github_repo, number, &tmux_window, &worktree)
        {
            return app.update(Message::Error(format!(
                "Failed to persist review agent: {e}"
            )));
        }
        vec![]
    }

    pub(super) fn exec_merge_bot_pr(&self, url: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            tracing::info!(url, "merging PR");
            match runner.run("gh", &["pr", "merge", "--squash", &url]) {
                Ok(output) if output.status.success() => {
                    let _ = tx.send(Message::BotPrsMerged(vec![url.clone()]));
                    let _ = tx.send(Message::StatusInfo(format!("Merged PR {url}")));
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::warn!(url, error = %stderr, "failed to merge PR");
                    let _ = tx.send(Message::StatusInfo(format!("Failed to merge PR: {stderr}")));
                }
                Err(e) => {
                    tracing::warn!(url, error = %e, "failed to run gh");
                    let _ = tx.send(Message::StatusInfo(format!("Failed to run gh: {e}")));
                }
            }
        });
    }
}
