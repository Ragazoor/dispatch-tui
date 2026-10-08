//! The agent-tree feature: the companion pane beside a dispatched agent that
//! shows its changed-file tree, its branch's commits and the live agents, and
//! the diff viewer pane beneath it. See `docs/specs/agent-tree.allium`.
//!
//! Both panes run as processes of their own in tmux, started by the
//! `dispatch agent-tree` and `dispatch agent-diff` subcommands; `crate::cli`
//! holds only those thin entry points. The board side — opening and closing
//! the panes beside an agent — is `crate::dispatch`, which reaches in here for
//! the [`diff_pane`] tmux effect and the [`open_set`] the two panes share.
//!
//! - [`model`]: the pure tree, built from what git printed.
//! - [`changes`]: the git queries behind the tree and the commits section.
//! - [`state`], [`keys`], [`render`], [`run`]: the tree pane's view state, key
//!   handling, drawing and polling loop.
//! - [`diff_viewer`]: the diff pane's renderer.
//! - [`pane`], [`list_cursor`]: the shell and list motion both panes share.

pub mod changes;
pub mod diff_pane;
pub mod diff_viewer;
pub mod keys;
pub mod list_cursor;
pub mod model;
pub mod open_set;
pub mod pane;
pub mod render;
pub mod run;
pub mod state;

#[cfg(test)]
pub(crate) mod test_repo;
#[cfg(test)]
mod tests;
