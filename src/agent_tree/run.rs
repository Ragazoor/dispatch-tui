//! `dispatch agent-tree <task_id>`'s loop — a small, standalone ratatui loop
//! that renders one task's changed-file tree (see `docs/specs/agent-tree.allium`'s
//! `AgentTreeCompanionPane` surface and `RefreshAgentTree` rule).
//!
//! Deliberately NOT part of the board TUI's `App`/message loop: this runs as its
//! own process in a tmux companion pane. This module owns the polling loop:
//! the tree's refresh, the background pollers for the commits and agents
//! sections, and keeping the diff pane below in step with the open set.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::backend::Backend;
use ratatui::Terminal;

use crate::agent_tree::changes::{git_branch_commits, git_changes};
use crate::agent_tree::keys::{handle_key, jump_to_agent, KeyAction};
use crate::agent_tree::model::{build_tree, TreeNode};
use crate::agent_tree::render::agents::AgentRow;
use crate::agent_tree::render::commits::{AgentCommit, COMMITS_REFRESH_INTERVAL};
use crate::agent_tree::render::render_pane;
use crate::agent_tree::state::{Notice, RenderState};
use crate::models::{TaskId, TmuxWindow};
use crate::process::{ProcessRunner, RealProcessRunner};

/// Redraw cadence — see `docs/specs/agent-tree.allium`'s
/// `config.agent_tree_refresh_interval`. Doubles as the crossterm event
/// poll timeout, so a key press and a plain timer tick share one wait.
pub(crate) const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// Everything the loop needs to keep the diff pane in step with the open set:
/// the worktree both panes read, and which database and task the pane below
/// should open.
///
/// Bundled because they travel together and none means anything without the
/// others — `root` alone cannot name the pane's task, and the task id alone
/// cannot say which database holds it.
pub(crate) struct DiffPaneContext<'a> {
    pub root: &'a Path,
    pub data_dir: &'a Path,
    pub task_id: TaskId,
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
        crate::agent_tree::open_set::write_open_set(&root, &state.open_diffs_in_tree_order(tree))
    {
        tracing::warn!(root, error = %format!("{e:#}"), "failed to record the open set");
        state.notice = Some(Notice::diff(format!("{e:#}")));
        return;
    }

    let my_pane = match crate::agent_tree::diff_pane::current_pane_from_env() {
        Ok(pane) => pane,
        Err(e) => {
            state.notice = Some(Notice::diff(e.to_string()));
            return;
        }
    };

    if let Err(e) = crate::agent_tree::diff_pane::reconcile_diff_pane(
        &my_pane,
        context.data_dir,
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

/// The loop's answer to [`KeyAction::SourceChanged`]: the tree switches now
/// and starts from nothing — the old source's tree is never drawn under the
/// new source's name — and the selection is published where the diff pane
/// reads it (`SelectAgentTreeSource`, `AgentTreeSourceIsOneSelection`).
pub(crate) fn adopt_selected_source(root: &Path, tree: &mut TreeNode, state: &mut RenderState) {
    *tree = build_tree(root, &[]);
    // The diff pane reads the selection from beside the open set. A failure
    // is the direct answer to a keypress, like a failed diff-pane split.
    if let Err(e) = crate::agent_tree::open_set::write_selected_source(
        &root.to_string_lossy(),
        state.selected_commit.as_deref(),
    ) {
        tracing::warn!(root = %root.display(), error = %format!("{e:#}"), "failed to record the selected source");
        state.notice = Some(Notice::diff(format!("{e:#}")));
    }
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
pub(super) fn adopt_tree(rebuilt: TreeNode, tree: &mut TreeNode, state: &mut RenderState) {
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

/// Reads of the agent's commits for the commits section, one per
/// `COMMITS_REFRESH_INTERVAL`, produced off the render loop: resolving the fork
/// point runs up to four git commands, which must never delay a keypress.
pub(crate) type CommitReads = std::sync::mpsc::Receiver<Result<Vec<AgentCommit>, String>>;

/// Take every read that has arrived since the last pass, each in turn so a
/// failure followed by a recovery leaves no stale notice behind. When a read
/// moved the selection (the selected commit left the branch) the tree switches
/// to unstaged work and the new selection is published, exactly as for a
/// keypress.
fn drain_commit_reads(
    commit_reads: &CommitReads,
    root: &Path,
    tree: &mut TreeNode,
    state: &mut RenderState,
) -> bool {
    let before = state.selected_commit.clone();
    while let Ok(read) = commit_reads.try_recv() {
        state.adopt_commit_list(read);
    }
    let moved = state.selected_commit != before;
    if moved {
        adopt_selected_source(root, tree, state);
    }
    moved
}

/// Read the agent's commits on their own timer from a worker thread and send
/// each read to the render loop. Only news is sent, and a failure always is.
/// The thread ends when the loop drops its receiver.
fn spawn_commit_list_poller(root: PathBuf, base_branch: String) -> CommitReads {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runner = RealProcessRunner::default();
        let mut last_sent: Option<Vec<AgentCommit>> = None;
        loop {
            let read =
                git_branch_commits(&root, &base_branch, &runner).map_err(|e| format!("{e:#}"));
            let news = match &read {
                Ok(commits) => last_sent.as_ref() != Some(commits),
                Err(_) => true,
            };
            if news {
                last_sent = read.as_ref().ok().cloned();
                if tx.send(read).is_err() {
                    return;
                }
            }
            std::thread::sleep(COMMITS_REFRESH_INTERVAL);
        }
    });
    rx
}

fn run_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    context: &DiffPaneContext<'_>,
    agent_reads: &AgentReads,
    commit_reads: &CommitReads,
    runner: &dyn ProcessRunner,
    usage: &mut crate::agent_tree::pane::PaneUsage,
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
    refresh(root, runner, &mut tree, &mut state);

    loop {
        drain_agent_reads(agent_reads, &mut state);
        if drain_commit_reads(commit_reads, root, &mut tree, &mut state) {
            refresh(root, runner, &mut tree, &mut state);
        }
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
                KeyAction::SourceChanged => {
                    adopt_selected_source(root, &mut tree, &mut state);
                    refresh(root, runner, &mut tree, &mut state);
                }
            }
            continue;
        }

        // Poll timed out with no key event: the ~1s timer tick.
        refresh(root, runner, &mut tree, &mut state);
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
pub(super) fn refresh(
    root: &Path,
    runner: &dyn ProcessRunner,
    tree: &mut TreeNode,
    state: &mut RenderState,
) {
    // The source is the selection: unstaged work, or one commit
    // (AgentTreeSourceIsOneSelection).
    match git_changes(root, state.selected_commit.as_deref(), runner) {
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
                selected_commit = state.selected_commit.as_deref(),
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
/// doc comment). Resolves the task's worktree and base branch from the board once,
/// then re-queries git on a 1-second timer and lists the agent's commits on a
/// worker thread.
pub async fn run(data_dir: &Path, board_port: u16, task_id: TaskId) -> Result<()> {
    // The task and the live-agent list come from the running board, which
    // already holds them (`PanesReadThroughTheBoard`).
    let source = std::sync::Arc::new(crate::agent_tree::pane::BoardPaneSource { port: board_port });
    let (agent_reads, poller) = spawn_agent_list_poller(source.clone(), task_id);
    let mut usage = crate::agent_tree::pane::PaneUsage::new(board_port, task_id);

    let result = crate::agent_tree::pane::with_pane_task(
        &*source,
        task_id,
        crate::keybindings::KeyNamespace::AgentTreeTree,
        |terminal, root, base_branch| {
            // Start from a clean slate. The open set is view state, like the cursor and
            // the manual expansions, and a set left behind by a killed renderer
            // describes nothing — see the AgentTreeCompanionPane surface's guidance.
            let _ = crate::agent_tree::open_set::clear_open_set(&root.to_string_lossy());
            // The selection starts on unstaged work like everything else, so
            // one a killed renderer left behind must not point the diff pane
            // at a commit this renderer is not showing.
            let _ =
                crate::agent_tree::open_set::write_selected_source(&root.to_string_lossy(), None);
            let commit_reads = spawn_commit_list_poller(root.clone(), base_branch);
            run_loop(
                terminal,
                &DiffPaneContext {
                    root: &root,
                    data_dir,
                    task_id,
                },
                &agent_reads,
                &commit_reads,
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
    source: &dyn crate::agent_tree::pane::PaneViewSource,
    own: TaskId,
) -> Result<Vec<AgentRow>, String> {
    let view = source.pane_view(own).await.map_err(|e| {
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
                id: agent.id,
                title: agent.title,
                window: TmuxWindow::parse(&agent.tmux_window)?,
                is_own: agent.id == own,
            })
        })
        .collect();
    rows.sort_by_key(|row| row.id);
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
    source: std::sync::Arc<dyn crate::agent_tree::pane::PaneViewSource>,
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
