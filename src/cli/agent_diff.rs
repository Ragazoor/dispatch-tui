//! `dispatch agent-diff <task_id>` — the pane beneath the agent-tree companion
//! pane, showing the contents of whatever the tree has open (see
//! `docs/specs/agent-tree.allium`'s `AgentTreeDiffPane` surface and
//! `RefreshAgentTreeDiff` rule).
//!
//! A separate process from the tree for the same reason the tree is a separate
//! process from the board: it is a separate tmux pane, and tmux moves the
//! cursor between panes itself. That is the whole reason the diff is a pane and
//! not a second region inside the tree's pane — a region would have needed a
//! focus model, a focus indicator, a key to move between the two, and a rule
//! for which region each existing motion key acted on. See the spec's
//! `DiffPaneNavigationIsTmuxNavigation`.
//!
//! **This process never writes the open set.** It reads it, and the tree owns
//! it — see [`crate::agent_tree_open_set`] and the spec's
//! `DiffPaneHasNoToggleOfItsOwn`. Everything else it shows comes from git,
//! exactly as the tree's badges do: the paths come from the tree, the contents
//! never do.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};

use crate::agent_tree::parse_untracked;
use crate::cli::agent_tree::{run_git, GIT_TIMEOUT, REFRESH_INTERVAL};
use crate::process::{ProcessRunner, RealProcessRunner};
use crate::tui::ui::palette::{FG, GREEN, RED, YELLOW};

/// How much diff text this pane will render for ONE file before refusing it.
///
/// Per file, not per pane: one enormous generated file must not cost the user
/// the diffs of the files either side of it, which is what a whole-pane budget
/// would do depending on sort order. Matches `config.agent_tree_diff_max_bytes`
/// in `docs/specs/agent-tree.allium`.
///
/// Bounded for the same reason the shared git timeout is: rendering runs inline in a
/// single-threaded loop that also has to answer keypresses, so an unbounded
/// diff is an unbounded freeze.
pub const DIFF_MAX_BYTES: usize = 1_048_576;

/// Why a file is shown as a placeholder rather than as contents.
///
/// Each value is a fact about the file, not a failure of this pane: it rendered
/// exactly what it could, and says which of these it hit. See the spec's
/// `DiffRefusal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRefusal {
    /// Git reports the change without line contents.
    Binary,
    /// The diff exceeds [`DIFF_MAX_BYTES`].
    TooLarge,
}

impl DiffRefusal {
    /// The one line shown in place of the file's contents.
    pub fn message(self) -> &'static str {
        match self {
            Self::Binary => "binary file",
            Self::TooLarge => "diff too large to display",
        }
    }
}

/// What the pane has to show for one open file: git's patch, or the reason
/// there isn't one.
///
/// A sum type rather than a pair of nullable fields, for exactly the reason
/// [`crate::agent_tree::LineCounts`] is one value rather than two: the pane
/// always has something to draw for an open file, and "neither" would leave the
/// user looking at a blank region wondering whether it was still loading. The
/// spec states that as `DiffBodyExcludesRefusal`; here it is unrepresentable
/// rather than merely forbidden, so there is no constructor to uphold it and no
/// unreachable arm to explain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDiffContent {
    Shown(String),
    Refused(DiffRefusal),
}

/// One open file, and what to draw for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub path: PathBuf,
    pub content: FileDiffContent,
}

impl FileDiff {
    fn new(path: &Path, content: FileDiffContent) -> Self {
        Self {
            path: path.to_path_buf(),
            content,
        }
    }
}

/// Whether git printed a binary-file notice rather than a patch.
///
/// Git says `Binary files a/x.png and b/x.png differ` in place of hunks. The
/// check is anchored to the start of a line so a patch that merely *contains*
/// that sentence — a diff of this very file, for instance — is not mistaken for
/// one.
fn is_binary_notice(diff: &str) -> bool {
    diff.lines().any(|line| line.starts_with("Binary files "))
}

