use std::sync::Arc;

use super::*;
use crate::models::{test_tmux_window, FeedRole, Signal, TaskId, TaskStatus, TaskTag};

use crate::store::{CreateTaskRequest, EpicCrud, EpicPatch, EpicRead, Store, TaskCrud, TaskPatch};

fn make_item(external_id: &str, url: &str) -> FeedItem {
    FeedItem {
        external_id: external_id.to_string(),
        title: external_id.to_string(),
        description: String::new(),
        url: url.to_string(),
        url_type: None,
        status: crate::models::TaskStatus::Backlog,
        tag: TaskTag::PrReview,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }
}

fn make_signal_item(external_id: &str, url: &str, signals: Vec<Signal>) -> FeedItem {
    FeedItem {
        signals,
        ..make_item(external_id, url)
    }
}

/// Zip three parallel test slices into paired [`FeedItemWithTarget`]
/// entries. Mirrors the assembly `FeedItemWithTarget::zip` performs at
/// the feed boundary.
fn entries(
    items: &[FeedItem],
    repo_paths: &[&str],
    base_branches: &[&str],
) -> Vec<FeedItemWithTarget> {
    items
        .iter()
        .zip(repo_paths.iter())
        .zip(base_branches.iter())
        .map(|((i, rp), bb)| FeedItemWithTarget {
            item: i.clone(),
            repo_path: rp.to_string(),
            base_branch: bb.to_string(),
        })
        .collect()
}

// Test shims. Every test written before the strategies moved into the module
// names the strategy it means (`run_role_routed_feed_sync`, `run_feed_sync`,
// `sync_grouped_feed`) and the ordinary trusted mode. The module now picks the
// strategy from the epic's own row, so each shim first makes the row say what
// the test means, then posts through `ingest_emission`. A test of
// `SyncMode::Additive` calls the `*_with_mode` form, so the mode under test is
// visible at the call site.
type FeedSyncOutcome = crate::store::FeedIngestOutcome;

async fn set_strategy(
    db: &dyn crate::store::TaskStore,
    epic_id: EpicId,
    feed_role: Option<FeedRole>,
    group_by_repo: Option<bool>,
) {
    let mut patch = EpicPatch::new();
    if let Some(role) = feed_role {
        patch = patch.feed_role(role);
    }
    if let Some(flag) = group_by_repo {
        patch = patch.group_by_repo(flag);
    }
    db.patch_epic(epic_id, &patch).await.unwrap();
}

async fn run_role_routed_feed_sync_with_mode(
    db: &dyn crate::store::TaskStore,
    parent_id: EpicId,
    entries: Vec<FeedItemWithTarget>,
    mode: SyncMode,
) -> anyhow::Result<FeedSyncOutcome> {
    set_strategy(db, parent_id, Some(FeedRole::ReviewsParent), None).await;
    ingest_emission(db, parent_id, entries, mode).await
}

async fn run_role_routed_feed_sync(
    db: &dyn crate::store::TaskStore,
    parent_id: EpicId,
    entries: Vec<FeedItemWithTarget>,
) -> anyhow::Result<FeedSyncOutcome> {
    run_role_routed_feed_sync_with_mode(db, parent_id, entries, SyncMode::Reconcile).await
}

async fn run_feed_sync_with_mode(
    db: &dyn crate::store::TaskStore,
    epic_id: EpicId,
    group_by_repo: bool,
    entries: Vec<FeedItemWithTarget>,
    mode: SyncMode,
) -> anyhow::Result<FeedSyncOutcome> {
    set_strategy(db, epic_id, None, Some(group_by_repo)).await;
    ingest_emission(db, epic_id, entries, mode).await
}

async fn run_feed_sync(
    db: &dyn crate::store::TaskStore,
    epic_id: EpicId,
    group_by_repo: bool,
    entries: Vec<FeedItemWithTarget>,
) -> anyhow::Result<FeedSyncOutcome> {
    run_feed_sync_with_mode(db, epic_id, group_by_repo, entries, SyncMode::Reconcile).await
}

async fn run_feed_sync_by_role(
    db: &dyn crate::store::TaskStore,
    epic_id: EpicId,
    feed_role: FeedRole,
    group_by_repo: bool,
    entries: Vec<FeedItemWithTarget>,
) -> anyhow::Result<FeedSyncOutcome> {
    set_strategy(db, epic_id, Some(feed_role), Some(group_by_repo)).await;
    ingest_emission(db, epic_id, entries, SyncMode::Reconcile).await
}

async fn sync_grouped_feed_with_mode(
    db: &dyn crate::store::TaskStore,
    parent_id: EpicId,
    entries: Vec<FeedItemWithTarget>,
    mode: SyncMode,
) -> FeedSyncOutcome {
    run_feed_sync_with_mode(db, parent_id, true, entries, mode)
        .await
        .unwrap()
}

async fn sync_grouped_feed(
    db: &dyn crate::store::TaskStore,
    parent_id: EpicId,
    entries: Vec<FeedItemWithTarget>,
) -> FeedSyncOutcome {
    sync_grouped_feed_with_mode(db, parent_id, entries, SyncMode::Reconcile).await
}

/// Create a manual (non-feed) task under `epic` — no `external_id`, so the
/// reconcile's stale pass must spare it. Every field but the title and the
/// epic is the same at all call sites, which is why this is a helper.
async fn create_manual_task(db: &Store, title: &str, epic: EpicId) -> TaskId {
    db.create_task(CreateTaskRequest {
        epic_id: Some(epic),
        ..CreateTaskRequest::fixture(title, "/repo")
    })
    .await
    .unwrap()
}

/// Find the sub-epic of `parent` carrying `role`, asserting exactly one.
async fn role_sub_epic(db: &Store, parent: EpicId, role: FeedRole) -> EpicId {
    let subs = db.list_sub_epics(parent).await;
    let matching: Vec<_> = subs.iter().filter(|e| e.feed_role == role).collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one {role:?} sub-epic, got {subs:?}"
    );
    matching[0].id
}

mod additive_sync;
mod excluded_prs;
mod repo_grouping;
mod retired_records;
mod role_routing;
