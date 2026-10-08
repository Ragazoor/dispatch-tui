//! The agent-tree pane's view state: the tree widget's cursor and manual
//! collapses, the open diffs, which section has focus, and the one-line notice
//! in the bottom border. Pure data plus the bookkeeping that keeps it coherent
//! across a rebuild; it runs no git and draws nothing.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use tui_tree_widget::TreeState;

use crate::agent_tree::model::{TreeNode, TreeNodeKind};
use crate::agent_tree::render::agents::{AgentRow, AgentsSection};
use crate::agent_tree::render::commits::{AgentCommit, CommitsSection};

/// A one-line failure notice, tagged with which of its writers set it.
/// Rendered in the pane's bottom border, and while one is set the whole border
/// is drawn red — see `AgentTreeNoticeRedensBorder`.
///
/// The tag is what lets a recovering git query clear its own stale notice
/// without also wiping the answer to a keypress the user made half a second ago
/// (see [`RenderState::clear_git_notice`] and the spec's `NoticeSource`).
/// Modelled as a variant rather than a field beside the text because the two
/// are only ever meaningful together — which is exactly what the spec's
/// `notice_source: NoticeSource when error_notice != null` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Git(String),
    /// A diff pane that could not be opened. The direct answer to a keypress
    /// the user made moments ago, which is why a recovering git query must not
    /// clear it — see [`RenderState::clear_git_notice`].
    Diff(String),
    /// The board's task list could not be read for the agents section. The
    /// agents timer's own writer, like `Git` is the tree's: the next working
    /// read clears it (`RefreshAgentTreeAgentList`).
    AgentList(String),
    /// Another agent's window could not be selected. The answer to a keypress,
    /// like `Diff`, so no timer clears it (`AgentTreeAgentJumpFailureIsVisible`).
    AgentJump(String),
    /// The agent's commits could not be read for the commits section. The
    /// commits timer's own writer: the next working read clears it
    /// (`RefreshAgentTreeCommitList`, `AgentTreeCommitListFailureKeepsLastList`).
    CommitList(String),
}

impl Notice {
    pub fn git(text: impl Into<String>) -> Self {
        Self::Git(text.into())
    }

    pub fn diff(text: impl Into<String>) -> Self {
        Self::Diff(text.into())
    }

    pub fn agent_list(text: impl Into<String>) -> Self {
        Self::AgentList(text.into())
    }

    pub fn agent_jump(text: impl Into<String>) -> Self {
        Self::AgentJump(text.into())
    }

    pub fn commit_list(text: impl Into<String>) -> Self {
        Self::CommitList(text.into())
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Git(text)
            | Self::Diff(text)
            | Self::AgentList(text)
            | Self::AgentJump(text)
            | Self::CommitList(text) => text,
        }
    }
}

/// Which of the pane's three sections the keys act on — the spec's
/// `AgentTreeFocus`. Tab cycles it tree -> commits -> agents
/// (`SwitchAgentTreeFocus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Tree,
    Commits,
    Agents,
}

