//! `dispatch agent-tree <task_id>` — a small, standalone ratatui loop that
//! renders one task's changed-file tree (see `docs/specs/agent-tree.allium`'s
//! `AgentTreeCompanionPane` surface and `RefreshAgentTree` rule).
//!
//! Deliberately NOT part of the board TUI's `App`/message loop: this runs as its
//! own process in a tmux companion pane.
//!
//! Git is the sole source of truth for what the tree shows — see the spec's
//! `AgentTreeIsGitDerived` guarantee. This module owns the running of git
//! ([`git_changes`]) and the polling loop around it; parsing its output and
//! folding the result into a tree belong to `crate::agent_tree`. It does not
//! merge in a full worktree filesystem scan and never will: git already answers
//! the question a scan was meant to approximate, which is what resolved the
//! spec's old `TreeScanExclusions` question.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use ratatui::{Frame, Terminal};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::agent_tree::{
    attach_line_counts, build_tree, parse_name_status, parse_numstat, parse_untracked, FileChange,
    GitFileChange, TreeNode, TreeNodeKind,
};
use crate::cli::agent_tree_agents::{border_style, render_agents, AgentRow, AgentsSection};
use crate::models::{TaskId, TmuxWindow};
use crate::process::{stderr_str, ProcessRunner, RealProcessRunner};
use crate::tui::ui::palette::{FG, GREEN, MUTED, RED, YELLOW};

/// Redraw cadence — see `docs/specs/agent-tree.allium`'s
/// `config.agent_tree_refresh_interval`. Doubles as the crossterm event
/// poll timeout, so a key press and a plain timer tick share one wait.
pub(crate) const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// How long ONE git command may run before it is killed and treated as a
/// failure — `config.agent_tree_git_timeout` in the spec.
///
/// Deliberately far below [`crate::process::SUBPROCESS_TIMEOUT`], which the
/// board's other git calls use. Those run on a worker while the TUI stays live;
/// these run inline in this loop, so the timeout bounds how long this pane can
/// ignore a keypress.
///
/// The bound is PER COMMAND, and a tick runs up to six of them
/// ([`git_changes`]), so the arithmetic worst case is six times this. Only
/// three of the six can realistically reach it: the two `diff`s and `ls-files`
/// touch the index, and a lock the agent's own git holds is by far the
/// commonest cause of a slow query. The three `merge-base` probes walk refs and
/// objects only and take no lock, so the practical ceiling is unchanged by the
/// baseline resolution.
pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(5);

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

    pub fn text(&self) -> &str {
        match self {
            Self::Git(text) | Self::Diff(text) | Self::AgentList(text) | Self::AgentJump(text) => {
                text
            }
        }
    }
}

/// Which of the pane's two sections the keys act on — the spec's
/// `AgentTreeFocus`. Tab toggles it (`SwitchAgentTreeFocus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Tree,
    Agents,
}

/// The `+N -M` half of a row, or nothing when the node has no counts.
///
/// Absence is rendered as absence, never as `+0 -0`: a zero is a real answer
/// for a tracked file that moved no lines (a permission change), and printing
/// one for an untracked file — which git could not count at all — would read as
/// "nothing changed in there" about a file the agent has just written. See the
/// spec's `UntrackedFilesHaveNoLineCounts`.
fn count_spans(node: &TreeNode) -> Vec<Span<'static>> {
    let Some(counts) = node.counts else {
        return Vec::new();
    };
    vec![
        Span::raw(" "),
        Span::styled(format!("+{}", counts.added), Style::default().fg(GREEN)),
        Span::raw(" "),
        Span::styled(format!("-{}", counts.removed), Style::default().fg(RED)),
    ]
}

/// Whether this row draws its counts at all — see the spec's
/// `CountsShowOnTheFolderNearestTheFiles`.
///
/// A row draws them when it holds no folder — which is what stops one file's
/// `+40 -10` from appearing again on every folder above it. A FILE satisfies
/// that by having no children at all, so it always draws its own numbers and
/// needs no arm of its own here.
///
/// A COLLAPSED row draws them whatever it holds: there the sum is not a
/// restatement of rows on screen, it is the only thing saying how much is
/// hidden.
fn shows_counts(node: &TreeNode, collapsed: bool) -> bool {
    collapsed
        || node
            .children
            .iter()
            .all(|child| child.kind == TreeNodeKind::File)
}

/// The node's name, split so a chain-merged directory's route recedes: the
/// leading segments dimmed, the last one in the ordinary colour. See the
/// spec's `MergedDirectoryChainRows`.
///
/// A name with no separator is one span, exactly as before merging existed —
/// there is no route to dim.
fn name_spans(node: &TreeNode) -> Vec<Span<'static>> {
    match node.name.rfind('/') {
        // `..=cut` keeps the separator on the dimmed half, so the eye lands on
        // the folder name rather than on the slash before it.
        Some(cut) => vec![
            Span::styled(node.name[..=cut].to_owned(), Style::default().fg(MUTED)),
            Span::styled(node.name[cut + 1..].to_owned(), Style::default().fg(FG)),
        ],
        None => vec![Span::styled(node.name.clone(), Style::default().fg(FG))],
    }
}

/// Marks a file whose diff is open in the pane below. Rendered for every FILE
/// row, as the marker or as a blank of the same width, so the names stay in one
/// column whatever is open — a marker that shifted its neighbours would make
/// the set harder to read, not easier.
///
/// Directories never carry it: only files can be open (`OnlyFilesOpenDiffs`),
/// and giving them the blank as well would indent them out of line with the
/// files beneath them.
const DIFF_OPEN_MARKER: &str = "\u{25cf} ";
const DIFF_CLOSED_MARKER: &str = "  ";

