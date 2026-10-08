//! Board-search and board-wide filter helpers.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

use super::*;

/// Format a title for display in confirmation prompts, truncating if longer than `max_len` chars.
pub(in crate::tui) fn truncate_title(title: &str, max_len: usize) -> String {
    if title.chars().count() <= max_len {
        format!("\"{title}\"")
    } else {
        let truncated: String = title.chars().take(max_len.saturating_sub(3)).collect();
        format!("\"{truncated}...\"")
    }
}

/// Returns true if every character in `query_lower` (already lowercased) appears in
/// `path` as a forward subsequence (case-insensitive on `path`).
/// An empty query matches everything.
pub(in crate::tui) fn fuzzy_matches_lower(path: &str, query_lower: &str) -> bool {
    if query_lower.is_empty() {
        return true;
    }
    let path_lower = path.to_lowercase();
    let mut path_chars = path_lower.chars();
    for qc in query_lower.chars() {
        if !path_chars.any(|pc| pc == qc) {
            return false;
        }
    }
    true
}

/// The digit payload of a board-search query, if it can address a task by id:
/// the query with one optional leading `#` stripped, provided the remainder is
/// non-empty and entirely ASCII digits. `None` for anything else (`"38a"`,
/// `"a38"`, a lone `"#"`, an empty query), which means title-only matching.
pub(in crate::tui) fn id_digits_query(query: &str) -> Option<&str> {
    let digits = query.strip_prefix('#').unwrap_or(query);
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

/// Returns true if the decimal spelling of `id` starts with `digits` (a payload
/// from [`id_digits_query`]). Prefix, not substring: `"38"` matches `3837` but
/// not `1385`.
pub(in crate::tui) fn id_prefix_matches(id: i64, digits: &str) -> bool {
    id.to_string().starts_with(digits)
}

/// Returns true if every character in `query` appears in `path` as a
/// forward subsequence (case-insensitive). An empty query matches everything.
pub(in crate::tui) fn fuzzy_matches(path: &str, query: &str) -> bool {
    fuzzy_matches_lower(path, &query.to_lowercase())
}

/// Returns the subset of `paths` that fuzzy-match `query`, preserving order.
pub(in crate::tui) fn filtered_repos(paths: &[String], query: &str) -> Vec<String> {
    paths
        .iter()
        .filter(|p| fuzzy_matches(p, query))
        .cloned()
        .collect()
}

/// Whether the epic (identified by `epic_ids` = epic + all descendants) should be shown
/// under the current repo filter.  A single pass over `tasks` tracks both "has any
/// subtask" and "has any repo-matching subtask", so the logic is O(tasks)
/// instead of two passes.
pub(in crate::tui) fn epic_repo_matches_for_ids(
    tasks: &[Task],
    filter: &FilterState,
    epic_ids: &HashSet<EpicId>,
) -> bool {
    if filter.repos.is_empty() {
        return true;
    }
    let (has_active, has_match) = tasks.iter().fold((false, false), |(active, matched), t| {
        if matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid)) {
            (true, matched || filter.matches(&t.repo_path))
        } else {
            (active, matched)
        }
    });
    !has_active || has_match
}

/// Whether the epic has at least one subtask with an active tmux window.
/// `epic_ids` must include the epic itself and all its descendants.
pub(in crate::tui) fn epic_active_matches_for_ids(
    tasks: &[Task],
    epic_ids: &HashSet<EpicId>,
) -> bool {
    tasks.iter().any(|t| {
        matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid)) && t.tmux_window.is_some()
    })
}

/// Whether `title`/`id` themselves satisfy the board-search query: a
/// case-insensitive forward-subsequence title match, or a decimal id-prefix
/// match against `id_digits` (see [`id_digits_query`]). Shared by the task
/// and epic own-match checks so the OR is expressed once. See
/// board_search_filter in `docs/specs/board-layout.allium`.
pub(in crate::tui) fn own_search_match(
    title: &str,
    id: i64,
    query_lower: &str,
    id_digits: Option<&str>,
) -> bool {
    fuzzy_matches_lower(title, query_lower)
        || id_digits.is_some_and(|digits| id_prefix_matches(id, digits))
}

