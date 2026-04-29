//! Per-domain `Message` handlers, organised by area of concern.
//!
//! `App::update()` (in `crate::tui`) is the single entry point for all
//! `Message` dispatch. The handler bodies live in this module split by
//! domain (PR flow, epics, repo filters, etc.) so each file stays small
//! enough to navigate quickly.

mod agent;
mod epics;
mod feeds;
mod forms;
mod lifecycle;
mod navigation;
mod pr;
mod proposed_learnings;
mod repo_filter;
mod retry;
mod selection;
mod split_pane;
mod system;
mod tips_projects;
mod wrap_up;
