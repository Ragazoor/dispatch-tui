//! The board's saved filter: the repo selection with its include/exclude mode
//! and the epic hide-list (`board-layout.allium`, "Hidden Epics and Work
//! Packages"). A leaf so the board and the companion-pane handler apply one
//! predicate: the pane lists only the agents this filter shows
//! (`agent-tree.allium: ListsFollowTheBoardFilter`).

use std::collections::HashSet;

use super::{build_children_map, descendant_epic_ids_with_map, Epic, EpicId, Task};

/// Settings keys the board saves its filter under when the overlay closes.
pub const REPO_FILTER_KEY: &str = "repo_filter";
pub const REPO_FILTER_MODE_KEY: &str = "repo_filter_mode";
/// Settings keys the hide-lists are saved under, as JSON arrays of ids.
pub const HIDDEN_EPICS_KEY: &str = "hidden_epics";
pub const HIDDEN_PACKAGES_KEY: &str = "hidden_packages";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepoFilterMode {
    #[default]
    Include,
    Exclude,
}

impl RepoFilterMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RepoFilterMode::Include => "include",
            RepoFilterMode::Exclude => "exclude",
        }
    }
}

impl std::str::FromStr for RepoFilterMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "include" => Ok(RepoFilterMode::Include),
            "exclude" => Ok(RepoFilterMode::Exclude),
            _ => Err(format!("unknown filter mode: {s}")),
        }
    }
}

/// Whether the repo selection admits `repo_path`. An empty selection admits
/// every repo, in either mode.
pub fn repo_filter_admits(repos: &HashSet<String>, mode: RepoFilterMode, repo_path: &str) -> bool {
    if repos.is_empty() {
        return true;
    }
    match mode {
        RepoFilterMode::Include => repos.contains(repo_path),
        RepoFilterMode::Exclude => !repos.contains(repo_path),
    }
}

/// The hidden epics and every descendant of them: what
/// `board-layout.allium: BoardHides.epic_is_hidden` holds for. Free when nothing
/// is hidden, which is the common case on the render hot path.
pub fn hidden_epic_closure(hidden: &HashSet<EpicId>, epics: &[Epic]) -> HashSet<EpicId> {
    if hidden.is_empty() {
        return HashSet::new();
    }
    let children = build_children_map(epics);
    hidden
        .iter()
        .flat_map(|&e| descendant_epic_ids_with_map(e, &children))
        .collect()
}

/// Whether the repo selection and the hidden-epic closure (`hidden`, from
/// [`hidden_epic_closure`]) admit `task`: the one predicate the board's cards
/// and the pane's agent list are both held to. A task with no epic is never
/// hidden by the hide-list.
pub fn task_admitted(
    repos: &HashSet<String>,
    mode: RepoFilterMode,
    hidden: &HashSet<EpicId>,
    task: &Task,
) -> bool {
    repo_filter_admits(repos, mode, &task.repo_path)
        && !task.epic_id.is_some_and(|e| hidden.contains(&e))
}

/// The repo filter and epic hide-list as the board last saved them. An absent
/// or unreadable setting filters nothing, as it does at board startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedBoardFilter {
    pub repos: HashSet<String>,
    pub mode: RepoFilterMode,
    pub hidden_epics: HashSet<EpicId>,
}

impl SavedBoardFilter {
    /// Parse the three saved setting values, each as read from the store.
    pub fn parse(repos: Option<&str>, mode: Option<&str>, hidden_epics: Option<&str>) -> Self {
        SavedBoardFilter {
            repos: repos
                .and_then(|v| serde_json::from_str::<Vec<String>>(v).ok())
                .unwrap_or_default()
                .into_iter()
                .collect(),
            mode: mode.and_then(|v| v.parse().ok()).unwrap_or_default(),
            hidden_epics: hidden_epics
                .and_then(|v| serde_json::from_str::<Vec<i64>>(v).ok())
                .unwrap_or_default()
                .into_iter()
                .map(EpicId)
                .collect(),
        }
    }

    /// Whether the board shows `task` under this filter: its repo is admitted
    /// and its epic is not hidden, on its own or through an ancestor.
    /// `hidden` is [`hidden_epic_closure`] of `self.hidden_epics`.
    pub fn admits(&self, task: &Task, hidden: &HashSet<EpicId>) -> bool {
        task_admitted(&self.repos, self.mode, hidden, task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_settings_filter_nothing() {
        let f = SavedBoardFilter::parse(Some("not json"), Some("sideways"), Some("{"));
        assert_eq!(f, SavedBoardFilter::default());
    }

    #[test]
    fn saved_settings_parse_into_the_filter() {
        let f = SavedBoardFilter::parse(Some(r#"["/a"]"#), Some("exclude"), Some("[3, 5]"));
        assert_eq!(f.repos, HashSet::from(["/a".to_string()]));
        assert_eq!(f.mode, RepoFilterMode::Exclude);
        assert_eq!(f.hidden_epics, HashSet::from([EpicId(3), EpicId(5)]));
    }

    #[test]
    fn an_empty_repo_selection_admits_every_repo_in_either_mode() {
        let none = HashSet::new();
        assert!(repo_filter_admits(&none, RepoFilterMode::Include, "/x"));
        assert!(repo_filter_admits(&none, RepoFilterMode::Exclude, "/x"));
    }
}
