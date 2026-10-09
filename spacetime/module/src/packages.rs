//! Work packages (`docs/specs/work-packages.allium`). Named `packages`, not
//! `work_packages`: a module of that name would shadow the table accessor
//! trait the `work_packages` table generates.

use super::*;

/// The derived key of a link row. `pub` so `MemoryReducerCaller` calls this
/// directly instead of carrying a second copy, the reuse `subscription_id`
/// already gets.
pub fn work_package_link_id(work_package_id: i64, kind: &str, item_id: i64) -> String {
    format!("{work_package_id}/{kind}/{item_id}")
}

/// The patch [`patch_work_package`] applies. `None` leaves a field alone.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, Default)]
pub struct WorkPackagePatch {
    pub title: Option<String>,
    pub description: Option<String>,
}

fn validate_work_package_title(title: &str) -> Result<(), String> {
    if title.trim().is_empty() {
        return Err("a work package needs a non-empty title".into());
    }
    Ok(())
}

/// Create a work package (`work-packages.allium: CreateWorkPackage`).
///
/// The client builds the row — timestamps and `created_by` included — on the
/// terms [`create_epic`] states; the store refuses a blank title and trims it.
#[spacetimedb::reducer]
pub fn create_work_package(ctx: &ReducerContext, row: WorkPackage) -> Result<(), String> {
    validate_work_package_title(&row.title)?;
    let title = row.title.trim().to_string();
    ctx.db.work_packages().insert(WorkPackage {
        id: 0,
        title,
        ..row
    });
    Ok(())
}

/// `work-packages.allium: UpdateWorkPackage`. A missing package is a refusal,
/// not a no-op: unlike a delete there is nothing a stale caller could have
/// meant by renaming something that is gone.
#[spacetimedb::reducer]
pub fn patch_work_package(
    ctx: &ReducerContext,
    id: i64,
    patch: WorkPackagePatch,
) -> Result<(), String> {
    let Some(mut row) = ctx.db.work_packages().id().find(id) else {
        return Err(format!("no work package {id}"));
    };
    if let Some(title) = patch.title {
        validate_work_package_title(&title)?;
        row.title = title.trim().to_string();
    }
    if let Some(description) = patch.description {
        row.description = description;
    }
    row.updated_at = now(ctx);
    ctx.db.work_packages().id().update(row);
    Ok(())
}

/// `work-packages.allium: DeleteWorkPackage`: the package and its links, and
/// nothing else. Idempotent on a missing package, like [`delete_epic`].
#[spacetimedb::reducer]
pub fn delete_work_package(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    let links: Vec<String> = ctx
        .db
        .work_package_links()
        .work_package_id()
        .filter(&id)
        .map(|l| l.id)
        .collect();
    for link in links {
        ctx.db.work_package_links().id().delete(&link);
    }
    ctx.db.work_packages().id().delete(id);
    Ok(())
}

fn validate_link_target(
    ctx: &ReducerContext,
    work_package_id: i64,
    kind: &str,
    item_id: i64,
) -> Result<(), String> {
    if ctx.db.work_packages().id().find(work_package_id).is_none() {
        return Err(format!("no work package {work_package_id}"));
    }
    let exists = match kind {
        WORK_PACKAGE_ITEM_TASK => ctx.db.tasks().id().find(item_id).is_some(),
        WORK_PACKAGE_ITEM_EPIC => ctx.db.epics().id().find(item_id).is_some(),
        other => return Err(format!("unknown work package item kind {other:?}")),
    };
    if !exists {
        return Err(format!("no {kind} {item_id}"));
    }
    Ok(())
}

/// `work-packages.allium: LinkItem`. Idempotent: a repeat finds the derived
/// key already present and writes nothing.
#[spacetimedb::reducer]
pub fn link_work_package_item(
    ctx: &ReducerContext,
    work_package_id: i64,
    kind: String,
    item_id: i64,
) -> Result<(), String> {
    validate_link_target(ctx, work_package_id, &kind, item_id)?;
    let id = work_package_link_id(work_package_id, &kind, item_id);
    if ctx.db.work_package_links().id().find(&id).is_none() {
        ctx.db.work_package_links().insert(WorkPackageLink {
            id,
            work_package_id,
            kind,
            item_id,
            created_at: now(ctx),
        });
    }
    Ok(())
}

/// `work-packages.allium: UnlinkItem`. Idempotent: no link, no error.
#[spacetimedb::reducer]
pub fn unlink_work_package_item(
    ctx: &ReducerContext,
    work_package_id: i64,
    kind: String,
    item_id: i64,
) -> Result<(), String> {
    let id = work_package_link_id(work_package_id, &kind, item_id);
    ctx.db.work_package_links().id().delete(&id);
    Ok(())
}

/// `work-packages.allium: DeletingAnItemRemovesItsLinks`: drop every link that
/// names this task or epic, in every package. Called from every path that
/// deletes a task row ([`delete_task_side_effects`]) or an epic row
/// ([`delete_epic_subtree`]).
pub(crate) fn delete_work_package_links_to(ctx: &ReducerContext, kind: &str, item_id: i64) {
    let links: Vec<String> = ctx
        .db
        .work_package_links()
        .item_id()
        .filter(&item_id)
        .filter(|l| l.kind == kind)
        .map(|l| l.id)
        .collect();
    for link in links {
        ctx.db.work_package_links().id().delete(&link);
    }
}