/// The board-wide filters every card is held to, resolved once per pass.
///
/// Three predicates compose here — the repo filter, the only-active filter and
/// the board-search query — and the one thing they must never do is disagree
/// between the places that ask. They were transcribed by hand in three of them
/// once (`tasks_for_current_view`, `epic_ids_owning_matching_task` and the epic
/// placement walk), so a fourth filter would have reached some and not others
/// while every doc comment went on claiming they were the same test.
///
/// The *scope* question — which tasks this view reaches at all — is deliberately
/// not here. That is what genuinely differs between callers, and keeping it out
/// leaves each call site reading as "these shared filters, plus my own scope".
///
/// See board_search_filter and only_active_filter in
/// `docs/specs/board-layout.allium`.
pub(in crate::tui) struct BoardFilters<'a> {
    pub(in crate::tui) filter: &'a FilterState,
    pub(in crate::tui) query_lower: String,
    pub(in crate::tui) id_digits: Option<&'a str>,
}

impl<'a> BoardFilters<'a> {
    pub(in crate::tui) fn new(filter: &'a FilterState, query: &'a str) -> Self {
        BoardFilters {
            filter,
            // Lowercased once per pass, not once per task: this is the render
            // hot path.
            query_lower: query.to_lowercase(),
            id_digits: id_digits_query(query),
        }
    }

    /// Whether `task` survives all three filters.
    pub(in crate::tui) fn admits(&self, task: &Task) -> bool {
        self.filter.matches(&task.repo_path)
            && self.filter.task_matches(task)
            && self.matches_query(&task.title, task.id.0)
    }

    /// The search half alone, for a caller that has already applied the other
    /// two or is asking about an epic rather than a task.
    pub(in crate::tui) fn matches_query(&self, title: &str, id: i64) -> bool {
        own_search_match(title, id, &self.query_lower, self.id_digits)
    }
}

/// The epic ids that *directly own* at least one task carrying the
/// board-search match: the task has an own match (title or id-prefix) AND the
/// board would actually show it under the repo and only-active filters — the
/// same predicates `tasks_for_current_view` applies. A task the board would hide
/// cannot keep an ancestor epic's card alive: drilling into that card would be a
/// dead end. See board_search_filter in `docs/specs/board-layout.allium`.
///
/// One O(tasks) pass for the whole board, so a view pass over N epic cards costs
/// one scan rather than N. An epic's own verdict is then a set-membership test
/// per descendant — see [`EpicSearchIndex`].
pub(in crate::tui) fn epic_ids_owning_matching_task(
    tasks: &[Task],
    filter: &FilterState,
    query_lower: &str,
    id_digits: Option<&str>,
) -> HashSet<EpicId> {
    let filters = BoardFilters {
        filter,
        query_lower: query_lower.to_string(),
        id_digits,
    };
    tasks
        .iter()
        .filter(|t| filters.admits(t))
        .filter_map(|t| t.epic_id)
        .collect()
}

/// Per-view-pass state for the epic board-search predicate, built once by
/// [`App::epic_search_index`] and reused across every epic in the pass — and,
/// wrapped in an [`EpicSearchPass`], across every column of a frame.
///
/// Collapses what used to be per-epic work: the query is parsed once (not once
/// per epic), the O(tasks) owner scan runs once (not once per epic), and the
/// epic children map and id lookup are built once so the own-match check is
/// O(1) and the sub-epic scan is O(descendants) rather than O(epics).
///
/// **Intra-call only.** This is a local value with the lifetime of one view
/// pass; it is never stored on `App` or in `App.layout`. `App.layout` is guarded
/// by `compute_layout_fingerprint()`, which folds ids, status, parent and sort
/// order but neither titles nor the query, so a cross-render cached search
/// verdict would go stale on a title edit or a keystroke in the search bar.
pub(in crate::tui) struct EpicSearchIndex<'a> {
    pub(in crate::tui) query_lower: String,
    pub(in crate::tui) id_digits: Option<&'a str>,
    pub(in crate::tui) by_id: HashMap<EpicId, &'a Epic>,
    pub(in crate::tui) children: HashMap<EpicId, Vec<EpicId>>,
    pub(in crate::tui) task_owners: HashSet<EpicId>,
}

