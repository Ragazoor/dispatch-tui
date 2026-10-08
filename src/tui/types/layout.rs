//! Per-frame board layout: column items, epic placement and stats, card
//! ordering, and the `LayoutCache` that holds them.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::models::{ColumnSection, Epic, EpicId, EpicSubstatus, Task, TaskId, TaskStatus};

// ColumnItem — resolves whether cursor is on a task or an epic
// ---------------------------------------------------------------------------

// `Copy` because every variant is a shared reference or a small plain struct,
// and the column builders regroup items into section runs on the render path —
// cloning there would be pure waste.
#[derive(Debug, Clone, Copy)]
pub enum ColumnItem<'a> {
    Task(&'a Task),
    Epic(&'a Epic),
    /// Non-selectable group header in flat view. Carries the epic so the renderer
    /// can read its title without an extra lookup.
    EpicHeader(&'a Epic),
    /// An open sub-status section header: decoration, like `EpicHeader`.
    /// Built by `column_items_for_status_with_view_tasks`, never injected by
    /// the renderer.
    SubstatusLabel(SectionRef),
    /// A folded sub-status section: its header stands in for every card it is
    /// hiding, so unlike `SubstatusLabel` it holds the cursor — it is the only
    /// way back into a section whose cards are all gone.
    ///
    /// A variant of its own rather than a flag on `SubstatusLabel`, so
    /// [`Self::is_selectable`] stays a fact about the variant and the hidden
    /// count exists only where it means something.
    FoldedSection(FoldedHeader),
    /// A folded flattened epic group: its header stands in for every card it
    /// is hiding, so like `FoldedSection` (and unlike `EpicHeader`) it holds
    /// the cursor — it is the only way back into a group whose cards are all
    /// gone. See "Epic Folding" in `docs/specs/board-layout.allium`.
    FoldedEpic(FoldedEpicHeader<'a>),
    /// Non-selectable separator inserted in flat view between the last epic-grouped
    /// task and the first orphan task (a task with no epic). Signals the visual
    /// boundary so the renderer can draw a divider line.
    OrphanSeparator,
}

/// Names one sub-status section: the column it is in, and the section within
/// it. The same section name in two columns is two independent sections, so
/// both halves are needed to identify one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionRef {
    pub status: TaskStatus,
    pub section: ColumnSection,
}

impl SectionRef {
    pub(in crate::tui) fn new(status: TaskStatus, section: ColumnSection) -> Self {
        Self { status, section }
    }
}

/// A folded section's header, which stands in for the cards it hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldedHeader {
    pub at: SectionRef,
    /// Cards this header is hiding. Counted after every board filter, so it
    /// never claims to hide a card the user could not have seen anyway, and
    /// always at least one: a section with no cards renders no header at all.
    pub hidden: usize,
}

/// Names one flattened epic group: the column it is in, and the epic within
/// it. The same epic in two columns is two independent groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpicFoldRef {
    pub status: TaskStatus,
    pub epic: EpicId,
}

impl EpicFoldRef {
    pub(in crate::tui) fn new(status: TaskStatus, epic: EpicId) -> Self {
        Self { status, epic }
    }
}

/// A folded epic group's header, which stands in for the cards it hides.
/// Carries the epic itself (unlike [`FoldedHeader`], whose section already
/// names its own label) so the renderer can show the same id and breadcrumb
/// the open header would.
#[derive(Debug, Clone, Copy)]
pub struct FoldedEpicHeader<'a> {
    pub at: EpicFoldRef,
    pub epic: &'a Epic,
    /// Cards this header is hiding, on the same terms as `FoldedHeader::hidden`.
    pub hidden: usize,
}

impl ColumnItem<'_> {
    /// Whether this item can hold the cursor. A fact about the variant, with
    /// no runtime condition: a caller that filters on this may then match on
    /// `Task | Epic | FoldedSection` and treat the rest as unreachable.
    pub fn is_selectable(&self) -> bool {
        matches!(
            self,
            ColumnItem::Task(_)
                | ColumnItem::Epic(_)
                | ColumnItem::FoldedSection(_)
                | ColumnItem::FoldedEpic(_)
        )
    }

    /// The anchor that identifies this item across a refresh, or `None` for a
    /// decorative one.
    ///
    /// `Some` exactly where [`Self::is_selectable`] is true — the two are one
    /// fact, so the anchor-cache builder can `filter_map` on this alone rather
    /// than filter on the predicate and then re-match.
    pub fn anchor(&self) -> Option<ColumnAnchor> {
        match self {
            ColumnItem::Task(t) => Some(ColumnAnchor::Task(t.id)),
            ColumnItem::Epic(e) => Some(ColumnAnchor::Epic(e.id)),
            ColumnItem::FoldedSection(h) => Some(ColumnAnchor::Section(h.at)),
            ColumnItem::FoldedEpic(h) => Some(ColumnAnchor::EpicFold(h.at)),
            ColumnItem::SubstatusLabel(_)
            | ColumnItem::EpicHeader(_)
            | ColumnItem::OrphanSeparator => None,
        }
    }
}

