//! Which store the board connects to, and the guards that tidy up after it:
//! a named store or dispatch's managed one, brought up, health-watched and
//! stopped (or its record forgotten) on every way out of `run_tui`.

use anyhow::Result;
use crossterm::terminal::disable_raw_mode;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::interval;

use crate::store_connection::{
    check_store_identity, connect_first, pin_store_after_connect, StoreParts,
};
use crate::tui::Message;

/// The first connection: resolve the store's address, refuse a store this
/// install does not use, connect, and record the address that answered.
/// Returns the address and the open session.
pub(super) async fn connect_to_store(
    target: &StoreTarget,
    data_dir: &Path,
    parts: &StoreParts,
    accept_store_switch: bool,
) -> Result<(String, crate::sync::SyncSession)> {
    let server = resolve_store_server(target).await?;
    // Before connecting: the first connection already writes, so a store
    // holding a different database from the one this install last used is
    // refused here (`AbortWhenTheStoreIsNotTheOneThisInstallUses`, task
    // #28710).
    let found = check_store_identity(parts, data_dir, &server, accept_store_switch).await?;
    let session = connect_first(server.clone(), parts, Some(&*parts.reducer_caller)).await?;
    pin_store_after_connect(data_dir, found.as_deref());
    // Only an address that answered is worth recording; failing to write
    // it is a warning, not an abort.
    record_store_server_for(target, data_dir, &server);
    Ok((server, session))
}

/// The store a board connects to: one the operator named, or dispatch's own.
#[derive(Clone)]
pub(super) enum StoreTarget {
    Named(String),
    Managed(Arc<crate::spacetime::managed_store::ManagedStore>),
}

impl StoreTarget {
    pub(super) fn is_named(&self) -> bool {
        matches!(self, Self::Named(_))
    }

    pub(super) fn managed(&self) -> Option<&Arc<crate::spacetime::managed_store::ManagedStore>> {
        match self {
            Self::Named(_) => None,
            Self::Managed(store) => Some(store),
        }
    }
}

/// Stops the managed store this board holds when dropped. Idempotent with the
/// abort paths: `ManagedStore` stops a store once.
pub(super) struct ManagedStoreGuard(
    pub(super) Option<Arc<crate::spacetime::managed_store::ManagedStore>>,
);

impl Drop for ManagedStoreGuard {
    fn drop(&mut self) {
        if let Some(store) = &self.0 {
            store.stop_on_exit();
        }
    }
}

/// `RecordTheNamedStoreOnceItAnswers`: record `server` beside `data_dir` for a
/// named store; a managed board records nothing. True when a record was kept.
/// Best-effort: a board that cannot write it still runs.
pub(super) fn record_store_server_for(target: &StoreTarget, data_dir: &Path, server: &str) -> bool {
    target.is_named() && crate::startup::record_store_server(data_dir, server)
}

/// `ForgetAStaleStoreRecordOnAManagedLaunch`: a managed launch clears any
/// record beside `data_dir`; a named launch leaves it for its own record to
/// replace.
pub(super) fn forget_stale_store_record_for(target: &StoreTarget, data_dir: &Path) {
    if matches!(target, StoreTarget::Managed(_)) {
        crate::startup::forget_store_server(data_dir);
    }
}

/// `ForgetTheStoreRecordWhenTheBoardExits`: removes a named board's record
/// when dropped, so every way out of `run_tui` -- a quit, an abort, an
/// unwinding panic -- takes the record with it. A managed board's guard does
/// nothing.
pub(super) struct StoreRecordGuard(Option<std::path::PathBuf>);

impl StoreRecordGuard {
    pub(super) fn for_target(target: &StoreTarget, data_dir: &Path) -> Self {
        Self(target.is_named().then(|| data_dir.to_path_buf()))
    }
}

impl Drop for StoreRecordGuard {
    fn drop(&mut self) {
        if let Some(data_dir) = &self.0 {
            crate::startup::forget_store_server(data_dir);
        }
    }
}

/// A later startup failure, with the managed store (if this board holds one)
/// stopped first and the failure's reason kept.
pub(super) fn abort_managed_startup(
    store: Option<&Arc<crate::spacetime::managed_store::ManagedStore>>,
    error: anyhow::Error,
) -> anyhow::Error {
    let Some(store) = store else { return error };
    // Blocks for up to the stop timeout, on a launch that is already failing.
    tokio::task::block_in_place(|| match error.downcast::<crate::startup::StartupAbort>() {
        Ok(abort) => store.abort_startup(abort).into(),
        Err(other) => {
            store.stop_on_exit();
            other
        }
    })
}

