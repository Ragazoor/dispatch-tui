//! Repo-filter overlay side-effect commands.

#[derive(Debug, Clone)]
pub enum RepoFilterCommand {
    DeleteRepoPath(String),
}
