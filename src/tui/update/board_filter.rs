//! Board-filter overlay handlers: the repo filter, the epic and work package
//! hide-lists, and the overlay's tabs (`board-layout.allium`, "Board Filter
//! Overlay Layout").

use std::collections::HashMap;

use super::super::types::*;
use super::super::{filtered_repos, fuzzy_matches_lower, has_new_repo_option, App};
use crate::models::{EpicId, TaskStatus, WorkPackageId};

impl App {
    pub(in crate::tui) fn handle_start_board_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::BoardFilter;
        self.input.repo_cursor = 0;
        self.filter_overlay = FilterOverlayState::default();
        vec![]
    }

    /// The Epics tab's rows: the epic tree in board order, children indented
    /// under their parent. An epic is listed when it qualifies — not done, or
    /// on the hide-list, or every epic once `d` is on; and matching the title
    /// query — or when some descendant qualifies, so a listed epic is never
    /// shown out of its place in the tree.
    pub(in crate::tui) fn filter_epic_rows(&self) -> Vec<FilterEpicRow> {
        let epics = &self.board.epics;
        let overlay = &self.filter_overlay;
        let query = overlay.epic_query.to_lowercase();
        let hidden = &self.filter.hidden_epics;
        let known: std::collections::HashSet<EpicId> = epics.iter().map(|e| e.id).collect();
        let mut children: HashMap<Option<EpicId>, Vec<usize>> = HashMap::new();
        for (i, e) in epics.iter().enumerate() {
            // A parent the board does not hold re-roots the child.
            let parent = e.parent_epic_id.filter(|p| known.contains(p));
            children.entry(parent).or_default().push(i);
        }
        let qualifies = |i: usize| {
            let e = &epics[i];
            (overlay.show_done_epics || e.status != TaskStatus::Done || hidden.contains(&e.id))
                && fuzzy_matches_lower(&e.title, &query)
        };

        // Post-order: a node is listed when it or a descendant qualifies.
        fn walk(
            node: Option<EpicId>,
            depth: usize,
            parent_hidden: bool,
            ctx: &mut WalkCtx<'_>,
        ) -> Vec<FilterEpicRow> {
            let mut out = vec![];
            let Some(kids) = ctx.children.get(&node).cloned() else {
                return out;
            };
            // A cycle written by a bad reparent must not hang the overlay.
            if depth > ctx.epics.len() {
                return out;
            }
            for i in kids {
                let e = &ctx.epics[i];
                let own_hidden = ctx.hidden.contains(&e.id);
                let mark = if parent_hidden {
                    FilterEpicMark::HiddenByParent
                } else if own_hidden {
                    FilterEpicMark::Hidden
                } else {
                    FilterEpicMark::Shown
                };
                let below = walk(Some(e.id), depth + 1, parent_hidden || own_hidden, ctx);
                if (ctx.qualifies)(i) || !below.is_empty() {
                    out.push(FilterEpicRow {
                        id: e.id,
                        depth,
                        title: e.title.clone(),
                        mark,
                    });
                    out.extend(below);
                }
            }
            out
        }
        struct WalkCtx<'a> {
            epics: &'a [crate::models::Epic],
            children: &'a HashMap<Option<EpicId>, Vec<usize>>,
            hidden: &'a std::collections::HashSet<EpicId>,
            qualifies: &'a dyn Fn(usize) -> bool,
        }
        walk(
            None,
            0,
            false,
            &mut WalkCtx {
                epics,
                children: &children,
                hidden,
                qualifies: &qualifies,
            },
        )
    }

    /// The number of rows on `tab`'s list.
    pub(in crate::tui) fn filter_tab_len(&self, tab: FilterTab) -> usize {
        match tab {
            FilterTab::Repos => self.board.repo_paths.len(),
            FilterTab::Epics => self.filter_epic_rows().len(),
            FilterTab::Packages => self.board.work_packages.len(),
        }
    }

    pub(in crate::tui) fn handle_move_filter_cursor(&mut self, delta: isize) -> Vec<Command> {
        let tab = self.filter_overlay.tab;
        if tab == FilterTab::Repos {
            return self.handle_move_repo_cursor(delta);
        }
        let count = self.filter_tab_len(tab);
        if count == 0 {
            return vec![];
        }
        let cursor = match tab {
            FilterTab::Epics => &mut self.filter_overlay.epic_cursor,
            _ => &mut self.filter_overlay.package_cursor,
        };
        *cursor = (*cursor as isize + delta).rem_euclid(count as isize) as usize;
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_switch_filter_tab(
        &mut self,
        direction: MoveDirection,
    ) -> Vec<Command> {
        let tab = self.filter_overlay.tab;
        self.filter_overlay.tab = match direction {
            MoveDirection::Forward => tab.next(),
            MoveDirection::Backward => tab.previous(),
        };
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_toggle_hidden_epic(&mut self, id: EpicId) -> Vec<Command> {
        if !self.filter.hidden_epics.remove(&id) {
            self.filter.hidden_epics.insert(id);
        }
        // Showing a hidden done epic again drops its row unless `d` is on.
        self.clamp_epic_cursor();
        self.after_filter_change()
    }

    pub(in crate::tui) fn handle_toggle_hidden_package(
        &mut self,
        id: WorkPackageId,
    ) -> Vec<Command> {
        if !self.filter.hidden_packages.remove(&id) {
            self.filter.hidden_packages.insert(id);
        }
        self.after_filter_change()
    }

    pub(in crate::tui) fn handle_toggle_show_done_epics(&mut self) -> Vec<Command> {
        self.filter_overlay.show_done_epics = !self.filter_overlay.show_done_epics;
        self.clamp_epic_cursor();
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_start_epic_search(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::BoardFilterSearch;
        self.dirty = true;
        vec![]
    }

    /// Append `Some(c)` to the epic query, or drop its last character on `None`.
    pub(in crate::tui) fn handle_epic_query_edit(&mut self, c: Option<char>) -> Vec<Command> {
        match c {
            Some(c) => self.filter_overlay.epic_query.push(c),
            None => {
                self.filter_overlay.epic_query.pop();
            }
        }
        self.clamp_epic_cursor();
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_end_epic_search(&mut self, keep: bool) -> Vec<Command> {
        if !keep {
            self.filter_overlay.epic_query.clear();
        }
        self.input.mode = InputMode::BoardFilter;
        self.clamp_epic_cursor();
        self.dirty = true;
        vec![]
    }

    /// Esc on the list: a standing epic query is cleared first; with none, the
    /// overlay closes.
    pub(in crate::tui) fn handle_board_filter_back(&mut self) -> Vec<Command> {
        if self.filter_overlay.tab == FilterTab::Epics && !self.filter_overlay.epic_query.is_empty()
        {
            return self.handle_end_epic_search(false);
        }
        self.handle_close_board_filter()
    }

    /// Climb out of every epic or package view the user has just hidden, so
    /// the board never sits inside work it no longer shows.
    fn leave_hidden_views(&mut self) {
        let hidden =
            super::super::hidden_epic_closure(&self.filter.hidden_epics, &self.board.epics);
        loop {
            let inside_hidden = match &self.board.view_mode {
                ViewMode::Epic { epic_id, .. } => hidden.contains(epic_id),
                ViewMode::Package { package_id, .. } => {
                    self.filter.hidden_packages.contains(package_id)
                }
                _ => false,
            };
            if !inside_hidden {
                break;
            }
            self.handle_exit_epic();
        }
    }

    fn clamp_epic_cursor(&mut self) {
        let len = self.filter_epic_rows().len();
        let cursor = &mut self.filter_overlay.epic_cursor;
        *cursor = (*cursor).min(len.saturating_sub(1));
    }

    /// What every board-filter change does to the board behind the overlay.
    fn after_filter_change(&mut self) -> Vec<Command> {
        self.sync_board_selection();
        self.reset_column_scroll();
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_move_repo_cursor(&mut self, delta: isize) -> Vec<Command> {
        let count = if let Some(candidates) = self.picker_candidates() {
            let filtered = filtered_repos(candidates, &self.input.buffer);
            let extra = has_new_repo_option(&self.input.buffer, &filtered) as usize;
            filtered.len() + extra
        } else {
            self.board.repo_paths.len()
        };
        if count == 0 {
            return vec![];
        }
        self.input.repo_cursor =
            (self.input.repo_cursor as isize + delta).rem_euclid(count as isize) as usize;
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_close_board_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.leave_hidden_views();
        self.sync_board_selection();
        self.reset_column_scroll();
        let mut paths: Vec<_> = self.filter.repos.iter().cloned().collect();
        paths.sort();
        let value = serde_json::to_string(&paths).unwrap_or_else(|_| "[]".to_string());
        let mode_value = self.filter.mode.as_str();
        let hidden_epics = sorted_ids_json(self.filter.hidden_epics.iter().map(|e| e.0));
        let hidden_packages = sorted_ids_json(self.filter.hidden_packages.iter().map(|p| p.0));
        vec![
            Command::Settings(
                crate::tui::commands::SettingsCommand::PersistStringSetting {
                    key: crate::tui::HIDDEN_EPICS_KEY.to_string(),
                    value: hidden_epics,
                },
            ),
            Command::Settings(
                crate::tui::commands::SettingsCommand::PersistStringSetting {
                    key: crate::tui::HIDDEN_PACKAGES_KEY.to_string(),
                    value: hidden_packages,
                },
            ),
            Command::Settings(
                crate::tui::commands::SettingsCommand::PersistStringSetting {
                    key: "repo_filter".to_string(),
                    value,
                },
            ),
            Command::Settings(
                crate::tui::commands::SettingsCommand::PersistStringSetting {
                    key: "repo_filter_mode".to_string(),
                    value: mode_value.to_string(),
                },
            ),
        ]
    }

    pub(in crate::tui) fn handle_toggle_repo_filter(&mut self, path: String) -> Vec<Command> {
        if self.filter.repos.contains(&path) {
            self.filter.repos.remove(&path);
        } else {
            self.filter.repos.insert(path);
        }
        self.sync_board_selection();
        self.reset_column_scroll();
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_toggle_repo_filter_mode(&mut self) -> Vec<Command> {
        self.filter.mode = match self.filter.mode {
            RepoFilterMode::Include => RepoFilterMode::Exclude,
            RepoFilterMode::Exclude => RepoFilterMode::Include,
        };
        self.sync_board_selection();
        self.reset_column_scroll();
        self.dirty = true;
        vec![]
    }

    pub(in crate::tui) fn handle_start_delete_repo_path(&mut self) -> Vec<Command> {
        if self.board.repo_paths.is_empty() {
            return vec![];
        }
        self.input.mode = InputMode::ConfirmDeleteRepoPath;
        vec![]
    }

    pub(in crate::tui) fn handle_delete_repo_path(&mut self, path: String) -> Vec<Command> {
        self.filter.repos.remove(&path);
        self.input.mode = InputMode::BoardFilter;
        self.status.set("Deleted repo path".to_string());
        vec![Command::RepoFilter(
            crate::tui::commands::RepoFilterCommand::DeleteRepoPath(path),
        )]
    }
}

/// A hide-list as its saved setting: a JSON array, sorted so an unchanged set
/// always saves the same string.
fn sorted_ids_json(ids: impl Iterator<Item = i64>) -> String {
    let mut ids: Vec<i64> = ids.collect();
    ids.sort_unstable();
    serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_string())
}
