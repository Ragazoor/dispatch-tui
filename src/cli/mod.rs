//! The standalone CLI renderers dispatch runs in tmux panes of its own, plus
//! the small non-rendering subcommands.
//!
//! The two pane renderers ([`agent_tree`] and [`agent_diff`]) share their
//! entry-point shape — resolve the task, take the terminal, run a loop, give
//! the terminal back — and that shape lives here rather than in either of them.

use std::io;
use std::path::PathBuf;

use anyhow::Result;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

pub mod agent_diff;
pub mod agent_tree;
pub mod agent_tree_agents;
pub mod caller_headers;
pub mod commands;
pub mod statusline;
pub mod store_import;

/// How far `Ctrl-D`/`Ctrl-U` move in a pane showing `viewport_rows` rows: half
/// of them, floored at one. A pane too short to show two rows would otherwise
/// halve to zero and turn both motions into no-ops, which reads as a broken key
/// rather than a small pane. One rule for the tree, its agents section and the
/// diff pane.
pub(crate) fn half_page(viewport_rows: usize) -> usize {
    (viewport_rows / 2).max(1)
}

/// A rendered test buffer as text, one line per row, trailing blanks trimmed.
#[cfg(test)]
pub(crate) fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area();
    let mut lines = Vec::with_capacity(area.height as usize);
    for y in area.top()..area.bottom() {
        let mut line = String::with_capacity(area.width as usize);
        for x in area.left()..area.right() {
            line.push_str(buf[(x, y)].symbol());
        }
        line.truncate(line.trim_end().len());
        lines.push(line);
    }
    lines.join("\n")
}

/// What one read of the store said about the pane's task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskLookup {
    /// No row yet. Rows arrive after the connection is up, so this is the
    /// normal answer for the first moments of a pane's life.
    NotYet,
    NoWorktree,
    Found {
        root: PathBuf,
        base_branch: String,
    },
    /// The board could not be reached, or answered something unreadable
    /// (agent-tree.allium: BoardPaneView, a failed read). Waited out like
    /// `NotYet`, and named -- with the address tried -- when the wait ends.
    BoardUnreachable {
        address: String,
    },
}

/// Where a pane renderer reads its task and the live agents from: the running
/// board, never the store (agent-tree.allium: PanesReadThroughTheBoard).
/// A trait so the pane's read paths can be driven by a fake board in tests;
/// production implements it over `crate::hooks::fetch_pane_view`.
#[async_trait::async_trait]
pub(crate) trait PaneViewSource: Send + Sync {
    /// One read of the board (BoardPaneView's `pane_view`). `Err` is a failed
    /// read, never an empty view.
    async fn pane_view(&self, task_id: i64) -> Result<crate::hooks::wire::PaneView>;
    /// The board address this source asks, for the failure text.
    fn board_address(&self) -> String;
}

/// The running board, asked over its port the way a hook asks it
/// (`crate::hooks::fetch_pane_view`).
pub(crate) struct BoardPaneSource {
    pub port: u16,
}

#[async_trait::async_trait]
impl PaneViewSource for BoardPaneSource {
    async fn pane_view(&self, task_id: i64) -> Result<crate::hooks::wire::PaneView> {
        crate::hooks::fetch_pane_view(self.port, task_id).await
    }
    fn board_address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

/// One read of the board, reduced to what the startup wait decides on.
pub(crate) async fn lookup_from_board(source: &dyn PaneViewSource, task_id: i64) -> TaskLookup {
    match source.pane_view(task_id).await {
        Err(e) => {
            tracing::warn!(task_id, "pane read of the board failed: {e:#}");
            TaskLookup::BoardUnreachable {
                address: source.board_address(),
            }
        }
        Ok(view) => match view.task {
            None => TaskLookup::NotYet,
            Some(task) => match task.worktree {
                None => TaskLookup::NoWorktree,
                Some(worktree) => TaskLookup::Found {
                    root: PathBuf::from(worktree),
                    base_branch: task.base_branch,
                },
            },
        },
    }
}

/// What a pane does next while it resolves its task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StartupStep {
    Wait,
    Proceed {
        root: PathBuf,
        base_branch: String,
    },
    /// Draw this text and stay open until the user quits.
    Fail(String),
}

/// How long a pane waits for its task row before giving up
/// (`agent-tree.allium`: `config.agent_tree_startup_wait`).
pub(crate) const STARTUP_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// How often the wait re-reads the board and redraws.
const STARTUP_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// Decide the next step from one read. Arrival wins over the limit: a row that
/// shows up on the last poll is still a row.
pub(crate) fn startup_step(
    task_id: i64,
    lookup: &TaskLookup,
    elapsed: std::time::Duration,
    limit: std::time::Duration,
) -> StartupStep {
    match lookup {
        TaskLookup::Found { root, base_branch } => StartupStep::Proceed {
            root: root.clone(),
            base_branch: base_branch.clone(),
        },
        TaskLookup::NoWorktree => StartupStep::Fail(format!("task {task_id} has no worktree")),
        TaskLookup::NotYet if elapsed < limit => StartupStep::Wait,
        TaskLookup::NotYet => StartupStep::Fail(format!("task {task_id} not found")),
        TaskLookup::BoardUnreachable { .. } if elapsed < limit => StartupStep::Wait,
        TaskLookup::BoardUnreachable { address } => StartupStep::Fail(format!(
            "could not reach the dispatch board at {address}; is it running? \
             (a board started with --port needs DISPATCH_PORT or --port here)"
        )),
    }
}