// ---------------------------------------------------------------------------
// ColumnAnchor — identity of the currently-selected task-board item
// ---------------------------------------------------------------------------

/// Identifies which item the cursor is anchored to across column refreshes.
/// Task and Epic IDs come from separate SQLite sequences and can overlap,
/// so we use a discriminated enum rather than a bare i64.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnAnchor {
    Task(crate::models::TaskId),
    Epic(crate::models::EpicId),
    /// A folded section's header.
    ///
    /// Unlike the other two this names no database row. It is still an
    /// identity rather than a position, and as durable as a task id: it
    /// survives refresh, reorder, and the section's cards turning over
    /// completely. A folded header is the only selectable item with no entity
    /// behind it, so without this the cursor could not survive a refresh while
    /// resting on one.
    Section(SectionRef),
    /// A folded epic group's header, on the same terms as `Section` above.
    EpicFold(EpicFoldRef),
}

// ---------------------------------------------------------------------------
// ColumnLayout — pre-computed column items for one render frame
// ---------------------------------------------------------------------------

/// Pre-computed column items for one render frame.
/// Built once at the top of `render()` to avoid recomputing per widget.
pub struct ColumnLayout<'a> {
    columns: [Vec<ColumnItem<'a>>; TaskStatus::COLUMN_COUNT],
    /// The map the columns were built from. Kept so the render pass can label
    /// each epic card with the substatus of the column it landed in, without
    /// recomputing the walk per card. `Arc`, so sharing it with the layout
    /// cache costs a refcount rather than a copy of the whole map.
    placements: std::sync::Arc<EpicPlacementMap>,
}

impl<'a> ColumnLayout<'a> {
    pub fn build(app: &'a crate::tui::App) -> Self {
        // Call tasks_for_current_view() and epic_search_pass() once each and
        // share them across all column builds instead of recomputing them
        // per-status inside column_items_for_status_with_placements. The
        // placement map comes from the layout cache the render pass warmed a
        // moment ago; the fallback is for a caller that has not.
        let view_tasks = app.view().tasks_for_current_view();
        let pass = app.view().epic_search_pass();
        let placements = app
            .view()
            .cached_placements()
            .unwrap_or_else(|| std::sync::Arc::new(app.view().compute_epic_placements()));
        let columns = std::array::from_fn(|i| {
            let status = TaskStatus::ALL[i];
            app.view().column_items_for_status_with_view_tasks(
                status,
                Some(&placements),
                &view_tasks,
                &pass,
            )
        });
        ColumnLayout {
            columns,
            placements,
        }
    }

    pub fn placements(&self) -> &EpicPlacementMap {
        &self.placements
    }

    pub fn get(&self, status: TaskStatus) -> &[ColumnItem<'a>] {
        &self.columns[status.column_index()]
    }

    pub fn count(&self, status: TaskStatus) -> usize {
        self.columns[status.column_index()].len()
    }
}

// ---------------------------------------------------------------------------
// SubtaskStats — pre-computed per-epic subtask status counts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SubtaskStats {
    pub backlog: usize,
    pub running: usize,
    pub review: usize,
    pub done: usize,
    pub total: usize,
    pub substatus: EpicSubstatus,
}

impl SubtaskStats {
    /// Compute stats for a single epic from its subtasks, including tasks
    /// owned by any descendant sub-epics. The `substatus`
    /// field also reflects the full subtree: a blocked task anywhere in the
    /// descendant hierarchy contributes to the `Blocked(N)` indicator.
    ///
    /// `children_map` is the parent→children adjacency map produced by
    /// [`crate::models::build_children_map`]. Build it once per stats
    /// computation and pass it here to avoid O(epics²) rebuilds.
    pub fn for_epic(
        epic: &Epic,
        all_tasks: &[Task],
        children_map: &HashMap<EpicId, Vec<EpicId>>,
    ) -> Self {
        let epic_ids = crate::models::descendant_epic_ids_with_map(epic.id, children_map);

        let mut backlog = 0;
        let mut running = 0;
        let mut review = 0;
        let mut done = 0;
        let mut owned: Vec<&Task> = Vec::new();

        for t in all_tasks {
            if matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid)) {
                match t.status {
                    TaskStatus::Backlog => backlog += 1,
                    TaskStatus::Running => running += 1,
                    TaskStatus::Review => review += 1,
                    TaskStatus::Done => done += 1,
                }
                owned.push(t);
            }
        }

        let substatus = crate::models::epic_substatus(epic, &owned);

        SubtaskStats {
            backlog,
            running,
            review,
            done,
            total: backlog + running + review + done,
            substatus,
        }
    }
}

