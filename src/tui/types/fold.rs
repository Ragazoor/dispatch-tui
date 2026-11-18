//! Which sections and epic groups the user has folded, and how the choice persists.

use std::collections::BTreeSet;

use crate::models::{ColumnSection, EpicId, TaskStatus};
use crate::tui::types::EpicFoldRef;

// SectionFoldState — which sub-status sections the user has folded
// ---------------------------------------------------------------------------

/// Settings key the folded-section list is stored under.
pub const COLLAPSED_SECTIONS_KEY: &str = "collapsed_sections";

/// The set of folded sub-status sections, keyed on `(column, section)` so the
/// same section name in two columns folds independently.
///
/// A persisted preference, unlike `BoardState.flattened` and the selection:
/// "not this pile, not now" outlives a session. It lives here beside
/// [`FilterState`](crate::tui::types::state::FilterState) rather than on `BoardState`, which holds ephemeral board
/// content.
///
/// A `BTreeSet` rather than a `HashSet` so [`Self::serialise`] has a stable
/// order — a settings row that reorders itself between runs churns the database
/// and any snapshot over it.
///
/// See "Collapsed Sections" in `docs/specs/board-layout.allium`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionFoldState {
    folded: BTreeSet<(TaskStatus, ColumnSection)>,
}

impl SectionFoldState {
    pub(in crate::tui) fn is_collapsed(&self, status: TaskStatus, section: ColumnSection) -> bool {
        self.folded.contains(&(status, section))
    }

    /// Whether this column has any folded section at all. The cheap guard that
    /// keeps an unfolded board on the analytic item-count path.
    pub(in crate::tui) fn any_in(&self, status: TaskStatus) -> bool {
        self.folded.iter().any(|&(s, _)| s == status)
    }

    pub(in crate::tui) fn toggle(&mut self, status: TaskStatus, section: ColumnSection) {
        if !self.folded.remove(&(status, section)) {
            self.folded.insert((status, section));
        }
    }

    /// Fold every entry into `acc`, so a fold change is visible to the layout
    /// cache's coherence fingerprint. Without this the cache's "same
    /// fingerprint means same derived view" guarantee would stop covering the
    /// one input that is not board data.
    pub(in crate::tui) fn fold_into_fingerprint(&self, mut acc: u64) -> u64 {
        acc = crate::tui::fnv_fold(acc, self.folded.len() as u64);
        for &(status, section) in &self.folded {
            acc = crate::tui::fnv_fold(acc, status as u64);
            acc = crate::tui::fnv_fold(acc, section as u64);
        }
        acc
    }

    /// `status/section` pairs, comma-separated, in the set's own sorted order.
    /// Parsed back by [`Self::parse`].
    pub fn serialise(&self) -> String {
        self.folded
            .iter()
            .map(|(status, section)| format!("{}/{}", status.as_str(), section.as_str()))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Read back [`Self::serialise`]'s output, skipping any entry this binary
    /// cannot resolve. A fold naming an unknown status or section is a display
    /// preference that cannot be honoured, not the data-integrity bug the
    /// storage-boundary drop rule guards against — see the carve-out under
    /// "Storage Boundary Validation" in `docs/specs/core.allium`.
    pub fn parse(text: &str) -> Self {
        let folded = text
            .split(',')
            .filter_map(|entry| {
                let (status, section) = entry.trim().split_once('/')?;
                Some((status.parse().ok()?, section.parse().ok()?))
            })
            .collect();
        Self { folded }
    }
}

// ---------------------------------------------------------------------------
// EpicFoldState — which flattened epic groups the user has folded
// ---------------------------------------------------------------------------

/// Settings key the folded-epic list is stored under.
pub const COLLAPSED_EPICS_KEY: &str = "collapsed_epics";

/// The set of folded flattened epic groups, keyed on `(column, section,
/// epic)`: folding an epic hides its cards in that one substatus section, and
/// the same epic's group in every other section of the column is untouched.
///
/// A persisted preference, on the same reasoning as [`SectionFoldState`], and
/// independent of it: an epic fold and a section fold are two separate
/// recorded sets over possibly-overlapping cards.
///
/// See "Epic Folding" in `docs/specs/board-layout.allium`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpicFoldState {
    folded: BTreeSet<EpicFoldRef>,
}

