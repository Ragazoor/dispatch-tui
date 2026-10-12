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
    ///
    /// Only rows this host's own owner requested are served
    /// (`dispatch.allium: ProvisionDispatch`, D11's second line behind the
    /// module's check): a row naming this host but requested by anyone else
    /// is ignored and logged, never provisioned. The owner is the identity
    /// this board's own connection settled as — what the module stamps on
    /// this board's requests — not the shared registry a client can write; a
    /// board with no settled identity serves nothing.
    pub(super) async fn serve(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let rows = self.database.dispatch_requests().await;
        self.serve_rows(&rows).await
    }

    /// [`Self::serve`] over rows the caller already read — the row pump's
    /// reload reads them once for the App and for this.
    pub(super) async fn serve_rows(
        &self,
        rows: &[models::DispatchRequest],
    ) -> Vec<tokio::task::JoinHandle<()>> {
        let to_serve = self.observer().observe(rows);
        if to_serve.is_empty() {
            return Vec::new();
        }
        let Some(owner) = self.database.own_identity().await else {
            tracing::warn!("this board has no settled identity yet; serving no dispatch requests");
            // Held back, not served: the next reload after the identity
            // settles hands them out again.
            let mut observer = self.observer();
            for row in &to_serve {
                observer.release(row.id);
            }
            return Vec::new();
        };
        to_serve
            .into_iter()
            .filter(|row| {
                let ours = row.requested_by == owner;
                if !ours {
                    tracing::warn!(
                        request = row.id.0,
                        task_id = row.task_id.0,
                        requested_by = %row.requested_by,
                        "ignoring a dispatch request this host's owner did not make"
                    );
                }
                ours
            })
            .map(|row| {
                let host = self.clone();
                tokio::spawn(async move { host.serve_one(row).await })
            })
            .collect()
    }

    fn observer(&self) -> std::sync::MutexGuard<'_, crate::dispatch::DispatchRequestObserver> {
        self.observer.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn serve_one(self, row: models::DispatchRequest) {
        let task_id = row.task_id;
        let request = row.id;
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
            // Only an answered claim marks a row served: one that never
            // reached the module is retried on the next reload.
            crate::service::ServedDispatch::ClaimNotSent(why) => {
                tracing::warn!(
                    task_id = task_id.0,
                    "could not send the claim of a dispatch request; retrying on the next reload: {why}"
                );
                self.observer().release(request);
            }
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