/// Pre-computed subtask stats for all epics, keyed by EpicId.
pub type EpicStatsMap = HashMap<EpicId, SubtaskStats>;

// ---------------------------------------------------------------------------
// EpicPlacement — which columns one epic's card is drawn in
// ---------------------------------------------------------------------------

/// Which columns an epic's card appears in, and what each copy's section needs.
///
/// An epic card is not placed once by `epic.status`: it is drawn in every column
/// where the epic's subtree holds a *visible* task of that status — visible
/// meaning the task survives the same repo, only-active and search predicates
/// every other card is held to. See `board-layout.allium`, "Epic Card
/// Placement".
///
/// Deliberately not part of [`EpicStatsMap`], which counts the whole subtree
/// unfiltered and is cached against a fingerprint that does not cover the
/// search query or the only-active filter. The two answer different questions:
/// stats say what the card *reports*, placement says where the card *is*.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpicPlacement {
    /// Indexed by [`TaskStatus::column_index`]: does this column hold at least
    /// one visible subtree task?
    columns: [bool; TaskStatus::COLUMN_COUNT],
    /// Visible running subtree tasks in a blocked sub-status. Decides whether
    /// the Running copy sits in `NeedsInput` rather than `Active`.
    blocked_running: usize,
    /// The newest completion in the subtree's *done* slice: the maximum
    /// `completed_at` over the visible done tasks credited here. `None` when
    /// the slice holds none. The derived half of the Done copy's ordering key
    /// — see [`Self::sort_key`].
    newest_completion: Option<DateTime<Utc>>,
}

impl EpicPlacement {
    /// Credit one *already-admitted* task to this epic. The visibility filter —
    /// repo, only-active, search — belongs to `BoardView::compute_epic_placements`,
    /// which is the only caller, so that the predicate has one owner rather
    /// than half of it living here.
    pub(in crate::tui) fn record(&mut self, task: &crate::models::Task) {
        let Some(slot) = self.columns.get_mut(task.status.column_index()) else {
            return;
        };
        *slot = true;
        if task.status == TaskStatus::Running && task.sub_status.is_blocked() {
            self.blocked_running += 1;
        }
        self.newest_completion =
            crate::models::fold_newest_completion(self.newest_completion, task);
    }

    /// Draw an epic with no admitted task anywhere in Backlog, so it stays
    /// reachable. Applied once by `BoardView::compute_epic_placements` after the walk,
    /// which is what lets every reader below be a plain lookup.
    ///
    /// Applied to every epic, with no exception: every epic row that exists is
    /// one the board can draw (`board-layout.allium`, "Epic Card Placement"),
    /// so the map carries the invariant "every placement names at least one
    /// column" unconditionally.
    pub(in crate::tui) fn apply_empty_fallback(&mut self) {
        if !self.columns.iter().any(|c| *c) {
            self.columns[TaskStatus::Backlog.column_index()] = true;
        }
    }

    /// Whether the epic's card is drawn in `status`.
    pub fn appears_in(&self, status: TaskStatus) -> bool {
        self.columns
            .get(status.column_index())
            .copied()
            .unwrap_or(false)
    }

    /// The substatus the copy in `status` carries — both the section it lands
    /// in and the label it renders. Derived from that column's own tasks, so a
    /// blocked task in Running has no say over the Review copy. The table is
    /// shared with `epic_substatus`; only the inputs narrow.
    pub fn substatus_in(&self, epic: &Epic, status: TaskStatus) -> EpicSubstatus {
        crate::models::epic_substatus_for(status, epic.plan_path.is_some(), self.blocked_running)
    }