impl EpicFoldState {
    pub(in crate::tui) fn is_folded(&self, group: EpicFoldRef) -> bool {
        self.folded.contains(&group)
    }

    /// Whether this column has any folded epic at all. The cheap guard
    /// `BoardView::column_has_rendered_fold` shares with
    /// [`SectionFoldState::any_in`].
    pub(in crate::tui) fn any_in(&self, status: TaskStatus) -> bool {
        self.folded.iter().any(|g| g.at.status == status)
    }

    pub(in crate::tui) fn toggle(&mut self, group: EpicFoldRef) {
        if !self.folded.remove(&group) {
            self.folded.insert(group);
        }
    }

    /// The folded epic groups in `status`'s column.
    pub(in crate::tui) fn folded_in(&self, status: TaskStatus) -> Vec<EpicFoldRef> {
        self.folded
            .iter()
            .filter(|g| g.at.status == status)
            .copied()
            .collect()
    }

    /// Move `from`'s fold onto each of `to` in the same column and section
    /// (tasks.allium: `FoldMovesToDrawnDescendant`).
    pub(in crate::tui) fn move_down(&mut self, from: EpicFoldRef, to: &[EpicId]) {
        self.folded.remove(&from);
        self.folded
            .extend(to.iter().map(|&epic| EpicFoldRef { at: from.at, epic }));
    }

    /// Fold every entry into `acc`, for the same reason
    /// [`SectionFoldState::fold_into_fingerprint`] does.
    pub(in crate::tui) fn fold_into_fingerprint(&self, mut acc: u64) -> u64 {
        acc = crate::tui::fnv_fold(acc, self.folded.len() as u64);
        for g in &self.folded {
            acc = crate::tui::fnv_fold(acc, g.at.status as u64);
            acc = crate::tui::fnv_fold(acc, g.at.section as u64);
            acc = crate::tui::fnv_fold(acc, g.epic.0 as u64);
        }
        acc
    }

    /// `status/section/epic_id` triples, comma-separated, in the set's own
    /// sorted order. Parsed back by [`Self::parse`].
    pub fn serialise(&self) -> String {
        self.folded
            .iter()
            .map(|g| {
                format!(
                    "{}/{}/{}",
                    g.at.status.as_str(),
                    g.at.section.as_str(),
                    g.epic.0
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Read back [`Self::serialise`]'s output, skipping any entry this binary
    /// cannot resolve — an unknown status or section, or a non-numeric epic
    /// id — on the same reasoning as [`SectionFoldState::parse`]. Unlike a
    /// section, an epic id is never checked against the epics that exist: a
    /// fold naming one that has since been deleted or reparented away from
    /// this column is inert (board-layout.allium, "Epic Folding": state
    /// model), not unresolvable.
    ///
    /// A `status/epic_id` pair is a fold saved before folds were per section.
    /// It folds the epic in every section, so the board looks as it did; a
    /// section the column never draws names nothing and stays inert.
    pub fn parse(text: &str) -> Self {
        let mut folded = BTreeSet::new();
        for entry in text.split(',') {
            let Some((status, rest)) = entry.trim().split_once('/') else {
                continue;
            };
            let Ok(status) = status.parse::<TaskStatus>() else {
                continue;
            };
            let (sections, epic): (Vec<ColumnSection>, &str) = match rest.split_once('/') {
                Some((section, epic)) => match section.parse() {
                    Ok(section) => (vec![section], epic),
                    Err(_) => continue,
                },
                None => (ColumnSection::ALL.to_vec(), rest),
            };
            let Ok(epic) = epic.parse() else {
                continue;
            };
            folded.extend(
                sections
                    .into_iter()
                    .map(|section| EpicFoldRef::new(status, section, EpicId(epic))),
            );
        }
        Self { folded }
    }
}

// ---------------------------------------------------------------------------
