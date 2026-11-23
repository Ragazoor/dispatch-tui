//! Who the store records as acting (`store-split.allium: IdentitiesAreTheCallers`).
//!
//! One person uses the store (`docs/plans/2026-11-23-remove-workspaces.md`), so
//! nothing here withholds a row from anyone: these are the identity stamps a
//! write records, the restore authority, and the module's own checks on a new
//! epic or package row.

use super::*;

/// Whether the caller is the store itself: a scheduled reducer firing. The
/// publisher is NOT an operator here; it is recognised only by
/// [`require_restore_authority`], for the `seed_*` reducers.
pub fn is_operator(ctx: &ReducerContext) -> bool {
    ctx.sender() == ctx.database_identity()
}

/// Whether the caller is the identity that published this database, as
/// recorded by `init`. False for a store that recorded none.
pub fn is_publisher(ctx: &ReducerContext) -> bool {
    ctx.db
        .store_owner()
        .id()
        .find(1)
        .is_some_and(|owner| owner.identity == caller_identity(ctx))
}

/// The identity a write records for a person: the caller's own, unless the
/// caller is the operator, who may name anyone (`store-split.allium:
/// IdentitiesAreTheCallers`). Pure, so the board's twin calls it too.
pub fn stamped_identity(operator: bool, caller: &str, claimed: String) -> String {
    if operator {
        claimed
    } else {
        caller.to_string()
    }
}

/// [`stamped_identity`] for the current call.
pub fn caller_stamp(ctx: &ReducerContext, claimed: String) -> String {
    stamped_identity(is_operator(ctx), &caller_identity(ctx), claimed)
}

/// Refuse a `seed_*` call from anyone but the store itself or its publisher
/// (`spacetime-seed.allium: StorePublisher`).
pub fn require_restore_authority(ctx: &ReducerContext) -> Result<(), String> {
    if is_operator(ctx) || is_publisher(ctx) {
        Ok(())
    } else {
        Err("only the store's publisher may restore (seed) data".into())
    }
}

/// `epics.allium: RepoGroupOriginIsSetByRouting`: the origin marking a repo
/// group is set by the module's routing and never changed by a patch, in
/// either direction.
pub fn check_origin_patch(current: &str, new: &str) -> Result<(), String> {
    if current != new && (current == REPO_GROUP_ORIGIN || new == REPO_GROUP_ORIGIN) {
        return Err("a repo-group origin is set by routing and cannot be changed".into());
    }
    Ok(())
}

/// The epic `id`, or the refusal naming it missing.
pub fn require_epic(ctx: &ReducerContext, id: i64) -> Result<Epic, String> {
    ctx.db
        .epics()
        .id()
        .find(id)
        .ok_or_else(|| format!("epic {id} does not exist"))
}

/// Refuse a placement into an epic that does not exist, the refusal prefixed
/// with `label`. `0` (no epic) needs no check.
pub fn check_epic_exists(ctx: &ReducerContext, label: &str, epic_id: i64) -> Result<(), String> {
    if epic_id == 0 {
        return Ok(());
    }
    require_epic(ctx, epic_id)
        .map(|_| ())
        .map_err(|why| format!("{label}: {why}"))
}

/// `epics.allium: CreateEpic`: settle the row a new epic is stored as. A
/// sub-epic's parent must exist. `created_by` is the caller's
/// (`IdentitiesAreTheCallers`); only the store itself may name a creator. A
/// command on a new epic is its creator's (`feeds.allium:
/// FeedCommandAuthority`). The dead `workspace_id` and `visibility` columns
/// take their defaults, whatever the row names.
pub fn settle_new_epic(ctx: &ReducerContext, row: Epic) -> Result<Epic, String> {
    if row.parent_epic_id != 0 {
        require_epic(ctx, row.parent_epic_id)?;
    }
    let created_by = caller_stamp(ctx, row.created_by.clone());
    let feed_command_set_by = if row.feed_command.is_empty() {
        String::new()
    } else {
        created_by.clone()
    };
    let blank = blank_epic();
    Ok(Epic {
        workspace_id: blank.workspace_id,
        visibility: blank.visibility,
        created_by,
        feed_command_set_by,
        ..row
    })
}

/// `work-packages.allium: CreateWorkPackage`: the row to store, `created_by`
/// stamped with the caller (the store itself may name one) and the dead
/// `workspace_id` and `visibility` columns at their defaults.
pub fn settle_new_package(ctx: &ReducerContext, row: WorkPackage) -> WorkPackage {
    let created_by = caller_stamp(ctx, row.created_by.clone());
    let blank = blank_work_package();
    WorkPackage {
        workspace_id: blank.workspace_id,
        visibility: blank.visibility,
        created_by,
        ..row
    }
}

/// Refuse a `subscriber` the caller is not: a board follows epics and clears
/// unseen marks as itself. The store itself may act for anyone.
pub fn require_own_identity(ctx: &ReducerContext, subscriber: &str) -> Result<(), String> {
    if is_operator(ctx) || subscriber == caller_identity(ctx) {
        Ok(())
    } else {
        Err("a subscriber must be the caller's own identity".into())
    }
}
