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
pub(crate) mod epic_tree;
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

/// A `$HOME` *value* that names a home directory: non-empty and absolute.
///
/// The one test every reader of `$HOME` applies. An empty or relative value
/// composes into a path under whatever directory the process was started from,
/// which is not the operator's anything — see docs/specs/startup.allium:
/// `AnUnavailableHomeDirectoryIsAFailureNotAPath`.
pub(crate) fn usable_home(home: Option<&str>) -> Option<&str> {
    home.filter(|home| std::path::Path::new(home).is_absolute())
}

/// `storage.allium: DataDirectory`'s default, or an error naming the home
/// directory when there is none. Resolve it at the edge of a command that
/// needs it and pass the result inward.
pub fn default_data_dir() -> anyhow::Result<std::path::PathBuf> {
    data_dir_from(
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var("HOME").ok(),
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "$HOME is not set to an absolute path, so there is no default data \
             directory; set $HOME or pass --data-dir"
        )
    })
}

/// `storage.allium: DataDirectory`'s default, from `$XDG_DATA_HOME` and
/// `$HOME`. A relative `$XDG_DATA_HOME` (empty included) is ignored, as the XDG
/// Base Directory spec says, and an unusable `$HOME` ([`usable_home`]) leaves
/// no default at all rather than one under the working directory.
fn data_dir_from(
    xdg_data_home: Option<std::ffi::OsString>,
    home: Option<String>,
) -> Option<std::path::PathBuf> {
    let base = xdg_data_home
        .map(std::path::PathBuf::from)
        .filter(|xdg| xdg.is_absolute())
        .or_else(|| {
            usable_home(home.as_deref())
                .map(|home| std::path::Path::new(home).join(".local").join("share"))
        })?;
    Some(base.join("dispatch"))
}

/// The one budget-snapshot location on this machine.
///
/// Takes no data-directory argument, deliberately. The Claude subscription
/// windows it holds are account-global, so publisher and reader must agree on a
/// single location that does not vary with the data directory the current
/// process was started with — see `docs/specs/observability.allium`:
/// `SnapshotLocationIsFixedPerMachine`. Resolved once, in
/// `runtime::StartupPaths::resolve`.
pub(crate) fn budget_snapshot_path() -> anyhow::Result<std::path::PathBuf> {
    Ok(default_data_dir()?.join(crate::setup::statusline::RATE_LIMITS_FILE_NAME))
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
            Some(PathBuf::from("/xdg/dispatch"))
        );
    }

    /// A set-but-empty `XDG_DATA_HOME` counts as unset, as the XDG spec says,
    /// rather than resolving to the relative path `dispatch`.
    #[test]
    fn an_empty_xdg_data_home_falls_back_to_home() {
        assert_eq!(
            data_dir_from(Some(OsString::new()), Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.local/share/dispatch"))
        );
    }

    /// A relative `XDG_DATA_HOME` is ignored too (XDG Base Directory spec).
    #[test]
    fn a_relative_xdg_data_home_falls_back_to_home() {
        assert_eq!(
            data_dir_from(Some(OsString::from("rel")), Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.local/share/dispatch"))
        );
    }

    /// storage.allium: DataDirectory — with neither variable usable there is no
    /// default, not one under the working directory.
    #[test]
    fn an_unusable_home_leaves_no_default() {
        for home in [None, Some(""), Some("rel/home"), Some(".")] {
            assert_eq!(
                data_dir_from(None, home.map(String::from)),
                None,
                "home {home:?}"
            );
        }
    }

    /// A usable `$XDG_DATA_HOME` needs no home directory at all.
    #[test]
    fn xdg_data_home_needs_no_home() {
        assert_eq!(
            data_dir_from(Some(OsString::from("/xdg")), None),
            Some(PathBuf::from("/xdg/dispatch"))
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
        let path = super::budget_snapshot_path().expect("the test environment has a home");

        assert_eq!(
            path.file_name(),
            Some(std::ffi::OsStr::new("rate-limits.json"))
        );
        assert_eq!(
            path.parent(),
            Some(super::default_data_dir().expect("a home").as_path())
        );
    }
}
