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
mod tests {
    use super::*;
    use crate::cli::agent_tree::GIT_TIMEOUT;
    use crate::process::MockProcessRunner;

    /// A commit the user has selected in the tree's commits section.
    const COMMIT: &str = "3333333333333333333333333333333333333333";

    fn no_untracked() -> BTreeSet<PathBuf> {
        BTreeSet::new()
    }

    fn diff_rig(stdout: &str) -> MockProcessRunner {
        MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(stdout.as_bytes())])
    }

    const A_PATCH: &str = "diff --git a/a.rs b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";

    #[test]
    fn a_tracked_files_diff_comes_back_as_its_body() {
        let runner = diff_rig(A_PATCH);

        let diff = file_diff(
            Path::new("/wt"),
            None,
            Path::new("a.rs"),
            &no_untracked(),
            &runner,
        )
        .unwrap()
        .unwrap();

        assert_eq!(diff.content, FileDiffContent::Shown(A_PATCH.to_owned()));
    }

    /// Unstaged work is the working tree against the INDEX, so the diff names
    /// no revision — the same comparison the tree's badge answers
    /// (RefreshAgentTreeDiff's "The same baseline, asked the same way"). `--`
    /// still separates the pathspec, so a path that looks like a ref is read
    /// as a path.
    #[test]
    fn unstaged_work_is_diffed_against_the_index_naming_no_revision() {
        let runner = diff_rig(A_PATCH);

        file_diff(
            Path::new("/wt"),
            None,
            Path::new("main"),
            &no_untracked(),
            &runner,
        )
        .unwrap();

        assert_eq!(
            runner.flattened_calls(),
            vec!["git -C /wt diff --no-renames -- main".to_string()]
        );
    }

    /// With a commit selected, the one diff asked for names that commit,
    /// keeps rename detection off, and still bounds the path with `--`.
    #[test]
    fn a_selected_commits_diff_names_the_commit_and_the_path() {
        let runner = diff_rig(A_PATCH);

        let diff = file_diff(
            Path::new("/wt"),
            Some(COMMIT),
            Path::new("main"),
            &no_untracked(),
            &runner,
        )
        .unwrap()
        .unwrap();

        assert_eq!(diff.content, FileDiffContent::Shown(A_PATCH.to_owned()));
        let calls = runner.flattened_calls();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert!(calls[0].contains(COMMIT), "{calls:?}");
        assert!(calls[0].contains("--no-renames"), "{calls:?}");
        assert!(calls[0].ends_with("-- main"), "{calls:?}");
    }

    #[test]
    fn a_binary_file_is_refused_with_its_own_reason() {
        let runner = diff_rig(
            "diff --git a/logo.png b/logo.png\nBinary files a/logo.png and b/logo.png differ\n",
        );

        let diff = file_diff(
            Path::new("/wt"),
            None,
            Path::new("logo.png"),
            &no_untracked(),
            &runner,
        )
        .unwrap()
        .unwrap();

        assert_eq!(diff.content, FileDiffContent::Refused(DiffRefusal::Binary));
    }

    /// The notice is matched at the START of a line, so a patch that merely
    /// contains the sentence — a diff of this very file, say — is still shown
    /// as a patch.
    #[test]
    fn a_patch_mentioning_the_binary_notice_is_not_mistaken_for_one() {
        let body = "diff --git a/x.rs b/x.rs\n@@ -1 +1 @@\n+// Binary files a and b differ\n";
        let runner = diff_rig(body);

        let diff = file_diff(
            Path::new("/wt"),
            None,
            Path::new("x.rs"),
            &no_untracked(),
            &runner,
        )
        .unwrap()
        .unwrap();

        assert_eq!(diff.content, FileDiffContent::Shown(body.to_owned()));
    }

    #[test]
    fn a_diff_over_the_cap_is_refused_rather_than_rendered() {
        let huge = format!(
            "diff --git a/big.rs b/big.rs\n{}",
            "+x\n".repeat(DIFF_MAX_BYTES)
        );
        let runner = diff_rig(&huge);

        let diff = file_diff(
            Path::new("/wt"),
            None,
            Path::new("big.rs"),
            &no_untracked(),
            &runner,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            diff.content,
            FileDiffContent::Refused(DiffRefusal::TooLarge)
        );
    }

    /// A path the user opened and the agent then reverted, staged or
    /// committed. Not an error, not a placeholder, and NOT a reason to close
    /// it — see OpenDiffPathsMaySurviveTheirFiles in docs/specs/agent-tree.allium.
    #[test]
    fn a_path_git_no_longer_reports_shows_nothing_at_all() {
        let runner = diff_rig("");

        let diff = file_diff(
            Path::new("/wt"),
            None,
            Path::new("reverted.rs"),
            &no_untracked(),
            &runner,
        )
        .unwrap();

        assert_eq!(diff, None);
    }

    #[test]
    fn a_git_failure_is_an_error_not_a_silently_empty_diff() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::fail("fatal: bad object")]);

        let err = file_diff(
            Path::new("/wt"),
            None,
            Path::new("a.rs"),
            &no_untracked(),
            &runner,
        )
        .expect_err("a failing git must not read as an empty diff");

        assert!(format!("{err:#}").contains("bad object"), "got {err:#}");
    }

    #[test]
    fn every_diff_query_is_bounded_by_the_shared_timeout() {
        let runner = diff_rig(A_PATCH);
        file_diff(
            Path::new("/wt"),
            None,
            Path::new("a.rs"),
            &no_untracked(),
            &runner,
        )
        .unwrap();
        assert_eq!(runner.recorded_timeouts(), vec![Some(GIT_TIMEOUT)]);
    }

    // -- untracked_paths ---------------------------------------------------

    /// One listing for the whole rebuild, and BOUNDED by the open paths: only
    /// membership for those is ever asked, and an unbounded listing walks every
    /// untracked file in the worktree to answer it.
    #[test]
    fn the_untracked_listing_is_taken_once_and_bounded_by_the_open_paths() {
        let runner = diff_rig("new.rs\0docs/my notes.md\0");
        let open = open_set(&["new.rs", "docs/my notes.md", "tracked.rs"]);

        let paths = untracked_paths(Path::new("/wt"), &open, &runner).unwrap();

        assert_eq!(paths, untracked_set(&["new.rs", "docs/my notes.md"]));
        assert_eq!(
            runner.flattened_calls(),
            vec![concat!(
                "git -C /wt ls-files --others --exclude-standard -z -- ",
                // In the order published, not sorted — the listing is bounded
                // by the open paths and does not care which order they come in.
                "new.rs docs/my notes.md tracked.rs"
            )
            .to_string()]
        );
    }

    /// The pathspec is the open set, so a path git does not report back is
    /// tracked.
    #[test]
    fn a_tracked_open_path_is_absent_from_the_untracked_answer() {
        let runner = diff_rig("");
        let paths = untracked_paths(Path::new("/wt"), &open_set(&["tracked.rs"]), &runner).unwrap();
        assert!(paths.is_empty());
    }
}

