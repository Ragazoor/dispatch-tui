//! This install's identity file: `host.json` in the data directory.
//!
//! See `docs/specs/host.allium`: `HostFile`, `IdentityLivesInHostFile`,
//! `HostFileIsWrittenWhole`, `MintHostIdentity`, `RenameHost`,
//! `AdoptUserIdentity`; and `docs/specs/cli.allium`:
//! `CliCommandsNeedAHostFile`.
//!
//! Every write is a whole-file replacement: the new content goes to a temporary
//! file in the same directory, created with the private mode already applied,
//! and is renamed over the old one. The private mode is therefore in force
//! before any byte of the credential is written.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::startup_abort::StartupAbort;

#[cfg(test)]
mod tests;

/// `host.allium` config `host_file_name`.
pub const HOST_FILE_NAME: &str = "host.json";

/// `host.allium` config `host_file_mode`: owner read/write only.
pub const HOST_FILE_MODE: u32 = 0o600;

/// The four fields the host file holds, and nothing else
/// (`host.allium: HostFile`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostIdentity {
    /// `core/Host.id`: minted once, immutable.
    pub host_id: String,
    /// `core/Host.label`: `None` until the operator names the machine.
    pub label: Option<String>,
    /// `core/Host.owner`: the user identity a shared store handed out.
    pub user_identity: Option<String>,
    /// The credential that proves `user_identity` to the store.
    pub credential: Option<String>,
}

/// `<data_dir>/host.json`.
pub fn host_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(HOST_FILE_NAME)
}

/// Read and parse the host file. `Ok(None)` is "no file"; any other failure to
/// read or parse is an error, never "no file".
fn read_existing(data_dir: &Path) -> anyhow::Result<Option<HostIdentity>> {
    let path = host_file_path(data_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow::Error::new(e).context(format!("read {}", path.display()))),
    };
    let identity: HostIdentity = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a readable identity", path.display()))?;
    Ok(Some(identity))
}

/// Replace the host file with `identity`, atomically and privately.
fn write_whole(data_dir: &Path, identity: &HostIdentity) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("create data directory {}", data_dir.display()))?;
    let path = host_file_path(data_dir);
    let bytes = serde_json::to_vec_pretty(identity).context("encode the host identity")?;
    let mut temp = tempfile::Builder::new()
        .prefix(".host.json.")
        .tempfile_in(data_dir)
        .with_context(|| format!("create a temporary file in {}", data_dir.display()))?;
    // `tempfile` creates 0600 already, but the mode is a spec obligation
    // (`HostFileIsPrivate`) and so is stated here rather than assumed.
    std::fs::set_permissions(
        temp.path(),
        std::os::unix::fs::PermissionsExt::from_mode(HOST_FILE_MODE),
    )
    .context("restrict the temporary host file")?;
    temp.write_all(&bytes).context("write the host identity")?;
    temp.as_file()
        .sync_all()
        .context("flush the host identity")?;
    temp.persist(&path)
        .map_err(|e| e.error)
        .with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

/// Read-modify-write: replace the file with `change` applied to what is there.
/// Fails, writing nothing, when there is no readable host file. A change that
/// leaves the identity as it was writes nothing.
fn update(data_dir: &Path, change: impl FnOnce(&mut HostIdentity)) -> anyhow::Result<HostIdentity> {
    let mut identity = read_for_cli(data_dir)?;
    let before = identity.clone();
    change(&mut identity);
    if identity != before {
        write_whole(data_dir, &identity)?;
    }
    Ok(identity)
}

/// `core.allium: LocalHostOwnerIsWrittenOnce`: set the owner only when none is
/// stored. Trims and rejects an empty identity.
fn set_owner_if_unset(host: &mut HostIdentity, identity: &str) -> anyhow::Result<()> {
    let identity = identity.trim();
    if identity.is_empty() {
        anyhow::bail!("user identity must not be empty");
    }
    if host.user_identity.is_none() {
        host.user_identity = Some(identity.to_string());
    }
    Ok(())
}

