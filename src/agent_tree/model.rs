//! Pure tree-building logic for the agent file tree companion pane (see
//! `docs/specs/agent-tree.allium`'s `AgentTreeNode` value type and the
//! badge/expansion rules in `RefreshAgentTree`).
//!
//! Git is the sole source of truth here — see the spec's `AgentTreeIsGitDerived`
//! guarantee. This module owns two halves of that: parsing what git printed
//! ([`parse_name_status`], [`parse_untracked`]) and folding the result into a
//! tree of only the changed paths and their ancestor directories
//! ([`build_tree`]). Running git is [`crate::agent_tree::changes`]';
//! nothing in this file touches the filesystem or spawns a process, which is
//! what keeps every rule below testable from a string literal.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// What git says happened to a file, relative to the selected source's
/// baseline: the index for unstaged work, the parent for a commit. Doubles as the badge vocabulary — see the
/// spec's `FileChange` enum, which is deliberately one enum for both so a
/// badge cannot claim something git did not say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChange {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeNodeKind {
    File,
    Directory,
}

/// How many lines one path gained and lost against the source's baseline — the spec's
/// `LineCounts` value type.
///
/// One struct rather than two `Option<u32>` fields because git either counts a
/// path or it does not: there is no answer where an addition count arrives
/// without a removal count. Modelling it this way makes half a count
/// unrepresentable instead of merely forbidden.
///
/// Both numbers may legitimately be zero. A permission-only change is a real
/// change that moved no lines, which is also why this is not collapsed into a
/// single signed total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LineCounts {
    pub added: u32,
    pub removed: u32,
}

impl std::ops::Add for LineCounts {
    type Output = LineCounts;

    /// Fold two paths' counts together. Used to sum a directory over its
    /// descendants — see [`compute_counts`]. Saturating rather than wrapping:
    /// a repo big enough to overflow this has bigger problems than an
    /// off-by-2^32 row, and a wrapped total would read as a small one.
    fn add(self, other: LineCounts) -> LineCounts {
        LineCounts {
            added: self.added.saturating_add(other.added),
            removed: self.removed.saturating_add(other.removed),
        }
    }
}

/// One changed file as git reported it, with a path relative to the pane root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileChange {
    pub path: PathBuf,
    pub change: FileChange,
    /// `None` when git reported no counts for this path, which happens in
    /// exactly two cases and both are facts about the file rather than
    /// decisions of ours: the path is untracked, so a diff against the
    /// baseline cannot see it; or git reports it binary. Rendering `+0 -0`
    /// instead would read as "nothing changed in there", which is the one
    /// thing that is certainly false of a file the agent just wrote. See the
    /// spec's `UntrackedFilesHaveNoLineCounts`.
    pub counts: Option<LineCounts>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub name: String,
    pub kind: TreeNodeKind,
    pub badge: Option<FileChange>,
    pub expanded: bool,
    pub children: Vec<TreeNode>,
    /// For a file, that file's own counts. For a directory, the sum over the
    /// changed files beneath it, so a collapsed directory still says how much
    /// is inside it.
    ///
    /// The sum SKIPS descendants that have none rather than reading them as
    /// zero, and is itself `None` when no descendant has any — see the spec's
    /// `DirectoryCountsSumDescendants`.
    pub counts: Option<LineCounts>,
}

impl TreeNode {
    /// Resolve a chain of name segments, relative to this node, to the node it
    /// names — `None` if any segment matches no child. Segment chains are how
    /// the companion pane's tree widget identifies a node (see
    /// `build_tree_items` in `src/agent_tree/render/mod.rs`), so this is what turns a
    /// widget selection back into a `TreeNode`.
    ///
    /// A segment here is a node's `name`, which for a chain-merged directory is
    /// a whole route rather than one path component — `node_at(&["a/b",
    /// "c.rs"])`, not `node_at(&["a", "b", "c.rs"])`. The chain is the widget's
    /// key, not the path; joining it with the separator gives the path.
    ///
    /// An empty path resolves to `self`. Callers that need to distinguish "the
    /// root" from "nothing selected" must check for that themselves — the
    /// synthetic root is a `Directory`, so it is otherwise indistinguishable
    /// from a real directory selection.
    pub fn node_at<S: AsRef<str>>(&self, path: &[S]) -> Option<&TreeNode> {
        let mut current = self;
        for segment in path {
            current = current
                .children
                .iter()
                .find(|c| c.name == segment.as_ref())?;
        }
        Some(current)
    }
}