/// One open file's diff for the selected source, or `None` when there is
/// nothing to show for it — the spec's `git_file_diff(root, commit, path)`.
///
/// `commit = None` is unstaged work: the working tree against the INDEX,
/// naming no revision — and, for a path in `untracked`, the whole file as
/// additions (RefreshAgentTreeDiff's "Untracked files"). `commit = Some(id)`
/// is that commit against its first parent (the empty tree for a root
/// commit). Rename detection is off in both.
///
/// `None` is not an error and not a placeholder: a path git no longer reports
/// for this source renders nothing and stays open
/// (`OpenDiffPathsMaySurviveTheirFiles`). Binary and over-size diffs — an
/// untracked file's contents included — come back as a [`DiffRefusal`].
pub fn file_diff(
    root: &Path,
    commit: Option<&str>,
    path: &Path,
    untracked: &BTreeSet<PathBuf>,
    runner: &dyn ProcessRunner,
) -> Result<Option<FileDiff>> {
    let root = root.to_string_lossy().into_owned();
    let path_arg = path.to_string_lossy().into_owned();

    let diff = match commit {
        // An untracked file is invisible to a diff against the index, so its
        // diff is taken against nothing: every line an addition. `--no-index`
        // exits 1 when the sides differ, which is the whole point here.
        None if untracked.contains(path) => {
            let output = runner
                .run_with_timeout(
                    "git",
                    &[
                        "-C",
                        &root,
                        "diff",
                        "--no-index",
                        "--no-renames",
                        "--",
                        "/dev/null",
                        &path_arg,
                    ],
                    GIT_TIMEOUT,
                )
                .context("could not run git")?;
            match output.status.code() {
                Some(0 | 1) => String::from_utf8_lossy(&output.stdout).into_owned(),
                _ => return Err(crate::cli::agent_tree::git_error(&output)),
            }
        }
        // `--` separates the revision from the pathspec, so a path that looks
        // like a ref ("main", "HEAD") is still read as a path.
        None => run_git(
            runner,
            &["-C", &root, "diff", "--no-renames", "--", &path_arg],
        )?,
        // `git show` diffs a commit against its first parent, and a root
        // commit against the empty tree; `--format=` suppresses the header.
        Some(commit) => run_git(
            runner,
            &[
                "-C",
                &root,
                "show",
                "--first-parent",
                "--format=",
                "--no-renames",
                commit,
                "--",
                &path_arg,
            ],
        )?,
    };

    if diff.trim().is_empty() {
        return Ok(None);
    }
    let content = if is_binary_notice(&diff) {
        FileDiffContent::Refused(DiffRefusal::Binary)
    } else if diff.len() > DIFF_MAX_BYTES {
        FileDiffContent::Refused(DiffRefusal::TooLarge)
    } else {
        FileDiffContent::Shown(diff)
    };
    Ok(Some(FileDiff::new(path, content)))
}

/// Which of `open` git considers untracked, as paths relative to `root`.
///
/// Taken once per rebuild and shared across every open path — see [`file_diff`]'s
/// `untracked` argument.
///
/// Bounded by `open` as a pathspec rather than listing the whole worktree: only
/// membership for the open paths is ever asked, and an unbounded listing walks
/// every untracked file there is — unbounded work in a repo with untracked build
/// output, to answer a question about a handful of paths.
///
/// The argv and the parsing are the tree's, not a second copy
/// ([`crate::agent_tree::parse_untracked`]). The tree's `[Added]` badge and this
/// pane's whole-file-additions diff of it are answers to the SAME question about
/// the same file, so a flag added to one query and not the other would leave the
/// two panes disagreeing about a path — the same drift `git_changes` guards
/// against by making its two diffs share a baseline.
pub fn untracked_paths(
    root: &Path,
    open: &[PathBuf],
    runner: &dyn ProcessRunner,
) -> Result<BTreeSet<PathBuf>> {
    let root = root.to_string_lossy().into_owned();
    let paths: Vec<String> = open
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let mut args: Vec<&str> = vec![
        "-C",
        &root,
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
        "--",
    ];
    args.extend(paths.iter().map(String::as_str));
    let listing = run_git(runner, &args)?;
    Ok(parse_untracked(&listing)
        .into_iter()
        .map(|change| change.path)
        .collect())
}

