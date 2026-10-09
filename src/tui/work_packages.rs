//! Work packages on the board (`docs/specs/work-packages.allium`): where a
//! package's card is placed on the main board, and which cards a package view
//! holds.
//!
//! Computed uncached on every call. A package placement reads the links, the
//! epic tree and the board filters; folding all of that into the layout-cache
//! fingerprint would buy little, since a board holds a handful of packages and
//! the walk is bounded by the tasks they reach.

use std::collections::{HashMap, HashSet};

use super::columns::BoardView;
use super::types::*;
use crate::models::{
    EpicId, Task, TaskId, TaskStatus, WorkPackage, WorkPackageId, WorkPackageItem,
};

/// Where each visible package's card is drawn, keyed by package id. A package
/// absent from the map has no card at all (hidden by the search query).
pub(in crate::tui) type PackagePlacementMap = HashMap<WorkPackageId, EpicPlacement>;

impl<'a> BoardView<'a> {
    /// The items `package` links to, in link order.
    pub(in crate::tui) fn package_items(
        self,
        package: WorkPackageId,
    ) -> impl Iterator<Item = WorkPackageItem> + 'a {
        self.board
            .work_package_links
            .iter()
            .filter(move |l| l.work_package_id == package)
            .map(|l| l.item)
    }

    /// The tasks a package's linked tasks name.
    pub(in crate::tui) fn package_task_ids(self, package: WorkPackageId) -> HashSet<TaskId> {
        self.package_items(package)
            .filter_map(|item| match item {
                WorkPackageItem::Task(id) => Some(id),
                WorkPackageItem::Epic(_) => None,
            })
            .collect()
    }

    /// The epics a package's linked epics name.
    pub(in crate::tui) fn package_epic_ids(self, package: WorkPackageId) -> HashSet<EpicId> {
        self.package_items(package)
            .filter_map(|item| match item {
                WorkPackageItem::Epic(id) => Some(id),
                WorkPackageItem::Task(_) => None,
            })
            .collect()
    }

    /// The tasks that place `package`'s card: its linked tasks plus every task
    /// in the subtree of each linked epic, each counted once
    /// (`PackageCardPlacement`'s `placement_tasks`). Unfiltered.
    fn package_placement_tasks(self, package: WorkPackageId) -> Vec<&'a Task> {
        let mut ids = self.package_task_ids(package);
        for epic in self.package_epic_ids(package) {
            ids.extend(crate::models::descendant_task_ids(
                epic,
                &self.board.epics,
                &self.board.tasks,
            ));
        }
        self.board
            .tasks
            .iter()
            .filter(|t| ids.contains(&t.id))
            .collect()
    }

    /// Where each package card sits on the main board (`PackageCardPlacement`).
    ///
    /// A column shows the card when a visible placement task holds its status;
    /// a package with none is drawn in Backlog. Under a search query a package
    /// keeps its card only when its title matches or some placement task is
    /// visible — the twin of board_search_filter's own-match clause for epics.
    pub(in crate::tui) fn compute_package_placements(self) -> PackagePlacementMap {
        let filters = self.board_filters();
        let searching = !self.search.query.is_empty();
        let mut map = PackagePlacementMap::new();
        for package in &self.board.work_packages {
            let mut placement = EpicPlacement::default();
            let mut any_visible = false;
            for task in self.package_placement_tasks(package.id) {
                if filters.admits(task) {
                    placement.record(task);
                    any_visible = true;
                }
            }
            let title_matches = package.title.to_lowercase().contains(&filters.query_lower);
            if searching && !title_matches && !any_visible {
                continue;
            }
            placement.apply_empty_fallback();
            map.insert(package.id, placement);
        }
        map
    }

    /// The package cards `status`'s column draws, oldest package first. Empty
    /// outside the main board (`PackageCardsOnlyOnMainBoard`).
    pub(in crate::tui) fn package_cards_for_status(
        self,
        status: TaskStatus,
    ) -> Vec<&'a WorkPackage> {
        if !matches!(self.effective_view_mode(), BoardViewMode::Board(_)) {
            return Vec::new();
        }
        let placements = self.compute_package_placements();
        let mut cards: Vec<&'a WorkPackage> = self
            .board
            .work_packages
            .iter()
            .filter(|p| {
                placements
                    .get(&p.id)
                    .is_some_and(|pl| pl.appears_in(status))
            })
            .collect();
        cards.sort_by_key(|p| p.id);
        cards
    }

    /// How many of `package`'s linked items are done, out of how many the
    /// board holds: the summary a package card shows.
    pub(in crate::tui) fn package_progress(self, package: WorkPackageId) -> (usize, usize) {
        let mut total = 0;
        let mut done = 0;
        for item in self.package_items(package) {
            let status = match item {
                WorkPackageItem::Task(id) => self.board.find_task(id).map(|t| t.status),
                WorkPackageItem::Epic(id) => self.board.find_epic(id).map(|e| e.status),
            };
            if let Some(status) = status {
                total += 1;
                if status == TaskStatus::Done {
                    done += 1;
                }
            }
        }
        (done, total)
    }
}
