mod kanban;
mod palette;
mod review;
mod security;
mod shared;

pub use kanban::render;
pub use shared::{refresh_status, truncate};

#[cfg(test)]
pub(in crate::tui) use kanban::{
    action_hints, column_bg_color, column_color, cursor_bg_color, epic_action_hints,
    task_detail_lines,
};
#[cfg(test)]
pub(in crate::tui) use review::{
    review_action_hints, review_column_bg_color, review_column_color, review_cursor_bg_color,
};
#[cfg(test)]
pub(in crate::tui) use security::{dependabot_action_hints, security_action_hints};