/// Map one of git's `--name-status` status letters onto a [`FileChange`].
///
/// Only the leading letter is consulted, so the score suffix git appends to
/// similarity-scored letters (`R100`, `C75`) does not need stripping first.
/// See the spec's `CollapsedGitStatusLetters` note for why three values are
/// enough: `T` (type change) is a modification as far as a file tree is
/// concerned, and `R`/`C` never arrive because rename detection is off.
///
/// An unrecognised letter yields `None` and the line is skipped — this repo's
/// soft-fail-decoding convention. One unparseable line must not blank the tree.
fn change_from_status(status: &str) -> Option<FileChange> {
    match status.chars().next()? {
        'A' => Some(FileChange::Added),
        'D' => Some(FileChange::Deleted),
        'M' | 'T' => Some(FileChange::Modified),
        _ => None,
    }
}

/// Parse the output of `git diff --name-status --no-renames -z <base>`: a flat
/// NUL-separated stream alternating status and path, paths relative to the
/// repository root.
///
/// `-z` is what makes this a plain split rather than a parser. Without it git
/// separates the pair with a tab and, worse, C-quotes any path containing a
/// non-ASCII byte — `src/é.rs` arrives as the literal `"src/\303\251.rs"`,
/// quotes and octal escapes included, which would render as that string and
/// then open nothing. With `-z` there is no quoting and no escaping to undo, at
/// any byte value, and a path may contain spaces, tabs or newlines without
/// ambiguity. Nothing here trims, for the same reason: a leading or trailing
/// space is part of the filename.
///
/// A trailing status with no path, and any status letter this build does not
/// recognise, are skipped rather than erroring — this repo's
/// soft-fail-decoding convention. One unreadable record must not blank the tree.
pub fn parse_name_status(output: &str) -> Vec<GitFileChange> {
    let mut changes = Vec::new();
    // The stream ends with a NUL, so the final split segment is empty; taking
    // status and path strictly in pairs ignores it without a special case.
    let mut fields = output.split('\0');
    while let Some(status) = fields.next() {
        if status.is_empty() {
            continue;
        }
        let Some(path) = fields.next().filter(|p| !p.is_empty()) else {
            tracing::warn!(status, "skipping trailing git diff status with no path");
            break;
        };
        let Some(change) = change_from_status(status) else {
            tracing::warn!(status, path, "skipping git diff record with unknown status");
            continue;
        };
        changes.push(GitFileChange {
            path: PathBuf::from(path),
            change,
            // Filled in from the separate numstat query by
            // [`attach_line_counts`]; this parser is told only what happened.
            counts: None,
        });
    }
    changes
}

/// Parse the output of `git ls-files --others --exclude-standard -z`: a
/// NUL-separated list of paths, every one of them a file that exists but that
/// git is not tracking, and so [`FileChange::Added`].
///
/// `-z` for the same two reasons as [`parse_name_status`]: no quoting of
/// non-ASCII paths, and no ambiguity about a path containing whitespace.
pub fn parse_untracked(output: &str) -> Vec<GitFileChange> {
    output
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| GitFileChange {
            path: PathBuf::from(path),
            change: FileChange::Added,
            // Never filled in, and not an omission: an untracked path is not in
            // the index, so the numstat query cannot see it at all.
            counts: None,
        })
        .collect()
}