#[cfg(test)]
thread_local! {
    /// Counts [`App::epic_search_index`] builds so tests can pin the
    /// once-per-pass shape (a view pass must build one index, not one per
    /// column). Thread-local, so parallel tests don't interfere.
    pub(in crate::tui) static EPIC_SEARCH_INDEX_BUILDS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

impl EpicSearchIndex<'_> {
    /// Whether `epic`'s own title or id satisfies the query. The epic and
    /// sub-epic branches of [`App::epic_search_matches_indexed`] ask the same
    /// question, so the parsed-query plumbing is expressed once.
    pub(in crate::tui) fn own_match(&self, epic: &Epic) -> bool {
        own_search_match(&epic.title, epic.id.0, &self.query_lower, self.id_digits)
    }

    /// Whether `epic_id` itself is in the index, and matches. `None` — an id with
    /// no epic behind it — is not a match.
    pub(in crate::tui) fn own_match_by_id(&self, epic_id: EpicId) -> bool {
        self.by_id.get(&epic_id).is_some_and(|e| self.own_match(e))
    }
}

/// The [`EpicSearchIndex`] for one pass over the board, threaded by parameter
/// through the column builders exactly as `view_tasks` is — so a frame that
/// builds four columns builds one index rather than four (see
/// [`ColumnLayout::build`](crate::tui::types::ColumnLayout::build)).
///
/// Built on first use rather than up front, so a pass that never reaches an
/// epic-visibility decision — a non-searching render, or a flattened column,
/// neither of which consults the index — pays nothing. The cell also makes the
/// once-per-pass property structural rather than a discipline the callers have
/// to keep.
///
/// **Intra-pass only**, for the reasons on [`EpicSearchIndex`]: it is a local
/// value, never stored on `App` or in `App.layout`.
#[derive(Default)]
pub(in crate::tui) struct EpicSearchPass<'a>(OnceCell<Option<EpicSearchIndex<'a>>>);

impl<'a> EpicSearchPass<'a> {
    /// Whether the epic survives this pass's search filter, building the index
    /// on the first call. With no query live, every epic is admitted.
    pub(in crate::tui) fn admits(&self, app: &'a App, epic_id: EpicId) -> bool {
        self.0
            .get_or_init(|| app.search_active().then(|| app.epic_search_index()))
            .as_ref()
            .is_none_or(|idx| app.epic_search_matches_indexed(idx, epic_id))
    }
}

/// Returns true when the buffer should be offered as a selectable "new path"
/// entry: the buffer is non-empty and is not already an exact member of
/// `filtered` (the user is typing a path that doesn't exist in the saved list).
pub(in crate::tui) fn has_new_repo_option(buffer: &str, filtered: &[String]) -> bool {
    !buffer.is_empty() && !filtered.iter().any(|p| p == buffer)
}

/// Resolve the item Enter selects in a picker (RepoPathPicker,
/// BaseBranchPicker, ...): `candidates` fuzzy-filtered by `buffer`, indexed at
/// `cursor` when that falls within the filtered list, otherwise the typed
/// `buffer` itself when it qualifies as a "new" entry. `None` when the
/// effective list is empty (buffer empty, no candidates).
pub(in crate::tui) fn resolve_picker_selection(
    candidates: &[String],
    buffer: &str,
    cursor: usize,
) -> Option<String> {
    let filtered = filtered_repos(candidates, buffer);
    if cursor < filtered.len() {
        Some(filtered[cursor].clone())
    } else if has_new_repo_option(buffer, &filtered) {
        Some(buffer.trim().to_string())
    } else {
        None
    }
}