pub(crate) fn render_startup_notice(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    task_id: i64,
    step: &StartupStep,
) {
    let text = match step {
        StartupStep::Fail(reason) => format!("{reason}\n\npress q to close this pane"),
        _ => format!("waiting for task {task_id}"),
    };
    frame.render_widget(
        ratatui::widgets::Paragraph::new(text)
            .wrap(ratatui::widgets::Wrap { trim: true })
            .block(ratatui::widgets::Block::bordered()),
        area,
    );
}

fn is_quit_key(key: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyModifiers};
    key.code == KeyCode::Char('q')
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

/// Resolve the pane's task, drawing progress. `None` means the user closed the
/// pane from the failure screen. A failure never returns: it draws and waits
/// for a quit key, so the reason stays readable instead of vanishing with the
/// pane (`agent-tree.allium`: `AgentTreePaneTaskNeverArrives`).
///
/// Runs on the runtime's calling thread, like the render loops it precedes.
fn wait_for_pane_task(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    source: &dyn PaneViewSource,
    task_id: i64,
) -> Result<Option<(PathBuf, String)>> {
    let handle = tokio::runtime::Handle::current();
    let started = std::time::Instant::now();
    let failure = loop {
        let lookup =
            tokio::task::block_in_place(|| handle.block_on(lookup_from_board(source, task_id)));
        match startup_step(task_id, &lookup, started.elapsed(), STARTUP_WAIT) {
            StartupStep::Proceed { root, base_branch } => return Ok(Some((root, base_branch))),
            StartupStep::Fail(reason) => break StartupStep::Fail(reason),
            StartupStep::Wait => {
                terminal
                    .draw(|f| render_startup_notice(f, f.area(), task_id, &StartupStep::Wait))?;
                if quit_requested(STARTUP_POLL)? {
                    return Ok(None);
                }
            }
        }
    };
    loop {
        terminal.draw(|f| render_startup_notice(f, f.area(), task_id, &failure))?;
        if quit_requested(STARTUP_POLL)? {
            return Ok(None);
        }
    }
}

/// Wait up to `timeout` for a key; true if it was a quit key.
fn quit_requested(timeout: std::time::Duration) -> Result<bool> {
    use crossterm::event::{self, Event, KeyEventKind};
    if event::poll(timeout)? {
        if let Event::Key(key) = event::read()? {
            return Ok(key.kind == KeyEventKind::Press && is_quit_key(&key));
        }
    }
    Ok(false)
}

/// [`with_pane_terminal`] for a pane that works from a task: waits for the
/// task, then hands `body` its worktree and base branch. Closing the pane from
/// the wait screen returns `Ok(())` without running `body`.
pub(crate) fn with_pane_task(
    source: &dyn PaneViewSource,
    task_id: i64,
    body: impl FnOnce(&mut Terminal<CrosstermBackend<io::Stdout>>, PathBuf, String) -> Result<()>,
) -> Result<()> {
    with_pane_terminal(
        |terminal| match wait_for_pane_task(terminal, source, task_id)? {
            Some((root, base_branch)) => body(terminal, root, base_branch),
            None => Ok(()),
        },
    )
}

