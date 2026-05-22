//! Per-domain command inner enums.
//!
//! Variants of the outer [`crate::tui::types::Command`] enum are progressively
//! migrated into per-domain inner enums. Each module here owns one domain's
//! side-effect commands.

pub mod editor;
pub mod epic;
pub mod feed;
pub mod learnings;
pub mod main_session;
pub mod pr;
pub mod project;
pub mod repo_filter;
pub mod split;
pub mod system;
pub mod task;
pub mod tips;

pub use editor::EditorCommand;
pub use epic::EpicCommand;
pub use feed::FeedCommand;
pub use learnings::LearningCommand;
pub use main_session::MainSessionCommand;
pub use pr::PrCommand;
pub use project::ProjectCommand;
pub use repo_filter::RepoFilterCommand;
pub use split::SplitCommand;
pub use system::SystemCommand;
pub use task::TaskCommand;
pub use tips::TipsCommand;
