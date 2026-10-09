//! Work packages on the board (`docs/specs/work-packages.allium`): where a
//! package's card is placed on the main board, and which cards a package view
//! holds.
//!
//! Package placement is computed at most once per pass over the board: the
//! pass's [`EpicSearchPass::package_placements`](super::search::EpicSearchPass::package_placements) cell builds it on first use
//! and every column in that pass reads the same map, so a frame pays for one
//! walk rather than one per column (and again per count). It is not kept in
//! the layout cache: a placement reads the links, the epic tree and the board
//! filters, and folding all of them into the layout-cache fingerprint would
//! buy one walk per frame at the cost of a coherence obligation.

use std::collections::{HashMap, HashSet};

use super::columns::BoardView;
use super::types::*;
use crate::models::{EpicId, TaskId, TaskStatus, WorkPackage, WorkPackageId, WorkPackageItem};

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

    /// The task ids and the epic ids `package` links to, split in one pass.
    pub(in crate::tui) fn package_item_ids(
        self,
        package: WorkPackageId,
    ) -> (HashSet<TaskId>, HashSet<EpicId>) {
        let mut tasks = HashSet::new();
        let mut epics = HashSet::new();
        for item in self.package_items(package) {
            match item {
                WorkPackageItem::Task(id) => {
                    tasks.insert(id);
                }
                WorkPackageItem::Epic(id) => {
                    epics.insert(id);
                }
            }
        }
        (tasks, epics)
    }

    /// Where each package card sits on the main board (`PackageCardPlacement`).
    /// Empty outside the main board (`PackageCardsOnlyOnMainBoard`).
    ///
    /// A column shows the card when a visible placement task — a linked task,
    /// or any task in the subtree of a linked epic, each counted once — holds
    /// its status; a package with none is drawn in Backlog. Under a search
    /// query a package keeps its card only when its title matches or some
    /// placement task is visible — the twin of board_search_filter's own-match
    /// clause for epics.
    ///
    /// Builds the epic children map once for every package, then makes one
    /// task scan per package. Callers inside a pass read it through
    /// [`EpicSearchPass::package_placements`](super::search::EpicSearchPass::package_placements) rather than calling this.
    pub(in crate::tui) fn compute_package_placements(self) -> PackagePlacementMap {
        let mut map = PackagePlacementMap::new();
        if !matches!(self.effective_view_mode(), BoardViewMode::Board(_))
            || self.board.work_packages.is_empty()
        {
            return map;
        }
        let filters = self.board_filters();
        let searching = !self.search.query.is_empty();
        let children = crate::models::build_children_map(&self.board.epics);
        for package in &self.board.work_packages {
            let (task_ids, linked_epics) = self.package_item_ids(package.id);
            let subtree: HashSet<EpicId> = linked_epics
                .into_iter()
                .flat_map(|e| crate::models::descendant_epic_ids_with_map(e, &children))
                .collect();
            let mut placement = EpicPlacement::default();
            let mut any_visible = false;
            for task in &self.board.tasks {
                let placed = task_ids.contains(&task.id)
                    || task.epic_id.is_some_and(|e| subtree.contains(&e));
                if placed && filters.admits(task) {
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

    /// The package cards `status`'s column draws, oldest package first (the
    /// board holds them in id order).
    pub(in crate::tui) fn package_cards_for_status<'p>(
        self,
        status: TaskStatus,
        placements: &'p PackagePlacementMap,
    ) -> impl Iterator<Item = &'a WorkPackage> + 'p
    where
        'a: 'p,
    {
        self.board.work_packages.iter().filter(move |p| {
            placements
                .get(&p.id)
                .is_some_and(|pl| pl.appears_in(status))
        })
    }

    /// How many of `package`'s linked items are done, out of how many the
    /// board holds: the summary a package card shows. One scan of the tasks
    /// and one of the epics, not a lookup per item.
    pub(in crate::tui) fn package_progress(self, package: WorkPackageId) -> (usize, usize) {
        let (task_ids, epic_ids) = self.package_item_ids(package);
        let statuses = self
            .board
            .tasks
            .iter()
            .filter(|t| task_ids.contains(&t.id))
            .map(|t| t.status)
            .chain(
                self.board
                    .epics
                    .iter()
                    .filter(|e| epic_ids.contains(&e.id))
                    .map(|e| e.status),
            );
        statuses.fold((0, 0), |(done, total), status| {
            (done + usize::from(status == TaskStatus::Done), total + 1)
        })
    }
}
