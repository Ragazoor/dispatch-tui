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

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::startup_abort::StartupAbort;

#[cfg(test)]
mod tests;

/// `host.allium` config `host_file_name`.
pub const HOST_FILE_NAME: &str = "host.json";

/// `host.allium` config `host_file_mode`: owner read/write only.
pub const HOST_FILE_MODE: u32 = 0o600;

/// The four fields the host file holds, plus any a newer binary added
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
    /// Fields a newer binary added. Kept so an older one rewriting the file
    /// does not drop them.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// `<data_dir>/host.json`.
pub fn host_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(HOST_FILE_NAME)
}

/// Read and parse the host file. `Ok(None)` is "no file"; any other failure to
/// read or parse is an error, never "no file".
fn read_existing(data_dir: &Path) -> anyhow::Result<Option<HostIdentity>> {
    use std::io::Read;
    let path = host_file_path(data_dir);
    let read_failed =
        |e: std::io::Error| anyhow::Error::new(e).context(format!("read {}", path.display()));
    let mut file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(read_failed(e)),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(read_failed)?;
    let identity: HostIdentity = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a readable identity", path.display()))?;
    repair_mode(&file, &path);
    Ok(Some(identity))
}

/// `host.allium: HostFileIsPrivate` on read: a host file someone loosened is
/// put back to [`HOST_FILE_MODE`], on the file already open. Best effort: a
/// repair that fails is logged and the read still answers, since the content
/// is already in hand.
fn repair_mode(file: &std::fs::File, path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mode = match file.metadata() {
        Ok(meta) => meta.permissions().mode() & 0o777,
        Err(e) => {
            tracing::warn!("cannot check the mode of {}: {e}", path.display());
            return;
        }
    };
    if mode == HOST_FILE_MODE {
        return;
    }
    match file.set_permissions(std::fs::Permissions::from_mode(HOST_FILE_MODE)) {
        Ok(()) => tracing::warn!(
            "{} had mode {mode:o}; set it back to {HOST_FILE_MODE:o}",
            path.display()
        ),
        Err(e) => tracing::warn!(
            "{} has mode {mode:o} and cannot be set to {HOST_FILE_MODE:o}: {e}",
            path.display()
        ),
    }
}

/// Replace the host file with `identity`, atomically and privately.
fn write_whole(data_dir: &Path, identity: &HostIdentity) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("create data directory {}", data_dir.display()))?;
    let path = host_file_path(data_dir);
    let bytes = serde_json::to_vec_pretty(identity).context("encode the host identity")?;
    // The mode is a spec obligation (`HostFileIsPrivate`), so it is fixed
    // rather than inherited.
    crate::atomic_write::write_bytes(
        &path,
        crate::atomic_write::Mode::Fixed(HOST_FILE_MODE),
        &bytes,
    )?;
    Ok(())
}

/// Read-modify-write: replace the file with `change` applied to what is there.
/// Fails, writing nothing, when there is no readable host file. A change that
/// leaves the identity as it was writes nothing, and a change that refuses
/// writes nothing either.
fn update(
    data_dir: &Path,
    change: impl FnOnce(&mut HostIdentity) -> anyhow::Result<()>,
) -> anyhow::Result<HostIdentity> {
    let mut identity = read_for_cli(data_dir)?;
    let before = identity.clone();
    change(&mut identity)?;
    if identity != before {
        write_whole(data_dir, &identity)?;
    }
    Ok(identity)
}

/// A user identity, trimmed; an empty one is refused.
fn checked_identity(identity: &str) -> anyhow::Result<&str> {
    let identity = identity.trim();
    if identity.is_empty() {
        anyhow::bail!("user identity must not be empty");
    }
    Ok(identity)
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
        extra: Default::default(),
    };
    write_whole(data_dir, &minted).map_err(unavailable)?;
    Ok(minted)
}

/// A one-shot CLI command's read (`cli.allium: CliCommandsNeedAHostFile`).
/// Never writes content (it may put a loosened mode back, `HostFileIsPrivate`):
/// with no host file it fails with a message telling the
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
        identity.label = Some(label.to_string());
        Ok(())
    })
}

/// `host.allium: AdoptUserIdentity` — the owner and its credential, written
/// together in one whole-file replacement. The owner is written once
/// (`core.allium: LocalHostOwnerIsWrittenOnce`): re-adopting the stored owner
/// refreshes only its credential, and a different identity is refused with
/// nothing written (`RefuseAChangedUserIdentity`), so the file never pairs one
/// owner with another's credential. Nothing is written when the file already
/// holds both.
pub fn adopt_user_identity(
    data_dir: &Path,
    identity: &str,
    credential: &str,
) -> anyhow::Result<HostIdentity> {
    let identity = checked_identity(identity)?;
    let credential = credential.trim();
    if credential.is_empty() {
        anyhow::bail!("user identity credential must not be empty");
    }
    update(data_dir, |host| {
        match host.user_identity.as_deref() {
            Some(stored) if stored != identity => anyhow::bail!(
                "this install belongs to user identity {stored}, not {identity}: refusing to store another identity's credential"
            ),
            Some(_) => {}
            None => host.user_identity = Some(identity.to_string()),
        }
        host.credential = Some(credential.to_string());
        Ok(())
    })
}

/// Store or refresh the credential alone. Rejects an empty credential.
pub fn set_credential(data_dir: &Path, credential: &str) -> anyhow::Result<HostIdentity> {
    let credential = credential.trim();
    if credential.is_empty() {
        anyhow::bail!("user identity credential must not be empty");
    }
    update(data_dir, |host| {
        host.credential = Some(credential.to_string());
        Ok(())
    })
}

/// Store the user identity once: a second call with a different identity
/// leaves the stored one alone.
pub fn adopt_user_identity_once(data_dir: &Path, identity: &str) -> anyhow::Result<HostIdentity> {
    let identity = checked_identity(identity)?;
    update(data_dir, |host| {
        host.user_identity
            .get_or_insert_with(|| identity.to_string());
        Ok(())
    })
}
