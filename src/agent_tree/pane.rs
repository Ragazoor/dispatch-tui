//! The shell both agent-tree pane renderers share — the tree and the diff
//! viewer: resolve the task, take the terminal, run a loop, give the terminal
//! back, and send the pane's keypress usage to the board. That shape lives
//! here rather than in either of them.

use std::io;
use std::path::PathBuf;

use anyhow::Result;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::models::TaskId;

/// How far `Ctrl-D`/`Ctrl-U` move in a pane showing `viewport_rows` rows: half
/// of them, floored at one. A pane too short to show two rows would otherwise
/// halve to zero and turn both motions into no-ops, which reads as a broken key
/// rather than a small pane. One rule for the tree, its agents section and the
/// diff pane.
pub(crate) fn half_page(viewport_rows: usize) -> usize {
    (viewport_rows / 2).max(1)
}

/// Sends a pane's keypress usage to the running board, which records it
/// (`PanesReadThroughTheBoard`: a pane opens no store connection of its own,
/// so its presses reach the usage store the way its reads do, over the
/// board's port). Each press is sent on a task beside the render loop so a
/// slow board never delays a keypress; [`PaneUsage::flush`] waits for them
/// before the pane exits, so the closing press is not lost. An unreachable
/// board drops the press with a warning, as an observed hook event is dropped.
pub(crate) struct PaneUsage {
    port: u16,
    task_id: TaskId,
    handle: tokio::runtime::Handle,
    pending: Vec<tokio::task::JoinHandle<()>>,
}

impl PaneUsage {
    /// Must be called from within the runtime that runs the pane.
    pub(crate) fn new(port: u16, task_id: TaskId) -> Self {
        Self {
            port,
            task_id,
            handle: tokio::runtime::Handle::current(),
            pending: Vec::new(),
        }
    }

    pub(crate) fn record(&mut self, events: Vec<crate::models::UsageEvent>) {
        self.pending.retain(|h| !h.is_finished());
        for event in events {
            let (port, task_id) = (self.port, self.task_id);
            self.pending.push(self.handle.spawn(async move {
                let observed = crate::hooks::wire::ObservedEvent::PaneKey {
                    task_id,
                    action: event.action.clone(),
                    key: event.detail.clone().unwrap_or_default(),
                };
                if let Err(e) = crate::hooks::deliver(port, observed).await {
                    tracing::warn!(
                        target: "usage",
                        action = %event.action,
                        error = %format!("{e:#}"),
                        "pane could not send a keypress usage event to the board"
                    );
                }
            }));
        }
    }

    /// Wait for every outstanding send. The pane's loop runs on a runtime
    /// worker thread, so this hands the thread back while it waits.
    pub(crate) fn flush(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        if pending.is_empty() {
            return;
        }
        tokio::task::block_in_place(|| {
            self.handle.block_on(async {
                for h in pending {
                    let _ = h.await;
                }
            })
        });
    }
}

/// The key event for a key as the table writes it ("Ctrl+D", "Space", "G").
/// A letter after Ctrl is reported lowercase, as a terminal does.
#[cfg(test)]
pub(crate) fn test_key_event(key: &str) -> crossterm::event::KeyEvent {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    if let Some(rest) = key.strip_prefix("Ctrl+") {
        let base = test_key_event(rest);
        let code = match base.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        };
        return KeyEvent::new(code, KeyModifiers::CONTROL);
    }
    let code = match key {
        "Space" => KeyCode::Char(' '),
        "Enter" => KeyCode::Enter,
        "Tab" => KeyCode::Tab,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        k if k.chars().count() == 1 => KeyCode::Char(k.chars().next().unwrap_or(' ')),
        other => panic!("test cannot press {other}"),
    };
    KeyEvent::new(code, KeyModifiers::NONE)
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
    async fn pane_view(&self, task_id: TaskId) -> Result<crate::hooks::wire::PaneView>;
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
    async fn pane_view(&self, task_id: TaskId) -> Result<crate::hooks::wire::PaneView> {
        crate::hooks::fetch_pane_view(self.port, task_id).await
    }
    fn board_address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

/// One read of the board, reduced to what the startup wait decides on.
pub(crate) async fn lookup_from_board(source: &dyn PaneViewSource, task_id: TaskId) -> TaskLookup {
    match source.pane_view(task_id).await {
        Err(e) => {
            tracing::warn!(task_id = task_id.0, "pane read of the board failed: {e:#}");
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
    task_id: TaskId,
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
    task_id: TaskId,
    step: &StartupStep,
    ns: crate::keybindings::KeyNamespace,
) {
    let text = match step {
        StartupStep::Fail(reason) => {
            format!(
                "{reason}\n\npress {} to close this pane",
                exit_keys(ns).join(" or ")
            )
        }
        _ => format!("waiting for task {task_id}"),
    };
    frame.render_widget(
        ratatui::widgets::Paragraph::new(text)
            .wrap(ratatui::widgets::Wrap { trim: true })
            .block(ratatui::widgets::Block::bordered()),
        area,
    );
}

/// Whether the press is one of the pane's exit keys: the keys of the
/// `exit_pane` row, so the startup screen and the pane proper agree. Nothing is
/// recorded: the pane has not resolved a task yet, and the screen answers one
/// key only (`PaneStartupScreenIsOutsideUsage` in `docs/specs/keybindings.allium`).
pub(crate) fn is_quit_key(
    key: &crossterm::event::KeyEvent,
    ns: crate::keybindings::KeyNamespace,
) -> bool {
    let name = crate::keybindings::key_name(*key);
    exit_keys(ns).contains(&name.as_str())
}

/// The keys of `ns`'s `exit_pane` row.
fn exit_keys(ns: crate::keybindings::KeyNamespace) -> Vec<&'static str> {
    crate::keybindings::bindings_in(ns)
        .filter(|b| b.action == "exit_pane")
        .flat_map(|b| b.keys.iter().copied())
        .collect()
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
    task_id: TaskId,
    ns: crate::keybindings::KeyNamespace,
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
                terminal.draw(|f| {
                    render_startup_notice(f, f.area(), task_id, &StartupStep::Wait, ns)
                })?;
                if quit_requested(STARTUP_POLL, ns)? {
                    return Ok(None);
                }
            }
        }
    };
    loop {
        terminal.draw(|f| render_startup_notice(f, f.area(), task_id, &failure, ns))?;
        if quit_requested(STARTUP_POLL, ns)? {
            return Ok(None);
        }
    }
}

/// Wait up to `timeout` for a key; true if it was a quit key.
fn quit_requested(
    timeout: std::time::Duration,
    ns: crate::keybindings::KeyNamespace,
) -> Result<bool> {
    use crossterm::event::{self, Event, KeyEventKind};
    if event::poll(timeout)? {
        if let Event::Key(key) = event::read()? {
            return Ok(key.kind == KeyEventKind::Press && is_quit_key(&key, ns));
        }
    }
    Ok(false)
}

/// [`with_pane_terminal`] for a pane that works from a task: waits for the
/// task, then hands `body` its worktree and base branch. Closing the pane from
/// the wait screen returns `Ok(())` without running `body`.
pub(crate) fn with_pane_task(
    source: &dyn PaneViewSource,
    task_id: TaskId,
    ns: crate::keybindings::KeyNamespace,
    body: impl FnOnce(&mut Terminal<CrosstermBackend<io::Stdout>>, PathBuf, String) -> Result<()>,
) -> Result<()> {
    with_pane_terminal(
        |terminal| match wait_for_pane_task(terminal, source, task_id, ns)? {
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
mod tests;
