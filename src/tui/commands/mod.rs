//! Per-domain command inner enums.
//!
//! Variants of the outer [`crate::tui::types::Command`] enum are progressively
//! migrated into per-domain inner enums. Each module here owns one domain's
//! side-effect commands.

pub mod editor;
pub mod feed;
pub mod learnings;
pub mod pr;
pub mod system;

pub use editor::EditorCommand;
pub use feed::FeedCommand;
pub use learnings::LearningCommand;
pub use pr::PrCommand;
pub use system::SystemCommand;
