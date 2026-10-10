//! Feed ingestion: post an emission's items to the store.
//!
//! Since store-split M3 the board does not decide what an emission does to the
//! store. [`ingest_emission`] hands the parsed items to the module's
//! `ingest_feed` reducer, which picks the strategy from the epic's own row,
//! routes by signals, makes the role and repo sub-epics, upserts, deletes the
//! stale and recalculates (`docs/specs/feeds.allium: IngestFeedEmission`).
//! What stays here needs the operator's machine: pairing each item with the
//! repo path and base branch read from local clones, and the teardown of
//! removed tasks' worktrees (`crate::feed::cleanup_removed_feed_tasks`).

#[cfg(test)]
mod tests;

use crate::models::{EpicId, FeedItem};
use crate::store::{FeedIngestOutcome, TaskStore};
use anyhow::Result;

/// Whether a sync pass may act on what its emission OMITS.
///
/// Decided once per feed cycle and threaded unchanged through every sync path
/// — no path re-decides it. TWO independent causes select [`Additive`], and
/// this type deliberately does not record which: the degraded-emission
/// predicates in [`crate::feed::exec`] (this EMISSION is not trusted), and an
/// epic's `feed_append_only` flag (this EPIC never mirrors, because its source
/// emits events that are never retracted). See feeds.allium:
/// `DegradedNonEmptyEmission` and `AppendOnlyFeed`.
///
/// [`Additive`]: SyncMode::Additive
///
/// An enum rather than a `bool` because both readings of a bare flag
/// (`delete_absent` vs `additive`) are plausible at a call site, and picking the
/// wrong one here force-removes live agents' worktrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncMode {
    /// The emission is trusted in full: absent feed tasks are stale and are
    /// deleted (and then torn down). The ordinary mode.
    Reconcile,
    /// Act only on what the emission contains. Inserts, field refreshes and
    /// cross-role moves still happen; every removal is skipped, so
    /// [`FeedIngestOutcome::removed`] comes back empty. Reached either because
    /// the emission is untrusted or because the epic never mirrors — see the
    /// type's docs for why the difference is not recorded here.
    Additive,
}

impl SyncMode {
    /// Whether this mode may remove feed tasks absent from the emission.
    fn removes_absent(self) -> bool {
        matches!(self, SyncMode::Reconcile)
    }
}

/// A feed item paired with its resolved repo path and base branch. Assembled
/// once at the `FeedCommandCompleted` boundary (see [`FeedItemWithTarget::zip`])
/// so the three values travel together as a unit through the rest of the feed
/// pipeline — there is no parallel-slice length invariant left to police.
pub(crate) struct FeedItemWithTarget {
    item: FeedItem,
    repo_path: String,
    base_branch: String,
}

impl FeedItemWithTarget {
    /// Zip co-indexed `items`/`repo_paths`/`base_branches` into paired entries.
    /// `repo_paths` and `base_branches` are derived one-per-item upstream
    /// (`resolve_feed_item_repo_paths` / `resolve_base_branches`), so the zip
    /// is lossless. Called once per emission, at each `FeedCommandCompleted`
    /// call site (`FeedRunner::tick`, `exec_trigger_epic_feed`) — the only
    /// place three parallel collections still exist, immediately before they
    /// collapse into these paired entries for the rest of the pipeline.
    pub(crate) fn zip(
        items: Vec<FeedItem>,
        repo_paths: Vec<String>,
        base_branches: Vec<String>,
    ) -> Vec<Self> {
        items
            .into_iter()
            .zip(repo_paths)
            .zip(base_branches)
            .map(|((item, repo_path), base_branch)| Self {
                item,
                repo_path,
                base_branch,
            })
            .collect()
    }
}

/// Post `entries` to the feed epic `epic_id`. `mode` is the board's verdict on
/// the emission; the module does the rest.
pub(crate) async fn ingest_emission(
    db: &dyn TaskStore,
    epic_id: EpicId,
    entries: Vec<FeedItemWithTarget>,
    mode: SyncMode,
) -> Result<FeedIngestOutcome> {
    let mut items = Vec::with_capacity(entries.len());
    let mut repo_paths = Vec::with_capacity(entries.len());
    let mut base_branches = Vec::with_capacity(entries.len());
    for entry in entries {
        items.push(entry.item);
        repo_paths.push(entry.repo_path);
        base_branches.push(entry.base_branch);
    }
    db.ingest_feed(
        epic_id,
        &items,
        &repo_paths,
        &base_branches,
        !mode.removes_absent(),
    )
    .await
}
