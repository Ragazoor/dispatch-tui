//! The store every process reads and writes through, and its first
//! connection: [`StoreParts`] (the `Store` and the connection pieces behind
//! it), [`connect_first`], and a one-shot command's own connection
//! ([`open_cli_store`]).
//!
//! Its own module so the board (`src/runtime/`) and the one-shot commands
//! (`src/cli/`, `src/main.rs`) share it without the commands depending on the
//! runtime. The board's own address resolution and reconnect loop stay in
//! `src/runtime/`.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::store;

/// The `Store` every process reads and writes through, and the pieces of the
/// connection behind it.
///
/// Built before anything connects: the store's reads answer from `rows`, which
/// the connection fills, and its writes go through `reducer_caller`, which the
/// connection carries. Both are fixed at construction, so which backing a read
/// or write goes to cannot change under a caller, and a process cannot read
/// one copy and write another.
pub struct StoreParts {
    /// Also the board's card-read handle (`crate::store::BoardReads`).
    pub database: Arc<store::Store>,
    pub rows: Arc<crate::sync::SharedRows>,
    pub connector: Arc<dyn crate::sync::StoreConnector>,
    pub settled_identity: Arc<crate::sync::SettledIdentity>,
    /// The store's own transport, also reused where a reducer call is needed
    /// outside the store traits — today, only the host-registry mirror
    /// (`sync.allium: RegisterHostOnConnect`/`RegisterHostOnRename`), which is
    /// deliberately not a store method.
    pub reducer_caller: Arc<dyn crate::sync::ReducerCaller>,
    /// Ask the store at an address for its database's identity, before any
    /// connection (`startup.allium`: `StoreIdentityPin.store_database_identity`).
    /// Blocking. A seam so the bootstrap tests can name the database without a
    /// server.
    pub store_identity: fn(&str) -> Option<String>,
}

impl StoreParts {
    /// A store over a fresh connection's rows, with this install's identity in
    /// `<data_dir>/host.json` (`host.allium: IdentityLivesInHostFile`).
    /// `host_id` is this install's own — known before any connection, because
    /// it is minted locally on first run (`host.allium: MintHostIdentity`) —
    /// and the claim needs it on every write.
    pub fn build(data_dir: &Path, host_id: &str) -> Self {
        let rows = Arc::new(crate::sync::SharedRows::new());
        let sdk = Arc::new(crate::sync::SpacetimeSdkConnector::new(
            crate::sync::SHARED_DATABASE_NAME,
            rows.clone(),
        ));
        let settled_identity = Arc::new(crate::sync::SettledIdentity::default());
        let reducer_caller: Arc<dyn crate::sync::ReducerCaller> = Arc::new(
            crate::sync::SdkReducerCaller::new(sdk.clone(), settled_identity.clone()),
        );
        let connector: Arc<dyn crate::sync::StoreConnector> = sdk;
        let database = Arc::new(store::Store::new(
            rows.clone(),
            reducer_caller.clone(),
            settled_identity.clone(),
            Arc::new(crate::clock::SystemClock),
            host_id.to_string(),
            data_dir,
        ));
        Self {
            store_identity: store_database_identity,
            database,
            rows,
            connector,
            settled_identity,
            reducer_caller,
        }
    }
}

/// A `dispatch` subcommand's own connection to the shared store.
///
/// Hold it for as long as the command runs: the connection lives in
/// `_session`, and dropping it leaves `database` with nothing to read from.
/// No reconnect loop — a command that loses its store mid-run fails, and the
/// operator runs it again.
pub struct CliStore {
    pub database: Arc<store::Store>,
    _session: crate::sync::SyncSession,
}

/// Route a handle through the store `server` names,
/// and make the first connection — the same one a board makes at startup,
/// with the same failures (`startup.allium`: `AbortWhenNoStoreIsConfigured`,
/// `AbortWhenTheStoreCannotBeReached`). For the subcommands that read or write
/// shared rows (`repo`, `plan`, the agent-tree and diff panes): with the store
/// mandatory, the local database no longer holds them.
pub async fn open_cli_store(data_dir: &Path, server: Option<String>) -> Result<CliStore> {
    // Flag, then the operator's environment, then the address the board
    // published on its session, then the record a board left beside this
    // database, then the managed address (`CliCommandsReachTheStoreWithoutManagingIt`).
    // The environment is read here too, so a blank flag falls through to it.
    let server = crate::startup::cli_store_server(
        server,
        std::env::var(crate::startup::STORE_SERVER_ENV).ok(),
        std::env::var(crate::startup::BOARD_STORE_ENV).ok(),
        data_dir,
    );
    let host_id = open_with_cli_identity(data_dir).await?;
    let parts = StoreParts::build(data_dir, &host_id);
    // No host-registry push: a short-lived command is not a board, and the
    // board already registers this host on every connect.
    let session = connect_first(server, &parts, None).await?;
    Ok(CliStore {
        database: parts.database,
        _session: session,
    })
}

/// The production [`StoreParts::store_identity`]: the shared database's
/// identity on `server`, over HTTP or TLS to match its address.
fn store_database_identity(server: &str) -> Option<String> {
    crate::startup::fetch_store_database_identity(
        server,
        crate::sync::SHARED_DATABASE_NAME,
        crate::startup::STORE_IDENTITY_TIMEOUT,
    )
}

