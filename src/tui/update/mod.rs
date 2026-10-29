//! Per-domain `Message` handlers, organised by area of concern.
//!
//! `App::update()` (in `crate::tui`) is the single entry point for all
//! `Message` dispatch. The handler bodies live in this module split by
//! domain (PR flow, epics, repo filters, etc.) so each file stays small
//! enough to navigate quickly.

mod agent;
mod board_filter;
mod budget;
pub(in crate::tui) mod epic_input;
mod epics;
mod feeds;
pub(in crate::tui) mod forms;
mod lifecycle;
mod move_task;
mod navigation;
mod people;
mod pr;
mod retry;
pub(in crate::tui) mod selection;
mod split_pane;
mod system;
mod work_packages;
pub(in crate::tui) use work_packages::delete_work_package_prompt;
