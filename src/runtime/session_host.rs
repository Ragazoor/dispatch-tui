//! The board's host reactor for session request rows (store-split M5).
//!
//! Spec: `docs/specs/store-split.allium` — `HostClaimsSessionRequest`,
//! `HostRestartFailsInterruptedSessionRequests` and W8 — and
//! `docs/specs/pr-workflow.allium`: `WrapUpRebase`, `ExitSessionTeardown`,
//! `KillWindowOfFailedExitSession`. The rows naming this host arrive through
//! the subscription (`sync.allium: SubscribeToThisHostsSessionRequests`); the
//! reactor hands each `requested` one, once, to
//! [`TaskServiceApi::serve_session_request`](crate::service::TaskServiceApi::serve_session_request)
//! — the one place the board rebases for a wrap-up or tears a closed session's
//! window down — and kills the window of an exit_session row it sees fail. It
//! runs after every board reload the row pump does, and once at startup behind
//! the restart sweep.

use super::*;

/// What the reactor needs, cloned into the row pump so a pushed row is served
/// without going through the event loop.
#[derive(Clone)]
pub(super) struct SessionHost {
    database: Arc<dyn store::TaskReadStore>,
    task_svc: Arc<dyn crate::service::TaskServiceApi>,
    runner: Arc<dyn ProcessRunner>,
    msg_tx: mpsc::UnboundedSender<Message>,
    /// Shared by every clone, so a row is handed out once whichever wake-up saw
    /// it first.
    observer: Arc<std::sync::Mutex<crate::dispatch::SessionRequestObserver>>,
}

impl SessionHost {
    /// Serve every `requested` row naming this host that has not been served
    /// yet, each on its own task, and kill the windows of the exit_session rows
    /// seen failing. The handles are for tests; production drops them.
    ///
    /// Only rows this host's own owner requested are acted on
    /// (`pr-workflow.allium: WrapUpRebase`, D11's second line behind the
    /// module's check): a row naming this host but requested by anyone else is
    /// ignored and logged. The owner is the identity this board's own connection
    /// settled as — what the module stamps on this board's requests; a board
    /// with no settled identity acts on nothing.
    #[cfg(test)]
    pub(super) async fn serve(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let rows = self.database.session_requests().await;
        self.serve_rows(&rows).await
    }

    /// What `serve` does, over rows the caller already read.
    pub(super) async fn serve_rows(
        &self,
        rows: &[models::SessionRequest],
    ) -> Vec<tokio::task::JoinHandle<()>> {
        let work = self.observer().observe(rows);
        self.act(work, false).await
    }

    /// Do what `work` asks: kill the windows of failed exit_session rows and
    /// serve the requested ones, each on its own task. `kills_first` awaits the
    /// kills before any row is served, so a rebase cannot race them for tmux
    /// (startup).
    async fn act(
        &self,
        work: crate::dispatch::SessionWork,
        kills_first: bool,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        if work.serve.is_empty() && work.tear_down.is_empty() {
            return Vec::new();
        }
        let Some(owner) = self.database.own_identity().await else {
            tracing::warn!("this board has no settled identity yet; acting on no session requests");
            // Held back, not acted on: the next reload after the identity
            // settles hands them out again.
            let mut observer = self.observer();
            for row in &work.serve {
                observer.release(row.id);
            }
            return Vec::new();
        };
        let ours = |row: &models::SessionRequest| {
            let ours = row.requested_by == owner;
            if !ours {
                tracing::warn!(
                    request = row.id.0,
                    task_id = row.task_id.0,
                    requested_by = %row.requested_by,
                    "ignoring a session request this host's owner did not make"
                );
            }
            ours
        };
        let mut handles = Vec::new();
        let mut kills = Vec::new();
        for row in work.tear_down.into_iter().filter(|r| ours(r)) {
            let svc = Arc::clone(&self.task_svc);
            kills.push(tokio::spawn(async move {
                svc.tear_down_failed_exit_window(&row).await;
            }));
        }
        if kills_first {
            for kill in kills.drain(..) {
                let _ = kill.await;
            }
        }
        handles.append(&mut kills);
        for row in work.serve.into_iter().filter(|r| ours(r)) {
            let host = self.clone();
            handles.push(tokio::spawn(async move { host.serve_one(row).await }));
        }
        handles
    }

    fn observer(&self) -> std::sync::MutexGuard<'_, crate::dispatch::SessionRequestObserver> {
        self.observer.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn serve_one(self, row: models::SessionRequest) {
        use crate::service::SessionOutcome;

        let request = row.id;
        let task_id = row.task_id;
        let served = self.task_svc.serve_session_request(row).await;
        // `BranchRebased`: the base branch moved on this disk whether or not the
        // module accepted the report, so the repository's drift is stale.
        if let Some(repo_path) = served.rebased {
            drop(super::repo_sync::spawn_repo_measurement(
                self.msg_tx.clone(),
                Arc::clone(&self.runner),
                repo_path,
                false,
            ));
        }
        match served.outcome {
            SessionOutcome::Done => {
                tracing::info!(
                    task_id = task_id.0,
                    request = request.0,
                    "session request done"
                );
            }
            // A failure reaches the waiting `wrap_up` through the row itself,
            // so nothing is sent from here.
            SessionOutcome::Failed(reason) => {
                tracing::info!(task_id = task_id.0, "session request failed: {reason}");
            }
            SessionOutcome::ClaimRefused(why) => {
                tracing::info!(task_id = task_id.0, "session request not claimed: {why}");
            }
            // Only an answered claim marks a row served: one that never
            // reached the module is retried on the next reload.
            SessionOutcome::ClaimNotSent(why) => {
                tracing::warn!(
                    task_id = task_id.0,
                    "could not send the claim of a session request; retrying on the next reload: {why}"
                );
                self.observer().release(request);
            }
        }
    }
}

impl TuiRuntime {
    pub(super) fn session_host(&self) -> SessionHost {
        SessionHost {
            database: Arc::clone(&self.database),
            task_svc: Arc::clone(&self.task_svc),
            runner: Arc::clone(&self.runner),
            msg_tx: self.msg_tx.clone(),
            observer: Arc::clone(&self.session_observer),
        }
    }

    /// `HostClaimsSessionRequest` for every `requested` row naming this host
    /// that has not been served yet, and `KillWindowOfFailedExitSession` for
    /// every exit_session row seen failing.
    #[cfg(test)]
    pub(super) async fn serve_session_requests(&self) -> Vec<tokio::task::JoinHandle<()>> {
        self.session_host().serve().await
    }

    /// Startup, on the first connection: note the rows a previous board left
    /// open, run `HostRestartFailsInterruptedSessionRequests` — which fails the
    /// rows left `running` ("host restarted") — kill the windows of the
    /// exit_session rows that failed (best effort, and before anything else
    /// touches tmux), THEN catch up on the rows still `requested` (W8). In that
    /// order, so a board never fails its own fresh claim.
    pub(super) async fn recover_session_requests(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let host = self.session_host();
        let before = self.database.session_requests().await;
        host.observer().note_open(&before);
        if let Err(e) = self.task_svc.fail_interrupted_session_requests().await {
            tracing::warn!("could not fail this host's interrupted session requests: {e}");
        }
        // The kills first and awaited: a rebase served below must not race them
        // for tmux.
        let after = self.database.session_requests().await;
        let work = host.observer().observe(&after);
        host.act(work, true).await
    }
}
