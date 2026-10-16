//! The one-way channel between the agent-tree companion pane and the diff pane
//! beneath it: which files' diffs the user has opened.
//!
//! The two are separate processes in separate tmux panes, so the set has to
//! reach the second one somehow. It goes in a file in the worktree's git
//! administrative directory — see [`crate::worktree_admin`] for why there, and
//! `OpenSetIsNotWorktreeState` in `docs/specs/agent-tree.allium` for what that
//! placement guarantees the user.
//!
//! **The tree writes, the diff pane reads, and never the other way round.**
//! That is what keeps the tree the single place a toggle is decided even though
//! two processes can see the file, and it is what makes the tree's row markers
//! unable to disagree with the pane below them (`NodeDiffOpenMatchesOpenSet`).
//!
//! Nothing here is durable state. The tree empties the set when it exits, so a
//! file left behind by a killed renderer describes nothing, and every read
//! soft-fails to "nothing is open" rather than erroring — a view of a git
//! query must not fail to draw because a scratch file was unreadable.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::worktree_admin::worktree_admin_dir;

/// Basename of the open-set file inside the worktree's git admin directory.
const OPEN_SET_FILE: &str = "dispatch-agent-tree.json";

/// Basename of the selected-source file, beside the open set.
const SELECTED_SOURCE_FILE: &str = "dispatch-agent-tree-source.json";

/// Where this worktree's open set lives, or `None` when the path is not a
/// linked worktree and so has no admin directory to put it in.
fn open_set_path(worktree_path: &str) -> Option<PathBuf> {
    Some(worktree_admin_dir(worktree_path)?.join(OPEN_SET_FILE))
}

/// Where this worktree's selected source lives: beside the open set, in the
/// same admin directory and for the same reason.
fn selected_source_path(worktree_path: &str) -> Option<PathBuf> {
    Some(worktree_admin_dir(worktree_path)?.join(SELECTED_SOURCE_FILE))
}

/// Replace the recorded open set.
///
/// Takes a SLICE, not a set, and the order is part of the message: the diff
/// pane renders these in the order the tree's rows appear, and the tree is the
/// only party that knows that order — where a directory chain compresses
/// depends on the whole change set, not on the subset the user opened. A set
/// here would throw the order away and leave the diff pane re-deriving it from
/// the paths alone, which cannot be done. The JSON body is an array either
/// way, so nothing about the file format changes.
///
/// Written to a sibling temporary file and renamed into place, because the diff
/// pane reads this on its own timer and the two are not synchronised. A rename
/// is atomic within a directory on every platform this runs on, so a reader
/// either sees the whole previous set or the whole new one — never half a file,
/// which would decode as "nothing is open" and blank the pane for a tick.
pub fn write_open_set(worktree_path: &str, paths: &[PathBuf]) -> Result<()> {
    let path = open_set_path(worktree_path)
        .with_context(|| format!("{worktree_path} is not a linked worktree"))?;

    let encoded: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let body = serde_json::to_vec(&encoded).context("could not encode the open set")?;

    write_atomic(&path, &body)
}

/// Write `body` to `path` through a sibling temporary file and a rename, so a
/// reader on another timer never sees half a file.
fn write_atomic(path: &Path, body: &[u8]) -> Result<()> {
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, body).with_context(|| format!("could not write {}", temp.display()))?;
    std::fs::rename(&temp, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    Ok(())
}

/// Read the recorded open set, or an empty one.
///
/// Every failure reads as empty and none of them is an error: the file has not
/// been written yet, the worktree is not linked, the file is half-written or
/// holds something this does not understand. The diff pane's answer to all of
/// those is the same — show nothing — and making it an error would let a
/// scratch file take down a view whose real source of truth is git.
pub fn read_open_set(worktree_path: &str) -> Vec<PathBuf> {
    let Some(path) = open_set_path(worktree_path) else {
        return Vec::new();
    };
    let Ok(body) = std::fs::read(&path) else {
        return Vec::new();
    };
    let Ok(paths) = serde_json::from_slice::<Vec<String>>(&body) else {
        tracing::warn!(
            path = %path.display(),
            "agent-tree open set is unreadable; treating it as empty"
        );
        return Vec::new();
    };
    // A `Vec`, in the order written — see `write_open_set` for why the order is
    // the message rather than an artefact of the container.
    //
    // A path that escapes the worktree cannot name a file the tree ever showed,
    // so it is dropped rather than trusted — the same fail-closed rule
    // `agent_tree::relative_components` applies to git's own output. A repeat
    // goes with it: the tree never writes one, and rendering a file's diff
    // twice would be the reader's problem to notice.
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| crate::agent_tree::model::relative_components(p).is_some())
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

/// Forget everything, leaving the file behind as an empty set.
///
/// Called when the tree exits, so a diff pane that outlives it by a moment sees
/// an honest answer rather than a stale one. A worktree with no admin directory
/// has nothing to clear and reports success.
pub fn clear_open_set(worktree_path: &str) -> Result<()> {
    if open_set_path(worktree_path).is_none() {
        return Ok(());
    }
    write_open_set(worktree_path, &[])
}

/// Record which source the tree is showing — `None` for unstaged work, or the
/// selected commit's full id — where the diff pane can read it
/// (docs/specs/agent-tree.allium: `AgentTreeSourceIsOneSelection`,
/// `OpenSetIsNotWorktreeState`). Written by the tree only, like the open set.
pub fn write_selected_source(worktree_path: &str, commit: Option<&str>) -> Result<()> {
    let path = selected_source_path(worktree_path)
        .with_context(|| format!("{worktree_path} is not a linked worktree"))?;

    let body = serde_json::to_vec(&commit).context("could not encode the selected source")?;

    // Temp file and rename, for the reason `write_open_set` gives: the diff
    // pane reads this on its own timer.
    write_atomic(&path, &body)
}

/// Read the selected source, or `None` (unstaged work) when nothing readable
/// is recorded. Soft-fails like [`read_open_set`].
pub fn read_selected_source(worktree_path: &str) -> Option<String> {
    let path = selected_source_path(worktree_path)?;
    let body = std::fs::read(&path).ok()?;
    match serde_json::from_slice::<Option<String>>(&body) {
        // The id reaches git's argument list, so anything that is not a
        // hex object id (a leading `-` most of all) reads as unstaged work.
        Ok(commit) => {
            commit.filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_hexdigit()))
        }
        Err(_) => {
            tracing::warn!(
                path = %path.display(),
                "agent-tree selected source is unreadable; treating it as unstaged work"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests;