    /// The key this epic's card sorts by in the `status` column.
    ///
    /// Everywhere but Done that is the epic's own `sort_key()`. In Done it is
    /// `done_sort_key`: the epic's OWN `completed_at` when it has one, else the
    /// newest completion in its done slice — see "Done Column Ordering" in
    /// `board-layout.allium`, which is where the reasoning lives.
    ///
    /// The own-first precedence is what makes a manual reorder of an epic card
    /// in Done mean something: the reorder writes `epic.completed_at`, and this
    /// prefers it over the derived subtask key. The derived branch is the one a
    /// still-RUNNING epic takes — its card lands in Done because part of its
    /// subtree finished, not because the epic did — and it is what keeps such a
    /// card next to the work that put it there.
    ///
    /// Only the hierarchical path calls this; a flattened column keys its
    /// groups on a task's direct epic instead.
    pub fn sort_key(&self, epic: &Epic, status: TaskStatus) -> CardOrderKey {
        if status != TaskStatus::Done {
            return CardOrderKey::Generic(epic.sort_key());
        }
        CardOrderKey::completion(self.done_completion(epic))
    }

    /// `done_sort_key` unwrapped: the completion time this epic's card is
    /// dated by in the Done column, or `None` when it has none.
    ///
    /// Split out of [`Self::sort_key`] for the manual reorder, which needs the
    /// bare timestamp to swap rather than the render key — and must ask the
    /// same question the render asked, or it would move the card somewhere the
    /// next frame disagrees with.
    pub(in crate::tui) fn done_completion(&self, epic: &Epic) -> Option<DateTime<Utc>> {
        epic.completed_at.or(self.newest_completion)
    }
}

/// Pre-computed column placement for all epics, keyed by EpicId.
pub type EpicPlacementMap = HashMap<EpicId, EpicPlacement>;

/// The group keys a flattened column orders its cards by, resolved once per
/// build rather than once per comparison. Built by
/// `App::flattened_group_keys`, whose doc comment carries the rule.
///
/// `done` is `Some` only in the Done column; everywhere else both accessors
/// fall through to the generic behaviour and the map is never allocated.
#[derive(Debug, Clone, Default)]
pub(in crate::tui) struct FlattenedGroupKeys {
    pub(in crate::tui) done: Option<HashMap<EpicId, DateTime<Utc>>>,
}

impl FlattenedGroupKeys {
    /// The group key for one card.
    ///
    /// A task with no epic — or one naming an epic the board does not hold,
    /// which has no group to join either — is an orphan: last in every column,
    /// except in Done, where it is a one-card group ranked like any other.
    pub(in crate::tui) fn key_for(
        &self,
        task: &crate::models::Task,
        epic_lookup: &HashMap<EpicId, &Epic>,
    ) -> CardOrderKey {
        let epic = task.epic_id.and_then(|eid| epic_lookup.get(&eid));
        match (epic, &self.done) {
            (Some(epic), None) => CardOrderKey::Generic(epic.sort_key()),
            // In Done a group takes its newest member's completion. The epic's
            // OWN completed_at is deliberately not consulted here, unlike
            // `EpicPlacement::sort_key`: a flattened group stands for the
            // column's tasks that name this epic directly, not for the epic.
            (Some(_), Some(done)) => {
                CardOrderKey::completion(task.epic_id.and_then(|eid| done.get(&eid).copied()))
            }
            (None, Some(_)) => CardOrderKey::for_task(task, TaskStatus::Done),
            (None, None) => CardOrderKey::Generic(i64::MAX),
        }
    }
}

// ---------------------------------------------------------------------------
// CardOrderKey — the key one card sorts by within its section run
// ---------------------------------------------------------------------------

/// The ordering key a card sorts by within its section run, in any column.
///
/// Sorting is always ascending on this type; the variants carry the direction.
/// The derived `Ord` also orders BETWEEN variants, which matters in exactly one
/// place — `Completed` before `Undated`, so a Done card the column cannot date
/// sinks below every dated one. Within any single column all keys share a
/// variant, because the column decides which one to build.
///
/// See "Done Column Ordering" in `docs/specs/board-layout.allium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CardOrderKey {
    /// The generic `sort_order ?? id`, ascending. Every column but Done.
    Generic(i64),
    /// A completion time in the Done column, read newest-first. Wrapped in
    /// `Reverse` so the ascending sort puts the most recent first — the
    /// ordering is expressed here, once, rather than by negating a stored
    /// value as the completion *rank* this replaced had to.
    Completed(std::cmp::Reverse<DateTime<Utc>>),
    /// A Done card with no completion time at all. Last, below every dated
    /// card: the column knows least about it. Should not occur: every done
    /// row carries a completion time.
    Undated,
}

impl CardOrderKey {
    /// A Done-column key from an optional completion time.
    pub fn completion(at: Option<DateTime<Utc>>) -> Self {
        match at {
            Some(at) => Self::Completed(std::cmp::Reverse(at)),
            None => Self::Undated,
        }
    }