/// The whole document the pane shows: every open file's diff, in tree order,
/// each under its own path heading.
///
/// Tree order arrives WITH the paths and is not re-derived here. The tree
/// writes its rows' order into the open set (`agent_tree_open_set`), because it
/// is the only party that can know it: a folder's own files sort ahead of its
/// subfolders and a directory chain compresses depending on the whole change
/// set, neither of which this pane can see from the subset the user opened. So
/// the paths are rendered in the order received, and re-sorting them here would
/// be a second, weaker answer to a question already answered.
///
/// ONE document, not one region per file. Scrolling past the end of one file
/// reaches the top of the next without a keystroke in between, and there is no
/// per-file cursor to keep track of.
///
/// Takes the files by value and consumes them: each is rendered once, and
/// reusing the patch's own `String` rather than copying it out line by line is
/// what keeps a megabyte diff from being materialised twice.
fn document_lines(files: Vec<FileDiff>) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    for file in files {
        lines.push(DiffLine {
            kind: DiffLineKind::Heading,
            text: file.path.to_string_lossy().into_owned(),
        });
        match file.content {
            FileDiffContent::Shown(body) => {
                lines.extend(body.lines().map(|line| DiffLine {
                    kind: DiffLineKind::of(line),
                    text: line.to_owned(),
                }));
            }
            FileDiffContent::Refused(refusal) => lines.push(DiffLine {
                kind: DiffLineKind::Refusal,
                text: refusal.message().to_owned(),
            }),
        }
    }
    lines
}

/// How one rendered line should be coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// The path heading that opens each file's section.
    Heading,
    Added,
    Removed,
    /// A hunk header (`@@ ... @@`) or git's own file headers.
    Meta,
    Context,
    /// The one-line stand-in for a file whose contents are not shown.
    Refusal,
}

impl DiffLineKind {
    fn of(line: &str) -> Self {
        // Order matters: `+++`/`---` are file headers, not content, and both
        // start with a character that would otherwise read as content, so they
        // have to be recognised BEFORE the single-character arms below.
        const META_PREFIXES: [&str; 4] = ["+++", "---", "@@", "diff --git "];
        if META_PREFIXES.iter().any(|p| line.starts_with(p)) {
            Self::Meta
        } else if line.starts_with('+') {
            Self::Added
        } else if line.starts_with('-') {
            Self::Removed
        } else {
            Self::Context
        }
    }
}

/// One rendered line of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

/// Build the document for `open` against the selected source (`commit`: `None`
/// for unstaged work).
///
/// Order is the order of `open`, which is a `BTreeSet` of relative paths and so
/// is TREE order — the tree is the index the user reads this pane through, so
/// the two must scroll the same way.
///
/// A path git no longer reports contributes nothing and is not an error; a
/// git failure fails the whole build, which the caller turns into a notice
/// while keeping the last good document on screen.
pub fn build_document(
    root: &Path,
    commit: Option<&str>,
    open: &[PathBuf],
    untracked: &BTreeSet<PathBuf>,
    runner: &dyn ProcessRunner,
) -> Result<Vec<DiffLine>> {
    let mut files = Vec::new();
    for path in open {
        if let Some(diff) = file_diff(root, commit, path, untracked, runner)? {
            files.push(diff);
        }
    }
    Ok(document_lines(files))
}

