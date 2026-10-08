//! Column layout: which cards each column shows, and the layout cache.

use std::sync::Arc;

use super::*;

impl App {
    /// Whether flattened mode applies to `status`. The exempt columns live on
    /// [`TaskStatus::UNFLATTENED`], so this is only the mode half of the
    /// question; nothing here restates which columns those are.
    pub(in crate::tui) fn is_flattened_for_status(&self, status: TaskStatus) -> bool {
        self.board.flattened && !status.is_unflattened()
    }

    /// Whether the main board shows `task` as a card of its own, rather than
    /// folded inside its epic's card. The one owner of that rule: the board
    /// view filter admits exactly these, and callers that need to reach a
    /// hidden task (by entering its epic) negate it.
    pub(in crate::tui) fn shown_on_main_board(&self, task: &Task) -> bool {
        self.is_flattened_for_status(task.status) || task.epic_id.is_none()
    }

    /// The warm placement map, or `None` when the cache is cold **or stale**.
    ///
    /// `cached_epic_stats()` populates it, but that takes `&mut self` and most
    /// readers here have only `&self`, so a miss falls back to computing rather
    /// than filling the cache. In practice the render pass warms it at the top
    /// of every frame, so the fallback is the exception.
    ///
    /// The fingerprint check is not optional here, which is why this is not a
    /// bare field read. `cached_epic_stats()` is where the cache normally
    /// self-heals, and a `&self` reader cannot call it — so without this check a
    /// caller would be served a map built before the last board or filter
    /// change. Stale *stats* only misorder a column; stale *placement* decides
    /// which columns a card appears in at all, so it makes cards vanish.
    pub(in crate::tui) fn cached_placements(&self) -> Option<Arc<EpicPlacementMap>> {
        if self.layout.layout_cache_fingerprint != Some(self.compute_layout_fingerprint()) {
            return None;
        }
        self.layout.epic_placements_cache.clone()
    }

