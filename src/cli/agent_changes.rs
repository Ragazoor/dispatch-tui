//! `dispatch agent-changes`: the agent tree mod pane's one read — the
//! companion pane's git query and tree, printed as JSON for the dispatch mod
//! to draw (docs/specs/agent-tree.allium: `AgentChangesCommand`,
//! `RefreshAgentTreeModPane`).
//!
//! Both halves are asked on every run and fail apart: a base branch that will
//! not resolve fails the commits and leaves the tree working, exactly as the
//! companion pane's two timers do.

use std::path::Path;

use serde::Serialize;

use crate::agent_tree::{build_tree, FileChange, LineCounts, TreeNode, TreeNodeKind};
use crate::cli::agent_tree::{git_branch_commits, git_changes, shows_counts};
use crate::cli::agent_tree_commits::AgentCommit;
use crate::process::{ProcessRunner, RealProcessRunner};

/// One run's answer: the spec's `AgentChangesReport`.
#[derive(Debug, Serialize)]
pub struct Report {
    pub tree: Half<TreeHalf>,
    pub commits: Half<CommitsHalf>,
}

/// One half of the report: its value, or the reason it failed.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Half<T> {
    Ok(T),
    Err { error: String },
}

impl<T> From<anyhow::Result<T>> for Half<T> {
    fn from(result: anyhow::Result<T>) -> Self {
        match result {
            Ok(value) => Self::Ok(value),
            Err(e) => Self::Err {
                error: format!("{e:#}"),
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TreeHalf {
    pub rows: Vec<Row>,
}

#[derive(Debug, Serialize)]
pub struct CommitsHalf {
    pub commits: Vec<AgentCommit>,
}

/// One drawn row: the spec's `AgentTreeModRow`.
#[derive(Debug, Serialize)]
pub struct Row {
    pub depth: usize,
    pub name: String,
    pub path: String,
    pub kind: TreeNodeKind,
    pub badge: Option<FileChange>,
    /// The counts this row draws, not the node's sum.
    pub counts: Option<LineCounts>,
}

/// Ask git both questions for the worktree at `root`.
pub fn report(
    root: &Path,
    base_branch: &str,
    commit: Option<&str>,
    runner: &dyn ProcessRunner,
) -> Report {
    let tree = git_changes(root, commit, runner).map(|changes| TreeHalf {
        rows: rows(&build_tree(root, &changes)),
    });
    let commits =
        git_branch_commits(root, base_branch, runner).map(|commits| CommitsHalf { commits });
    Report {
        tree: tree.into(),
        commits: commits.into(),
    }
}

/// The tree below its root, depth first, in the order it is already sorted
/// in (`RowsPutAFoldersOwnFilesFirst`).
fn rows(root: &TreeNode) -> Vec<Row> {
    let mut out = Vec::new();
    for child in &root.children {
        push_rows(child, 0, "", &mut out);
    }
    out
}

fn push_rows(node: &TreeNode, depth: usize, parent: &str, out: &mut Vec<Row>) {
    let path = if parent.is_empty() {
        node.name.clone()
    } else {
        format!("{parent}/{}", node.name)
    };
    out.push(Row {
        depth,
        name: node.name.clone(),
        path: path.clone(),
        kind: node.kind,
        badge: node.badge,
        // Every directory is open in the mod (AgentTreeModEveryDirectoryIsOpen).
        counts: node.counts.filter(|_| shows_counts(node, false)),
    });
    for child in &node.children {
        push_rows(child, depth + 1, &path, out);
    }
}

/// The subcommand: print the report as one line of JSON.
pub fn run(root: &Path, base_branch: &str, commit: Option<&str>) -> anyhow::Result<()> {
    let report = report(root, base_branch, commit, &RealProcessRunner::default());
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

#[cfg(test)]
mod tests;