/// The pane's view state: where the document is scrolled to, and the notice in
/// its border.
///
/// Deliberately small. Everything the pane SHOWS is re-derived from git every
/// tick; this is only where the user has scrolled to, which no git query can
/// answer. It is not modelled in the spec for the same reason the tree's cursor
/// is not — see the AgentTreeViewState open question, which covers both.
pub struct DiffState {
    /// Index of the first visible line.
    offset: usize,
    /// Rows the document had to draw into at the last render — the pane's
    /// height less its two borders. Recorded by [`render`], because the
    /// half-page motions are defined against the VISIBLE height and
    /// [`handle_key`] never sees a `Rect`.
    viewport_rows: usize,
    /// Whether a lone `g` is waiting for the second half of the `gg` chord.
    /// No deadline, exactly as in the tree pane: `g` is bound to nothing else
    /// here, so nothing is waiting for the chord to expire.
    pending_g: bool,
    /// Keypress usage events for presses that took effect, waiting for the
    /// loop to write them through the pane's store connection.
    pub usage: Vec<crate::models::UsageEvent>,
    /// A one-line failure notice from the last git query, rendered in the
    /// bottom border. While it is set the border is drawn in the error colour,
    /// for the same reason the tree's is: a document kept on screen after a
    /// failed query is indistinguishable from a correct one at a glance.
    pub notice: Option<String>,
}

impl Default for DiffState {
    fn default() -> Self {
        Self::new()
    }
}

impl DiffState {
    pub fn new() -> Self {
        Self {
            offset: 0,
            viewport_rows: 0,
            pending_g: false,
            usage: Vec::new(),
            notice: None,
        }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    /// How far `Ctrl-D`/`Ctrl-U` move — see [`crate::cli::half_page`].
    fn half_page(&self) -> usize {
        crate::cli::half_page(self.viewport_rows)
    }

    /// The furthest the document can scroll: far enough to put its last line on
    /// screen, and no further. Scrolling past the end into blank rows would let
    /// the user lose the document entirely and have to guess their way back.
    fn max_offset(&self, line_count: usize) -> usize {
        line_count.saturating_sub(self.viewport_rows)
    }

    fn scroll_to(&mut self, target: usize, line_count: usize) {
        self.offset = target.min(self.max_offset(line_count));
    }
}

/// What the event loop should do after [`handle_key`] has processed a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKeyAction {
    /// Stay in the loop and redraw.
    Continue,
    /// Leave the loop, which exits the process and so closes the tmux pane.
    ///
    /// The open set is deliberately NOT cleared on the way out: the user closed
    /// this pane, not their selection, and the next toggle in the tree brings
    /// it back with everything still open. See SplitAgentTreeDiffPane in
    /// docs/specs/agent-tree.allium.
    Exit,
}

