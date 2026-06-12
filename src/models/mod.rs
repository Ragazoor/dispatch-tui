//! Domain model.
//!
//! Types are split per concern into submodules and re-exported here, so
//! external code continues to use flat paths (`models::Task`, `models::Epic`,
//! `models::expand_tilde`, …) regardless of which submodule owns a type.
//!
//! - [`ids`] — the `define_id_newtype!` macro behind `TaskId`/`EpicId`/`LearningId`
//! - [`paths`] — path utilities (`expand_tilde`)
//! - [`tasks`] — tasks, statuses, tags, dispatch mode, slugify, age formatting
//! - [`epics`] — epics, epic sub-status, descendant traversal
//! - [`review`] — review decisions, PR-URL parsing
//! - [`learnings`] — knowledge-base entries
//! - [`usage`] — usage events
//! - [`columns`] — `VisualColumn` kanban board layout
//! - [`url`] — typed task URLs

// `define_id_newtype!` is `#[macro_export]`ed (crate root); consuming modules
// bring it into scope with `use crate::define_id_newtype;`.
mod ids;

mod paths;
pub use paths::expand_tilde;

pub mod learnings;
pub use learnings::*;

pub mod review;
pub use review::*;
pub mod tasks;
pub use tasks::*;

pub mod epics;
pub use epics::*;

pub mod usage;
pub use usage::*;

mod columns;
pub use columns::VisualColumn;

mod url;
pub use url::{TaskUrl, UrlType};
