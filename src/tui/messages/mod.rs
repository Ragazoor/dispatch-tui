//! Per-domain message inner enums.
//!
//! Variants of the outer [`crate::tui::types::Message`] enum are progressively
//! migrated into per-domain inner enums to keep the dispatcher manageable as
//! the TUI grows. Each module here owns one domain's messages.

pub mod editor;
pub mod feed;
pub mod learnings;
pub mod wrap_up;

pub use editor::EditorMessage;
pub use feed::FeedMessage;
pub use learnings::LearningMessage;
pub use wrap_up::WrapUpMessage;