/// `file_diff` against a real repository: what the unstaged and one-commit
/// comparisons actually answer, which a mock can only assume.
#[cfg(test)]
mod real_git_tests {
    use super::*;
    use crate::cli::agent_tree::test_repo::TestRepo;
    use crate::process::RealProcessRunner;

    fn diff_of(
        repo: &TestRepo,
        commit: Option<&str>,
        path: &str,
        untracked: &[&str],
    ) -> Option<FileDiff> {
        file_diff(
            repo.root(),
            commit,
            Path::new(path),
            &untracked_set(untracked),
            &RealProcessRunner::default(),
        )
        .expect("file_diff")
    }

    fn body(diff: Option<FileDiff>) -> String {
        match diff.map(|d| d.content) {
            Some(FileDiffContent::Shown(body)) => body,
            other => panic!("expected a shown diff, got {other:?}"),
        }
    }

    /// RefreshAgentTreeDiff's "Untracked files": an open untracked path is a
    /// diff against nothing — every line an addition. Not a refusal, and no
    /// advice to stage it: in this view staging would remove it.
    #[test]
    fn an_untracked_file_is_shown_whole_as_additions() {
        let repo = TestRepo::new();
        repo.write("new.rs", "one\ntwo\n");

        let body = body(diff_of(&repo, None, "new.rs", &["new.rs"]));

        let added: Vec<&str> = body
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .collect();
        assert_eq!(added, vec!["+one", "+two"], "{body}");
        assert!(!body
            .lines()
            .any(|l| l.starts_with('-') && !l.starts_with("---")));
    }

    /// Both refusals apply to an untracked file's contents as to any diff.
    #[test]
    fn an_untracked_binary_file_is_refused_as_binary() {
        let repo = TestRepo::new();
        repo.write_bytes("logo.png", b"\x89PNG\0\x01\x02\x03\0binary");

        let diff = diff_of(&repo, None, "logo.png", &["logo.png"]).expect("a refusal");

        assert_eq!(diff.content, FileDiffContent::Refused(DiffRefusal::Binary));
    }

    #[test]
    fn an_untracked_file_too_large_to_show_is_refused() {
        let repo = TestRepo::new();
        repo.write("huge.txt", &"x\n".repeat(DIFF_MAX_BYTES));

        let diff = diff_of(&repo, None, "huge.txt", &["huge.txt"]).expect("a refusal");

        assert_eq!(
            diff.content,
            FileDiffContent::Refused(DiffRefusal::TooLarge)
        );
    }

