//! Repo-filter overlay messages.

use std::collections::HashSet;

use crate::tui::types::{Command, RepoFilterMode};
use crate::tui::App;

#[derive(Debug, Clone)]
pub enum RepoFilterMessage {
    Start,
    Close,
    Toggle(String),
    ToggleAll,
    ToggleMode,
    MoveCursor(isize),
    StartSavePreset,
    SavePreset(String),
    LoadPreset(String),
    StartDeletePreset,
    DeletePreset(String),
    StartDeleteRepoPath,
    DeleteRepoPath(String),
    CancelPresetInput,
    ToggleOnlyActive,
    PresetsLoaded(Vec<(String, HashSet<String>, RepoFilterMode)>),
}

impl RepoFilterMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            RepoFilterMessage::Start => app.handle_start_repo_filter(),
            RepoFilterMessage::Close => app.handle_close_repo_filter(),
            RepoFilterMessage::Toggle(path) => app.handle_toggle_repo_filter(path),
            RepoFilterMessage::ToggleAll => app.handle_toggle_all_repo_filter(),
            RepoFilterMessage::ToggleMode => app.handle_toggle_repo_filter_mode(),
            RepoFilterMessage::ToggleOnlyActive => app.handle_toggle_only_active(),
            RepoFilterMessage::MoveCursor(delta) => app.handle_move_repo_cursor(delta),
            RepoFilterMessage::StartSavePreset => app.handle_start_save_preset(),
            RepoFilterMessage::SavePreset(name) => app.handle_save_filter_preset(name),
            RepoFilterMessage::LoadPreset(name) => app.handle_load_filter_preset(name),
            RepoFilterMessage::StartDeletePreset => app.handle_start_delete_preset(),
            RepoFilterMessage::DeletePreset(name) => app.handle_delete_filter_preset(name),
            RepoFilterMessage::StartDeleteRepoPath => app.handle_start_delete_repo_path(),
            RepoFilterMessage::DeleteRepoPath(path) => app.handle_delete_repo_path(path),
            RepoFilterMessage::CancelPresetInput => app.handle_cancel_preset_input(),
            RepoFilterMessage::PresetsLoaded(presets) => app.handle_filter_presets_loaded(presets),
        }
    }
}