/// Parse the output of `git diff --numstat --no-renames -z` (or the same
/// asked of one commit): one
/// NUL-terminated record per path, holding `<added>\t<removed>\t<path>`.
///
/// A path git could not count carries `-` in both number fields — that is how
/// git spells a binary diff — and yields no entry rather than a zero. The
/// caller cannot tell that apart from "git said nothing about this path", and
/// does not need to: both mean the row shows no counts.
///
/// `-z` for the same two reasons as [`parse_name_status`]: git does not C-quote
/// non-ASCII paths, and a path containing whitespace stays unambiguous. That
/// second reason is why the path is taken as *everything after the second tab*
/// rather than as a third whitespace-delimited field — a filename may contain a
/// tab, and splitting on every tab would truncate it.
///
/// An unreadable record is skipped, not guessed at, in line with this repo's
/// soft-fail-decoding convention. One bad record must not cost the caller the
/// counts of every other path.
pub fn parse_numstat(output: &str) -> BTreeMap<PathBuf, LineCounts> {
    let mut counts = BTreeMap::new();

    for record in output.split('\0').filter(|r| !r.is_empty()) {
        let mut fields = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            tracing::warn!(record, "skipping malformed git numstat record");
            continue;
        };
        if path.is_empty() {
            tracing::warn!(record, "skipping git numstat record with no path");
            continue;
        }
        // A binary diff. Not an error and not a zero — see the doc comment.
        if added == "-" || removed == "-" {
            continue;
        }
        let (Ok(added), Ok(removed)) = (added.parse::<u32>(), removed.parse::<u32>()) else {
            tracing::warn!(
                record,
                "skipping git numstat record with unparseable counts"
            );
            continue;
        };
        counts.insert(PathBuf::from(path), LineCounts { added, removed });
    }

    counts
}

/// Fold a numstat map into the change set, matching on path.
///
/// A change with no entry in the map keeps `None`. That covers both the
/// untracked paths — which the numstat query cannot see at all — and any
/// tracked path whose record was skipped, and the two are deliberately not
/// distinguished: the row shows no counts either way.
pub fn attach_line_counts(changes: &mut [GitFileChange], counts: &BTreeMap<PathBuf, LineCounts>) {
    for change in changes {
        if let Some(found) = counts.get(&change.path) {
            change.counts = Some(*found);
        }
    }
}

/// Build the changed-paths tree rooted at `root` from git's answer.
///
/// `root` supplies only the tree's display name; `changes` carry paths already
/// relative to it, because that is how git reports them. A path that escapes
/// the root (`..`, or an absolute path) contributes no node — git does not
/// produce such paths, and rejecting them keeps a malformed one from rendering
/// above the worktree.
///
/// A path can appear twice, because the two git queries overlap: `git rm
/// --cached foo` leaves `foo` reported as `D` by the diff *and* listed as
/// untracked. [`change_precedence`] resolves it, so which query ran first
/// cannot change a badge.
pub fn build_tree(root: &Path, changes: &[GitFileChange]) -> TreeNode {
    let mut badges: BTreeMap<Vec<OsString>, (FileChange, Option<LineCounts>)> = BTreeMap::new();

    for change in changes {
        let Some(components) = relative_components(&change.path) else {
            continue;
        };
        let (badge, counts) = badges
            .entry(components)
            .or_insert((change.change, change.counts));
        if change_precedence(change.change) > change_precedence(*badge) {
            *badge = change.change;
        }
        // Counts are merged independently of the badge, because the two queries
        // that collide on a path do not both supply them: only the diff counts,
        // and the untracked listing never does. Taking the counted answer
        // whichever query produced it is what keeps the result independent of
        // which ran first — the same property `change_precedence` gives the
        // badge.
        *counts = counts.or(change.counts);
    }

    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let mut root_node = TreeNode {
        name: root_name,
        kind: TreeNodeKind::Directory,
        badge: None,
        expanded: false,
        children: Vec::new(),
        counts: None,
    };

    for (components, (badge, counts)) in badges {
        insert_path(&mut root_node, &components, badge, counts);
    }

    // Before sorting, so the order the user sees is the order of the names the
    // user sees; before the count and expansion passes, so neither writes to a
    // node that merging is about to remove.
    merge_single_child_chains(&mut root_node);
    sort_children(&mut root_node);
    compute_expansion(&mut root_node);
    compute_counts(&mut root_node);

    root_node
}

/// Rank for resolving a path reported by both git queries. Higher wins.
///
/// `Added` beats `Deleted` because the two only collide when the file is on
/// disk but out of the index (`git rm --cached`), and "it is there" is the more
/// useful of the two true statements. `Deleted` beats `Modified` because a
/// deletion is the more consequential fact and the one a file tree exists to
/// show.
fn change_precedence(change: FileChange) -> u8 {
    match change {
        FileChange::Modified => 0,
        FileChange::Deleted => 1,
        FileChange::Added => 2,
    }
}

