//! Domain model.
//!
//! Types are split per concern into submodules and re-exported here, so
//! external code continues to use flat paths (`models::Task`, `models::Epic`,
//! `models::expand_tilde`, …) regardless of which submodule owns a type.
//!
//! - [`ids`] — the `define_id_newtype!` macro behind `TaskId`/`EpicId`/`LearningId`
//! - [`string_enum`] — the `define_str_enum!` macro behind status/tag/mode string conversions
//! - [`paths`] — path utilities (`expand_tilde`) and the repo-grouping family
//!   (`repo_name_from_path`/`repo_name_from_url`/`extract_github_repo`)
//! - [`tmux_window`] — [`TmuxWindow`], the `task-<id>` window/session naming convention
//! - [`tasks`] — tasks, statuses, tags, dispatch mode, slugify, age formatting
//! - [`dispatch_request`] — the store-split M4 dispatch request row and its vocabulary
//! - [`session_request`] — the store-split M5 wrap-up and exit-session request row
//! - [`agents`] — the agents A1 agent row and its vocabulary
//! - [`epics`] — epics, epic sub-status, descendant traversal
//! - [`review`] — review decisions, PR-URL parsing
//! - [`learnings`] — knowledge-base entries
//! - [`usage`] — usage events
//! - [`budget`] — Claude subscription rate-limit windows
//! - [`columns`] — `ColumnSection`, the sub-status sections inside a board column
//! - [`interval`] — the interval literal (`10m`, `600`) every cadence field takes
//! - [`url`] — typed task URLs
//! - [`poll_owner`] / [`poll_action`] — `core/PollOwner` scopes and the per-tick claim decision

// `define_id_newtype!` is `#[macro_export]`ed (crate root); consuming modules
// bring it into scope with `use crate::define_id_newtype;`.
mod ids;

// `define_str_enum!` is `#[macro_export]`ed (crate root); consuming modules
// bring it into scope with `use crate::define_str_enum;`.
mod string_enum;

pub mod hex;

mod paths;
pub use paths::{
    expand_tilde, expand_tilde_with_home, extract_github_repo, repo_name_from_path,
    repo_name_from_url, validate_repo_path, UNKNOWN_REPO_GROUP,
};

mod tmux_window;
pub(crate) use tmux_window::is_pane_id;
#[cfg(any(test, feature = "test-support"))]
pub use tmux_window::{test_pane_id, test_tmux_window};
pub use tmux_window::{PaneId, TmuxWindow};

#[cfg(any(test, feature = "test-support"))]
mod epic_builder;
#[cfg(any(test, feature = "test-support"))]
pub use epic_builder::EpicBuilder;
#[cfg(any(test, feature = "test-support"))]
mod task_builder;
#[cfg(any(test, feature = "test-support"))]
pub use task_builder::TaskBuilder;

pub mod learnings;
pub use learnings::*;

pub mod review;
pub use review::*;
pub mod tasks;
pub use tasks::*;

pub mod dispatch_request;
pub use dispatch_request::*;
pub mod agents;
pub use agents::*;
pub mod session_request;
pub use session_request::*;

pub mod agent_events;
pub use agent_events::*;

pub mod epics;
pub use epics::*;

pub mod people;
pub mod visibility;
pub use people::{
    resolve_person, shown_name, AdminSnapshot, DirectoryPerson, MemberRole, UnseenItemRow,
    WorkspaceDetail, WorkspaceMember, WorkspaceRef, NO_WORKSPACE,
};
pub use visibility::Visibility;
pub mod work_packages;
pub use work_packages::*;

pub mod usage;
pub use usage::*;

pub mod budget;
pub use budget::*;

mod columns;
pub use columns::{section_sort_priority, ColumnSection};

mod interval;
pub use interval::{parse_interval_secs, INTERVAL_EXAMPLES, MIN_FEED_INTERVAL_SECS};

mod url;
pub use url::{TaskUrl, UrlType};

mod poll_owner;
pub use poll_owner::PollScopeId;

mod patches;
pub use patches::{
    CreateLearningRow, CreateTaskRequest, EpicPatch, LearningFilter, LearningPatch, TaskPatch,
    UsageQuery,
};

mod poll_action;
pub(crate) use poll_action::{decide_poll_action, PollAction};

/// Whether `query` matches an item with this `title` and `description`.
///
/// The query splits on whitespace into words; every word must be a
/// case-insensitive substring of the title or the description. A blank query
/// has no words and matches everything. See `ListTasksViaMcp` in
/// `docs/specs/mcp-task-tools.allium`.
pub fn text_query_matches(query: &str, title: &str, description: &str) -> bool {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let title = title.to_lowercase();
    let description = description.to_lowercase();
    words
        .iter()
        .all(|w| title.contains(w.as_str()) || description.contains(w.as_str()))
}
