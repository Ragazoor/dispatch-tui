#![recursion_limit = "256"]

/// Default port for the MCP server, used when `DISPATCH_PORT` is not set.
pub const DEFAULT_PORT: u16 = 3142;

pub mod agent_tree;
pub mod agent_tree_diff_pane;
pub mod agent_tree_open_set;
pub mod backoff;
pub mod board_event;
pub(crate) mod claude_paths;
pub mod cli;
pub mod clock;
pub mod dispatch;
pub mod editor;
pub mod embeddings;
pub mod feed;
pub mod git;
pub mod hooks;
pub mod host_file;
pub mod keybindings;
pub mod mcp;
pub mod models;
pub mod notify;
pub(crate) mod palette;
pub mod plan;
pub mod process;
pub mod repo_sync;
pub mod runtime;
pub mod service;
pub mod setup;
pub mod spacetime;
pub mod startup;
pub mod startup_abort;
pub mod store;
pub mod store_connection;
pub mod sync;
#[cfg(test)]
mod test_log;
pub mod tmux;
pub mod tui;
pub mod worktree_admin;

pub fn default_data_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            // An empty `$HOME` takes the same fallback as an absent one: it
            // would otherwise resolve `.local/share` against whatever
            // directory the process was started from. See
            // `crate::setup::home_dir`.
            let home = std::env::var("HOME")
                .ok()
                .filter(|home| !home.is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            home.join(".local").join("share")
        });
    base.join("dispatch")
}

/// The one budget-snapshot location on this machine.
///
/// Takes no database argument, deliberately. The Claude subscription windows it
/// holds are account-global, so publisher and reader must agree on a single
/// location that does not vary with whichever task database the current process
/// happens to have open — see `docs/specs/observability.allium`:
/// `SnapshotLocationIsFixedNotDerivedFromTheOpenDatabase`.
pub(crate) fn budget_snapshot_path() -> std::path::PathBuf {
    default_data_dir().join(crate::setup::statusline::RATE_LIMITS_FILE_NAME)
}

#[cfg(test)]
mod budget_snapshot_path_tests {
    /// The snapshot sits in the default data directory, under its own fixed name.
    /// The file name is spelled out rather than imported from the constant the
    /// code reads: an expectation derived from the code under test asserts
    /// nothing.
    #[test]
    fn sits_beside_the_default_database() {
        let path = super::budget_snapshot_path();

        assert_eq!(
            path.file_name(),
            Some(std::ffi::OsStr::new("rate-limits.json"))
        );
        assert_eq!(path.parent(), Some(super::default_data_dir().as_path()));
    }
}