/// Take the terminal, run `body`, and give the terminal back — whatever `body`
/// did.
///
/// The restore is deliberately NOT behind `?`. A renderer that returns an error
/// still has to leave raw mode and the alternate screen, or the user is dropped
/// back into a shell that echoes nothing and shows no cursor, with no
/// indication why. Both panes get that property from here rather than each
/// re-deriving it, because the failure is silent and identical in both.
pub(crate) fn with_pane_terminal<T>(
    body: impl FnOnce(&mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<T>,
) -> Result<T> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = body(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    use std::time::Duration;

    const LIMIT: Duration = Duration::from_secs(10);

    fn found() -> TaskLookup {
        TaskLookup::Found {
            root: PathBuf::from("/wt"),
            base_branch: "main".into(),
        }
    }

    #[test]
    fn a_missing_row_is_waited_for_inside_the_limit() {
        let step = startup_step(7, &TaskLookup::NotYet, Duration::from_secs(9), LIMIT);
        assert_eq!(step, StartupStep::Wait);
    }

    #[test]
    fn a_row_that_arrives_proceeds() {
        let step = startup_step(7, &found(), Duration::from_secs(2), LIMIT);
        assert_eq!(
            step,
            StartupStep::Proceed {
                root: PathBuf::from("/wt"),
                base_branch: "main".into()
            }
        );
    }

    #[test]
    fn a_row_arriving_at_the_limit_still_proceeds() {
        let step = startup_step(7, &found(), LIMIT, LIMIT);
        assert!(matches!(step, StartupStep::Proceed { .. }));
    }

    #[test]
    fn a_row_that_never_arrives_fails_with_not_found_at_the_limit() {
        let step = startup_step(7, &TaskLookup::NotYet, LIMIT, LIMIT);
        assert_eq!(step, StartupStep::Fail("task 7 not found".into()));
    }

    #[test]
    fn a_task_without_a_worktree_fails_at_once() {
        let step = startup_step(7, &TaskLookup::NoWorktree, Duration::ZERO, LIMIT);
        assert_eq!(step, StartupStep::Fail("task 7 has no worktree".into()));
    }

    fn drawn(step: &StartupStep) -> String {
        let backend = ratatui::backend::TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_startup_notice(f, f.area(), 7, step))
            .unwrap();
        buffer_to_string(terminal.backend().buffer())
    }

    #[test]
    fn the_pane_says_what_it_is_waiting_for() {
        assert!(drawn(&StartupStep::Wait).contains("waiting for task 7"));
    }

    #[test]
    fn the_pane_draws_the_failure_text() {
        let out = drawn(&StartupStep::Fail("task 7 not found".into()));
        assert!(out.contains("task 7 not found"), "{out}");
    }

    // -- PanesReadThroughTheBoard (task #4982) -------------------------------

    use crate::hooks::wire::{PaneTask, PaneView};

    const BOARD: &str = "127.0.0.1:8899";

    fn unreachable() -> TaskLookup {
        TaskLookup::BoardUnreachable {
            address: BOARD.into(),
        }
    }

    /// A board that is mid-restart is waited out like a row that has not
    /// arrived: it is most often back within the wait
    /// (agent-tree.allium: "Companion Pane Startup").
    #[test]
    fn an_unreachable_board_is_waited_out_inside_the_limit() {
        let step = startup_step(7, &unreachable(), Duration::from_secs(3), LIMIT);
        assert_eq!(step, StartupStep::Wait);
    }

    /// AgentTreePaneTaskNeverArrives: when the last attempt could not reach
    /// the board, the pane says THAT, naming the address it tried -- "not
    /// found" would send the user looking for a task that may well exist.
    #[test]
    fn an_unreachable_board_at_the_limit_is_named_not_reported_as_a_missing_task() {
        let StartupStep::Fail(reason) = startup_step(7, &unreachable(), LIMIT, LIMIT) else {
            panic!("an unreachable board at the limit must end the wait");
        };
        assert!(
            reason.contains(BOARD),
            "must name the board address: {reason}"
        );
        assert!(
            !reason.contains("not found"),
            "an unreachable board is not a missing task: {reason}"
        );
    }

    /// One scripted answer per read. `Err` stands for a board that could not
    /// be reached or answered something unreadable.
    struct FakeBoard {
        answer: std::result::Result<PaneView, String>,
    }

    #[async_trait::async_trait]
    impl PaneViewSource for FakeBoard {
        async fn pane_view(&self, _task_id: i64) -> Result<PaneView> {
            self.answer.clone().map_err(|e| anyhow::anyhow!(e))
        }
        fn board_address(&self) -> String {
            BOARD.into()
        }
    }

    fn view(task: Option<PaneTask>) -> FakeBoard {
        FakeBoard {
            answer: Ok(PaneView {
                task,
                live_agents: vec![],
            }),
        }
    }

    #[tokio::test]
    async fn a_task_the_board_holds_with_a_worktree_is_found() {
        let board = view(Some(PaneTask {
            worktree: Some("/wt/7".into()),
            base_branch: "develop".into(),
        }));
        assert_eq!(
            lookup_from_board(&board, 7).await,
            TaskLookup::Found {
                root: PathBuf::from("/wt/7"),
                base_branch: "develop".into()
            }
        );
    }

    #[tokio::test]
    async fn a_task_the_board_holds_without_a_worktree_has_none() {
        let board = view(Some(PaneTask {
            worktree: None,
            base_branch: "main".into(),
        }));
        assert_eq!(lookup_from_board(&board, 7).await, TaskLookup::NoWorktree);
    }

    /// PaneView.task = null: the row has not reached the board's rows yet.
    #[tokio::test]
    async fn a_task_the_board_does_not_hold_yet_is_not_yet() {
        assert_eq!(lookup_from_board(&view(None), 7).await, TaskLookup::NotYet);
    }

    /// A failed read is its own answer, carrying the address -- never read as
    /// "no such task".
    #[tokio::test]
    async fn a_board_that_does_not_answer_is_unreachable_not_missing() {
        let board = FakeBoard {
            answer: Err("connection refused".into()),
        };
        assert_eq!(lookup_from_board(&board, 7).await, unreachable());
    }
}
