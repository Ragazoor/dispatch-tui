//! The board's host reactor for dispatch request rows (store-split M4).
//!
//! Spec: `docs/specs/store-split.allium` — `HostClaimsDispatch`,
//! `HostRestartFailsInterrupted` and D5 — and `docs/specs/dispatch.allium:
//! ProvisionDispatch`. The rows naming this host arrive through the
//! subscription (`sync.allium: SubscribeToThisHostsDispatchRequests`); the
//! reactor hands each `requested` one, once, to
//! [`TaskServiceApi::serve_dispatch_request`](crate::service::TaskServiceApi::serve_dispatch_request)
//! — the one place the board provisions a dispatch — and tells the App what
//! landed. It runs after every board reload the row pump does, and once at
//! startup behind the restart sweep.

use super::*;

/// What the reactor needs, cloned into the row pump so a pushed row is served
/// without going through the event loop.
#[derive(Clone)]
pub(super) struct DispatchHost {
    database: Arc<dyn store::TaskReadStore>,
    task_svc: Arc<dyn crate::service::TaskServiceApi>,
    emb_svc: Arc<EmbeddingService>,
    msg_tx: mpsc::UnboundedSender<Message>,
    /// Shared by every clone, so a row is handed out once whichever wake-up
    /// saw it first.
    observer: Arc<std::sync::Mutex<crate::dispatch::DispatchRequestObserver>>,
}

impl DispatchHost {
    /// Serve every `requested` row naming this host that has not been served
    /// yet, each on its own task. The handles are for tests; production drops
    /// them.
    pub(super) async fn serve(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let rows = self.database.dispatch_requests().await;
        let to_serve = self
            .observer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observe(&rows);
        to_serve
            .into_iter()
            .map(|row| {
                let host = self.clone();
                tokio::spawn(async move { host.serve_one(row).await })
            })
            .collect()
    }

    async fn serve_one(self, row: models::DispatchRequest) {
        let task_id = row.task_id;
        // Quick dispatch is the one launch path that jumps to the new agent
        // (tasks.allium: CreateQuickTask).
        let switch_focus = row.origin == models::DispatchOrigin::Quick;
        match self
            .task_svc
            .serve_dispatch_request(row, Arc::clone(&self.emb_svc))
            .await
        {
            crate::service::ServedDispatch::Dispatched(dr) => {
                let _ = self.msg_tx.send(Message::Task(
                    crate::tui::messages::TaskMessage::Dispatched {
                        id: task_id,
                        worktree: dr.worktree_path,
                        tmux_window: dr.tmux_window,
                        switch_focus,
                    },
                ));
            }
            // A failure reaches the App through the row itself
            // (`FailedRequestReported`), whichever way it failed, so nothing
            // is sent from here.
            crate::service::ServedDispatch::Failed(reason) => {
                tracing::info!(task_id = task_id.0, "dispatch request failed: {reason}");
            }
            crate::service::ServedDispatch::ClaimRefused(why) => {
                tracing::info!(task_id = task_id.0, "dispatch request not claimed: {why}");
            }
            crate::service::ServedDispatch::NotThisHost => {}
        }
    }
}

impl TuiRuntime {
    pub(super) fn dispatch_host(&self) -> DispatchHost {
        DispatchHost {
            database: Arc::clone(&self.database),
            task_svc: Arc::clone(&self.task_svc),
            emb_svc: Arc::clone(&self.emb_svc),
            msg_tx: self.msg_tx.clone(),
            observer: Arc::clone(&self.dispatch_observer),
        }
    }

    /// `HostClaimsDispatch` for every `requested` row naming this host that
    /// has not been served yet.
    pub(super) async fn serve_dispatch_requests(&self) -> Vec<tokio::task::JoinHandle<()>> {
        self.dispatch_host().serve().await
    }

    /// Startup, on the first connection: `HostRestartFailsInterrupted`
    /// first — the rows a previous board left `provisioning` fail "host
    /// restarted", releasing their claims — THEN the D5 catch-up on the rows
    /// still `requested`. In that order, so a board never fails its own fresh
    /// claim.
    pub(super) async fn recover_dispatch_requests(&self) -> Vec<tokio::task::JoinHandle<()>> {
        if let Err(e) = self.task_svc.fail_interrupted_dispatch_requests().await {
            tracing::warn!("could not fail this host's interrupted dispatch requests: {e}");
        }
        self.serve_dispatch_requests().await
    }
}
