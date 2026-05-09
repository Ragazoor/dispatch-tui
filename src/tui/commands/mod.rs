//! Per-domain command inner enums.
//!
//! Variants of the outer [`crate::tui::types::Command`] enum are progressively
//! migrated into per-domain inner enums. Each module here owns one domain's
//! side-effect commands.

pub mod editor;
pub mod learnings;

pub use editor::EditorCommand;
pub use learnings::LearningCommand;