/// Tree-widget navigation/expansion state, plus the set of directories the
/// user has collapsed by hand.
///
/// `sync_expansion` opens every directory holding a change EXCEPT the ones in
/// that set, on every rebuild. Both halves of `RefreshAgentTree`'s expansion
/// rule fall out of it: a directory that appears is expanded, and a manual
/// collapse is never overwritten.
///
/// Recording what the user CLOSED, rather than what has already been opened
/// once, is what makes it survive a row being renamed. A chain-merged
/// directory's row is renamed whenever a sibling change merges or unmerges the
/// chain, and the two failure modes are mirror images: remember the opens by
/// row identity and a renamed row springs back open after a manual collapse;
/// remember them by path and a renamed row the user never touched comes back
/// COLLAPSED, hiding a badge. Only "did the user close this path" answers both.
pub struct RenderState {
    pub tree_state: TreeState<String>,
    /// Directories the user has collapsed by hand, keyed by the node's relative
    /// PATH rather than by the chain of widget identifiers — see the struct
    /// doc for why the path is the stable key.
    ///
    /// Written from the widget's own open set, diffed across each key press
    /// ([`RenderState::absorb_manual_expansion`]), because three separate keys
    /// move expansion and two of them are cursor motions as well: only the
    /// widget knows which of the two a press turned out to be.
    user_collapsed: HashSet<PathBuf>,
    /// A one-line failure notice, rendered in the pane's bottom border and
    /// cleared by the next key press. Two things set it: a failed git query
    /// and a failed diff-pane split. While it is set the border is drawn red —
    /// see `AgentTreeNoticeRedensBorder` in docs/specs/agent-tree.allium.
    pub notice: Option<Notice>,
    /// Which files' diffs the user has opened, as paths relative to the pane
    /// root.
    ///
    /// The only state here the user builds up rather than git supplying:
    /// everything else the pane shows is re-derived from a git query every
    /// tick. A set rather than a cursor, because the diff pane shows every open
    /// file at once — which is what makes the all-files key a bulk version of
    /// the single-file toggle rather than a second mode.
    ///
    /// It holds PATHS, not nodes, so it survives a refresh that rebuilds every
    /// node. A path whose file git stops reporting simply stops matching
    /// anything; it is not pruned, because the agent may change the file again
    /// and the user did not ask for it to be closed. See
    /// `OpenDiffPathsMaySurviveTheirFiles` in docs/specs/agent-tree.allium.
    pub(super) open_diffs: BTreeSet<PathBuf>,
    /// Whether a lone `g` is waiting for the second half of the `gg` chord.
    /// Unlike the board's chord this one carries no deadline, so there is no
    /// timestamp beside it — see `AgentTreeGgChordNeverExpires` in
    /// docs/specs/agent-tree.allium for why a clock would buy nothing here.
    pub(super) pending_g: bool,
    /// Rows the tree had to draw into at the last render — the pane's height
    /// less its two borders. Recorded by [`render`](crate::agent_tree::render::render) because the half-page
    /// motions are defined against the *visible* height, which only the
    /// renderer knows, and `handle_key` never sees a `Rect`.
    pub(super) viewport_rows: usize,
    /// Which section the keys act on. Starts on the tree.
    pub focus: Focus,
    /// The agents section beneath the tree, with its own cursor.
    pub agents: AgentsSection,
    /// The commits section between the tree and the agents section, with its
    /// own cursor.
    pub commits: CommitsSection,
    /// Which change the tree and the diff pane show: `None` for unstaged work,
    /// or the id of one listed commit — the spec's `pane.selected_commit`.
    pub selected_commit: Option<String>,
    /// Keypress usage events for presses that took effect, waiting for the
    /// loop to write them through the pane's store connection.
    pub usage: Vec<crate::models::UsageEvent>,
}

impl RenderState {
    pub fn new() -> Self {
        Self {
            tree_state: TreeState::default(),
            user_collapsed: HashSet::new(),
            notice: None,
            open_diffs: BTreeSet::new(),
            pending_g: false,
            viewport_rows: 0,
            focus: Focus::Tree,
            agents: AgentsSection::new(),
            commits: CommitsSection::new(),
            selected_commit: None,
            usage: Vec::new(),
        }
    }

    /// How far `Ctrl-D`/`Ctrl-U` move: half the last-rendered visible height,
    /// floored at one row. A pane too short to show two rows would otherwise
    /// halve to zero and turn both motions into no-ops, which reads as a
    /// broken key rather than a small pane.
    pub(super) fn half_page(&self) -> usize {
        crate::agent_tree::pane::half_page(self.viewport_rows)
    }

    /// The paths whose diffs are open, in the set's own order — which is NOT
    /// tree order, and has not been since a folder's own files started sorting
    /// ahead of its subfolders. The diff pane re-applies the row order itself
    /// (`crate::agent_tree::model::file_paths_in_tree_order`); nothing here depends on
    /// the order the set happens to iterate in.
    pub fn open_diffs(&self) -> &BTreeSet<PathBuf> {
        &self.open_diffs
    }

    /// The open paths in TREE ORDER — the order their rows appear in the pane,
    /// which is the order the diff pane renders them in.
    ///
    /// The tree is the only party that can answer this. Where a directory chain
    /// compresses depends on the whole change set, so a reader holding only the
    /// opened subset could never re-derive it (see the spec's
    /// `RowsPutAFoldersOwnFilesFirst`). That is why the order travels with the
    /// paths rather than being recomputed at the far end.
    ///
    /// A path the tree no longer knows about is appended rather than dropped:
    /// the open set outlives its files on purpose (`OpenDiffPathsMaySurviveTheirFiles`),
    /// and such a path renders nothing, so where it sits cannot be seen.
    pub(super) fn open_diffs_in_tree_order(&self, tree: &TreeNode) -> Vec<PathBuf> {
        let mut ordered: Vec<PathBuf> = crate::agent_tree::model::file_paths_in_tree_order(tree)
            .into_iter()
            .filter(|path| self.open_diffs.contains(path))
            .collect();
        let known: BTreeSet<&PathBuf> = ordered.iter().collect();
        let mut orphans: Vec<PathBuf> = self
            .open_diffs
            .iter()
            .filter(|path| !known.contains(path))
            .cloned()
            .collect();
        ordered.append(&mut orphans);
        ordered
    }

