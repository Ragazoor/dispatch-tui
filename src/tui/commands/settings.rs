//! Settings- and preference-persistence side-effect commands.
//!
//! Everything here writes a durable preference the board reloads at startup:
//! the `settings` table, or a repo's most-recently-used path/base-branch
//! history.

/// Wrapped by [`crate::tui::types::Command::Settings`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum SettingsCommand {
    /// Record a repo path into the most-recently-used repo-path history.
    SaveRepoPath(String),
    /// Record a base_branch into a repo's most-recently-used history (see
    /// docs/specs/dispatch.allium: rule RecordBaseBranch). Emitted only from
    /// `finish_task_creation` (the manual "new task" form) — never
    /// quick-dispatch or MCP `create_task`.
    SaveBaseBranch(String, String),
    /// Ask a repository what its own default branch is, so the base-branch
    /// field can stop showing the literal "main" to a repo that does not have
    /// one (dispatch.allium: `DefaultBaseBranchIsDetectedNotAssumed`).
    ///
    /// Emitted only when the chosen repo has no remembered branch history —
    /// history is the user's own previous answer and beats origin/HEAD.
    ///
    /// `replacing` is the prefill the field put in its buffer while this was
    /// in flight. The answer is applied only if the buffer still holds exactly
    /// that, which is what makes a late answer unable to overwrite typing
    /// (`DetectedPrefillNeverOverwritesTyping`).
    DetectDefaultBranch {
        repo_path: String,
        replacing: String,
    },
    /// Persist a boolean setting under `key`.
    PersistSetting { key: String, value: bool },
    /// Persist a string setting under `key`.
    PersistStringSetting { key: String, value: String },
}
