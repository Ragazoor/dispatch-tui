//! `tasks.allium: ClearBuildOutputOfDoneTask`: a Done task's worktree keeps its
//! code and branch but loses its build output.

use std::path::{Path, PathBuf};

/// The first line of a cache directory tag (<https://bford.info/cachedir/>).
pub const CACHEDIR_TAG_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

/// What [`clear_build_output`] did.
#[derive(Debug, Default)]
pub struct BuildOutputCleared {
    pub removed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

/// Every top-level directory of `worktree` holding a `CACHEDIR.TAG` that starts
/// with [`CACHEDIR_TAG_SIGNATURE`]. A symlink is never a candidate, and nothing
/// below the top level is examined. A worktree that cannot be read has none.
pub fn build_output_dirs(worktree: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(worktree) else {
        return vec![];
    };
    entries
        .flatten()
        // `DirEntry::file_type` does not follow symlinks.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|dir| has_signed_cachedir_tag(dir))
        .collect()
}

fn has_signed_cachedir_tag(dir: &Path) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(dir.join("CACHEDIR.TAG")) else {
        return false;
    };
    let mut head = Vec::with_capacity(CACHEDIR_TAG_SIGNATURE.len());
    file.take(CACHEDIR_TAG_SIGNATURE.len() as u64)
        .read_to_end(&mut head)
        .is_ok_and(|_| head == CACHEDIR_TAG_SIGNATURE.as_bytes())
}

/// Remove every [`build_output_dirs`] entry of `worktree`. Best effort: a
/// directory that cannot be removed is reported in `failed` and the rest are
/// still removed. Blocking; run it off the board's thread.
///
/// Only a linked git worktree is touched: its `.git` must be a regular file (the
/// `gitdir:` pointer). A task row's `worktree` is stored data, so a path that
/// is a home directory, a plain directory or a repository's own checkout (whose
/// `.git` is a directory) is refused rather than trusted.
pub fn clear_build_output(worktree: &Path) -> BuildOutputCleared {
    let mut cleared = BuildOutputCleared::default();
    let is_linked_worktree =
        std::fs::symlink_metadata(worktree.join(".git")).is_ok_and(|m| m.file_type().is_file());
    if !is_linked_worktree {
        if worktree.exists() {
            tracing::warn!(
                worktree = %worktree.display(),
                "not a linked git worktree, build output left alone"
            );
        }
        return cleared;
    }
    for dir in build_output_dirs(worktree) {
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => cleared.removed.push(dir),
            Err(e) => cleared.failed.push((dir, e.to_string())),
        }
    }
    cleared
}
