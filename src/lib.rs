#![recursion_limit = "256"]

/// Default port for the MCP server, used when `DISPATCH_PORT` is not set.
pub const DEFAULT_PORT: u16 = 3142;

pub mod agent_tree;
pub mod atomic_write;
pub mod backoff;
pub mod board_event;
pub(crate) mod claude_paths;
pub mod cli;
pub mod clock;
pub mod dispatch;
pub mod editor;
pub mod embeddings;
pub(crate) mod epic_header;
pub mod feed;
pub mod git;
pub mod hooks;
pub mod host_file;
pub mod keep_awake;
pub mod keybindings;
pub mod mcp;
pub mod models;
pub(crate) mod palette;
pub mod plan;
pub mod process;
pub mod runtime;
pub mod service;
pub mod setup;
pub mod spacetime;
pub mod startup;
pub mod startup_abort;
pub mod store;
pub mod store_connection;
pub mod sync;
#[cfg(any(test, feature = "test-support"))]
pub mod test_git;
#[cfg(test)]
mod test_log;
pub(crate) mod text;
pub mod tmux;
pub mod tui;
pub mod worktree_admin;

pub fn default_data_dir() -> std::path::PathBuf {
    data_dir_from(
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var("HOME").ok(),
    )
}

/// `storage.allium: DataDirectory`'s default, from `$XDG_DATA_HOME` and
/// `$HOME`. An empty value of either takes the same fallback as an absent one:
/// it would otherwise resolve against whatever directory the process was
/// started from. See `crate::setup::home_dir`.
fn data_dir_from(
    xdg_data_home: Option<std::ffi::OsString>,
    home: Option<String>,
) -> std::path::PathBuf {
    let base = xdg_data_home
        .filter(|xdg| !xdg.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = home
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
mod default_data_dir_tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::data_dir_from;

    /// storage.allium: DataDirectory — `$XDG_DATA_HOME/dispatch`, else
    /// `~/.local/share/dispatch`.
    #[test]
    fn xdg_data_home_wins_when_set() {
        assert_eq!(
            data_dir_from(Some(OsString::from("/xdg")), Some("/home/u".into())),
            PathBuf::from("/xdg/dispatch")
        );
    }

    /// A set-but-empty `XDG_DATA_HOME` counts as unset, as the XDG spec says,
    /// rather than resolving to the relative path `dispatch`.
    #[test]
    fn an_empty_xdg_data_home_falls_back_to_home() {
        assert_eq!(
            data_dir_from(Some(OsString::new()), Some("/home/u".into())),
            PathBuf::from("/home/u/.local/share/dispatch")
        );
    }

    #[test]
    fn an_empty_home_falls_back_to_the_current_directory() {
        assert_eq!(
            data_dir_from(None, Some(String::new())),
            PathBuf::from("./.local/share/dispatch")
        );
    }
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
