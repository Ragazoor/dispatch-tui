//! Whole-file replacement for files the operator owns.
//!
//! The new content goes to a temporary file in the same directory and is
//! renamed over the target, so a reader never sees a half-written file and a
//! crash leaves the old one intact.
//!
//! This protects readers, not concurrent writers: a caller that reads, edits
//! and writes back can still lose another process's write made in between.
//! Callers narrow that window by reading immediately before they write.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::Context;

#[cfg(test)]
mod tests;

/// The mode a replaced file ends up with.
#[derive(Debug, Clone, Copy)]
pub enum Mode {
    /// Exactly this mode, whatever the old file had.
    Fixed(u32),
    /// The old file's mode; `default` when there is no old file.
    KeepOr { default: u32 },
}

/// Replace `path` with whatever `fill` writes, atomically.
///
/// A symlinked `path` keeps its link: the file it points at is replaced.
///
/// `fill` failing leaves `path` untouched and no temporary file behind.
pub fn write_with(
    path: &Path,
    mode: Mode,
    fill: impl FnOnce(&mut dyn Write) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let resolved;
    let path = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            resolved = std::fs::canonicalize(path)
                .with_context(|| format!("resolve the symlink {}", path.display()))?;
            resolved.as_path()
        }
        _ => path,
    };
    let dir = parent_dir(path);
    let mode = match mode {
        Mode::Fixed(mode) => mode,
        Mode::KeepOr { default } => match std::fs::metadata(path) {
            Ok(meta) => meta.permissions().mode() & 0o7777,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => default,
            Err(e) => return Err(anyhow::Error::new(e).context(format!("stat {}", path.display()))),
        },
    };
    let temp = filled_temp(dir, mode, fill)?;
    temp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("replace {}", path.display()))?;
    sync_dir(dir)
}

/// Make a rename in `dir` durable: without this a power loss right after the
/// rename can bring the old file, or none, back.
fn sync_dir(dir: &Path) -> anyhow::Result<()> {
    std::fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .with_context(|| format!("sync the directory {}", dir.display()))
}

/// The directory `path` sits in, where its temporary file goes: `.` for a
/// bare file name.
fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// A synced temporary file in `dir` with `mode` set before `fill` writes it.
fn filled_temp(
    dir: &Path,
    mode: u32,
    fill: impl FnOnce(&mut dyn Write) -> anyhow::Result<()>,
) -> anyhow::Result<tempfile::NamedTempFile> {
    let mut temp = tempfile::Builder::new()
        .prefix(".atomic-write.")
        .tempfile_in(dir)
        .with_context(|| format!("create a temporary file in {}", dir.display()))?;
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(mode))
        .context("set the temporary file's mode")?;
    {
        let mut writer = std::io::BufWriter::new(temp.as_file_mut());
        fill(&mut writer)?;
        writer.flush().context("flush the temporary file")?;
    }
    temp.as_file()
        .sync_all()
        .context("sync the temporary file")?;
    Ok(temp)
}

/// Create `path` with `bytes` and `mode`, only if nothing is there yet.
///
/// Whole-file like [`write_bytes`], but never replaces: `Ok(false)` when
/// `path` already exists, which is then left as it was. Two processes racing
/// to create the same file end with exactly one's content.
pub fn create_new_bytes(path: &Path, mode: u32, bytes: &[u8]) -> anyhow::Result<bool> {
    let temp = filled_temp(parent_dir(path), mode, |w| {
        w.write_all(bytes).context("write the file")
    })?;
    match temp.persist_noclobber(path) {
        Ok(_) => sync_dir(parent_dir(path)).map(|()| true),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(anyhow::Error::new(e.error).context(format!("create {}", path.display()))),
    }
}

/// [`write_with`] for content already in memory.
pub fn write_bytes(path: &Path, mode: Mode, bytes: &[u8]) -> anyhow::Result<()> {
    write_with(path, mode, |w| w.write_all(bytes).context("write the file"))
}