    /// Unstaged work shows the unstaged hunk only — never staged lines under
    /// a row that exists because of an unstaged one.
    #[test]
    fn only_the_unstaged_hunk_of_a_file_is_shown() {
        let repo = TestRepo::new();
        repo.write("seed.txt", "seed\nstaged line\n");
        repo.git(&["add", "seed.txt"]);
        repo.append("seed.txt", "unstaged line\n");

        let body = body(diff_of(&repo, None, "seed.txt", &[]));

        assert!(body.contains("+unstaged line"), "{body}");
        assert!(!body.contains("+staged line"), "{body}");
    }

    /// A file whose whole change is staged has nothing to show for unstaged
    /// work — the same answer as a reverted one.
    #[test]
    fn a_fully_staged_file_has_nothing_to_show() {
        let repo = TestRepo::new();
        repo.write("seed.txt", "seed\nstaged line\n");
        repo.git(&["add", "seed.txt"]);

        assert_eq!(diff_of(&repo, None, "seed.txt", &[]), None);
    }

    /// A selected commit's diff is that commit against its parent, whatever
    /// the working tree holds meanwhile.
    #[test]
    fn a_selected_commits_diff_is_that_commit_against_its_parent() {
        let repo = TestRepo::new();
        repo.write("a.rs", "a1\n");
        repo.commit_all("first");
        repo.write("a.rs", "a1\na2\n");
        let second = repo.commit_all("second");
        repo.write("a.rs", "work in progress\n");

        let body = body(diff_of(&repo, Some(&second), "a.rs", &[]));

        assert!(body.contains("+a2"), "{body}");
        assert!(!body.contains("work in progress"), "{body}");
    }

    /// A root commit is diffed against the empty tree.
    #[test]
    fn a_root_commits_file_is_diffed_against_nothing() {
        let repo = TestRepo::new();
        let root_commit = repo.git(&["rev-list", "--max-parents=0", "HEAD"]);

        let body = body(diff_of(&repo, Some(root_commit.trim()), "seed.txt", &[]));

        assert!(body.contains("+seed"), "{body}");
    }

    /// A path the selected commit did not touch contributes nothing
    /// (OpenDiffPathsMaySurviveTheirFiles: "the user selected a source that
    /// does not touch it").
    #[test]
    fn a_path_the_selected_commit_did_not_touch_shows_nothing() {
        let repo = TestRepo::new();
        repo.write("a.rs", "a\n");
        let commit = repo.commit_all("add a");

        assert_eq!(diff_of(&repo, Some(&commit), "seed.txt", &[]), None);
    }
}

#[cfg(test)]
mod document_tests {
    use super::*;
    use crate::process::MockProcessRunner;

    fn patch(path: &str) -> String {
        format!("diff --git a/{path} b/{path}\n@@ -1 +1 @@\n-old\n+new\n")
    }

    fn binary(path: &str) -> String {
        format!("diff --git a/{path} b/{path}\nBinary files a/{path} and b/{path} differ\n")
    }

    fn rig(outputs: &[&str]) -> MockProcessRunner {
        MockProcessRunner::new(
            outputs
                .iter()
                .map(|o| MockProcessRunner::ok_with_stdout(o.as_bytes()))
                .collect(),
        )
    }

    fn texts(lines: &[DiffLine]) -> Vec<String> {
        lines.iter().map(|l| l.text.clone()).collect()
    }