/// Split a git-reported path into name segments, or `None` if it is not a plain
/// relative path below the root.
///
/// Every segment must be `Component::Normal`: `..`, a leading `/`, a `./` and a
/// Windows prefix all reject the whole path, as does an empty result. Git emits
/// none of these — its paths are normalised and repo-relative — so this is a
/// fail-closed backstop, not a normaliser. Rejecting outright rather than
/// sanitising is the point: a path this does not recognise is one whose meaning
/// we cannot vouch for, and the pane's rooting at the worktree
/// (`TaskPaneRootIsTaskWorktree`) is what depends on getting it right.
///
/// Shared with [`crate::agent_tree::open_set::read_open_set`], which applies it
/// to every path it reads back off disk — one guard, so the two cannot drift
/// apart on which components they consider safe.
pub(crate) fn relative_components(path: &Path) -> Option<Vec<OsString>> {
    use std::path::Component;

    // `collect` into `Option<Vec<_>>` short-circuits on the first `None`, so one
    // non-Normal component rejects the path.
    let components: Option<Vec<OsString>> = path
        .components()
        .map(|component| match component {
            Component::Normal(segment) => Some(OsString::from(segment)),
            _ => None,
        })
        .collect();
    components.filter(|c| !c.is_empty())
}

fn insert_path(
    node: &mut TreeNode,
    components: &[OsString],
    badge: FileChange,
    counts: Option<LineCounts>,
) {
    let Some((head, rest)) = components.split_first() else {
        return;
    };
    let name = head.to_string_lossy().into_owned();

    if rest.is_empty() {
        node.children.push(TreeNode {
            name,
            kind: TreeNodeKind::File,
            badge: Some(badge),
            expanded: false,
            children: Vec::new(),
            counts,
        });
        return;
    }

    let child_index = match node.children.iter().position(|c| c.name == name) {
        Some(index) => index,
        None => {
            node.children.push(TreeNode {
                name,
                kind: TreeNodeKind::Directory,
                badge: None,
                expanded: false,
                children: Vec::new(),
                counts: None,
            });
            node.children.len() - 1
        }
    };
    insert_path(&mut node.children[child_index], rest, badge, counts);
}

/// Fold every chain of directories that hold nothing but the next one into a
/// single node, named by the whole route — the spec's
/// `NoSingleChildDirectoryChains`.
///
/// `a/b/c/d.rs` becomes one directory node `a/b/c` holding `d.rs`, where it
/// was four nested nodes. The route is kept in the name rather than elided,
/// because a bare `c` is not something the user can act on: two directories of
/// that name in different subtrees would be indistinguishable.
///
/// A directory holding a changed file of its own is never merged away,
/// whatever else it holds — it is not a link in a single-child chain — which
/// is what keeps a file's own parent visible as the row above it.
///
/// This merges a node's CHILDREN into their own grandchildren and never merges
/// `node` itself into anything, so calling it on the synthetic root leaves the
/// root alone. That matters: the root's name is the pane root, and it is not
/// part of any node's relative path.
fn merge_single_child_chains(node: &mut TreeNode) {
    for child in &mut node.children {
        merge_single_child_chains(child);
        // One absorb step, not a loop: the line above already merged the
        // child's own subtree, so the directory it absorbs here has either two
        // or more children or a single FILE child, and the condition cannot
        // hold a second time.
        if child.kind == TreeNodeKind::Directory
            && child.children.len() == 1
            && child.children[0].kind == TreeNodeKind::Directory
        {
            let mut only = child.children.remove(0);
            child.name.push('/');
            child.name.push_str(&only.name);
            child.children = std::mem::take(&mut only.children);
        }
    }
}