/// One rendered row: the open marker on a file, the name, then the badge if the
/// node has one, then the line counts if it has any AND this row is one that
/// draws them ([`shows_counts`]).
///
/// A directory can reach the counts too. It carries no badge — git says nothing
/// about directories, and `OnlyFilesCarryBadges` holds that — but it does hold
/// the sum over everything beneath it, which is what a collapsed row draws to
/// say how much is inside without being opened.
fn node_label(node: &TreeNode, diff_open: bool, collapsed: bool) -> Line<'static> {
    let mut spans = Vec::new();
    if node.kind == TreeNodeKind::File {
        spans.push(Span::styled(
            if diff_open {
                DIFF_OPEN_MARKER
            } else {
                DIFF_CLOSED_MARKER
            },
            Style::default().fg(YELLOW),
        ));
    }
    spans.extend(name_spans(node));

    if let Some(change) = node.badge {
        let (badge, style) = match change {
            FileChange::Added => ("[Added]", Style::default().fg(GREEN)),
            FileChange::Modified => (
                "[Modified]",
                Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
            ),
            FileChange::Deleted => (
                "[Deleted]",
                Style::default().fg(RED).add_modifier(Modifier::BOLD),
            ),
        };
        spans.push(Span::raw(" "));
        spans.push(Span::styled(badge, style));
    }

    if shows_counts(node, collapsed) {
        spans.extend(count_spans(node));
    }
    Line::from(spans)
}

/// Everything the item walk reads out of the view state, so a node's label can
/// be built without handing the walk the whole [`RenderState`].
struct ItemContext<'a> {
    /// Which files' diffs are open, as paths relative to the pane root.
    open_diffs: &'a BTreeSet<PathBuf>,
    /// The widget's open set, keyed exactly as `path` below is built. A
    /// directory MISSING from it is collapsed, which is what decides whether
    /// its row draws its counts (`shows_counts`).
    opened: &'a HashSet<Vec<String>>,
}

fn node_to_item(
    node: &TreeNode,
    path: &mut Vec<String>,
    parent: &Path,
    ctx: &ItemContext<'_>,
) -> Option<TreeItem<'static, String>> {
    path.push(node.name.clone());
    // `relative` names this node below the root, which is exactly the shape the
    // open set holds — see `RenderState::open_diffs`. Joined from the parent
    // rather than pushed onto a shared accumulator: a chain-merged directory's
    // name spans several components, and `PathBuf::pop` removes one component
    // rather than undoing one `push`, so an accumulator would have to track
    // each name's arity to unwind correctly. One allocation per node per frame
    // is the cheaper mistake — `file_paths_in_tree_order` made the same trade.
    let relative = parent.join(&node.name);
    let diff_open = ctx.open_diffs.contains(&relative);
    // `path` is the widget's identifier for this node — see `build_tree_items`
    // — so it is also the key the widget's own open set is stored under.
    let collapsed = node.kind == TreeNodeKind::Directory && !ctx.opened.contains(path);
    let label = node_label(node, diff_open, collapsed);
    let item = match node.kind {
        TreeNodeKind::File => Some(TreeItem::new_leaf(node.name.clone(), label)),
        TreeNodeKind::Directory => {
            let children = to_items(&node.children, path, &relative, ctx);
            match TreeItem::new(node.name.clone(), label, children) {
                Ok(item) => Some(item),
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        path = path.join("/"),
                        "skipping agent-tree node with duplicate child identifiers"
                    );
                    None
                }
            }
        }
    };
    path.pop();
    item
}

fn to_items(
    children: &[TreeNode],
    path: &mut Vec<String>,
    parent: &Path,
    ctx: &ItemContext<'_>,
) -> Vec<TreeItem<'static, String>> {
    children
        .iter()
        .filter_map(|node| node_to_item(node, path, parent, ctx))
        .collect()
}

/// Convert the changed-paths tree into `tui_tree_widget` items. The root node
/// itself is not rendered as a wrapping item — its children become the
/// top-level list, like a normal file browser.
///
/// A node is identified by its own `name` verbatim, which is all the widget
/// requires (identifiers must be unique among siblings only — it already
/// scopes lookups by the chain of ancestor identifiers). A chain-merged
/// directory's name is a whole route, so an identifier is NOT always one path
/// component and a node's widget key is not always its path segments: the key
/// `["a/b"]` and the segments `["a", "b"]` name the same node. What does hold
/// is that joining a key's elements with the path separator gives the node's
/// relative path, which is the property `RenderState::user_collapsed` relies
/// on to survive a row being renamed.
///
/// The `path` accumulator is that key: it keys the widget's own open set (which
/// decides whether a row draws its counts) and gives the duplicate-identifier
/// warning somewhere useful to point.
pub fn build_tree_items(
    root: &TreeNode,
    open_diffs: &BTreeSet<PathBuf>,
    opened: &HashSet<Vec<String>>,
) -> Vec<TreeItem<'static, String>> {
    to_items(
        &root.children,
        &mut Vec::new(),
        Path::new(""),
        &ItemContext { open_diffs, opened },
    )
}

/// The commit where this worktree forked from `git_ref`, or git's own error if
/// the ref does not resolve.
fn merge_base(root: &str, git_ref: &str, runner: &dyn ProcessRunner) -> Result<String> {
    let sha = run_git(runner, &["-C", root, "merge-base", "HEAD", git_ref])?
        .trim()
        .to_string();
    // Git prints a commit id whenever it exits zero, so this is defensive
    // rather than reachable — but an empty string would be handed to `git diff`
    // as its baseline, where it means something else entirely. Soft-fail into
    // "this ref is not a candidate" instead.
    if sha.is_empty() {
        return Err(anyhow!("git: no common ancestor of HEAD and {git_ref}"));
    }
    Ok(sha)
}

