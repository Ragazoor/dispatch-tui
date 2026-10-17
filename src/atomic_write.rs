//! Whole-file replacement for files the operator owns.
//!
//! The new content goes to a temporary file in the same directory and is
//! renamed over the target, so a reader never sees a half-written file and a
//! crash leaves the old one intact.

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
/// `fill` failing leaves `path` untouched and no temporary file behind.
pub fn write_with(
    path: &Path,
    mode: Mode,
    fill: impl FnOnce(&mut dyn Write) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mode = match mode {
        Mode::Fixed(mode) => mode,
        Mode::KeepOr { default } => match std::fs::metadata(path) {
            Ok(meta) => meta.permissions().mode() & 0o7777,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => default,
            Err(e) => return Err(anyhow::Error::new(e).context(format!("stat {}", path.display()))),
        },
    };
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
    temp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

/// [`write_with`] for content already in memory.
pub fn write_bytes(path: &Path, mode: Mode, bytes: &[u8]) -> anyhow::Result<()> {
    write_with(path, mode, |w| w.write_all(bytes).context("write the file"))
}
