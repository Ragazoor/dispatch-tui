//! Drawing the agent-tree pane: the changed-file tree's rows, and the whole
//! pane — tree, then the [`commits`] and [`agents`] sections below it. Does no
//! I/O, so the snapshot tests draw it straight from a [`TreeNode`].

pub mod agents;
pub mod commits;

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;
use tui_tree_widget::{Tree, TreeItem};

use crate::agent_tree::model::{FileChange, TreeNode, TreeNodeKind};
use crate::agent_tree::render::agents::{border_style, render_agents};
use crate::agent_tree::render::commits::{render_commits, short_id};
use crate::agent_tree::state::{Focus, RenderState};
use crate::palette::{FG, GREEN, MUTED, RED, YELLOW};

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

/// Render the whole pane: the tree above, the commits section beneath it and
/// the agents section in a band across the bottom
/// (`CommitsSectionSitsBetweenTreeAndAgents`, `AgentsSectionSitsBelowTheTree`).
/// Each section takes one row per entry up to its cap; the tree keeps the rest.
///
/// The tree's title names its source (`TreeTitleNamesItsSource`): the pane
/// root alone for unstaged work, the selected commit's short id beside it for a
/// commit.
pub fn render_pane(
    frame: &mut Frame,
    area: Rect,
    root: &TreeNode,
    state: &mut RenderState,
    title: &str,
) {
    let agents_height = state.agents.height().min(area.height);
    let commits_height = state.commits.height().min(area.height - agents_height);
    let tree_area = Rect {
        height: area.height - agents_height - commits_height,
        ..area
    };
    let commits_area = Rect {
        y: area.y + tree_area.height,
        height: commits_height,
        ..area
    };
    let agents_area = Rect {
        y: commits_area.y + commits_height,
        height: agents_height,
        ..area
    };
    let tree_title = match &state.selected_commit {
        Some(id) => format!("{title} {}", short_id(id)),
        None => title.to_string(),
    };
    render(frame, tree_area, root, state, &tree_title);
    let alert = state.notice.is_some();
    render_commits(
        frame,
        commits_area,
        &mut state.commits,
        state.selected_commit.as_deref(),
        state.focus == Focus::Commits,
        alert,
    );
    render_agents(
        frame,
        agents_area,
        &mut state.agents,
        state.focus == Focus::Agents,
        alert,
    );
}