/// Give every directory node the sum of the counts beneath it, and return what
/// this node contributes to its own parent.
///
/// The sum SKIPS descendants with no counts rather than folding them in as
/// zero, and a directory whose descendants all lack counts is left with `None`.
/// A directory holding one modified file and three brand-new ones must not
/// report the modified file's numbers as though they were the whole story of
/// the directory, and `+0 -0` on a directory full of unstaged files would read
/// as "nothing changed in there" — which is the one thing that is certainly
/// false. See the spec's `DirectoryCountsSumDescendants`.
///
/// A file node's own counts are already in place from [`insert_path`] and are
/// left untouched: this pass only ever writes to directories.
fn compute_counts(node: &mut TreeNode) -> Option<LineCounts> {
    if node.kind == TreeNodeKind::File {
        return node.counts;
    }

    // `filter_map` drops the descendants with no counts and `reduce` yields
    // `None` when every one of them did — which is exactly the rule this pass
    // exists for, expressed rather than assembled.
    let total = node
        .children
        .iter_mut()
        .filter_map(compute_counts)
        .reduce(std::ops::Add::add);
    node.counts = total;
    total
}

/// Order siblings files first, then directories, each group by the FIRST
/// segment of the name — the spec's `RowsPutAFoldersOwnFilesFirst`.
///
/// Runs after [`merge_single_child_chains`], so a merged directory's name is a
/// whole route; only its first segment is compared. Two reasons, and the
/// second is the load-bearing one:
///
///   * Compressing a chain then never reorders rows. `src/agent/…` sits where
///     `src/agent` sat, whether or not a sibling change happened to split the
///     chain apart this second.
///   * A reader given only paths sees the same order. Comparing whole routes
///     would make row order depend on where the merges fell, which is a
///     property of the whole change set — so a sibling's name having the
///     route's first segment as a proper prefix followed by a byte below `/`
///     (0x2f) would reorder the rows: `agent-health` sorts before
///     `agent/tree` but after `agent`. This is what lets the diff pane render
///     the paths it is handed without knowing anything about the tree.
fn sort_children(node: &mut TreeNode) {
    node.children.sort_by(|a, b| {
        kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then_with(|| first_segment(&a.name).cmp(first_segment(&b.name)))
    });
    for child in &mut node.children {
        sort_children(child);
    }
}

/// The first path component of a node's name — the whole name for everything
/// but a chain-merged directory.
fn first_segment(name: &str) -> &str {
    name.split_once('/').map_or(name, |(head, _)| head)
}

/// Sort key putting files ahead of directories.
fn kind_rank(kind: TreeNodeKind) -> u8 {
    match kind {
        TreeNodeKind::File => 0,
        TreeNodeKind::Directory => 1,
    }
}

/// Every FILE path in the tree, relative to the root, in TREE ORDER — the
/// order the rows appear in the pane.
///
/// Directories contribute their descendants but never themselves: a directory
/// has no contents of its own to diff, which is what the spec's
/// `OnlyFilesOpenDiffs` says.
///
/// A `Vec`, not a set, because the order is the point: it is the order the
/// tree publishes to the diff pane, which renders the paths as received (see
/// `crate::agent_tree::open_set::write_open_set`).
pub fn file_paths_in_tree_order(root: &TreeNode) -> Vec<PathBuf> {
    // `join`, not a shared `PathBuf` pushed and popped down the walk. A
    // chain-merged directory's name spans several components, and
    // `PathBuf::pop` removes one component rather than undoing one `push` — so
    // an accumulator would need to know each name's arity to stay in step, and
    // getting it wrong truncates every path below that row while still looking
    // like a valid relative path. Joining cannot get it wrong.
    fn walk(node: &TreeNode, prefix: &Path, out: &mut Vec<PathBuf>) {
        for child in &node.children {
            let path = prefix.join(&child.name);
            match child.kind {
                TreeNodeKind::File => out.push(path),
                TreeNodeKind::Directory => walk(child, &path, out),
            }
        }
    }

    let mut out = Vec::new();
    walk(root, Path::new(""), &mut out);
    out
}

fn compute_expansion(node: &mut TreeNode) -> bool {
    if node.kind == TreeNodeKind::File {
        return node.badge.is_some();
    }
    let mut has_changed_descendant = false;
    for child in &mut node.children {
        if compute_expansion(child) {
            has_changed_descendant = true;
        }
    }
    node.expanded = has_changed_descendant;
    has_changed_descendant
}

#[cfg(test)]
pub(super) mod tests;