/// Handle one key press. Pure: it moves the offset and nothing else.
///
/// The same vocabulary the tree pane uses for the same motions, so moving
/// between the two panes does not mean changing keyboards. What it does NOT
/// have is any way to open or close a file — Space, Enter and the all-files key
/// do nothing here, because the open set is decided in the tree and only in the
/// tree (`DiffPaneHasNoToggleOfItsOwn`).
pub fn handle_key(state: &mut DiffState, line_count: usize, key: KeyEvent) -> DiffKeyAction {
    // Any key acknowledges a notice, exactly as in the tree pane: the next
    // keypress is the earliest moment the user has demonstrably seen it.
    state.notice = None;

    // The `gg` chord: its first half is pending input, not a lookup, and only
    // the completed chord is looked up. Taking the flag disarms it
    // unconditionally: a second `g` completes the chord, and any other key
    // falls through to its own row having quietly cancelled it. Spelled as the
    // tree pane spells it — same semantics, same shape. See
    // AgentTreeGgChordNeverExpires in docs/specs/agent-tree.allium: there is no
    // deadline, so the only thing that can end a pending chord is the next key,
    // whenever it comes.
    use crate::keybindings::{key_label, key_name, lookup, KeyNamespace, KEY_BINDINGS};
    let was_pending_g = std::mem::take(&mut state.pending_g);
    let mut name = key_name(key);
    let mut label = key_label(key);
    if name == "g" {
        if !was_pending_g {
            state.pending_g = true;
            return DiffKeyAction::Continue;
        }
        name = "gg".to_string();
        label = "gg".to_string();
    }

    // The table decides which action this press runs; a press with no row
    // does nothing.
    let Some(row) = lookup(KEY_BINDINGS, KeyNamespace::AgentDiff, &name, |_| false) else {
        return DiffKeyAction::Continue;
    };
    let action = row.action;
    let half_page = state.half_page();
    let result = match action {
        "exit_pane" => DiffKeyAction::Exit,
        "navigate_row" => {
            if matches!(key.code, KeyCode::Char('j') | KeyCode::Down) {
                state.scroll_to(state.offset.saturating_add(1), line_count);
            } else {
                state.offset = state.offset.saturating_sub(1);
            }
            DiffKeyAction::Continue
        }
        "navigate_row_first" => {
            state.offset = 0;
            DiffKeyAction::Continue
        }
        "navigate_row_last" => {
            state.offset = state.max_offset(line_count);
            DiffKeyAction::Continue
        }
        "navigate_half_page" => {
            if matches!(key.code, KeyCode::Char('d' | 'D')) {
                state.scroll_to(state.offset.saturating_add(half_page), line_count);
            } else {
                state.offset = state.offset.saturating_sub(half_page);
            }
            DiffKeyAction::Continue
        }
        _ => return DiffKeyAction::Continue,
    };
    state.usage.push(crate::cli::pane_key_event(action, &label));
    result
}