/// A board that is signalled -- SIGHUP is what tmux sends when a later launch
/// retires this board's window (`kill-window`), SIGTERM what `kill` sends --
/// would otherwise die on the default disposition without running its exit,
/// and leave its managed store, or a named board's store record, behind. Catch
/// both, stop the store or forget the record, and go.
pub(super) fn clean_up_on_termination(
    store: Option<Arc<crate::spacetime::managed_store::ManagedStore>>,
    record_db: Option<std::path::PathBuf>,
) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut hangup), Ok(mut terminate)) = (
        signal(SignalKind::hangup()),
        signal(SignalKind::terminate()),
    ) else {
        tracing::warn!(
            "could not listen for SIGHUP/SIGTERM; a managed store outlives a signalled board"
        );
        return;
    };
    tokio::spawn(async move {
        tokio::select! {
            _ = hangup.recv() => {}
            _ = terminate.recv() => {}
        }
        if let Some(store) = store {
            let _ = tokio::task::spawn_blocking(move || store.stop_on_exit()).await;
        }
        if let Some(data_dir) = record_db {
            crate::startup::forget_store_server(&data_dir);
        }
        let _ = disable_raw_mode();
        std::process::exit(0);
    });
}

/// Pick the store this launch uses: the one the operator named, or dispatch's
/// own managed one. `cli_on_path` is consulted only when none is named.
pub(super) fn select_store_target(
    data_dir: &Path,
    spacetime_server: Option<String>,
    cli_on_path: impl FnOnce() -> bool,
) -> Result<StoreTarget> {
    Ok(
        match crate::spacetime::managed_store::select_store(spacetime_server, cli_on_path)? {
            crate::spacetime::managed_store::StoreSelection::Named(server) => {
                StoreTarget::Named(server)
            }
            crate::spacetime::managed_store::StoreSelection::Managed => {
                // Fixed, not derived from `--db`: a throwaway database must not
                // start a second store or lose sight of the module hash the first
                // recorded.
                let store_data_dir = crate::default_data_dir().join("spacetime");
                let log_dir = data_dir;
                StoreTarget::Managed(Arc::new(
                    crate::spacetime::managed_store::ManagedStore::for_launch(
                        store_data_dir,
                        log_dir,
                    ),
                ))
            }
        },
    )
}

/// `RestartTheManagedStoreWhenItStopsSaving`: every health interval, ask the
/// managed store this board holds whether it is still saving, and tell the
/// board when it had to be restarted. Ends with the board's message channel.
pub(super) fn watch_managed_store(
    store: Arc<crate::spacetime::managed_store::ManagedStore>,
    msg_tx: mpsc::UnboundedSender<Message>,
) {
    use crate::spacetime::managed_store::{HealthOutcome, MANAGED_STORE_HEALTH_INTERVAL};
    tokio::spawn(async move {
        let mut ticker = interval(MANAGED_STORE_HEALTH_INTERVAL);
        ticker.tick().await; // the first tick is immediate
        loop {
            ticker.tick().await;
            if msg_tx.is_closed() {
                return;
            }
            let checked = store.clone();
            let Ok(outcome) = tokio::task::spawn_blocking(move || checked.check_health()).await
            else {
                continue;
            };
            let text = match outcome {
                HealthOutcome::Healthy => continue,
                HealthOutcome::Restarted => {
                    "The local store had stopped saving (disk full?) and was restarted. \
                     Changes made since it failed were lost."
                        .to_string()
                }
                HealthOutcome::RestartFailed(reason) => {
                    format!("The local store stopped saving and could not be restarted: {reason}")
                }
            };
            tracing::warn!("{text}");
            let _ = msg_tx.send(Message::System(crate::tui::messages::SystemMessage::Error(
                text,
            )));
        }
    });
}

/// The store address to connect to, bringing a managed store up first.
async fn resolve_store_server(target: &StoreTarget) -> Result<String> {
    match target {
        StoreTarget::Named(server) => Ok(server.clone()),
        StoreTarget::Managed(store) => {
            // Blocking: probes, a process start, a publish. Off the async
            // threads so the runtime keeps turning meanwhile.
            let store = store.clone();
            eprintln!("Starting the local store...");
            let ready = tokio::task::spawn_blocking(move || store.bring_up())
                .await
                .map_err(|e| anyhow::anyhow!("managed store thread panicked: {e}"))??;
            Ok(ready.server)
        }
    }
}
