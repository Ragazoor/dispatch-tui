//! StoreIdentityPin (startup.allium, task #28710): which store database this
//! install's boards last used, and the check that stops a launch reaching a
//! different one (`ConnectWhenTheStoreIsTheOneThisInstallUses`,
//! `AbortWhenTheStoreIsNotTheOneThisInstallUses`, `PinTheStoreOnceItAnswers`).
//!
//! The comparison is on the database's own identity, which SpacetimeDB gives
//! it when it is first published, not on the address: two stores over two
//! data directories can take turns on one address.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::StartupAbort;

/// The pin's file name, in the same folder as the database file.
pub const STORE_PIN_FILE: &str = "store-identity";

/// How long asking the store for its database identity may take.
pub const STORE_IDENTITY_TIMEOUT: Duration = Duration::from_secs(2);

fn store_pin_path(db_path: &Path) -> PathBuf {
    super::beside_database(db_path, STORE_PIN_FILE)
}

/// One spelling for an identity: lowercase hex with no `0x`, so a pin
/// written from one source compares equal to a reply from another.
fn normalize_identity(identity: &str) -> Option<String> {
    let trimmed = identity.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    (!hex.is_empty()).then(|| hex.to_ascii_lowercase())
}

/// The identity the last board on `db_path` pinned, else `None`. Blank is
/// `None`. `StoreIdentityPin.pinned_store_identity`.
pub fn pinned_store_identity(db_path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(store_pin_path(db_path)).ok()?;
    normalize_identity(&text)
}

/// Pin `identity` beside `db_path`, replacing whatever was there. True when
/// the pin was kept. `StoreIdentityPin.pin_store_identity`.
pub fn pin_store_identity(db_path: &Path, identity: &str) -> bool {
    let Some(identity) = normalize_identity(identity) else {
        return false;
    };
    match std::fs::write(store_pin_path(db_path), format!("{identity}\n")) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("could not pin the store database identity beside the database: {e}");
            false
        }
    }
}

/// The database identity in the store's reply to `GET /v1/database/<name>`.
pub fn parse_database_identity(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let identity = value
        .get("database_identity")?
        .get("__identity__")?
        .as_str()?;
    normalize_identity(identity)
}

/// Ask the store at `server` for the identity of `database`: over plain HTTP
/// for `http://`, over TLS for `https://`. `None` when it cannot be asked:
/// nothing listening, no such database, a certificate the board does not
/// trust, or a reply without an identity. Blocking; bounded by `timeout` for
/// the whole exchange. `StoreIdentityPin.store_database_identity`.
pub fn fetch_store_database_identity(
    server: &str,
    database: &str,
    timeout: Duration,
) -> Option<String> {
    let server = server.trim().trim_end_matches('/');
    if !server.starts_with("http://") && !server.starts_with("https://") {
        return None;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into();
    let mut reply = agent
        .get(format!("{server}/v1/database/{database}"))
        .call()
        .ok()?;
    if reply.status() != 200 {
        return None;
    }
    parse_database_identity(&reply.body_mut().read_to_string().ok()?)
}

/// `ConnectWhenTheStoreIsTheOneThisInstallUses` and
/// `AbortWhenTheStoreIsNotTheOneThisInstallUses`: `Ok` means connect.
pub fn check_store_pin(
    found: Option<&str>,
    pinned: Option<&str>,
    accepted: bool,
    address: &str,
) -> Result<(), StartupAbort> {
    let (Some(found), Some(pinned)) = (
        found.and_then(normalize_identity),
        pinned.and_then(normalize_identity),
    ) else {
        return Ok(());
    };
    if found == pinned || accepted {
        return Ok(());
    }
    Err(StartupAbort::StoreSwitched {
        address: address.to_string(),
        pinned,
        found,
    })
}

/// Remove the store address from `session`'s environment, so what tmux starts
/// there from now on does not inherit it. Best-effort: false (and a warning)
/// when tmux will not answer, or when there is no session to name.
pub fn forget_session_store_server(
    session: &str,
    runner: &dyn crate::process::ProcessRunner,
) -> bool {
    if session.is_empty() {
        return false;
    }
    match crate::tmux::unset_session_environment(session, super::STORE_SERVER_ENV, runner) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("could not clear the store address from the tmux session: {e:#}");
            false
        }
    }
}

/// `ClearTheSessionStoreAddressOnAManagedLaunch`, in the process that hands
/// off to tmux. A launch that named no store removes the address from
/// `session`'s environment BEFORE tmux starts the board there: the board tmux
/// starts takes the session's environment, not this shell's, so an address an
/// earlier named board left would otherwise make it a named-store board, which
/// never reaches the board's own clean-up (task #28710). False when a store
/// was named, else [`forget_session_store_server`]'s answer.
pub fn forget_session_store_server_before_handoff(
    explicit: Option<String>,
    session: &str,
    runner: &dyn crate::process::ProcessRunner,
) -> bool {
    crate::spacetime::managed_store::normalize_server(explicit).is_none()
        && forget_session_store_server(session, runner)
}