/// Draw the document into `area`.
///
/// Lines wider than the pane are TRUNCATED, not wrapped. The pane is narrow by
/// design — it inherits the tree's column — so one long line would wrap into
/// many rows and push several files' diffs off screen, making the cost of a
/// single minified or generated line fall on everything the user opened
/// alongside it. Truncation costs only the line it happens to. See
/// `DiffLinesTruncateRatherThanWrap` in docs/specs/agent-tree.allium.
pub fn render(frame: &mut Frame, area: Rect, lines: &[DiffLine], state: &mut DiffState) {
    // Two borders, so the drawable height is the pane's less two. Recorded for
    // the half-page motions, which only the renderer knows the height for.
    state.viewport_rows = usize::from(area.height).saturating_sub(2);
    // A pane that shrank under the user can leave the offset past the end.
    state.offset = state.offset.min(state.max_offset(lines.len()));

    let border_style = if state.notice.is_some() {
        Style::default().fg(RED)
    } else {
        Style::default().fg(FG)
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title("diff");
    if let Some(notice) = &state.notice {
        block = block.title_bottom(notice.as_str());
    }

    // Borrowed, not cloned: `lines` outlives this call, so the spans can point
    // into it. Cloning here allocated a `String` per visible row on every
    // frame — a few dozen allocations a second, dropped unread.
    let visible: Vec<Line<'_>> = lines
        .iter()
        .skip(state.offset)
        .take(state.viewport_rows)
        .map(|line| {
            let style = match line.kind {
                DiffLineKind::Heading => Style::default().fg(FG).add_modifier(Modifier::BOLD),
                DiffLineKind::Added => Style::default().fg(GREEN),
                DiffLineKind::Removed => Style::default().fg(RED),
                DiffLineKind::Meta => Style::default().fg(YELLOW),
                DiffLineKind::Context => Style::default().fg(FG),
                DiffLineKind::Refusal => Style::default().fg(YELLOW).add_modifier(Modifier::ITALIC),
            };
            Line::from(Span::styled(line.text.as_str(), style))
        })
        .collect();

    // No `.wrap(..)`: absence is the truncation, and it is load-bearing — see
    // the doc comment above.
    frame.render_widget(Paragraph::new(visible).block(block), area);
}

/// A cheap fingerprint of what the open files currently look like to git for
/// the selected source, so a pass where nothing moved can skip the per-file
/// diffs (RefreshAgentTreeDiff's "Steady-state cost"). For unstaged work it
/// must also see an open untracked path's contents change and its leaving the
/// untracked listing, which the line counts alone cannot.
fn open_files_fingerprint(
    root: &Path,
    commit: Option<&str>,
    open: &[PathBuf],
    runner: &dyn ProcessRunner,
) -> Result<String> {
    let root_arg = root.to_string_lossy().into_owned();
    let paths: Vec<String> = open
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    let mut args: Vec<&str> = vec!["-C", &root_arg];
    match commit {
        // Unstaged work: the working tree against the index, naming no
        // revision — the same comparison the tree's counts answer.
        None => args.extend(["diff", "--numstat", "--no-renames", "-z", "--"]),
        Some(commit) => args.extend([
            "show",
            "--first-parent",
            "--format=",
            "--numstat",
            "--no-renames",
            "-z",
            commit,
            "--",
        ]),
    }
    args.extend(paths.iter().map(String::as_str));
    let mut fingerprint = run_git(runner, &args)?;

    if commit.is_none() {
        // The counts never see an untracked path. Which open paths are
        // untracked, and each one's size and modification time, stand in for
        // its contents: a growing new file moves them, and staging one takes
        // it out of the listing.
        let untracked = untracked_paths(root, open, runner)?;
        for path in &untracked {
            fingerprint.push_str(&format!("\0untracked {}", path.display()));
            match std::fs::metadata(root.join(path)) {
                Ok(meta) => {
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    fingerprint.push_str(&format!(" {} {mtime}", meta.len()));
                }
                Err(_) => fingerprint.push_str(" missing"),
            }
        }
    }
    Ok(fingerprint)
}

/// What the last successful refresh saw, so the next one can tell whether
/// anything moved.
#[derive(Default)]
struct LastSeen {
    /// The open paths as the tree last published them, ORDER INCLUDED — a
    /// reorder is a reason to rebuild, because the document is rendered in it.
    open: Vec<PathBuf>,
    /// The source the document was built for; a selection change is a reason
    /// to rebuild even when nothing else moved.
    source: Option<String>,
    fingerprint: String,
}

/// One refresh pass: re-read the open set, and rebuild the document only if it
/// or the files in it have moved.
///
/// A failed git query leaves the document untouched and sets a notice, the same
/// way the tree keeps its last good tree: the commonest failure is a transient
/// index lock taken by the agent's own git, and blanking the pane on that would
/// make it flicker empty exactly when the user most wants to read it.
fn refresh(
    root: &Path,
    runner: &dyn ProcessRunner,
    last: &mut LastSeen,
    lines: &mut Vec<DiffLine>,
    state: &mut DiffState,
) {
    let open = crate::agent_tree_open_set::read_open_set(&root.to_string_lossy());

    if open.is_empty() {
        state.notice = None;
        *last = LastSeen::default();
        lines.clear();
        return;
    }

    match rebuild(root, &open, runner, last) {
        Ok(fresh) => {
            state.notice = None;
            if let Some(fresh) = fresh {
                *lines = fresh;
            }
        }
        Err(e) => {
            tracing::warn!(
                root = %root.display(),
                // `{e:#}` so the log carries the cause (timeout, spawn failure), not
                // just the outermost context.
                error = format_args!("{e:#}"),
                "agent-diff: git query failed, keeping the last good document"
            );
            // `{:#}`, not `{}`: anyhow's plain Display prints only the outermost
            // context, so a git that could not be spawned — or that overran
            // GIT_TIMEOUT — would put the bare word "git" in the border.
            state.notice = Some(format!("{e:#}"));
        }
    }
}

/// Re-read the open files' diffs, or `None` when nothing has moved since the
/// last pass.
///
/// Split out of [`refresh`] so the "did anything change" decision and the
/// notice handling stay separable: everything here either answers with fresh
/// lines or fails, and `refresh` decides what a failure looks like to the user.
fn rebuild(
    root: &Path,
    open: &[PathBuf],
    runner: &dyn ProcessRunner,
    last: &mut LastSeen,
) -> Result<Option<Vec<DiffLine>>> {
    // The source is the tree's selection, read from where the open set is
    // (AgentTreeSourceIsOneSelection). Whether it moved since the last pass is
    // part of the short-circuit below.
    let source = crate::agent_tree_open_set::read_selected_source(&root.to_string_lossy());
    let fingerprint = open_files_fingerprint(root, source.as_deref(), open, runner)?;
    if open == last.open && source == last.source && fingerprint == last.fingerprint {
        return Ok(None);
    }
    // A commit has no untracked files.
    let untracked = if source.is_none() {
        untracked_paths(root, open, runner)?
    } else {
        BTreeSet::new()
    };
    let lines = build_document(root, source.as_deref(), open, &untracked, runner)?;
    last.open = open.to_vec();
    last.source = source;
    last.fingerprint = fingerprint;
    Ok(Some(lines))
}

fn run_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Path,
    runner: &dyn ProcessRunner,
    usage: &mut crate::cli::PaneUsage,
) -> Result<()> {
    let mut state = DiffState::new();
    let mut lines: Vec<DiffLine> = Vec::new();
    let mut last = LastSeen::default();

    // Draw the empty pane BEFORE the first query, for the same reason the tree
    // does: git runs inline in this single-threaded loop, and one frame of an
    // empty bordered pane is a better answer than whatever tmux left in the
    // cell.
    terminal.draw(|frame| render(frame, frame.area(), &lines, &mut state))?;
    refresh(root, runner, &mut last, &mut lines, &mut state);

    loop {
        terminal.draw(|frame| render(frame, frame.area(), &lines, &mut state))?;

        if event::poll(REFRESH_INTERVAL)? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let action = handle_key(&mut state, lines.len(), key);
            usage.record(std::mem::take(&mut state.usage));
            match action {
                DiffKeyAction::Exit => {
                    usage.flush();
                    return Ok(());
                }
                DiffKeyAction::Continue => {}
            }
            continue;
        }

        refresh(root, runner, &mut last, &mut lines, &mut state);
    }
}