/// Whether `ancestor` is an ancestor of `descendant`.
///
/// `merge-base --is-ancestor` answers with an exit code and no output: 0 for
/// yes, 1 for no. Any other code — or a git that could not be spawned, or one
/// that overran [`GIT_TIMEOUT`] — is not an answer, and is reported as a
/// failure rather than folded into "no".
///
/// That distinction is load-bearing. "No" keeps the LOCAL fork point, so
/// reading an unanswered probe as "no" would silently reinstate exactly the
/// mis-attribution `AgentTreeBaselineIsTaskBaseBranch` exists to forbid — and
/// present it as a correct tree, with no notice and no red border. A failure
/// instead reaches `AgentTreeGitFailureKeepsLastGoodTree`, which keeps the last
/// good tree and says so.
fn is_ancestor(
    root: &str,
    ancestor: &str,
    descendant: &str,
    runner: &dyn ProcessRunner,
) -> Result<bool> {
    let output = runner
        .run_with_timeout(
            "git",
            &[
                "-C",
                root,
                "merge-base",
                "--is-ancestor",
                ancestor,
                descendant,
            ],
            GIT_TIMEOUT,
        )
        .context("could not run git")?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(git_error(&output)),
    }
}

/// Resolve the baseline the tree measures against: this worktree's fork point
/// from `base_branch` — step 1 of the spec's `AgentTreeGitQuery`.
///
/// A base branch is a NAME, and a repo can hold two refs under it: the local
/// branch and its remote-tracking counterpart. Dispatch branches a worktree
/// from whichever of the two is ahead at provision time
/// (`crate::dispatch::worktree`'s `select_start_point`), and the two drift in
/// BOTH directions during normal operation — a base branch the human has not
/// pulled leaves local behind, while a wrap-up that fast-forwards local without
/// pushing leaves it ahead. So each ref is probed and the fork point nearer
/// HEAD wins; see the spec's `AgentTreeBaselineIsTaskBaseBranch` for why
/// picking either ref unconditionally mis-attributes other people's commits to
/// the agent.
///
/// The remote ref comes from [`crate::git::origin_ref`], the crate's one
/// definition of it — deliberately the same one `select_start_point` reaches
/// through, so the two cannot disagree about which ref a worktree branched
/// from. In a repo whose remote is named anything else that ref never
/// resolves, and the baseline falls back to the local branch alone, with the
/// stale-branch mis-attribution that implies.
///
/// A ref that does not resolve is simply not a candidate: a base branch never
/// checked out locally is ordinary and must leave the pane working. Only when
/// NEITHER resolves is there no baseline, and then the LOCAL branch's error is
/// the one returned — that is the name the user put on the task, so it is the
/// one they can act on. A ranking probe that could not answer is a different
/// thing and fails the whole query; see [`is_ancestor`].
pub(crate) fn fork_point(
    root: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> Result<String> {
    let local = merge_base(root, base_branch, runner);
    let remote = merge_base(root, &crate::git::origin_ref(base_branch), runner);

    match (local, remote) {
        // Nearness is ancestry, not commit count. Equal fork points need no
        // ranking, so the probe is skipped — that is the ordinary case, where
        // the two refs agree. Where neither is an ancestor of the other the
        // refs have truly diverged, no choice is right, and the spec settles it
        // by fixed rule: the local ref wins.
        (Ok(local), Ok(remote)) => {
            if local != remote && is_ancestor(root, &local, &remote, runner)? {
                Ok(remote)
            } else {
                Ok(local)
            }
        }
        (Ok(local), Err(_)) => Ok(local),
        (Err(_), Ok(remote)) => Ok(remote),
        (Err(local_err), Err(_)) => Err(local_err),
    }
}

/// Run the git queries behind the tree and return everything they reported,
/// with paths relative to `root`.
///
/// The sequence is the spec's `AgentTreeGitQuery`:
///
///   1. [`fork_point`] — resolve the baseline from the task's base branch.
///   2. `git diff --name-status --no-renames -z <fork point>` — every tracked
///      change against that baseline, committed or not, because the diff is
///      taken against the WORKING TREE. An agent that commits mid-session does
///      not watch its work vanish.
///   3. `git diff --numstat --no-renames -z <fork point>` — how many lines
///      each of those paths gained and lost. A separate ask against the same
///      baseline and the same rename setting, not a richer form of step 2:
///      drift between the two would leave a row's badge and its numbers
///      answering different questions.
///   4. `git ls-files --others --exclude-standard -z` — files the agent created
///      and has not staged, which a diff cannot see. All of them are Added,
///      and none of them carries counts — step 3 cannot see a path that is not
///      in the index.
///
/// Rename detection is off (`--no-renames`): with it on a rename is one entry
/// naming two paths, which the three-value [`FileChange`] vocabulary cannot
/// express. Off, git reports the same rename as a delete plus an add — which is
/// both true and what a file tree should show.
///
/// `-z` on both path-emitting queries is load-bearing, not a style choice:
/// git's default output C-quotes any path containing a non-ASCII byte and
/// separates fields with a tab, so `src/é.rs` would arrive as
/// `"src/\303\251.rs"` and render as that literal string. See
/// [`parse_name_status`] for the full reasoning.
///
/// A path both path-listing queries name (`git rm --cached foo`) is resolved by
/// `build_tree` on precedence, not on the order the two run in — and so are its
/// counts, which the diff supplies and the untracked listing never does.
///
/// Every command is read-only: nothing here fetches, commits, stages or writes
/// to the index, which is what keeps the pane's `ReadOnlyObservation` guarantee
/// true while it runs git against a worktree an agent is actively using.
pub fn git_changes(
    root: &Path,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> Result<Vec<GitFileChange>> {
    let root = root.to_string_lossy().into_owned();
    let baseline = fork_point(&root, base_branch, runner)?;

    let diff = run_git(
        runner,
        &[
            "-C",
            &root,
            "diff",
            "--name-status",
            "--no-renames",
            "-z",
            &baseline,
        ],
    )?;
    // The SAME baseline and the same rename setting as the diff above. The two
    // are one comparison asked twice, and if they ever drifted apart a row's
    // badge and its numbers would be answering different questions.
    let numstat = run_git(
        runner,
        &[
            "-C",
            &root,
            "diff",
            "--numstat",
            "--no-renames",
            "-z",
            &baseline,
        ],
    )?;
    let untracked = run_git(
        runner,
        &[
            "-C",
            &root,
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;

    let mut changes = parse_name_status(&diff);
    attach_line_counts(&mut changes, &parse_numstat(&numstat));
    // Appended after the counts are attached, and deliberately: an untracked
    // path is not in the index, so the numstat query never saw it and there is
    // nothing to attach. See the spec's UntrackedFilesHaveNoLineCounts.
    changes.extend(parse_untracked(&untracked));
    Ok(changes)
}

/// Run one git command, returning its stdout or an error carrying git's own
/// first line of stderr — that line is what reaches the user's border, so it
/// has to say something they can act on ("unknown revision", "index.lock").
///
/// Stdout is returned untrimmed. [`crate::process::stdout_str`] trims the whole
/// buffer, which would eat a leading space off the first `-z` path; these two
/// commands emit NUL-delimited records where every byte between delimiters
/// belongs to the filename.
pub(crate) fn run_git(runner: &dyn ProcessRunner, args: &[&str]) -> Result<String> {
    let output = runner
        .run_with_timeout("git", args, GIT_TIMEOUT)
        .context("could not run git")?;
    if !output.status.success() {
        return Err(git_error(&output));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Git's own first line of stderr, as an error. Shared by every caller here
/// because that line is what reaches the user's border and all of them need it
/// to say the same kind of thing.
fn git_error(output: &std::process::Output) -> anyhow::Error {
    let stderr = stderr_str(output);
    let detail = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("git failed");
    anyhow!("git: {detail}")
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
    open_diffs: BTreeSet<PathBuf>,
    /// Whether a lone `g` is waiting for the second half of the `gg` chord.
    /// Unlike the board's chord this one carries no deadline, so there is no
    /// timestamp beside it — see `AgentTreeGgChordNeverExpires` in
    /// docs/specs/agent-tree.allium for why a clock would buy nothing here.
    pending_g: bool,
    /// Rows the tree had to draw into at the last render — the pane's height
    /// less its two borders. Recorded by [`render`] because the half-page
    /// motions are defined against the *visible* height, which only the
    /// renderer knows, and `handle_key` never sees a `Rect`.
    viewport_rows: usize,
    /// Which section the keys act on. Starts on the tree.
    pub focus: Focus,
    /// The agents section beneath the tree, with its own cursor.
    pub agents: AgentsSection,
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
            usage: Vec::new(),
        }
    }

    /// How far `Ctrl-D`/`Ctrl-U` move: half the last-rendered visible height,
    /// floored at one row. A pane too short to show two rows would otherwise
    /// halve to zero and turn both motions into no-ops, which reads as a
    /// broken key rather than a small pane.
    fn half_page(&self) -> usize {
        crate::cli::half_page(self.viewport_rows)
    }

    /// The paths whose diffs are open, in the set's own order — which is NOT
    /// tree order, and has not been since a folder's own files started sorting
    /// ahead of its subfolders. The diff pane re-applies the row order itself
    /// (`crate::agent_tree::compare_in_tree_order`); nothing here depends on
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
    fn open_diffs_in_tree_order(&self, tree: &TreeNode) -> Vec<PathBuf> {
        let mut ordered: Vec<PathBuf> = crate::agent_tree::file_paths_in_tree_order(tree)
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
    fn toggle_diff(&mut self, path: PathBuf) {
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
    fn toggle_all_diffs(&mut self, root: &TreeNode) {
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
    fn clear_git_notice(&mut self) {
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
    fn absorb_manual_expansion(&mut self, before: &HashSet<Vec<String>>) {
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

/// Render the tree, with `[Added]`/`[Modified]`/`[Deleted]` badges, into `area`.
/// Does no I/O of its own — used by both the real polling loop and snapshot
/// tests. It is not, however, read-only in `state`: besides the widget's own
/// cursor bookkeeping it records `viewport_rows`, which the half-page motions
/// then read. A `handle_key` that has never been preceded by a `render` sees
/// the fallback height, so tests must draw before they press.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    root: &TreeNode,
    state: &mut RenderState,
    title: &str,
) {
    let items = build_tree_items(root, &state.open_diffs, state.tree_state.opened());
    // The half-page motions are defined against the rows the user can actually
    // see, and this is the only place that number exists. Recorded on every
    // draw, so resizing the pane resizes the jump with no further plumbing.
    state.viewport_rows = usize::from(area.height.saturating_sub(2));
    let tree_focused = state.focus == Focus::Tree;
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(tree_focused, state.notice.is_some()))
        .title(format!(" {title} "));
    // The tree's bottom border is where the pane says anything: the tree fills
    // the rows above it and the agents section the rows below, and stealing a
    // row for a status line would move every node the moment a notice appeared.
    //
    // The whole border reddens with it. A single line of border text is easy to
    // miss, and a tree left on screen after a failed git query
    // (AgentTreeGitFailureKeepsLastGoodTree) is indistinguishable from a correct
    // one at a glance — the red frame is the part that carries across the room.
    if let Some(notice) = &state.notice {
        block = block.title_bottom(Line::from(Span::styled(
            format!(" {} ", notice.text()),
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )));
    }

    match Tree::new(&items) {
        Ok(tree) => {
            // The cursor is drawn only in the focused section; the tree keeps
            // its position while unfocused, so Tab back lands where it was.
            let highlight = if tree_focused {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            let tree = tree.block(block).highlight_style(highlight);
            frame.render_stateful_widget(tree, area, &mut state.tree_state);
        }
        Err(e) => {
            tracing::warn!(
                error = ?e,
                "agent-tree: duplicate identifiers building tree, rendering title only"
            );
            frame.render_widget(block, area);
        }
    }
}

/// Render the whole pane: the tree above, the agents section in a band across
/// the bottom (`AgentsSectionSitsBelowTheTree`). The section takes one row per
/// agent up to its cap; the tree keeps the rest.
pub fn render_pane(
    frame: &mut Frame,
    area: Rect,
    root: &TreeNode,
    state: &mut RenderState,
    title: &str,
) {
    let agents_height = state.agents.height().min(area.height);
    let tree_area = Rect {
        height: area.height - agents_height,
        ..area
    };
    let agents_area = Rect {
        y: area.y + tree_area.height,
        height: agents_height,
        ..area
    };
    render(frame, tree_area, root, state, title);
    let focused = state.focus == Focus::Agents;
    let alert = state.notice.is_some();
    render_agents(frame, agents_area, &mut state.agents, focused, alert);
}

/// Select `window`, reporting a failure in the pane's notice — the spec's
/// `JumpToAgentWindow` and `AgentTreeAgentJumpFailureIsVisible`. The commonest
/// failure is a window that closed since the list was last read.
pub(crate) fn jump_to_agent(
    window: &TmuxWindow,
    state: &mut RenderState,
    runner: &dyn ProcessRunner,
) {
    if let Err(e) = crate::tmux::select_window(window, runner) {
        tracing::warn!(window = window.as_str(), error = %format!("{e:#}"), "agent-tree: jump failed");
        state.notice = Some(Notice::agent_jump(format!("{e:#}")));
    }
}

/// What the event loop should do after `handle_key` has processed a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Stay in the loop and redraw.
    Continue,
    /// Leave the loop, which exits the process and so closes the tmux pane.
    Exit,
    /// The set of open diffs moved. The loop reconciles the diff pane with it —
    /// splitting one when the set became non-empty and no pane exists, killing
    /// it when the set emptied. `handle_key` stays pure: every tmux call
    /// belongs to the loop.
    DiffSetChanged,
    /// Space or Enter on another agent's row: select its window. Kept out of
    /// `handle_key` for the same reason as `DiffSetChanged` — tmux belongs to
    /// the loop.
    JumpTo(TmuxWindow),
}

/// Every FILE path in the tree, relative to the root, as the open set holds
/// them.
///
/// The walk itself belongs to `crate::agent_tree`, where the tree does — the
/// diff pane needs the same walk's ORDER, so one home for it is the point (see
/// `file_paths_in_tree_order`). The set here keeps only membership: the open
/// set is asked "is this path open", never "what came first".
fn collect_file_paths(root: &TreeNode) -> BTreeSet<PathBuf> {
    crate::agent_tree::file_paths_in_tree_order(root)
        .into_iter()
        .collect()
}

/// The node the widget's current selection names, if any. A selection path is
/// exactly a node's chain of `name`s below the root — which for a chain-merged
/// directory is a whole route, not one segment (see `build_tree_items` and
/// `sync_expansion_at`) — so resolving it is `TreeNode::node_at`.
///
/// Fails closed, and this is the one place that rule is stated — every key that
/// acts on the selection goes through here: `None` for an empty selection —
/// which `node_at` would otherwise resolve to the root, itself a directory —
/// and `None` for a path that resolves to nothing, i.e. a stale selection left
/// over from before a rebuild.
fn selected_node<'a>(root: &'a TreeNode, selected: &[String]) -> Option<&'a TreeNode> {
    if selected.is_empty() {
        return None;
    }
    root.node_at(selected)
}

/// Apply one key press to the view state — see `docs/specs/agent-tree.allium`'s
/// `AgentTreeCompanionPane` surface for the bindings. One-step cursor and
/// expansion keys each have a vim motion and an arrow key bound to the same
/// action; the four jump motions (`gg`, `G`, `Ctrl-D`, `Ctrl-U`) are vim-only.
///
/// Space and Enter dispatch on the selected node's kind — a directory toggles, a
/// file opens — and expand is directory-only, which is why `root` is a
/// parameter: `tui_tree_widget`'s `open()` has no leaf guard, so without the tree
/// to consult, `l` on a *file* would record a phantom open that the next `h`
/// silently consumes instead of stepping out to the parent (#3834). Collapse
/// needs no guard — on a file it always falls through to stepping out.
///
/// Pure with respect to everything but `state`, so the loop's key handling is
/// testable without a terminal, an event source, or a tmux server.
///
/// Wraps [`dispatch_key`] to record what the press did to the widget's
/// expansion state, which is what lets a manual collapse outlive a row being
/// renamed — see [`RenderState::user_collapsed`]. Every key goes through the
/// recording, including the ones that cannot move expansion: enumerating the
/// exceptions would be one more list to keep in step with the match below.
pub fn handle_key(state: &mut RenderState, root: &TreeNode, key: KeyEvent) -> KeyAction {
    let opened_before = state.tree_state.opened().clone();
    let action = dispatch_key(state, root, key);
    state.absorb_manual_expansion(&opened_before);
    action
}

/// Whether one [`crate::keybindings::KeyContext`] holds for the tree pane's
/// selection. Only the file-tree contexts exist here; a stale or empty
/// selection satisfies neither.
fn tree_context_holds(
    state: &RenderState,
    root: &TreeNode,
    context: crate::keybindings::KeyContext,
) -> bool {
    use crate::keybindings::KeyContext as C;
    let kind = selected_node(root, state.tree_state.selected()).map(|n| n.kind);
    match context {
        C::OnDirectory => kind == Some(TreeNodeKind::Directory),
        C::OnFile => kind == Some(TreeNodeKind::File),
        _ => false,
    }
}

fn dispatch_key(state: &mut RenderState, root: &TreeNode, key: KeyEvent) -> KeyAction {
    use crate::keybindings::{key_label, key_name, lookup, KeyNamespace, KEY_BINDINGS};
    // Any key acknowledges a notice — docs/specs/agent-tree.allium's
    // ClearAgentTreeErrorNotice. Cleared before dispatching, so a key that sets
    // a fresh one wins.
    state.notice = None;

    // The `gg` chord: its first half is pending input, not a lookup, and only
    // the completed chord is looked up. Taking the flag disarms it
    // unconditionally: a second `g` completes the chord, and any other key
    // falls through to its own row having quietly cancelled it. See
    // AgentTreeGgChordNeverExpires in docs/specs/agent-tree.allium — there is
    // no deadline, so the only thing that can end a pending chord is the next
    // key, whenever it comes.
    let was_pending_g = std::mem::take(&mut state.pending_g);
    let mut name = key_name(key);
    let mut label = key_label(key);
    if name == "g" {
        if !was_pending_g {
            state.pending_g = true;
            return KeyAction::Continue;
        }
        name = "gg".to_string();
        label = "gg".to_string();
    }

    // The table decides which action this press runs: the pane-wide keys are
    // rows in both sections (AgentKeysFollowFocus), the cursor keys rows of
    // the focused one. A press with no row does nothing.
    let ns = match state.focus {
        Focus::Tree => KeyNamespace::AgentTreeTree,
        Focus::Agents => KeyNamespace::AgentTreeAgents,
    };
    let Some(row) = lookup(KEY_BINDINGS, ns, &name, |c| {
        tree_context_holds(state, root, c)
    }) else {
        return KeyAction::Continue;
    };
    let action = row.action;
    let half_page = state.half_page();
    let mut took_effect = true;
    let result = match action {
        "exit_pane" => KeyAction::Exit,
        "toggle_focus" => {
            state.focus = match state.focus {
                Focus::Tree => Focus::Agents,
                Focus::Agents => Focus::Tree,
            };
            KeyAction::Continue
        }
        // The all-files key. Unlike Space/Enter it does NOT dispatch on the
        // selection — it acts on the whole tree, whatever the cursor is on,
        // including a directory or nothing at all.
        "toggle_all_diffs" => {
            state.toggle_all_diffs(root);
            KeyAction::DiffSetChanged
        }
        // `TreeState`'s navigation methods return whether anything changed;
        // the loop redraws unconditionally, so the answer is discarded. The
        // jump motions resolve against the identifiers of the last render —
        // the visible rows — so a collapsed directory's children are skipped
        // and nothing is expanded to reach a target. With nothing selected
        // yet they all land on the first row, as `j`/`k` do from that state.
        "navigate_row" => {
            let down = matches!(key.code, KeyCode::Char('j') | KeyCode::Down);
            match (ns, down) {
                (KeyNamespace::AgentTreeAgents, true) => state.agents.down(),
                (KeyNamespace::AgentTreeAgents, false) => state.agents.up(),
                (_, true) => {
                    state.tree_state.key_down();
                }
                (_, false) => {
                    state.tree_state.key_up();
                }
            }
            KeyAction::Continue
        }
        "navigate_row_first" => {
            if ns == KeyNamespace::AgentTreeAgents {
                state.agents.top();
            } else {
                state.tree_state.select_first();
            }
            KeyAction::Continue
        }
        "navigate_row_last" => {
            if ns == KeyNamespace::AgentTreeAgents {
                state.agents.bottom();
            } else {
                state.tree_state.select_last();
            }
            KeyAction::Continue
        }
        "navigate_half_page" => {
            let down = matches!(key.code, KeyCode::Char('d' | 'D'));
            match (ns, down) {
                (KeyNamespace::AgentTreeAgents, true) => state.agents.half_page_down(),
                (KeyNamespace::AgentTreeAgents, false) => state.agents.half_page_up(),
                (_, true) => {
                    state.tree_state.select_relative(|current| {
                        current.map_or(0, |c| c.saturating_add(half_page))
                    });
                }
                (_, false) => {
                    state.tree_state.select_relative(|current| {
                        current.map_or(0, |c| c.saturating_sub(half_page))
                    });
                }
            }
            KeyAction::Continue
        }
        "collapse_directory" => {
            state.tree_state.key_left();
            KeyAction::Continue
        }
        "expand_directory" => {
            state.tree_state.key_right();
            KeyAction::Continue
        }
        // No badge guard on the diff, and deliberately none. The editor this
        // replaced refused a node badged Deleted, because an editor given a
        // missing path opens a misleading empty buffer. A diff has the
        // opposite property: a deleted file's diff is exactly its former
        // contents, so deleted is the case where opening it is most useful.
        // See OpenAgentTreeFileDiff in docs/specs/agent-tree.allium.
        "toggle_diff" => {
            let selected = state.tree_state.selected();
            state.toggle_diff(selected.iter().collect());
            KeyAction::DiffSetChanged
        }
        "toggle_directory" => {
            state.tree_state.toggle_selected();
            KeyAction::Continue
        }
        "jump_to_agent" => match state.agents.jump_target() {
            Some(window) => KeyAction::JumpTo(window),
            None => {
                took_effect = false;
                KeyAction::Continue
            }
        },
        _ => {
            took_effect = false;
            KeyAction::Continue
        }
    };
    if took_effect {
        state.usage.push(crate::cli::pane_key_event(action, &label));
    }
    result
}

/// Everything the loop needs to keep the diff pane in step with the open set:
/// the worktree both panes read, and which database and task the pane below
/// should open.
///
/// Bundled because they travel together and none means anything without the
/// others — `root` alone cannot name the pane's task, and the task id alone
/// cannot say which database holds it.
pub(crate) struct DiffPaneContext<'a> {
    pub root: &'a Path,
    pub db_path: &'a Path,
    pub task_id: i64,
}

/// Publish the open set and make the panes agree with it.
///
/// Both halves report through the pane's own notice, because both are the
/// direct answer to a keypress the user just made. The renderer keeps running
/// either way: a diff pane that will not open must not take the tree with it
/// (docs/specs/agent-tree.allium: `AgentTreeDiffPaneFailureIsVisible`).
///
/// The open set is left ALONE by a failure here. The user asked for a file to
/// be open and it is open; what failed is showing it, and the next toggle
/// retries rather than making them re-open everything.
///
/// Also the teardown path, called with an emptied set — see
/// [`tear_down_diff_pane`]. Publishing "nothing is open" is exactly what
/// retiring the pane means, so the two are one function rather than two that
/// must stay in step.
fn publish_open_set(
    context: &DiffPaneContext<'_>,
    tree: &TreeNode,
    state: &mut RenderState,
    runner: &dyn ProcessRunner,
) {
    let root = context.root.to_string_lossy().into_owned();
    if let Err(e) =
        crate::agent_tree_open_set::write_open_set(&root, &state.open_diffs_in_tree_order(tree))
    {
        tracing::warn!(root, error = %format!("{e:#}"), "failed to record the open set");
        state.notice = Some(Notice::diff(format!("{e:#}")));
        return;
    }

    let my_pane = match crate::agent_tree_diff_pane::current_pane_from_env() {
        Ok(pane) => pane,
        Err(e) => {
            state.notice = Some(Notice::diff(e.to_string()));
            return;
        }
    };

    if let Err(e) = crate::agent_tree_diff_pane::reconcile_diff_pane(
        &my_pane,
        context.db_path,
        context.task_id,
        context.root,
        !state.open_diffs.is_empty(),
        runner,
    ) {
        tracing::warn!(error = %format!("{e:#}"), "failed to reconcile the diff pane");
        state.notice = Some(Notice::diff(format!("{e:#}")));
    }
}

/// Leave nothing behind on the way out: empty the open set, which retires the
/// diff pane with it.
///
/// A diff pane outliving its tree is orphaned — nothing drives its open set,
/// nothing refreshes it, and the toggle that would bring the tree back does not
/// act on it. See `KillAgentTreeDiffPaneWithItsTree` in
/// docs/specs/agent-tree.allium.
///
/// Whatever notice this leaves behind goes unread, which is correct: the
/// renderer is already leaving and there is nowhere left to show one.
fn tear_down_diff_pane(
    context: &DiffPaneContext<'_>,
    tree: &TreeNode,
    state: &mut RenderState,
    runner: &dyn ProcessRunner,
) {
    state.open_diffs.clear();
    publish_open_set(context, tree, state, runner);
}

/// Take a freshly built tree as the one on screen, re-syncing expansion only
/// when it actually differs.
///
/// Compared as TREES, not as change lists: the tree is what the user sees, so
/// it is the thing whose sameness matters — and two change lists that differ
/// only in a duplicate entry render identically, which a list comparison would
/// mistake for news.
///
/// Shared with the test rig, so a test's idea of "a refresh happened" is the
/// loop's idea of it, short-circuit included.
fn adopt_tree(rebuilt: TreeNode, tree: &mut TreeNode, state: &mut RenderState) {
    if rebuilt == *tree {
        return;
    }
    *tree = rebuilt;
    state.sync_expansion(tree);
}

/// Reads of the board's task list for the agents section, one per
/// `AGENTS_REFRESH_INTERVAL`, produced off the render loop so a slow database
/// never delays a keypress or a git tick.
pub(crate) type AgentReads = std::sync::mpsc::Receiver<Result<Vec<AgentRow>, String>>;

/// The agents section's re-read cadence — the spec's
/// `config.agent_tree_agents_refresh_interval`.
pub(crate) const AGENTS_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// Take every read that has arrived since the last pass. Only the newest one
/// matters for the rows, but each is adopted in turn so a failure followed by
/// a recovery leaves no stale notice behind.
fn drain_agent_reads(agent_reads: &AgentReads, state: &mut RenderState) {
    while let Ok(read) = agent_reads.try_recv() {
        state.adopt_agent_list(read);
    }
}

fn run_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    base_branch: &str,
    context: &DiffPaneContext<'_>,
    agent_reads: &AgentReads,
    runner: &dyn ProcessRunner,
    usage: &mut crate::cli::PaneUsage,
) -> Result<()> {
    let root = context.root;
    let mut state = RenderState::new();
    let mut tree = build_tree(root, &[]);
    // `build_tree` names the root node after the worktree directory, which is
    // exactly the pane title — so there is one basename computation, not two.
    let title = tree.name.clone();

    // Draw the empty tree BEFORE the first query. git runs inline in this
    // single-threaded loop, so a slow first query — a cold index, a large repo
    // — is time the pane has painted nothing and still shows whatever tmux left
    // in that cell. One frame of an empty bordered pane is a better answer than
    // a stale one; the query below fills it in immediately after.
    terminal.draw(|frame| render_pane(frame, frame.area(), &tree, &mut state, &title))?;
    refresh(root, base_branch, runner, &mut tree, &mut state);

    loop {
        drain_agent_reads(agent_reads, &mut state);
        terminal.draw(|frame| render_pane(frame, frame.area(), &tree, &mut state, &title))?;

        if event::poll(REFRESH_INTERVAL)? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let action = handle_key(&mut state, &tree, key);
            usage.record(std::mem::take(&mut state.usage));
            match action {
                KeyAction::Exit => {
                    tear_down_diff_pane(context, &tree, &mut state, runner);
                    usage.flush();
                    return Ok(());
                }
                KeyAction::Continue => {}
                KeyAction::DiffSetChanged => {
                    publish_open_set(context, &tree, &mut state, runner);
                }
                KeyAction::JumpTo(window) => jump_to_agent(&window, &mut state, runner),
            }
            continue;
        }

        // Poll timed out with no key event: the ~1s timer tick.
        refresh(root, base_branch, runner, &mut tree, &mut state);
    }
}

/// One refresh pass: ask git, and rebuild only if the answer moved.
///
/// A failed query leaves `changes` and `tree` untouched and sets a notice — the
/// spec's `AgentTreeGitFailureKeepsLastGoodTree`. The commonest failure is a
/// transient index lock taken by the agent's own git commands, and blanking the
/// tree on that would make the pane flicker empty exactly when the user most
/// wants to watch it.
///
/// The unchanged-result short-circuit is a performance optimisation with one
/// behavioural consequence worth stating: the user's manual expansion state
/// survives a tick precisely because nothing is rebuilt on it.
fn refresh(
    root: &Path,
    base_branch: &str,
    runner: &dyn ProcessRunner,
    tree: &mut TreeNode,
    state: &mut RenderState,
) {
    match git_changes(root, base_branch, runner) {
        Ok(fresh) => {
            state.clear_git_notice();
            // Compared as TREES, not as change lists. The tree is what the user
            // sees, so it is the thing whose sameness matters — and two change
            // lists that differ only in a duplicate entry render identically,
            // which a list comparison would mistake for news.
            adopt_tree(build_tree(root, &fresh), tree, state);
        }
        Err(e) => {
            tracing::warn!(
                root = %root.display(),
                base_branch,
                // `{e:#}` so the log carries the cause (timeout, spawn failure), not
                // just the outermost context.
                error = format_args!("{e:#}"),
                "agent-tree: git query failed, keeping the last good tree"
            );
            // `{:#}`, not `{}`: anyhow's plain Display prints only the outermost
            // context, so a git that could not be spawned at all — or that
            // overran GIT_TIMEOUT — would put the bare word "git" in the
            // border and nothing else. Those are the two failures the user can
            // least afford to have unexplained.
            state.notice = Some(Notice::git(format!("{e:#}")));
        }
    }
}

/// Entry point for `dispatch agent-tree <task_id>`. Standalone ratatui loop
/// — not part of the board TUI's `App`/message loop (see the module-level
/// doc comment). Resolves the task's worktree and base branch from the DB once,
/// then re-queries git on a 1-second timer.
pub async fn run(db_path: &Path, board_port: u16, task_id: i64) -> Result<()> {
    // The task and the live-agent list come from the running board, which
    // already holds them (`PanesReadThroughTheBoard`).
    let source = std::sync::Arc::new(crate::cli::BoardPaneSource { port: board_port });
    let (agent_reads, poller) = spawn_agent_list_poller(source.clone(), TaskId(task_id));
    let mut usage = crate::cli::PaneUsage::new(board_port, task_id);

    let result = crate::cli::with_pane_task(
        &*source,
        task_id,
        crate::keybindings::KeyNamespace::AgentTreeTree,
        |terminal, root, base_branch| {
            // Start from a clean slate. The open set is view state, like the cursor and
            // the manual expansions, and a set left behind by a killed renderer
            // describes nothing — see the AgentTreeCompanionPane surface's guidance.
            let _ = crate::agent_tree_open_set::clear_open_set(&root.to_string_lossy());
            run_loop(
                terminal,
                &base_branch,
                &DiffPaneContext {
                    root: &root,
                    db_path,
                    task_id,
                },
                &agent_reads,
                &RealProcessRunner::default(),
                &mut usage,
            )
        },
    );
    // An exit through an error path leaves sends outstanding.
    usage.flush();
    poller.abort();
    result
}

/// One read of the agents section, from the running board
/// (agent-tree.allium: RefreshAgentTreeAgentList, `board_tasks()` =
/// BoardPaneView's `live_agents`). `Err` carries the notice to show -- naming
/// the board -- and the loop keeps its last list
/// (AgentTreeAgentListFailureKeepsLastList).
pub(crate) async fn read_agent_rows(
    source: &dyn crate::cli::PaneViewSource,
    own: TaskId,
) -> Result<Vec<AgentRow>, String> {
    let view = source.pane_view(own.0).await.map_err(|e| {
        format!(
            "could not read the agent list from the board at {}: {e:#}",
            source.board_address()
        )
    })?;
    let mut rows: Vec<AgentRow> = view
        .live_agents
        .into_iter()
        .filter_map(|agent| {
            Some(AgentRow {
                id: TaskId(agent.id),
                title: agent.title,
                window: TmuxWindow::parse(&agent.tmux_window)?,
                is_own: agent.id == own.0,
            })
        })
        .collect();
    rows.sort_by_key(|row| row.id.0);
    Ok(rows)
}

/// Read the board's task list on its own timer and send each read, reduced to
/// the agents section's rows, to the render loop. Read-only: the pane never
/// writes to the board (`RefreshAgentTreeAgentList`).
///
/// A task rather than an inline read because the render loop is synchronous
/// and the board read is not; the loop runs on the runtime's calling thread, so
/// this runs on a worker beside it.
fn spawn_agent_list_poller(
    source: std::sync::Arc<dyn crate::cli::PaneViewSource>,
    own: TaskId,
) -> (AgentReads, tokio::task::JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = tokio::spawn(async move {
        // Only news is sent: in steady state the list does not change, and
        // the loop would otherwise re-adopt an identical list every second.
        // A failure is always sent, since it carries a notice to show.
        let mut last_sent: Option<Vec<AgentRow>> = None;
        // `interval`'s first tick resolves immediately, so the read below still
        // fires right away on the first pass — the tick before it only delays
        // every pass after that, same as the sleep-after-read it replaces.
        let mut ticker = tokio::time::interval(AGENTS_REFRESH_INTERVAL);
        loop {
            ticker.tick().await;
            let read = read_agent_rows(&*source, own).await;
            let news = match &read {
                Ok(rows) => last_sent.as_ref() != Some(rows),
                Err(_) => true,
            };
            if news {
                last_sent = read.as_ref().ok().cloned();
                if tx.send(read).is_err() {
                    return;
                }
            }
        }
    });
    (rx, handle)
}

#[cfg(test)]
mod tests;