    /// The key one TASK card sorts by in `status`.
    pub fn for_task(task: &crate::models::Task, status: TaskStatus) -> Self {
        if status == TaskStatus::Done {
            Self::completion(task.completed_at)
        } else {
            Self::Generic(task.sort_key())
        }
    }
}

// ---------------------------------------------------------------------------
// LayoutCache — derived per-frame layout state, invalidated as a unit
// ---------------------------------------------------------------------------

/// Derived layout state computed from `board.tasks`/`board.epics`, populated
/// together by `App::cached_epic_stats()` and cleared together by
/// `App::invalidate_layout_cache()`. Grouped into one struct so the fields
/// that must stay coherent with each other (and with the board) can only be
/// invalidated as a unit — see `LayoutCache::invalidate()`. `cached_epic_stats()`
/// also self-heals on a fingerprint mismatch even if invalidation was
/// forgotten; see `App::compute_layout_fingerprint()`.
#[derive(Debug, Default)]
pub(in crate::tui) struct LayoutCache {
    /// Cached result of `compute_epic_stats_with_map()`, wrapped in an `Arc` so that
    /// `cached_epic_stats()` returns a reference-counted handle (O(1) clone)
    /// rather than cloning the full `HashMap` on every call.
    pub(in crate::tui) epic_stats_cache: Option<std::sync::Arc<EpicStatsMap>>,
    /// Cached result of `compute_epic_placements()` — where each epic's cards
    /// are drawn. Built and cleared with `epic_stats_cache`, and `Arc`-wrapped
    /// for the same reason.
    ///
    /// Unlike the stats, this depends on the repo filter, the only-active
    /// filter and the search query as well as on the board, which is why
    /// `App::compute_layout_fingerprint()` folds all three in. Without that it
    /// would keep serving yesterday's columns the moment the user typed a query.
    pub(in crate::tui) epic_placements_cache: Option<std::sync::Arc<EpicPlacementMap>>,
    /// Parent→children adjacency map over `board.epics`. Built once alongside
    /// `epic_stats_cache` in `cached_epic_stats()`; passed into
    /// `compute_epic_stats_with_map()` so the map is not rebuilt for each epic.
    pub(in crate::tui) children_map_cache: Option<HashMap<EpicId, Vec<EpicId>>>,
    /// Pre-sorted selectable items (tasks + epics) per status in display order.
    /// Built once alongside `epic_stats_cache`; `update_anchor_from_current`
    /// reads from this (O(1) per nav event) instead of re-sorting the column.
    pub(in crate::tui) column_anchor_cache: Option<HashMap<TaskStatus, Vec<ColumnAnchor>>>,
    /// Per-epic `(epic_repo_matches, epic_matches)` results, built once per render frame
    /// inside `cached_epic_stats()` using a single shared `build_children_map()` call.
    pub(in crate::tui) epic_filter_cache: Option<HashMap<EpicId, (bool, bool)>>,
    /// Fingerprint of the cache-relevant fields of `board.tasks`/`board.epics`
    /// (id, status, epic_id/parent_epic_id, sort_order) captured when
    /// `epic_stats_cache` and friends were last populated. `cached_epic_stats()`
    /// recomputes this fingerprint on every call and self-heals (discards and
    /// rebuilds) if it no longer matches — so a handler that forgets to call
    /// `invalidate_layout_cache()` cannot serve stale data, it only pays for
    /// an extra rebuild. See `App::compute_layout_fingerprint()`.
    pub(in crate::tui) layout_cache_fingerprint: Option<u64>,
    /// TaskId → Vec index for O(1) lookups in `find_task_mut`. Not primed in
    /// `App::new()` to avoid staleness when tests mutate `board.tasks` directly.
    /// Rebuilt lazily in `find_task_mut` whenever `task_index_fingerprint`
    /// no longer matches `App::compute_task_ids_fingerprint()` (covers both
    /// length changes and same-length id-set replacement).
    pub(in crate::tui) task_index: Option<HashMap<TaskId, usize>>,
    /// Fingerprint of `board.tasks` ids captured when `task_index` was last
    /// built. See `App::compute_task_ids_fingerprint()`.
    pub(in crate::tui) task_index_fingerprint: Option<u64>,
}

impl LayoutCache {
    /// Clear every cache field as a unit. Called whenever `board.tasks` or
    /// `board.epics` are mutated; also a no-op-safe fallback since
    /// `cached_epic_stats()` self-heals on a fingerprint mismatch regardless.
    pub(in crate::tui) fn invalidate(&mut self) {
        *self = Self::default();
    }
}

// ---------------------------------------------------------------------------
