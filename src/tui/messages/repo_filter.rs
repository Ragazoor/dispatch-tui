//! Repo-filter overlay messages.

use std::collections::HashSet;

use crate::tui::types::RepoFilterMode;

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
