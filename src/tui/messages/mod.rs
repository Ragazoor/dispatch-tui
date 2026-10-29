//! Per-domain message inner enums.
//!
//! Variants of the outer [`crate::tui::types::Message`] enum are progressively
//! migrated into per-domain inner enums to keep the dispatcher manageable as
//! the TUI grows. Each module here owns one domain's messages.

pub mod board_filter;
pub mod budget;
pub mod editor;
pub mod epic;
pub mod feed;
pub mod input;
pub mod pr;
pub mod split;
pub mod system;
pub mod task;
pub mod work_package;

pub use board_filter::BoardFilterMessage;
pub use budget::BudgetMessage;
pub use editor::EditorMessage;
pub use epic::EpicMessage;
pub use feed::FeedMessage;
pub use input::InputMessage;
pub use pr::PrMessage;
pub use split::{EnterFailure, SplitMessage};
pub use system::SystemMessage;
pub use task::TaskMessage;
pub use work_package::WorkPackageMessage;
