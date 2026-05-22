//! Repo-filter overlay side-effect commands.

use crate::tui::types::RepoFilterMode;

#[derive(Debug, Clone)]
pub enum RepoFilterCommand {
    PersistFilterPreset {
        name: String,
        repo_paths: Vec<String>,
        mode: RepoFilterMode,
    },
    DeleteFilterPreset(String),
    DeleteRepoPath(String),
}