/// `startup.allium`: `ConnectWhenTheStoreIsTheOneThisInstallUses` /
/// `AbortWhenTheStoreIsNotTheOneThisInstallUses`, run before the first
/// connection, which already writes. Returns the identity found, for
/// [`pin_store_after_connect`].
pub(crate) async fn check_store_identity(
    parts: &StoreParts,
    data_dir: &Path,
    server: &str,
    accept_store_switch: bool,
) -> Result<Option<String>> {
    let probe = parts.store_identity;
    let address = server.to_string();
    let pin_db = data_dir.to_path_buf();
    // One deadline over the whole probe: its own timeouts are per step, and a
    // name lookup has none. Past it the store counts as unaskable.
    let probed = tokio::time::timeout(
        crate::startup::STORE_IDENTITY_TIMEOUT * 2,
        tokio::task::spawn_blocking(move || {
            (
                probe(&address),
                crate::startup::pinned_store_identity(&pin_db),
            )
        }),
    )
    .await;
    let (found, pinned) = match probed {
        Ok(joined) => joined?,
        Err(_elapsed) => (None, crate::startup::pinned_store_identity(data_dir)),
    };
    if found.is_none() {
        tracing::warn!(
            server,
            "could not read the store's database identity; connecting without the store pin check"
        );
    }
    crate::startup::check_store_pin(
        found.as_deref(),
        pinned.as_deref(),
        accept_store_switch,
        server,
    )?;
    Ok(found)
}

/// `startup.allium`: `PinTheStoreOnceItAnswers`. Best-effort: a pin that
/// cannot be written is logged by `pin_store_identity` and the launch goes on.
pub(crate) fn pin_store_after_connect(data_dir: &Path, found: Option<&str>) {
    if let Some(identity) = found {
        crate::startup::pin_store_identity(data_dir, identity);
    }
}

/// A one-shot command's identity: read from the host file, never minted
/// (`cli.allium: CliCommandsNeedAHostFile`).
async fn open_with_cli_identity(data_dir: &Path) -> Result<String> {
    let dir = data_dir.to_path_buf();
    let identity =
        tokio::task::spawn_blocking(move || crate::host_file::read_for_cli(&dir)).await??;
    // A host file no board has connected with yet holds no user identity, so
    // there is no one for the command to act as: refuse, and write nothing.
    if identity.user_identity.is_none() {
        anyhow::bail!(
            "this machine's identity in {} has no user identity yet: run `dispatch tui` once \
             so a board can connect to the store and complete it",
            crate::host_file::host_file_path(data_dir).display()
        );
    }
    Ok(identity.host_id)
}

/// The first connection, shared by the board and the CLI: connect, settle the
/// identity, apply the initial subscription — or abort with the attempt's
/// reason (`startup.allium`: `AbortWhenTheStoreCannotBeReached`). With a
/// `register_with` caller, the host is mirrored into the shared registry too
/// (`sync.allium: RegisterHostOnConnect`) — the board's case.
pub(crate) async fn connect_first(
    server: String,
    parts: &StoreParts,
    register_with: Option<&dyn crate::sync::ReducerCaller>,
) -> Result<crate::sync::SyncSession> {
    let mut session = crate::sync::SyncSession::open(server, parts.connector.clone());
    session
        .connect_at_startup(&*parts.database, std::time::Instant::now())
        .await
        .map_err(|reason| {
            tracing::error!("first connection to the shared store failed: {reason}");
            crate::startup_abort::StartupAbort::StoreUnavailable { reason }
        })?;
    match register_with {
        Some(caller) => on_store_connected(&*parts.database, &parts.settled_identity, caller).await,
        None => {
            settle_from_store(&*parts.database, &parts.settled_identity).await;
        }
    }
    Ok(session)
}

/// What follows every successful connection — the first one at startup and
/// every reconnect alike: publish the settled identity so a write may stamp
/// it, and mirror this host into the shared registry
/// (`sync.allium: RegisterHostOnConnect`).
pub(crate) async fn on_store_connected(
    store: &dyn crate::sync::SyncStore,
    settled_identity: &crate::sync::SettledIdentity,
    reducer_caller: &dyn crate::sync::ReducerCaller,
) {
    let Some(user) = settle_from_store(store, settled_identity).await else {
        return;
    };
    // Fires on every settle, reconnects included, for the same reason
    // subscriptions are re-asserted unconditionally: a dropped connection does
    // not say whether the registry's copy of this row is still current.
    // Best-effort; a failure here must not stop the board from using the
    // connection it just got.
    match store.ensure_host_identity().await {
        Ok((id, label)) => {
            crate::sync::push_host_registration(
                reducer_caller,
                id,
                label.unwrap_or_default(),
                user,
            )
            .await;
        }
        Err(e) => tracing::warn!("could not read this host's identity to register it: {e:#}"),
    }
}

/// Publish the identity the session just settled, and answer it.
///
/// Read from the store rather than returned by the step, because the step's
/// job is the connection and widening its outcome to carry an identity would
/// put two unrelated answers on one return value. `None` — connected with no
/// stored identity — is not reachable, since the settled arm writes one; it is
/// a broken settings store, and is left unset so a user-board create is
/// refused with a message rather than stamped with a guess.
pub(crate) async fn settle_from_store(
    store: &dyn crate::sync::SyncStore,
    settled_identity: &crate::sync::SettledIdentity,
) -> Option<String> {
    match store.user_identity().await {
        Ok(Some(user)) => {
            settled_identity.settle(user.clone());
            Some(user)
        }
        Ok(None) => {
            tracing::warn!("connected to the shared store but no user identity was stored");
            None
        }
        Err(e) => {
            tracing::warn!("could not read the user identity: {e:#}");
            None
        }
    }
}

#[cfg(test)]
mod tests;