    pub fn is_diff_open(&self, path: &Path) -> bool {
        self.open_diffs.contains(path)
    }

    /// Open `path`'s diff if it is closed, close it if it is open.
    pub(super) fn toggle_diff(&mut self, path: PathBuf) {
        if !self.open_diffs.remove(&path) {
            self.open_diffs.insert(path);
        }
    }

    /// Open every changed file's diff, or close everything if anything is
    /// already open.
    ///
    /// Direction is decided by the set rather than by a remembered mode, so one
    /// press always has one meaning for the state the user can see. The paths
    /// come from the last rendered tree — the one the user is looking at —
    /// which is also why the key still works while a notice is showing and the
    /// pane is holding its last good answer.
    pub(super) fn toggle_all_diffs(&mut self, root: &TreeNode) {
        if !self.open_diffs.is_empty() {
            self.open_diffs.clear();
            return;
        }
        self.open_diffs = collect_file_paths(root);
    }

    /// Clear a notice left by a failed git query, leaving a diff-open notice
    /// alone. Called after every SUCCESSFUL query: a working git retracts its
    /// own complaint, but must not swallow the answer to a keypress the user
    /// made moments ago (`RefreshAgentTree`'s clearing expression).
    pub(super) fn clear_git_notice(&mut self) {
        if matches!(self.notice, Some(Notice::Git(_))) {
            self.notice = None;
        }
    }

    /// Take one read of the board's task list. A working read replaces the
    /// rows and clears the agents timer's own stale notice, nothing else; a
    /// failed one keeps the last rows and says why — the spec's
    /// `RefreshAgentTreeAgentList` and `AgentTreeAgentListFailureKeepsLastList`.
    pub fn adopt_agent_list(&mut self, read: Result<Vec<AgentRow>, String>) {
        match read {
            Ok(rows) => {
                self.agents.set_rows(rows);
                if matches!(self.notice, Some(Notice::AgentList(_))) {
                    self.notice = None;
                }
            }
            Err(reason) => self.notice = Some(Notice::AgentList(reason)),
        }
    }

    /// Take one read of the agent's commits — the spec's
    /// `RefreshAgentTreeCommitList` and `AgentTreeCommitListFailureKeepsLastList`.
    pub fn adopt_commit_list(&mut self, read: Result<Vec<AgentCommit>, String>) {
        match read {
            Ok(commits) => {
                // Kept by id: a new commit above the selection leaves it on
                // the same commit, and one the branch no longer holds falls
                // back to unstaged work, silently.
                if let Some(id) = &self.selected_commit {
                    if !commits.iter().any(|c| &c.id == id) {
                        self.selected_commit = None;
                    }
                }
                self.commits.set_commits(commits);
                if matches!(self.notice, Some(Notice::CommitList(_))) {
                    self.notice = None;
                }
            }
            Err(reason) => self.notice = Some(Notice::CommitList(reason)),
        }
    }

    /// Open every directory holding a change, except the ones the user has
    /// collapsed by hand — see the struct doc comment.
    ///
    /// Idempotent, and run on every rebuild rather than once per directory: the
    /// answer is a function of `user_collapsed` and the tree, so re-deriving it
    /// cannot drift from either.
    pub fn sync_expansion(&mut self, root: &TreeNode) {
        let mut represented = HashSet::new();
        self.sync_expansion_at(&root.children, &mut Vec::new(), &mut represented);
        // A path no row stands for any more cannot be collapsed, and forgetting
        // it is deliberate: the agent reverts its last edit under `src/`, the
        // row goes, and if it later edits there again that is news — the row
        // must open rather than come back closed from a collapse the user made
        // about a different state of the worktree.
        //
        // "Stands for", not "is the exact path of": a merged row stands for
        // every link it absorbed, so collapsing `a` and then watching `a` be
        // absorbed into `a/b` keeps the collapse rather than dropping it.
        self.user_collapsed
            .retain(|path| represented.contains(path));
    }