/// The board launch's identity step (`startup.allium`:
/// `CheckHostLabelAfterStartupConfigResolves` offering `FirstRun`).
///
/// No host file: mint a fresh id with a null label and write it
/// (`MintHostIdentity`). A host file that exists but cannot be read or parsed:
/// mint nothing, overwrite nothing, and abort with
/// `StartupAbort::HostIdentityUnavailable`. A first-run file that cannot be
/// written aborts the same way.
pub fn resolve_for_launch(data_dir: &Path) -> Result<HostIdentity, StartupAbort> {
    let unavailable = |e: anyhow::Error| {
        tracing::error!("host identity unavailable: {e:#}");
        StartupAbort::HostIdentityUnavailable
    };
    if let Some(existing) = read_existing(data_dir).map_err(unavailable)? {
        return Ok(existing);
    }
    let minted = HostIdentity {
        host_id: uuid::Uuid::new_v4().to_string(),
        label: None,
        user_identity: None,
        credential: None,
    };
    write_whole(data_dir, &minted).map_err(unavailable)?;
    Ok(minted)
}

/// A one-shot CLI command's read (`cli.allium: CliCommandsNeedAHostFile`).
/// Never writes: with no host file it fails with a message telling the
/// operator to run `dispatch tui` once.
pub fn read_for_cli(data_dir: &Path) -> anyhow::Result<HostIdentity> {
    read_existing(data_dir)?.with_context(|| {
        format!(
            "no {} in {}: this machine has no identity yet. Run `dispatch tui` once to create it",
            HOST_FILE_NAME,
            data_dir.display()
        )
    })
}

/// `host.allium: RenameHost` — a whole-file replacement that changes only the
/// label. Rejects an empty or whitespace-only label.
pub fn rename_host(data_dir: &Path, label: &str) -> anyhow::Result<HostIdentity> {
    let label = label.trim();
    if label.is_empty() {
        anyhow::bail!("host label must not be empty");
    }
    update(data_dir, |identity| {
        identity.label = Some(label.to_string())
    })
}

/// `host.allium: AdoptUserIdentity` — the owner and its credential, written
/// together in one whole-file replacement. The owner is written once
/// (`core.allium: LocalHostOwnerIsWrittenOnce`): an owner already stored is
/// kept, and only the credential is refreshed. Nothing is written when the
/// file already holds both.
pub fn adopt_user_identity(
    data_dir: &Path,
    identity: &str,
    credential: &str,
) -> anyhow::Result<HostIdentity> {
    let credential = credential.trim();
    if identity.trim().is_empty() {
        anyhow::bail!("user identity must not be empty");
    }
    if credential.is_empty() {
        anyhow::bail!("user identity credential must not be empty");
    }
    update(data_dir, |host| {
        // Validated above, so this cannot fail.
        let _ = set_owner_if_unset(host, identity);
        host.credential = Some(credential.to_string());
    })
}

/// Store or refresh the credential alone. Rejects an empty credential.
pub fn set_credential(data_dir: &Path, credential: &str) -> anyhow::Result<HostIdentity> {
    let credential = credential.trim();
    if credential.is_empty() {
        anyhow::bail!("user identity credential must not be empty");
    }
    update(data_dir, |host| {
        host.credential = Some(credential.to_string())
    })
}

/// Store the user identity once: a second call with a different identity
/// leaves the stored one alone.
pub fn adopt_user_identity_once(data_dir: &Path, identity: &str) -> anyhow::Result<HostIdentity> {
    let mut checked = HostIdentity {
        host_id: String::new(),
        label: None,
        user_identity: None,
        credential: None,
    };
    set_owner_if_unset(&mut checked, identity)?;
    update(data_dir, |host| {
        if host.user_identity.is_none() {
            host.user_identity = checked.user_identity;
        }
    })
}