/// `dispatch agent-diff <task_id>`: render the diffs of whatever the task's
/// agent-tree pane currently has open.
///
/// Takes the task id rather than a worktree path so the two panes cannot
/// disagree about which worktree they are looking at, and so this pane reads
/// the same selected source the tree publishes.
pub async fn run(board_port: u16, task_id: i64) -> Result<()> {
    let source = crate::cli::BoardPaneSource { port: board_port };
    let mut usage = crate::cli::PaneUsage::new(board_port, task_id);
    let result = crate::cli::with_pane_task(
        &source,
        task_id,
        crate::keybindings::KeyNamespace::AgentDiff,
        // The diff pane needs no base branch: its source is the tree's
        // selection (AgentTreeSourceIsOneSelection).
        |terminal, root, _base_branch| {
            run_loop(terminal, &root, &RealProcessRunner::default(), &mut usage)
        },
    );
    // An exit through an error path leaves sends outstanding.
    usage.flush();
    result
}

/// The open paths as the TREE publishes them: an ordered list, in row order.
/// The document follows it verbatim, so a test that cares about order writes
/// the order it means here.
#[cfg(test)]
fn open_set(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

/// The untracked paths, which are only ever asked "does this contain the
/// path" — hence a set, where the open list is a list.
#[cfg(test)]
fn untracked_set(paths: &[&str]) -> BTreeSet<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

#[cfg(test)]
mod tests;

/// `file_diff` against a real repository: what the unstaged and one-commit
/// comparisons actually answer, which a mock can only assume.
#[cfg(test)]
mod real_git_tests;

#[cfg(test)]
mod document_tests;

#[cfg(test)]
mod refresh_tests;

#[cfg(test)]
mod view_tests;
