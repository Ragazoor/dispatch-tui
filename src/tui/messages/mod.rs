//! Per-domain message inner enums.
//!
//! Variants of the outer [`crate::tui::types::Message`] enum are progressively
//! migrated into per-domain inner enums to keep the dispatcher manageable as
//! the TUI grows. Each module here owns one domain's messages.

pub mod learnings;

pub use learnings::LearningMessage;