    /// The path heading that opens each file's section — the document's own
    /// account of which files it is showing, and in what order.
    fn headings(lines: &[DiffLine]) -> Vec<String> {
        lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Heading)
            .map(|l| l.text.clone())
            .collect()
    }

    /// Tree order, not the order the user opened them in: the tree is the index
    /// this pane is read through, so the two must scroll the same way.
    #[test]
    fn each_file_renders_under_its_own_path_heading() {
        let runner = rig(&[&patch("a.rs"), &patch("src/lib.rs")]);

        let doc = build_document(
            Path::new("/wt"),
            None,
            &open_set(&["a.rs", "src/lib.rs"]),
            &BTreeSet::new(),
            &runner,
        )
        .unwrap();

        assert_eq!(headings(&doc), vec!["a.rs", "src/lib.rs"]);
        assert_eq!(texts(&doc)[0], "a.rs");
    }

    /// The document follows the order the TREE published, and does not sort.
    /// Tree order is not path order — a folder's own files sort ahead of its
    /// subfolders (`RowsPutAFoldersOwnFilesFirst`), so `z.rs` has a row above
    /// `src/lib.rs` while sorting after it lexicographically.
    ///
    /// The input is deliberately an order no sort of these paths produces.
    #[test]
    fn the_document_follows_the_published_order_rather_than_sorting() {
        let runner = rig(&[&patch("a.rs"), &patch("z.rs"), &patch("src/lib.rs")]);

        let doc = build_document(
            Path::new("/wt"),
            None,
            &open_set(&["a.rs", "z.rs", "src/lib.rs"]),
            &BTreeSet::new(),
            &runner,
        )
        .unwrap();

        assert_eq!(headings(&doc), vec!["a.rs", "z.rs", "src/lib.rs"]);
    }

    /// A refusal renders its reason in place of contents. There are exactly
    /// two refusals now — binary and too large — and no "not yet staged"
    /// placeholder (DiffRefusal).
    #[test]
    fn a_refused_file_renders_its_reason_in_place_of_contents() {
        let runner = rig(&[&binary("logo.png")]);

        let doc = build_document(
            Path::new("/wt"),
            None,
            &open_set(&["logo.png"]),
            &BTreeSet::new(),
            &runner,
        )
        .unwrap();

        let lines = texts(&doc);
        assert_eq!(lines, vec!["logo.png", DiffRefusal::Binary.message()]);
        assert_eq!(doc[1].kind, DiffLineKind::Refusal);
    }

    /// One refused file must not cost the user the diffs either side of it —
    /// which is why a refusal rides on the file rather than on the pane.
    #[test]
    fn a_refusal_does_not_stop_the_files_around_it_rendering() {
        let runner = rig(&[&patch("a.rs"), &binary("logo.png"), &patch("z.rs")]);

        let doc = build_document(
            Path::new("/wt"),
            None,
            &open_set(&["a.rs", "logo.png", "z.rs"]),
            &BTreeSet::new(),
            &runner,
        )
        .unwrap();

        assert_eq!(headings(&doc), vec!["a.rs", "logo.png", "z.rs"]);
        let lines = texts(&doc);
        assert!(lines.iter().any(|l| l == DiffRefusal::Binary.message()));
        assert_eq!(
            lines.iter().filter(|l| l.starts_with("@@")).count(),
            2,
            "both neighbours must still have their hunks; got {lines:?}"
        );
    }

    /// The agent reverted a file the user had open. It renders nothing and
    /// stays open — see OpenDiffPathsMaySurviveTheirFiles in the spec.
    #[test]
    fn a_path_with_nothing_to_show_contributes_no_section() {
        let runner = rig(&["", &patch("b.rs")]);

        let doc = build_document(
            Path::new("/wt"),
            None,
            &open_set(&["a.rs", "b.rs"]),
            &BTreeSet::new(),
            &runner,
        )
        .unwrap();

        assert_eq!(headings(&doc), vec!["b.rs"]);
    }

    #[test]
    fn an_empty_open_set_builds_an_empty_document() {
        let runner = rig(&[]);
        let doc = build_document(Path::new("/wt"), None, &[], &BTreeSet::new(), &runner).unwrap();
        assert!(doc.is_empty());
    }

    #[test]
    fn added_and_removed_lines_are_classified_apart_from_the_file_headers() {
        let runner = rig(&[&patch("a.rs")]);
        let doc = build_document(
            Path::new("/wt"),
            None,
            &open_set(&["a.rs"]),
            &BTreeSet::new(),
            &runner,
        )
        .unwrap();

        let kinds: Vec<_> = doc.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DiffLineKind::Heading,
                DiffLineKind::Meta,
                DiffLineKind::Meta,
                DiffLineKind::Removed,
                DiffLineKind::Added,
            ]
        );
    }

    /// `+++` and `---` open git's file headers. Reading them as content would
    /// paint two lines of every single diff the wrong colour.
    #[test]
    fn the_triple_dash_file_headers_are_not_read_as_content() {
        assert_eq!(DiffLineKind::of("--- a/x.rs"), DiffLineKind::Meta);
        assert_eq!(DiffLineKind::of("+++ b/x.rs"), DiffLineKind::Meta);
        assert_eq!(DiffLineKind::of("-old"), DiffLineKind::Removed);
        assert_eq!(DiffLineKind::of("+new"), DiffLineKind::Added);
        assert_eq!(DiffLineKind::of(" same"), DiffLineKind::Context);
    }

    #[test]
    fn a_git_failure_fails_the_whole_build() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::fail("fatal: bad object")]);
        assert!(build_document(
            Path::new("/wt"),
            None,
            &open_set(&["a.rs"]),
            &BTreeSet::new(),
            &runner,
        )
        .is_err());
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use crate::agent_tree_open_set::{read_open_set, write_open_set, write_selected_source};
    use crate::cli::agent_tree::test_repo::TestRepo;
    use crate::process::{MockProcessRunner, RealProcessRunner};
    use crate::worktree_admin::tests::make_linked_worktree;

    /// A diff pane over a real worktree, refreshed with real git.
    struct Pane {
        repo: TestRepo,
        last: LastSeen,
        lines: Vec<DiffLine>,
        state: DiffState,
    }

    impl Pane {
        fn new() -> Self {
            Self {
                repo: TestRepo::new(),
                last: LastSeen::default(),
                lines: Vec::new(),
                state: DiffState::new(),
            }
        }

        /// Publish the open set, as the tree does.
        fn open(&self, open: &[&str]) {
            let paths: Vec<PathBuf> = open.iter().map(PathBuf::from).collect();
            write_open_set(&self.repo.root_str(), &paths).unwrap();
        }

        /// Publish the selected source, as the tree does.
        fn select(&self, commit: Option<&str>) {
            write_selected_source(&self.repo.root_str(), commit).unwrap();
        }

        fn refresh_with(&mut self, runner: &dyn ProcessRunner) {
            refresh(
                self.repo.root(),
                runner,
                &mut self.last,
                &mut self.lines,
                &mut self.state,
            );
        }

        fn refresh(&mut self) {
            self.refresh_with(&RealProcessRunner::default());
        }

        fn text(&self) -> String {
            self.lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    /// Real git, with every argv recorded — for the steady-state cost.
    #[derive(Default)]
    struct Recording {
        inner: RealProcessRunner,
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl Recording {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.calls.lock().unwrap())
        }
    }

    impl ProcessRunner for Recording {
        fn run(&self, program: &str, args: &[&str]) -> Result<std::process::Output> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            self.inner.run(program, args)
        }

        fn run_with_timeout(
            &self,
            program: &str,
            args: &[&str],
            timeout: std::time::Duration,
        ) -> Result<std::process::Output> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            self.inner.run_with_timeout(program, args, timeout)
        }
    }

    #[test]
    fn an_empty_open_set_blanks_the_pane_without_asking_git() {
        let dir = tempfile::tempdir().unwrap();
        let (root, _) = make_linked_worktree(dir.path(), "task");
        let mut last = LastSeen::default();
        let mut lines = vec![DiffLine {
            kind: DiffLineKind::Context,
            text: "stale".into(),
        }];
        let mut state = DiffState::new();
        state.notice = Some("old".into());
        let runner = MockProcessRunner::new(vec![]);

        refresh(Path::new(&root), &runner, &mut last, &mut lines, &mut state);

        assert!(lines.is_empty());
        assert_eq!(state.notice, None);
        assert!(runner.recorded_calls().is_empty());
    }

    /// By default the pane shows unstaged work.
    #[test]
    fn a_first_refresh_builds_the_document_of_unstaged_work() {
        let mut pane = Pane::new();
        pane.repo.append("seed.txt", "more\n");
        pane.open(&["seed.txt"]);

        pane.refresh();

        assert!(pane
            .lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Heading && l.text == "seed.txt"));
        assert!(pane
            .lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Added && l.text == "+more"));
        assert_eq!(pane.state.notice, None);
    }

    /// An open untracked file shows its whole contents as additions.
    #[test]
    fn an_open_untracked_file_shows_its_whole_contents() {
        let mut pane = Pane::new();
        pane.repo.write("new.rs", "one\n");
        pane.open(&["new.rs"]);

        pane.refresh();

        assert!(
            pane.lines.iter().any(|l| l.text == "+one"),
            "{}",
            pane.text()
        );
    }

    /// The counts cannot see an untracked file change, so its contents are
    /// part of the fingerprint: a growing new file must not freeze at its
    /// first read.
    #[test]
    fn a_growing_untracked_file_is_re_read() {
        let mut pane = Pane::new();
        pane.repo.write("new.rs", "one\n");
        pane.open(&["new.rs"]);
        pane.refresh();

        pane.repo.append("new.rs", "two\n");
        pane.refresh();

        assert!(
            pane.lines.iter().any(|l| l.text == "+two"),
            "{}",
            pane.text()
        );
    }

    /// Staging a new file and editing nothing further moves it from untracked
    /// to absent, with an empty count answer both before and after. It must
    /// leave the pane rather than keep its whole-file diff on screen.
    #[test]
    fn staging_an_open_untracked_file_takes_it_out_of_the_pane() {
        let mut pane = Pane::new();
        pane.repo.write("new.rs", "one\n");
        pane.open(&["new.rs"]);
        pane.refresh();
        assert!(!pane.lines.is_empty());

        pane.repo.git(&["add", "new.rs"]);
        pane.refresh();

        assert!(pane.lines.is_empty(), "{}", pane.text());
    }

    /// AgentTreeSourceIsOneSelection: the pane reads the selection from where
    /// it reads the open set and follows it — to a commit and back again —
    /// even though nothing in the worktree moved in between.
    #[test]
    fn the_pane_follows_the_selected_source() {
        let mut pane = Pane::new();
        pane.repo.write("a.rs", "a1\n");
        pane.repo.commit_all("first");
        pane.repo.write("a.rs", "a1\na2\n");
        let second = pane.repo.commit_all("second");
        pane.repo.write("a.rs", "a1\na2\nwork in progress\n");
        pane.open(&["a.rs"]);

        pane.refresh();
        assert!(pane.text().contains("+work in progress"), "{}", pane.text());
        assert!(!pane.text().contains("+a2"), "{}", pane.text());

        pane.select(Some(&second));
        pane.refresh();
        assert!(pane.text().contains("+a2"), "{}", pane.text());
        assert!(!pane.text().contains("work in progress"), "{}", pane.text());

        pane.select(None);
        pane.refresh();
        assert!(pane.text().contains("+work in progress"), "{}", pane.text());
    }

    /// A path the agent committed since it was opened shows nothing — and
    /// stays open (OpenDiffPathsMaySurviveTheirFiles).
    #[test]
    fn a_path_committed_since_it_was_opened_shows_nothing_and_stays_open() {
        let mut pane = Pane::new();
        pane.repo.append("seed.txt", "more\n");
        pane.open(&["seed.txt"]);
        pane.refresh();
        assert!(!pane.lines.is_empty());

        pane.repo.commit_all("commit it");
        pane.refresh();

        assert!(pane.lines.is_empty(), "{}", pane.text());
        assert_eq!(
            read_open_set(&pane.repo.root_str()),
            vec![PathBuf::from("seed.txt")]
        );
    }

    /// The steady-state cost: a pass where nothing moved re-reads no contents,
    /// so it runs fewer git commands than the pass that built the document,
    /// and leaves the document as it was.
    #[test]
    fn a_refresh_where_nothing_moved_skips_the_diff() {
        let mut pane = Pane::new();
        pane.repo.append("seed.txt", "more\n");
        pane.open(&["seed.txt"]);
        let runner = Recording::default();

        pane.refresh_with(&runner);
        let building = runner.take();
        let built = pane.lines.clone();
        pane.refresh_with(&runner);
        let steady = runner.take();

        assert_eq!(pane.lines, built);
        assert!(
            steady.len() < building.len(),
            "steady {steady:?} vs building {building:?}"
        );
    }

    /// A failed git query leaves the document untouched and says so, the same
    /// way the tree keeps its last good tree.
    #[test]
    fn a_failed_git_query_keeps_the_last_document_and_sets_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let (root, _) = make_linked_worktree(dir.path(), "task");
        write_open_set(&root, &[PathBuf::from("a.rs")]).unwrap();
        let mut last = LastSeen::default();
        let kept = vec![DiffLine {
            kind: DiffLineKind::Context,
            text: "the last good document".into(),
        }];
        let mut lines = kept.clone();
        let mut state = DiffState::new();
        let runner = MockProcessRunner::new(vec![MockProcessRunner::fail(
            "fatal: unable to read index.lock",
        )]);

        refresh(Path::new(&root), &runner, &mut last, &mut lines, &mut state);

        assert_eq!(lines, kept);
        let notice = state.notice.expect("a notice");
        assert!(notice.contains("index.lock"), "{notice}");
    }

    #[test]
    fn the_notice_clears_on_the_next_good_pass() {
        let mut pane = Pane::new();
        pane.repo.append("seed.txt", "more\n");
        pane.open(&["seed.txt"]);
        pane.state.notice = Some("earlier failure".into());

        pane.refresh();

        assert_eq!(pane.state.notice, None);
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// A pane with `rows` of height, already drawn once — the half-page motions
    /// resolve against the LAST render's height, so a key test has to draw
    /// before pressing anything.
    struct Rig {
        lines: Vec<DiffLine>,
        state: DiffState,
        terminal: Terminal<TestBackend>,
    }

    impl Rig {
        fn new(line_count: usize, rows: u16) -> Self {
            let lines = (0..line_count)
                .map(|n| DiffLine {
                    kind: DiffLineKind::Context,
                    text: format!("line{n:02}"),
                })
                .collect();
            let mut rig = Self {
                lines,
                state: DiffState::new(),
                terminal: Terminal::new(TestBackend::new(40, rows)).unwrap(),
            };
            rig.draw();
            rig
        }

        fn draw(&mut self) {
            let lines = &self.lines;
            let state = &mut self.state;
            self.terminal
                .draw(|frame| render(frame, frame.area(), lines, state))
                .unwrap();
        }

        fn press(&mut self, code: KeyCode) -> DiffKeyAction {
            self.press_with(code, KeyModifiers::NONE)
        }

        fn press_ctrl(&mut self, code: KeyCode) -> DiffKeyAction {
            self.press_with(code, KeyModifiers::CONTROL)
        }

        fn press_with(&mut self, code: KeyCode, modifiers: KeyModifiers) -> DiffKeyAction {
            let action = handle_key(
                &mut self.state,
                self.lines.len(),
                KeyEvent::new(code, modifiers),
            );
            self.draw();
            action
        }

        fn rendered(&self) -> String {
            let buffer = self.terminal.backend().buffer();
            let area = buffer.area();
            (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    // -- exit ---------------------------------------------------------------

    #[test]
    fn q_and_ctrl_c_leave_the_renderer() {
        let mut rig = Rig::new(50, 10);
        assert_eq!(rig.press(KeyCode::Char('q')), DiffKeyAction::Exit);
        assert_eq!(rig.press_ctrl(KeyCode::Char('c')), DiffKeyAction::Exit);
    }

    #[test]
    fn a_bare_c_does_not_leave_the_renderer() {
        let mut rig = Rig::new(50, 10);
        assert_eq!(rig.press(KeyCode::Char('c')), DiffKeyAction::Continue);
    }

    // -- scrolling ----------------------------------------------------------

    #[test]
    fn j_and_down_both_scroll_one_line() {
        for code in [KeyCode::Char('j'), KeyCode::Down] {
            let mut rig = Rig::new(50, 10);
            rig.press(code);
            assert_eq!(rig.state.offset(), 1, "{code:?}");
        }
    }

    #[test]
    fn k_and_up_both_scroll_back_one_line() {
        for code in [KeyCode::Char('k'), KeyCode::Up] {
            let mut rig = Rig::new(50, 10);
            rig.press(KeyCode::Char('j'));
            rig.press(KeyCode::Char('j'));
            rig.press(code);
            assert_eq!(rig.state.offset(), 1, "{code:?}");
        }
    }

    /// Half of the VISIBLE height, which is the pane less its two borders — so
    /// a 10-row pane moves by four, and dragging the pane taller moves further.
    #[test]
    fn ctrl_d_and_ctrl_u_move_half_the_visible_height() {
        let mut rig = Rig::new(50, 10);
        rig.press_ctrl(KeyCode::Char('d'));
        assert_eq!(rig.state.offset(), 4);
        rig.press_ctrl(KeyCode::Char('u'));
        assert_eq!(rig.state.offset(), 0);
    }

    /// A pane too short to show two rows still moves by one. Halving to zero
    /// would read as a broken key rather than as a small pane.
    #[test]
    fn a_pane_too_short_to_halve_still_moves_by_one() {
        let mut rig = Rig::new(50, 3);
        rig.press_ctrl(KeyCode::Char('d'));
        assert_eq!(rig.state.offset(), 1);
    }

    // -- clamping -----------------------------------------------------------

    /// Scrolling past the end into blank rows would let the user lose the
    /// document and have to guess their way back.
    #[test]
    fn scrolling_down_stops_with_the_last_line_on_screen() {
        let mut rig = Rig::new(12, 10);
        for _ in 0..50 {
            rig.press(KeyCode::Char('j'));
        }
        // 12 lines, 8 visible rows: the furthest useful offset is 4.
        assert_eq!(rig.state.offset(), 4);
        assert!(rig.rendered().contains("line11"), "{}", rig.rendered());
    }

    #[test]
    fn scrolling_up_stops_at_the_top() {
        let mut rig = Rig::new(50, 10);
        for _ in 0..50 {
            rig.press(KeyCode::Char('k'));
        }
        assert_eq!(rig.state.offset(), 0);
    }

    /// A document shorter than the pane cannot scroll at all.
    #[test]
    fn a_document_that_fits_does_not_scroll() {
        let mut rig = Rig::new(3, 20);
        rig.press(KeyCode::Char('j'));
        rig.press_ctrl(KeyCode::Char('d'));
        rig.press(KeyCode::Char('G'));
        assert_eq!(rig.state.offset(), 0);
    }

    #[test]
    fn an_empty_document_cannot_scroll() {
        let mut rig = Rig::new(0, 10);
        rig.press(KeyCode::Char('G'));
        assert_eq!(rig.state.offset(), 0);
    }

    // -- jumps --------------------------------------------------------------

    #[test]
    fn capital_g_jumps_to_the_end() {
        let mut rig = Rig::new(12, 10);
        rig.press(KeyCode::Char('G'));
        assert_eq!(rig.state.offset(), 4);
    }

    #[test]
    fn gg_jumps_to_the_top() {
        let mut rig = Rig::new(50, 10);
        rig.press(KeyCode::Char('G'));
        assert!(rig.state.offset() > 0);

        rig.press(KeyCode::Char('g'));
        assert_eq!(rig.press(KeyCode::Char('g')), DiffKeyAction::Continue);
        assert_eq!(rig.state.offset(), 0);
    }

    /// A lone `g` arms the chord and moves nothing, and it never expires —
    /// exactly as in the tree pane (AgentTreeGgChordNeverExpires).
    #[test]
    fn a_lone_g_moves_nothing() {
        let mut rig = Rig::new(50, 10);
        rig.press(KeyCode::Char('j'));
        rig.press(KeyCode::Char('g'));
        assert_eq!(rig.state.offset(), 1);
    }

    /// Any other key disarms the chord and is then handled normally, so the
    /// swallowed `g` is the only trace a lone press leaves.
    #[test]
    fn a_key_between_the_two_gs_disarms_the_chord() {
        let mut rig = Rig::new(50, 10);
        rig.press(KeyCode::Char('G'));
        let before = rig.state.offset();

        rig.press(KeyCode::Char('g'));
        rig.press(KeyCode::Char('k'));
        rig.press(KeyCode::Char('g'));

        assert_eq!(rig.state.offset(), before - 1);
    }

    // -- the pane has no toggle of its own ----------------------------------

    /// Space, Enter and the all-files key do nothing here. The open set is
    /// decided in the tree and only in the tree, which is what makes one file
    /// have one open state — see DiffPaneHasNoToggleOfItsOwn in the spec.
    #[test]
    fn the_trees_toggle_keys_do_nothing_in_this_pane() {
        for code in [KeyCode::Char(' '), KeyCode::Enter, KeyCode::Char('a')] {
            let mut rig = Rig::new(50, 10);
            rig.press(KeyCode::Char('j'));
            assert_eq!(rig.press(code), DiffKeyAction::Continue, "{code:?}");
            assert_eq!(rig.state.offset(), 1, "{code:?}");
        }
    }

    // -- notices ------------------------------------------------------------

    #[test]
    fn a_notice_is_shown_and_cleared_by_the_next_key() {
        let mut rig = Rig::new(50, 10);
        rig.state.notice = Some("git: index.lock".to_string());
        rig.draw();
        assert!(rig.rendered().contains("index.lock"), "{}", rig.rendered());

        rig.press(KeyCode::Char('j'));
        assert!(rig.state.notice.is_none());
    }

    // -- truncation ---------------------------------------------------------

    /// A long line is cut at the pane's edge, not wrapped onto a second row.
    /// Wrapping would let one minified line push several files off screen — see
    /// DiffLinesTruncateRatherThanWrap in docs/specs/agent-tree.allium.
    #[test]
    fn a_long_line_is_truncated_rather_than_wrapped() {
        let mut rig = Rig::new(0, 6);
        rig.lines = vec![
            DiffLine {
                kind: DiffLineKind::Added,
                text: format!("+{}", "x".repeat(200)),
            },
            DiffLine {
                kind: DiffLineKind::Context,
                text: "second".to_string(),
            },
        ];
        rig.draw();

        let rendered = rig.rendered();
        assert!(
            rendered.contains("second"),
            "the next line must still be on screen; got:\n{rendered}"
        );
    }

    /// press_every_row_key / RecordedActionMatchesRow for the diff pane.
    #[test]
    fn pressing_each_key_of_each_diff_row_records_the_rows_action() {
        use crate::keybindings::{bindings_in, KeyNamespace};
        let mut pressed = 0;
        for binding in bindings_in(KeyNamespace::AgentDiff) {
            assert_eq!(binding.context, None);
            for key in binding.keys {
                let mut rig = Rig::new(60, 12);
                if *key == "gg" {
                    rig.press(KeyCode::Char('g'));
                    assert!(rig.state.usage.is_empty(), "first g is pending input");
                    rig.press(KeyCode::Char('g'));
                } else {
                    let ev = crate::cli::test_key_event(key);
                    handle_key(&mut rig.state, rig.lines.len(), ev);
                }
                pressed += 1;
                let detail = match *key {
                    k if k.starts_with("Ctrl+") => k[5..].to_lowercase(),
                    k => k.to_string(),
                };
                let got: Vec<_> = rig
                    .state
                    .usage
                    .iter()
                    .map(|e| (e.action.clone(), e.detail.clone()))
                    .collect();
                assert_eq!(
                    got,
                    vec![(binding.action.to_string(), Some(detail))],
                    "{key}"
                );
            }
        }
        assert!(pressed >= 10, "{pressed}");
    }

    /// Keys with no row — Space, Enter, `a`, Tab, and a modified `j` — do
    /// nothing and record nothing.
    #[test]
    fn a_press_with_no_row_does_nothing_in_the_diff_pane() {
        for key in ["Space", "Enter", "a", "Tab", "Ctrl+J", "Ctrl+G", "Ctrl+Q"] {
            let mut rig = Rig::new(60, 12);
            let ev = crate::cli::test_key_event(key);
            assert_eq!(
                handle_key(&mut rig.state, rig.lines.len(), ev),
                DiffKeyAction::Continue,
                "{key}"
            );
            assert!(rig.state.usage.is_empty(), "{key}");
            assert_eq!(rig.state.offset, 0, "{key}");
        }
    }
}