    /// Every directory path one row stands for: its own, and each intermediate
    /// one a chain-merged row absorbed. The row `a/b/c` stands for `a`, `a/b`
    /// and `a/b/c`, because collapsing it hides exactly what collapsing any of
    /// the three used to hide.
    ///
    /// Returned innermost-last, so the final entry is the row's own path.
    ///
    /// Takes the row's WIDGET KEY, which is how both callers already identify a
    /// row — `sync_expansion_at` accumulates it and a key press reads it off
    /// the selection. Its last element is the row's own name and the elements
    /// before it join to its parent's path, so no separate parent has to be
    /// threaded anywhere.
    ///
    /// Its ANCESTORS' paths are deliberately excluded: a row can only be
    /// pressed while it is visible, so a collapse on an ancestor is not
    /// something a press on this row could have meant to clear.
    fn represented_paths(key: &[String]) -> Vec<PathBuf> {
        let Some((name, ancestors)) = key.split_last() else {
            return Vec::new();
        };
        let mut prefix: PathBuf = ancestors.iter().collect();
        name.split('/')
            .map(|segment| {
                prefix.push(segment);
                prefix.clone()
            })
            .collect()
    }

    /// Fold whatever the last key press did to the widget's open set into
    /// [`RenderState::user_collapsed`], translated from widget identifiers to
    /// relative paths.
    ///
    /// Read as a diff rather than recorded key by key because the widget
    /// decides what a press meant: `h` collapses a directory OR steps out to
    /// the parent, and Space dispatches on the selected node's kind. The
    /// before/after difference is the only place that knows which happened.
    pub(super) fn absorb_manual_expansion(&mut self, before: &HashSet<Vec<String>>) {
        let after = self.tree_state.opened();
        if before == after {
            return;
        }
        // Collected before either loop mutates `user_collapsed`, which ends the
        // borrow of `after`.
        //
        // The two sides are deliberately asymmetric. A CLOSE records the row's
        // own path only: the user closed this row, and if the chain later
        // unmerges, the outer link has gained something of its own worth
        // seeing, so only the inner one stays closed. An OPEN clears every path
        // the row stands for, because a collapse recorded on a link this row
        // absorbed is exactly what would hide it again — and an explicit open
        // must not be weaker than an explicit close.
        let closed: Vec<PathBuf> = before
            .difference(after)
            .map(|key| key.iter().collect())
            .collect();
        let opened: Vec<PathBuf> = after
            .difference(before)
            .flat_map(|key| Self::represented_paths(key))
            .collect();
        for path in closed {
            self.user_collapsed.insert(path);
        }
        for path in opened {
            self.user_collapsed.remove(&path);
        }
    }

    /// `path` doubles as the widget's open-set key: it looks a node up by
    /// the chain of its ancestors' identifiers, and `node_to_item` identifies
    /// each node by its own `name`, so the two coincide — merged names
    /// included, since the widget only requires sibling uniqueness.
    fn sync_expansion_at(
        &mut self,
        children: &[TreeNode],
        path: &mut Vec<String>,
        represented: &mut HashSet<PathBuf>,
    ) {
        for child in children {
            if child.kind != TreeNodeKind::Directory {
                continue;
            }
            path.push(child.name.clone());
            // `path` is the widget's key, and joining its elements gives the
            // row's relative path — which is the stable key, see
            // `user_collapsed`. A merged name contributes several components,
            // so the keys `["a/b"]` and `["a", "b"]` both name the path `a/b`.
            let stands_for = Self::represented_paths(path);
            let closed = stands_for
                .iter()
                .any(|candidate| self.user_collapsed.contains(candidate));
            if child.expanded && !closed {
                self.tree_state.open(path.clone());
            }
            represented.extend(stands_for);
            self.sync_expansion_at(&child.children, path, represented);
            path.pop();
        }
    }
}

impl Default for RenderState {
    fn default() -> Self {
        Self::new()
    }
}

/// Every FILE path in the tree, relative to the root, as the open set holds
/// them.
///
/// The walk itself belongs to `crate::agent_tree::model`, where the tree does — the
/// diff pane needs the same walk's ORDER, so one home for it is the point (see
/// `file_paths_in_tree_order`). The set here keeps only membership: the open
/// set is asked "is this path open", never "what came first".
fn collect_file_paths(root: &TreeNode) -> BTreeSet<PathBuf> {
    crate::agent_tree::model::file_paths_in_tree_order(root)
        .into_iter()
        .collect()
}
