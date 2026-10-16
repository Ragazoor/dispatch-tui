use super::*;
use crate::models::{test_tmux_window, EpicId, SubStatus, TaskId, TaskStatus};
use crate::palette::{FG, MUTED, RED, YELLOW};
use crate::tui::commands::SettingsCommand;
use crossterm::event::KeyCode;
use std::time::{Duration, Instant};

mod keys_and_selection;
mod moves;
mod notifications;
mod quick_dispatch;
mod refresh_status;
mod review_and_detach;
mod selection_and_reorder;