    /// Borrow the caller's map, or compute one. The `Cow` is what lets the two
    /// cases share a single expression: a caller that has the map pays nothing,
    /// and one that does not still gets an answer rather than silently
    /// disagreeing with the board about where a card is.
    pub(in crate::tui) fn placements_or_compute<'m>(
        &self,
        placements: Option<&'m EpicPlacementMap>,
    ) -> std::borrow::Cow<'m, EpicPlacementMap> {
        match placements {
            Some(p) => std::borrow::Cow::Borrowed(p),
            None => std::borrow::Cow::Owned(self.compute_epic_placements()),
        }
    }

    /// The board-wide filters, resolved against the current query and filter
    /// state. Build one per pass and share it; see [`BoardFilters`].
    pub(in crate::tui) fn board_filters(&self) -> BoardFilters<'_> {
        BoardFilters::new(&self.filter, &self.search.query)
    }

    /// Return tasks visible in the current view.
    /// Board view: standalone tasks only (epic_id is None).
    /// Epic view: only subtasks of the active epic.
    pub fn tasks_for_current_view(&self) -> Vec<&Task> {
        // Built once per call, not per task: this is the render hot path.
        let filters = self.board_filters();
        match self.effective_view_mode() {
            BoardViewMode::Board(_) => self
                .board
                .tasks
                .iter()
                .filter(|t| self.shown_on_main_board(t))
                .filter(|t| filters.admits(t))
                .collect(),
            BoardViewMode::Epic { epic_id, .. } => {
                let current = epic_id;
                // Only a flattened column reaches past the epic's own tasks, so
                // the subtree is worth walking only when some column will use
                // it. With flattening off, every column takes the else arm and
                // this stays `None`.
                let subtree = self.board.flattened.then(|| {
                    crate::models::descendant_task_ids(
                        current,
                        &self.board.epics,
                        &self.board.tasks,
                    )
                });
                self.board
                    .tasks
                    .iter()
                    .filter(|t| {
                        if self.is_flattened_for_status(t.status) {
                            subtree.as_ref().is_some_and(|s| s.contains(&t.id))
                        } else {
                            t.epic_id == Some(current)
                        }
                    })
                    .filter(|t| filters.admits(t))
                    .collect()
            }
        }
    }

    /// Where every epic's card is drawn, keyed by epic id.
    ///
    /// An epic card appears in every column where the epic's subtree holds a
    /// *visible* task of that status, so one epic can hold four cards at once
    /// (`board-layout.allium`, "Epic Card Placement"). Visible means the task
    /// survives the same three predicates `tasks_for_current_view` applies —
    /// the repo filter, the only-active filter and the search query. A task
    /// the board is hiding cannot place its ancestor's card: entering it
    /// would be a dead end.
    ///
    /// Walks `board.tasks` once and credits each admitted task to every epic on
    /// its ancestor chain, so the cost is O(tasks × depth) rather than
    /// O(epics × tasks).
    ///
    /// Cached alongside the rest of the layout cache — see
    /// [`Self::cached_epic_stats`]. Placement moves with the search query and
    /// the filters as well as with the board, which is why
    /// `compute_layout_fingerprint` folds those in.
    pub(in crate::tui) fn compute_epic_placements(&self) -> EpicPlacementMap {
        let filters = self.board_filters();
        let parent_of: HashMap<EpicId, Option<EpicId>> = self
            .board
            .epics
            .iter()
            .map(|e| (e.id, e.parent_epic_id))
            .collect();

        let mut placements: EpicPlacementMap = self
            .board
            .epics
            .iter()
            .map(|e| (e.id, EpicPlacement::default()))
            .collect();

        // A malformed parent chain (a cycle written by a bad reparent) must not
        // hang the render thread, so the walk is bounded: no chain can pass
        // through more epics than the board holds without revisiting one.
        let max_depth = self.board.epics.len();

        for task in &self.board.tasks {
            if !filters.admits(task) {
                continue;
            }
            // Credit the owning epic and every ancestor: a parent whose work
            // all sits one level down still earns a card in that column.
            let mut next = task.epic_id;
            for _ in 0..max_depth {
                let Some(id) = next else { break };
                match placements.get_mut(&id) {
                    Some(p) => p.record(task),
                    // An epic_id pointing at no board epic (an orphan task):
                    // nothing to credit, and no chain to keep walking.
                    None => break,
                }
                next = parent_of.get(&id).copied().flatten();
            }
        }

        // One pass to settle each epic's columns, so every placement names its
        // own outright rather than each reader re-deriving them.
        //
        // An epic with no admitted task anywhere is drawn in Backlog, so it
        // stays reachable (`board-layout.allium`, "Epic Card Placement").
        for epic in &self.board.epics {
            let Some(placement) = placements.get_mut(&epic.id) else {
                continue;
            };
            placement.apply_empty_fallback();
        }

        placements
    }

    /// Return tasks for a given status in the current view.
    #[cfg(test)]
    pub fn tasks_by_status(&self, status: TaskStatus) -> Vec<&Task> {
        self.tasks_for_current_view()
            .into_iter()
            .filter(|t| t.status == status)
            .collect()
    }

    /// Pre-compute subtask stats for all epics using a pre-built children map.
    /// The `children_map` argument avoids rebuilding the adjacency map per epic.
    pub(in crate::tui) fn compute_epic_stats_with_map(
        &self,
        children_map: &HashMap<EpicId, Vec<EpicId>>,
    ) -> EpicStatsMap {
        self.board
            .epics
            .iter()
            .map(|e| {
                (
                    e.id,
                    SubtaskStats::for_epic(e, &self.board.tasks, children_map),
                )
            })
            .collect()
    }

    /// Pre-compute subtask stats for all epics. Call once per render frame.
    pub fn compute_epic_stats(&self) -> EpicStatsMap {
        // Build the parent→children map once so each for_epic call is O(depth)
        // rather than O(epics) — total cost goes from O(epics²) to O(epics).
        let children_map = crate::models::build_children_map(&self.board.epics);
        self.compute_epic_stats_with_map(&children_map)
    }

    /// Return an `Arc`-wrapped `EpicStatsMap`, computing and caching on first call.
    ///
    /// Cloning the returned `Arc` is O(1) (atomic ref-count); the underlying
    /// `HashMap` is not copied.  Also populates `children_map_cache`,
    /// `column_anchor_cache`, and `epic_filter_cache` on first call so that
    /// rendering and navigation handlers can do O(1) lookups without re-scanning.
    ///
    /// Call `invalidate_layout_cache()` whenever `board.tasks` or `board.epics`
    /// are mutated to force a fresh computation on the next call. This is an
    /// optimization, not a correctness requirement: this method compares a
    /// fingerprint of the current board against the one captured when the
    /// cache was last populated, and self-heals (rebuilds) on mismatch even
    /// if invalidation was never called. See `compute_layout_fingerprint()`.
    pub(in crate::tui) fn cached_epic_stats(&mut self) -> Arc<EpicStatsMap> {
        let fingerprint = self.compute_layout_fingerprint();
        if self.layout.epic_stats_cache.is_some()
            && self.layout.layout_cache_fingerprint != Some(fingerprint)
        {
            self.invalidate_layout_cache();
        }
        if let Some(arc) = &self.layout.epic_stats_cache {
            return Arc::clone(arc);
        }
        // Build the children map once; store it so callers can reuse it.
        let children_map = crate::models::build_children_map(&self.board.epics);
        let stats = Arc::new(self.compute_epic_stats_with_map(&children_map));

        // Build epic_filter_cache: (epic_repo_matches, epic_matches) per epic,
        // using the already-built children_map so descendant traversal is O(1) per epic.
        // Computed before children_map is moved into children_map_cache.
        let filter_cache: HashMap<EpicId, (bool, bool)> = {
            let tasks = &self.board.tasks;
            let filter = &self.filter;
            self.board
                .epics
                .iter()
                .map(|e| {
                    let epic_ids = crate::models::descendant_epic_ids_with_map(e.id, &children_map);
                    let repo_matches = epic_repo_matches_for_ids(tasks, filter, &epic_ids);
                    let active_matches = if !filter.only_active {
                        true
                    } else {
                        epic_active_matches_for_ids(tasks, &epic_ids)
                    };
                    (e.id, (repo_matches, active_matches))
                })
                .collect()
        };
        self.layout.epic_filter_cache = Some(filter_cache);
        self.layout.children_map_cache = Some(children_map);

        // Build column_anchor_cache: sorted selectable items per status.
        // Hoist tasks_for_current_view() and the search pass out of the loop
        // so each is computed once, not once per status.
        let view_tasks = self.tasks_for_current_view();
        let pass = self.epic_search_pass();
        let placements = self.compute_epic_placements();
        let mut anchor_cache: HashMap<TaskStatus, Vec<ColumnAnchor>> = HashMap::new();
        for &status in TaskStatus::ALL.iter() {
            let anchors: Vec<ColumnAnchor> = self
                .column_items_for_status_with_view_tasks(
                    status,
                    Some(&placements),
                    &view_tasks,
                    &pass,
                )
                .into_iter()
                .filter_map(|item| item.anchor())
                .collect();
            anchor_cache.insert(status, anchors);
        }
        self.layout.column_anchor_cache = Some(anchor_cache);

        self.layout.epic_placements_cache = Some(Arc::new(placements));
        self.layout.epic_stats_cache = Some(Arc::clone(&stats));
        self.layout.layout_cache_fingerprint = Some(fingerprint);
        stats
    }

    /// Fingerprint of the board state feeding `epic_stats_cache`,
    /// `children_map_cache`, `column_anchor_cache`, and `epic_filter_cache`:
    /// from `board.tasks`/`board.epics`, each task/epic id, status, epic
    /// membership (`epic_id`/`parent_epic_id`) and `sort_order`; plus the
    /// folded-section set, which decides which cards a column renders at all.
    /// A change to any of those forces a rebuild regardless of whether
    /// `invalidate_layout_cache()` was called.
    ///
    /// **A partial guarantee, not a total one.** The repo filter, the
    /// only-active filter and the search query also feed those caches, through
    /// `tasks_for_current_view`, and none of them is fingerprinted — they rely
    /// on their handlers calling `sync_board_selection()`, which every one of
    /// them does. Do not read this as "any input change self-heals": only the
    /// listed ones do.
    ///
    /// The folded set is fingerprinted rather than left to its handler because
    /// a fold also arrives from the startup restore, which runs nowhere near
    /// the selection machinery.
    ///
    /// Deliberately cheaper than a full rebuild (no allocation, no sorting,
    /// no `HashMap`s, and no cryptographic hashing — a plain FNV-1a fold is
    /// plenty for a non-adversarial in-memory fingerprint) so
    /// `cached_epic_stats()` can call it unconditionally on every
    /// invocation, including the cache-hit fast path.
    pub(in crate::tui) fn compute_layout_fingerprint(&self) -> u64 {
        let mut acc = fnv_seed();
        acc = fnv_fold(acc, self.board.tasks.len() as u64);
        for t in &self.board.tasks {
            acc = fnv_fold(acc, t.id.0 as u64);
            acc = fnv_fold(acc, t.status as u64);
            acc = fnv_fold(acc, t.epic_id.map_or(u64::MAX, |e| e.0 as u64));
            acc = fnv_fold(acc, t.sort_order.map_or(u64::MAX, |s| s as u64));
            // The Done column's ordering key, and the input
            // `EpicPlacement::newest_completion` is folded from — so a
            // completion the cache has not seen must read as a change.
            acc = fnv_fold(
                acc,
                t.completed_at
                    .map_or(u64::MAX, |at| at.timestamp_millis() as u64),
            );
        }
        acc = fnv_fold(acc, self.board.epics.len() as u64);
        for e in &self.board.epics {
            acc = fnv_fold(acc, e.id.0 as u64);
            acc = fnv_fold(acc, e.status as u64);
            acc = fnv_fold(acc, e.parent_epic_id.map_or(u64::MAX, |p| p.0 as u64));
            acc = fnv_fold(acc, e.sort_order.map_or(u64::MAX, |s| s as u64));
            // An epic's own completion outranks the derived subtask key in
            // Done (`EpicPlacement::sort_key`), so it moves the card too.
            acc = fnv_fold(
                acc,
                e.completed_at
                    .map_or(u64::MAX, |at| at.timestamp_millis() as u64),
            );
        }
        // Folded sections and folded epic groups are cached-view inputs that
        // are not board data. Without them the "same fingerprint means same
        // derived view" guarantee would stop holding the moment either folds.
        let acc = self.folds.fold_into_fingerprint(acc);
        let acc = self.epic_folds.fold_into_fingerprint(acc);

        // The three board-wide filters (see `BoardFilters`). `epic_filter_cache`
        // and `epic_placements_cache` are both derived through them, so a
        // fingerprint blind to them would let a filter change serve a stale
        // board — the one hazard this fingerprint exists to catch.
        let mut acc = fnv_fold(acc, self.filter.only_active as u64);
        acc = fnv_fold(acc, self.filter.mode as u64);
        acc = fnv_fold(acc, self.filter.repos.len() as u64);
        // Each repo is hashed on its own and the results combined with XOR, not
        // folded in sequence: `repos` is a `HashSet`, so a set rebuilt with the
        // same contents can iterate in a different order. A sequential fold
        // would read that as a change and throw the cache away for nothing.
        let mut repos = 0u64;
        for repo in &self.filter.repos {
            repos ^= fnv_bytes(repo.as_bytes());
        }
        acc = fnv_fold(acc, repos);
        fnv_fold(acc, fnv_bytes(self.search.query.as_bytes()))
    }

    /// Fingerprint of `board.tasks` id/position only, used to self-heal
    /// `task_index` in `find_task_mut`. Cheaper than
    /// `compute_layout_fingerprint()` (no epics, no status/sort_order) since
    /// `task_index` only maps id → Vec position and doesn't care about
    /// anything else. Catches the case a plain length check misses: a
    /// same-length wholesale replacement of `board.tasks` with a different
    /// id set (a length-only check would wrongly consider the old index
    /// still valid).
    pub(in crate::tui) fn compute_task_ids_fingerprint(&self) -> u64 {
        let mut acc = fnv_seed();
        acc = fnv_fold(acc, self.board.tasks.len() as u64);
        for t in &self.board.tasks {
            acc = fnv_fold(acc, t.id.0 as u64);
        }
        acc
    }

    /// Discard all layout caches so the next `cached_epic_stats()` call
    /// recomputes from the current board state. Handlers that mutate
    /// `board.tasks`/`board.epics` should still call this (directly or via
    /// `sync_board_selection`) as a perf optimization — it forces an
    /// immediate rebuild rather than waiting for the next
    /// `cached_epic_stats()` call to detect the fingerprint mismatch — but it
    /// is no longer required for correctness.
    pub(in crate::tui) fn invalidate_layout_cache(&mut self) {
        self.layout.invalidate();
    }

    /// Build a list of items (tasks + epics) for a column in the current view.
    /// In board view, epics are included (positioned by derived status).
    /// In epic view, only subtasks are included (no epic cards).
    ///
    /// Passes `stats = None`: in non-flat mode with epics, epic sort order is derived
    /// by cloning all subtasks per epic. Prefer
    /// [`Self::column_items_for_status_with_placements`] with a pre-computed map
    /// whenever `compute_epic_placements()` can be called at the same site.
    #[cfg(test)]
    pub(crate) fn column_items_for_status(&self, status: TaskStatus) -> Vec<ColumnItem<'_>> {
        self.column_items_for_status_with_placements(status, None)
    }

    /// Like `column_items_for_status` but uses a pre-computed placement map.
    ///
    /// This is the board's only column builder. A *task* card's column is its
    /// `TaskStatus` and nothing else (see `board-layout.allium`, "Board
    /// Columns"); an *epic* card is drawn in every column its subtree has
    /// visible work in, which is what the placement map answers ("Epic Card
    /// Placement"). Sub-status groups cards into sections *within* the column,
    /// which [`Self::column_items_for_status_with_view_tasks`] emits as headers.
    pub fn column_items_for_status_with_placements<'a>(
        &'a self,
        status: TaskStatus,
        placements: Option<&EpicPlacementMap>,
    ) -> Vec<ColumnItem<'a>> {
        let view_tasks = self.tasks_for_current_view();
        let pass = self.epic_search_pass();
        self.column_items_for_status_with_view_tasks(status, placements, &view_tasks, &pass)
    }

    /// Like `column_items_for_status_with_placements` but accepts a pre-computed view-task
    /// list and search pass, allowing `tasks_for_current_view()` and
    /// `epic_search_pass()` to be called once and reused across all columns (e.g. in
    /// `ColumnLayout::build`).
    pub(in crate::tui) fn column_items_for_status_with_view_tasks<'a>(
        &'a self,
        status: TaskStatus,
        placements: Option<&EpicPlacementMap>,
        view_tasks: &[&'a Task],
        pass: &EpicSearchPass<'a>,
    ) -> Vec<ColumnItem<'a>> {
        let tasks: Vec<&'a Task> = view_tasks
            .iter()
            .filter(|t| t.status == status)
            .copied()
            .collect();

        if self.is_flattened_for_status(status) {
            return self.flattened_column_items(status, tasks);
        }
        self.hierarchical_column_items(status, tasks, placements, pass)
    }

    /// Flattened column: cards sorted by section, then epic group, then card
    /// key, with section and epic headers interleaved.
    pub(in crate::tui) fn flattened_column_items<'a>(
        &'a self,
        status: TaskStatus,
        tasks: Vec<&'a Task>,
    ) -> Vec<ColumnItem<'a>> {
        let epic_lookup = crate::models::epic_id_lookup(&self.board.epics);

        // Sort: (section_priority, epic_sort_key, task_sort_key, task_id).
        // Orphan tasks (epic not in board) sort last within each section.
        // The section and the epic key are resolved once per card and
        // carried through, since `sort_by_key` calls its key function once
        // per *comparison* — and the chunking below needs the same section
        // answer the sort used.
        let group_keys = self.flattened_group_keys(status, &tasks, &epic_lookup);
        let mut sorted_tasks: Vec<FlatCard<'a>> = tasks
            .into_iter()
            .map(|t| {
                (
                    ColumnSection::for_task(t),
                    group_keys.key_for(t, &epic_lookup),
                    CardOrderKey::for_task(t, status),
                    t,
                )
            })
            .collect();

        // The card's own key is hoisted for the same reason the section and
        // the group key are: `sort_by_key` calls its key function once per
        // COMPARISON, so anything built inside the closure is paid
        // O(n log n) times instead of n.
        sorted_tasks.sort_by_key(|&(section, epic_sk, card_sk, t)| {
            (section_sort_priority(section), epic_sk, card_sk, t.id.0)
        });

        // One pass over contiguous section runs: emit the section's header,
        // then — unless the section is folded — its epic headers, orphan
        // separator and cards. A folded section contributes its header and
        // nothing else; the epic header and the separator are decoration on
        // cards that are not being drawn.
        let mut items: Vec<ColumnItem<'a>> = Vec::with_capacity(sorted_tasks.len());
        for run in sorted_tasks.chunk_by(|(a, _, _, _), (b, _, _, _)| a == b) {
            let Some(section) = run[0].0 else {
                // A column with no sections (Backlog, Done): no header, and
                // nothing to fold.
                items.extend(run.iter().map(|&(_, _, _, t)| ColumnItem::Task(t)));
                continue;
            };
            let at = SectionRef::new(status, section);
            if self.section_renders_collapsed(status, section) {
                items.push(ColumnItem::FoldedSection(FoldedHeader {
                    at,
                    hidden: run.len(),
                }));
                continue;
            }
            items.push(ColumnItem::SubstatusLabel(at));

            // One pass over contiguous epic-id runs within the section:
            // groups are already adjacent because `group_keys` sorted by
            // epic key before card key. A folded group contributes its
            // header alone; an open one contributes the epic header (when
            // the epic resolves) followed by its cards, and an orphan
            // separator when the run transitions away from an epic group.
            self.push_epic_groups(status, run, &epic_lookup, &mut items);
        }

        items
    }

    /// Emit the epic headers, orphan separator and cards of one open section
    /// run of a flattened column.
    pub(in crate::tui) fn push_epic_groups<'a>(
        &self,
        status: TaskStatus,
        run: &[FlatCard<'a>],
        epic_lookup: &HashMap<EpicId, &'a Epic>,
        items: &mut Vec<ColumnItem<'a>>,
    ) {
        // One pass over contiguous epic-id runs within the section:
        // groups are already adjacent because `group_keys` sorted by
        // epic key before card key. A folded group contributes its
        // header alone; an open one contributes the epic header (when
        // the epic resolves) followed by its cards, and an orphan
        // separator when the run transitions away from an epic group.
        let mut current_epic_id: Option<EpicId> = None;
        for group in run.chunk_by(|(_, _, _, a), (_, _, _, b)| a.epic_id == b.epic_id) {
            let t0 = group[0].3;
            let Some(eid) = t0.epic_id else {
                if current_epic_id.is_some() {
                    items.push(ColumnItem::OrphanSeparator);
                    current_epic_id = None;
                }
                items.extend(group.iter().map(|&(_, _, _, t)| ColumnItem::Task(t)));
                continue;
            };
            let Some(&epic) = epic_lookup.get(&eid) else {
                // The epic named by this group does not resolve (e.g.
                // filtered out of the current view): fall back to
                // plain cards, exactly as the single-pass builder did.
                items.extend(group.iter().map(|&(_, _, _, t)| ColumnItem::Task(t)));
                continue;
            };
            current_epic_id = Some(eid);
            let fold_ref = EpicFoldRef::new(status, eid);
            if self.epic_group_renders_folded(fold_ref) {
                items.push(ColumnItem::FoldedEpic(FoldedEpicHeader {
                    at: fold_ref,
                    epic,
                    hidden: group.len(),
                }));
            } else {
                items.push(ColumnItem::EpicHeader(epic));
                items.extend(group.iter().map(|&(_, _, _, t)| ColumnItem::Task(t)));
            }
        }
    }

    /// Hierarchical column: tasks and epic cards sorted together by section,
    /// with a header per section run.
    pub(in crate::tui) fn hierarchical_column_items<'a>(
        &'a self,
        status: TaskStatus,
        tasks: Vec<&'a Task>,
        placements: Option<&EpicPlacementMap>,
        pass: &EpicSearchPass<'a>,
    ) -> Vec<ColumnItem<'a>> {
        //
        // Decorate, sort, chunk. Each card's section AND ordering key are
        // resolved exactly once, up front, and carried through the sort:
        // `sort_by_key` calls its key function once per *comparison*, and
        // resolving an epic's section can mean a scan of `board.tasks`, so
        // computing it inside the comparator would pay for it O(n log n) times
        // and then again when grouping. The epic key is hoisted for the same
        // reason — and because the placement it needs is already in hand at the
        // push site below.
        let mut cards: Vec<(Option<ColumnSection>, CardOrderKey, ColumnItem<'a>)> = tasks
            .into_iter()
            .map(|t| {
                (
                    ColumnSection::for_task(t),
                    CardOrderKey::for_task(t, status),
                    ColumnItem::Task(t),
                )
            })
            .collect();

        // An epic card is not placed by epic.status and is not placed once: it
        // is drawn in every column its subtree has visible work in
        // (board-layout.allium, "Epic Card Placement").
        let placements = &self.placements_or_compute(placements);
        for epic in self.visible_epics_for_effective_view(pass) {
            let Some(placement) = placements.get(&epic.id) else {
                continue;
            };
            if placement.appears_in(status) {
                cards.push((
                    self.epic_column_section(epic, status, Some(placements)),
                    placement.sort_key(epic, status),
                    ColumnItem::Epic(epic),
                ));
            }
        }

        cards.sort_by_key(|(section, key, item)| {
            let tie_break = match item {
                ColumnItem::Task(t) => t.id.0,
                ColumnItem::Epic(e) => e.id.0,
                ColumnItem::FoldedSection(_)
                | ColumnItem::FoldedEpic(_)
                | ColumnItem::EpicHeader(_)
                | ColumnItem::SubstatusLabel(_)
                | ColumnItem::OrphanSeparator => {
                    unreachable!("only Task and Epic items are built here")
                }
            };
            (section_sort_priority(*section), *key, tie_break)
        });

        // Same shape as the flattened path: a header per section run, and a
        // folded section contributes its header alone.
        let mut items: Vec<ColumnItem<'a>> = Vec::with_capacity(cards.len());
        for run in cards.chunk_by(|(a, _, _), (b, _, _)| a == b) {
            let Some(section) = run[0].0 else {
                items.extend(run.iter().map(|&(_, _, item)| item));
                continue;
            };
            let at = SectionRef::new(status, section);
            if self.section_renders_collapsed(status, section) {
                items.push(ColumnItem::FoldedSection(FoldedHeader {
                    at,
                    hidden: run.len(),
                }));
                continue;
            }
            items.push(ColumnItem::SubstatusLabel(at));
            items.extend(run.iter().map(|&(_, _, item)| item));
        }

        items
    }

    /// The per-group ordering keys a flattened column sorts its cards by.
    ///
    /// Outside Done a task's group is its epic, keyed by the epic's own
    /// `sort_key()`. In Done the group is keyed by the newest completion inside
    /// it — on the DIRECT epic, never the subtree that
    /// `EpicPlacement::sort_key` walks, because a sub-epic's tasks are a
    /// separate group with a key of their own, and never the epic's own
    /// `completed_at` either, for the same reason. The reasoning for all of it
    /// is in "Done Column Ordering" in `docs/specs/board-layout.allium`.
    pub(in crate::tui) fn flattened_group_keys(
        &self,
        status: TaskStatus,
        tasks: &[&Task],
        epic_lookup: &HashMap<EpicId, &Epic>,
    ) -> FlattenedGroupKeys {
        if status != TaskStatus::Done {
            return FlattenedGroupKeys::default();
        }
        // One pass, and only over epics the board actually holds: an entry for
        // an epic_id naming no board epic could never be read, because such a
        // task takes the orphan path.
        let mut done: HashMap<EpicId, chrono::DateTime<chrono::Utc>> =
            HashMap::with_capacity(epic_lookup.len());
        for t in tasks.iter().copied() {
            let Some(eid) = t.epic_id.filter(|eid| epic_lookup.contains_key(eid)) else {
                continue;
            };
            if let Some(at) = crate::models::fold_newest_completion(done.get(&eid).copied(), t) {
                done.insert(eid, at);
            }
        }
        FlattenedGroupKeys { done: Some(done) }
    }

    /// The section an epic card renders under in the `status` column. `None`
    /// in a column with no sections.
    ///
    /// An epic card can sit in all four columns at once, so the answer is per
    /// column: it comes off that column's own slice of the subtree, not the
    /// epic's board-wide substatus (board-layout.allium, "Epic Card
    /// Placement"). Every caller of this question must go through here.
    ///
    /// `placements` is the per-frame map when the caller has it; without one
    /// this recomputes, because a caller that answered `None` instead would
    /// report "no section" for a card the board draws under a header.
    pub(in crate::tui) fn epic_column_section(
        &self,
        epic: &Epic,
        status: TaskStatus,
        placements: Option<&EpicPlacementMap>,
    ) -> Option<ColumnSection> {
        let placements = self.placements_or_compute(placements);
        placements
            .get(&epic.id)
            .map(|p| p.substatus_in(epic, status))
            .unwrap_or(crate::models::EpicSubstatus::Unplanned)
            .column_section()
    }

    /// Whether `section` in the `status` column draws folded *right now*, as
    /// opposed to being recorded folded.
    ///
    /// A live search query forces every folded section open. That is the whole
    /// override: a section the query leaves empty renders no header either way,
    /// so "expand a folded section holding a match" and "ignore folds while a
    /// query is live" are the same rule (board-layout.allium: "Collapsed Sections").
    pub(in crate::tui) fn section_renders_collapsed(
        &self,
        status: TaskStatus,
        section: ColumnSection,
    ) -> bool {
        self.column_has_rendered_fold(status) && self.is_section_collapsed(status, section)
    }

    /// Whether `ref_` draws folded *right now*, on the same terms as
    /// `section_renders_collapsed` (board-layout.allium: "Epic Folding").
    pub(in crate::tui) fn epic_group_renders_folded(&self, ref_: EpicFoldRef) -> bool {
        self.column_has_rendered_fold(ref_.status) && self.is_epic_folded(ref_.status, ref_.epic)
    }

    /// Count the column items that can hold the cursor, for a status. Use this
    /// wherever only a count is needed — navigation bounds, clamp guards —
    /// rather than calling `column_items_for_status(s).len()`, which also counts
    /// the decorators (`EpicHeader`, an expanded `SubstatusLabel`,
    /// `OrphanSeparator`).
    ///
    /// Answers analytically — no sort, no item list — while the column has no
    /// folded section, which is the overwhelmingly common case and the reason
    /// this exists. With a fold active the arithmetic no longer holds (hidden
    /// cards drop out, folded headers join in), so it falls back to counting
    /// the built list.
    ///
    /// Derives the view tasks and the search pass itself, so it suits a caller
    /// with a single status in hand (`handle_navigate_row`). A caller that needs
    /// several statuses in one action should use [`Self::column_item_counts`],
    /// which derives both once for all of them.
    pub(in crate::tui) fn column_item_count(&self, status: TaskStatus) -> usize {
        let view_tasks = self.tasks_for_current_view();
        let cached = self.cached_placements();
        let placements = self.placements_or_compute(cached.as_deref());
        self.column_item_count_with(status, &view_tasks, &self.epic_search_pass(), &placements)
    }

    /// [`Self::column_item_count`] against pre-computed view tasks and search
    /// pass, so counting every column in one action scans the board once and
    /// builds one index rather than one of each per column.
    pub(in crate::tui) fn column_item_count_with<'a>(
        &'a self,
        status: TaskStatus,
        view_tasks: &[&'a Task],
        pass: &EpicSearchPass<'a>,
        placements: &EpicPlacementMap,
    ) -> usize {
        if self.column_has_rendered_fold(status) {
            return self
                .column_items_for_status_with_view_tasks(status, Some(placements), view_tasks, pass)
                .iter()
                .filter(|i| i.is_selectable())
                .count();
        }
        let task_count = view_tasks.iter().filter(|t| t.status == status).count();
        if self.is_flattened_for_status(status) {
            return task_count;
        }
        // Epic cards are placed per column, so this counts the ones this column
        // draws rather than the ones whose recorded status matches it.
        let epic_count = self
            .visible_epics_for_effective_view(pass)
            .filter(|e| placements.get(&e.id).is_some_and(|p| p.appears_in(status)))
            .count();
        task_count + epic_count
    }

    /// Selectable item counts for every board column, in `TaskStatus::ALL`
    /// order, from one board scan and one search pass. Used by
    /// [`Self::clamp_selection`], which needs all four counts in one action and
    /// interleaves `selection_mut()` writes — so it takes the counts up front
    /// rather than holding a board borrow across the writes.
    pub(in crate::tui) fn column_item_counts(&self) -> [usize; TaskStatus::COLUMN_COUNT] {
        let view_tasks = self.tasks_for_current_view();
        let pass = self.epic_search_pass();
        let cached = self.cached_placements();
        let placements = self.placements_or_compute(cached.as_deref());
        std::array::from_fn(|i| {
            self.column_item_count_with(TaskStatus::ALL[i], &view_tasks, &pass, &placements)
        })
    }
}
