//! Confirmation overlays and popup helpers (error, help, repo filter, task detail).

mod board_filter;
mod error;
mod help;
mod reparent_epic;
mod task_detail;
mod work_package;

pub(super) use board_filter::render_board_filter_overlay;
pub(super) use error::render_error_popup;
pub(super) use help::render_help_overlay;
pub(in crate::tui) use reparent_epic::build_reparent_tree;
pub(super) use reparent_epic::{render_move_task_overlay, render_reparent_epic_overlay};
pub(super) use task_detail::render_task_detail_overlay;
pub(super) use work_package::{
    render_work_package_picker_overlay, render_work_package_rename_overlay,
};
