//! The standalone CLI renderers dispatch runs in tmux panes of its own, plus
//! the small non-rendering subcommands.
//!
//! The two pane renderers ([`agent_tree`] and [`agent_diff`]) share their
//! entry-point shape — resolve the task, take the terminal, run a loop, give
//! the terminal back — and that shape lives here rather than in either of them.

use std::io;
use std::path::PathBuf;

use anyhow::{Context, Result};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::models::TaskId;

pub mod agent_diff;
pub mod agent_tree;
pub mod agent_tree_agents;
pub mod caller_headers;
pub mod commands;
pub mod statusline;

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

/// The worktree and base branch a pane renderer works from.
///
/// Both panes take a task id rather than a path, so they cannot disagree about
/// which worktree they are looking at and both resolve their baseline from the
/// same base branch. This is that lookup, once.
pub(crate) async fn pane_task_context(
    database: &dyn crate::db::TaskRead,
    task_id: i64,
) -> Result<(PathBuf, String)> {
    let task = database
        .get_task(TaskId(task_id))
        .await?
        .with_context(|| format!("task {task_id} not found"))?;
    let worktree = task
        .worktree
        .clone()
        .with_context(|| format!("task {task_id} has no worktree"))?;
    Ok((PathBuf::from(worktree), task.base_branch))
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
