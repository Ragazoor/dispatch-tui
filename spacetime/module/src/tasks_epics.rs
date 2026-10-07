//! Tasks and epics: patches, create/patch/delete, the epic status chain and the backlog claim.

use super::*;

/// The only way a task reaches the store.
///
/// Every writer goes through here so the ownership rule is unavoidable rather
/// than merely available. A free `validate_task_ownership` makes the rule
/// *callable*; this makes it the only path, which is what stops Phase 6's
/// writer from being asked by a doc comment to remember it.
///
/// It is also what makes [`SCRATCH_OWNER`] load-bearing instead of decorative:
/// the burn's throwaway rows and the probe row are epic-less, so without an
/// owner they are rows this function refuses.
///
/// Upserts by id, which is what makes a re-seed a no-op. An id of zero means
/// "generate one", so it can only be an insert.
pub(crate) fn write_task(ctx: &ReducerContext, row: Task) -> Result<(), String> {
    validate_task_ownership(row.epic_id, &row.owner)
        .map_err(|why| format!("task {}: {why}", row.id))?;
    validate_status("task", row.id, &row.status)?;
    // A TASK BORN IN DONE IS A COMPLETED TASK. `stamps_completion` covers every
    // TRANSITION into done, but a create is not a transition, and a Done card
    // with no `completed_at` sorts to the bottom of the column forever
    // (`board-layout.allium`, "Done Column Ordering"). The SQLite side stamps
    // it in `insert_task_row` for the same reason. Nothing creates a Done task
    // today; this is here so that whatever does first does not have to know.
    let row = if row.status == DONE && row.completed_at.is_empty() {
        Task {
            completed_at: now(ctx),
            ..row
        }
    } else {
        row
    };
    if row.id != 0 && ctx.db.tasks().id().find(row.id).is_some() {
        ctx.db.tasks().id().update(row);
    } else {
        ctx.db.tasks().insert(row);
    }
    Ok(())
}

// Seeding

// One reducer per table, each taking a BATCH.
//
// Per table rather than one generic reducer, because the module has to name the
// row type either way and a match over a JSON blob would move the column list
// from the compiler's problem to ours — on the one code path whose whole job is
// to not lose a column.
//
// Batched rather than per row, because every call is a round trip and a real
// board has thousands of rows. The caller chunks; see `SpacetimeCliStore` for
// the size it picks and why.
//
// Every one of these is an UPSERT: a row whose key is present is overwritten
// with this version of it, a row whose key is absent is inserted. That is what
// makes a second restore a no-op and a restore interrupted halfway safe to run
// again.

/// `core.allium: OwnerTracksUserBoardTask`, as a predicate.
///
/// Both directions refuse: an epic-less task with no owner, and an epic task
/// that carries one. The spec says why; the short version is that two answers
/// to "whose board is this?" are worse than none.
///
/// A free function rather than reducer-inline code so it can be tested without
/// a live `ReducerContext`. [`write_task`] is what makes it unavoidable.
/// Both arguments are in the STORE's vocabulary, not the domain's: `0` is no
/// epic and `""` is no owner. See "Absence is a sentinel, not a null" in this
/// file's header. Whitespace counts as absent too — a blank owner answers
/// nothing while passing any test for presence, which is the gap this closes.
pub fn validate_task_ownership(epic_id: i64, owner: &str) -> Result<(), String> {
    let has_epic = epic_id != 0;
    let has_owner = !owner.trim().is_empty();
    match (has_epic, has_owner) {
        (false, false) => Err("a task with no epic sits on a user board and needs an owner".into()),
        (true, true) => Err(format!(
            "task is in epic {epic_id} and must not also carry the owner {owner:?}"
        )),
        _ => Ok(()),
    }
}

// Mutations (Phase 6)
//
// EVERY SHARED WRITE ENTERS HERE. `sync.allium: BoardWritesThroughTheStore` is
// what makes that true from the client's side; this section is the other half.
// The seed reducers above are not an exception — they are the restore path, and
// `spacetime-seed.allium` scopes them to it.
//
// WHY A REDUCER PER MUTATION rather than one "apply this row" entry point. Two
// reasons, and the second is the load-bearing one:
//
//   1. A reducer is a transaction. Whatever a mutation has to keep true — an
//      epic's derived status, a claim that only one host may win — is kept true
//      inside the same transaction as the write, so no client ever observes the
//      half-way state. `sync.allium: EveryMutationIsAtomicAndAnswered`.
//   2. A full-row write is a full-row CLOBBER. Two hosts editing different
//      fields of the same task would each send a complete row built from what
//      they last saw, and the later one would undo the earlier one's change to
//      a field it never touched. The patch reducers below take one `Option` per
//      field, where `None` means "leave it alone" — so two hosts touching
//      different fields converge on both changes rather than on one.
//
// `Option` IS FINE IN A REDUCER ARGUMENT. The module header's "absence is a
// sentinel" rule is about COLUMNS, because a subscription is a `WHERE` clause
// and SQL cannot filter an optional column. An argument is never filtered on,
// so the patch structs below use `Option` for the thing it is actually good at:
// distinguishing "set this to absent" (`Some("")`) from "do not touch it"
// (`None`).

/// One field of a patch: `None` leaves it alone.
///
/// A type alias rather than a bare `Option`, so the intent reads at every use
/// site. The inner value is already the column's own type, absence sentinel
/// included — `Some(String::new())` clears a string column.
pub(crate) type Patch<T> = Option<T>;

/// The fields of a task a patch may change.
///
/// `id` is not among them: it names the row rather than describing it. Neither
/// is `created_at`, which records something that already happened.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, Default)]
pub struct TaskPatch {
    pub title: Patch<String>,
    pub description: Patch<String>,
    pub repo_path: Patch<String>,
    pub status: Patch<String>,
    pub worktree: Patch<String>,
    pub tmux_window: Patch<String>,
    pub plan_path: Patch<String>,
    pub epic_id: Patch<i64>,
    pub sub_status: Patch<String>,
    pub tag: Patch<String>,
    /// Doubly optional, and the one place the module's sentinel rule does not
    /// reach: `sort_order` is a genuinely nullable column (see the header), so
    /// "do not touch" and "set to null" are two different `None`s and need two
    /// levels to tell apart.
    pub sort_order: Patch<Option<i64>>,
    pub base_branch: Patch<String>,
    pub external_id: Patch<String>,
    pub labels: Patch<String>,
    pub last_pre_tool_use_at: Patch<String>,
    pub last_notification_at: Patch<String>,
    pub wrap_up_mode: Patch<String>,
    pub url: Patch<String>,
    pub url_type: Patch<String>,
    pub pr_learnings_gate_shown_at: Patch<String>,
    pub auto_run_plan: Patch<bool>,
    pub live_subagents: Patch<i64>,
    pub stop_pending: Patch<bool>,
    pub stop_pending_at: Patch<String>,
    pub last_peer_message_sent_at: Patch<String>,
    pub last_peer_message_received_at: Patch<String>,
    pub phoenix: Patch<bool>,
    pub host: Patch<String>,
    pub owner: Patch<String>,
    pub completed_at: Patch<String>,
}

/// The fields of an epic a patch may change.
#[derive(spacetimedb::SpacetimeType, Clone, Debug, Default)]
pub struct EpicPatch {
    pub title: Patch<String>,
    pub description: Patch<String>,
    pub status: Patch<String>,
    pub plan_path: Patch<String>,
    pub sort_order: Patch<Option<i64>>,
    pub auto_dispatch: Patch<bool>,
    pub parent_epic_id: Patch<i64>,
    pub feed_command: Patch<String>,
    pub feed_interval_secs: Patch<i64>,
    pub group_by_repo: Patch<bool>,
    pub feed_role: Patch<String>,
    pub origin: Patch<String>,
    pub feed_append_only: Patch<bool>,
    pub completed_at: Patch<String>,
}

/// Apply a task patch. Extracted from the reducer so it is testable without a
/// store — see the macro's note about what it does not guarantee.
pub fn apply_task_patch(row: &mut Task, patch: TaskPatch) {
    apply_patch!(
        row,
        patch,
        title,
        description,
        repo_path,
        status,
        worktree,
        tmux_window,
        plan_path,
        epic_id,
        sub_status,
        tag,
        sort_order,
        base_branch,
        external_id,
        labels,
        last_pre_tool_use_at,
        last_notification_at,
        wrap_up_mode,
        url,
        url_type,
        pr_learnings_gate_shown_at,
        auto_run_plan,
        live_subagents,
        stop_pending,
        stop_pending_at,
        last_peer_message_sent_at,
        last_peer_message_received_at,
        phoenix,
        host,
        owner,
        completed_at,
    );
}

/// Apply an epic patch. See [`apply_task_patch`].
pub fn apply_epic_patch(row: &mut Epic, patch: EpicPatch) {
    apply_patch!(
        row,
        patch,
        title,
        description,
        status,
        plan_path,
        sort_order,
        auto_dispatch,
        parent_epic_id,
        feed_command,
        feed_interval_secs,
        group_by_repo,
        feed_role,
        origin,
        feed_append_only,
        completed_at,
    );
}

// -- Tasks ------------------------------------------------------------------

/// Create a task, and recalculate the epic it lands in.
///
/// The row arrives with `id = 0`, which is how `#[auto_inc]` is asked to
/// generate one. The caller learns the generated id by watching the row arrive
/// on its subscription, because a reducer returns no value — see
/// `sync.allium: EveryMutationIsAtomicAndAnswered`.
#[spacetimedb::reducer]
pub fn create_task(ctx: &ReducerContext, row: Task) -> Result<(), String> {
    let epic_id = row.epic_id;
    write_task(
        ctx,
        Task {
            // Never trust an incoming id on a create. A caller that supplied
            // one would be choosing an id the store has not reserved, and the
            // next generated id would collide with it.
            id: 0,
            ..row
        },
    )?;
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

/// Change some fields of a task, and recalculate whatever epics it touched.
///
/// A missing task is a silent no-op rather than an error. The realistic
/// producer is a hook or a watcher firing against a task somebody deleted
/// meanwhile, and that is a race rather than a mistake — the same bargain the
/// SQLite side already makes.
#[spacetimedb::reducer]
pub fn patch_task(ctx: &ReducerContext, id: i64, patch: TaskPatch) -> Result<(), String> {
    let Some(mut row) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    let was_in_epic = row.epic_id;
    let prior_status = row.status.clone();
    // Read before `apply_patch!` consumes the patch's fields.
    let caller_named_a_completion = patch.completed_at.is_some();
    let moved_to_epic = patch.epic_id;

    apply_task_patch(&mut row, patch);

    // The completion stamp is the store's, not the caller's, unless the caller
    // named one explicitly. A client that computed it from its own clock would
    // make the Done column's ordering depend on whose laptop was fast.
    if !caller_named_a_completion && stamps_completion(&prior_status, &row.status) {
        row.completed_at = now(ctx);
    }
    row.updated_at = now(ctx);

    let now_in_epic = moved_to_epic.unwrap_or(was_in_epic);
    let status_changed = prior_status != row.status;
    write_task(ctx, row)?;

    // ONLY WHEN THE DERIVATION'S INPUTS MOVED. `derive_epic_status` reads the
    // child statuses and nothing else, so a patch that changes a url, a
    // sub-status or a timestamp cannot change any ancestor's answer — and
    // `patch_task` is the most frequent mutation on the board. Recalculating
    // unconditionally walked the whole ancestry, scanning tasks and epics at
    // each level, for every keystroke-driven write.
    //
    // BOTH epics when it moved, and in that order. A task leaving one epic
    // makes it possibly complete and makes the other possibly incomplete;
    // recalculating only the destination leaves the source stuck open.
    if status_changed || now_in_epic != was_in_epic {
        recalculate_epic_chain(ctx, was_in_epic);
        if now_in_epic != was_in_epic {
            recalculate_epic_chain(ctx, now_in_epic);
        }
    }
    Ok(())
}

/// Delete a task and everything that only referred to it.
///
/// The watcher rows go with it in the same transaction. A watch pointing at a
/// task that no longer exists is not a row anybody can act on, and leaving it
/// would make "who is watching me?" answerable with a ghost.
///
/// Also detaches this task's learnings (`source_task_id` set to `None`, a
/// learning outlives its source as orphaned provenance) and cascades its
/// `learning_retrievals` rows — reproducing, as explicit reducer logic, the
/// two SQLite `ON DELETE` clauses `learnings.source_task_id` and
/// `learning_retrievals.task_id` used to carry
/// (`docs/specs/learnings.allium`'s Storage Backend section).
///
/// `tasks.allium: DeleteTask`'s `requires: task.status = done` is checked
/// again HERE, over this row's true status, not only by the TUI's
/// `DeleteKeyRouting` over a subscription view that can be stale. Permanent
/// delete makes a stale-view false pass irrecoverable, the same reasoning
/// `delete_epic`'s `first_undone_task_in` guard already applies to a whole
/// subtree — here there is only the one row to check. Refusing leaves the row
/// (and everything it would have retired or torn down) untouched.
#[spacetimedb::reducer]
pub fn delete_task(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    if row.status != DONE {
        return Err(format!("task {id}: cannot delete because it is not done"));
    }
    let epic_id = row.epic_id;
    retire_task_if_feed_backed(ctx, epic_id, &row.external_id);
    ctx.db.tasks().id().delete(id);
    delete_task_side_effects(ctx, id);
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

/// Every non-row side effect a task removal owes, beyond deleting the `tasks`
/// row itself: live-session state, the watch rows naming it in either
/// direction, and learning detachment/retrieval-cascade. SQLite gets most of
/// this for free from `ON DELETE CASCADE`/`SET NULL`; the module has no such
/// mechanism, so every path that deletes a task row explicitly reproduces it
/// by calling here — `delete_task` itself, `batch_delete`'s plain-task pass,
/// `delete_epic_subtree`, and `delete_stale_feed_tasks_in_epic`. A second, hand-copied version of this
/// list is exactly how the latter two DRIFTED from `delete_task` and shipped
/// without it (task #4971's design doc flagged the gap).
pub(crate) fn delete_task_side_effects(ctx: &ReducerContext, task_id: i64) {
    delete_agent_state_for(ctx, task_id);
    // Two indexed lookups rather than one scan of every watch in the store.
    // Collected first because deleting while walking an index is not something
    // the table API promises.
    let watches: Vec<i64> = ctx
        .db
        .task_watchers()
        .watcher_task_id()
        .filter(&task_id)
        .chain(ctx.db.task_watchers().target_task_id().filter(&task_id))
        .map(|w| w.id)
        .collect();
    for watch in watches {
        ctx.db.task_watchers().id().delete(watch);
    }
    detach_learnings_from_task(ctx, task_id);
}

/// Move a task into an epic, out of one, or between two.
///
/// Separate from [`patch_task`] because it is the one field whose change has to
/// move something else with it: `core.allium: OwnerTracksUserBoardTask` ties
/// the owner to the epic being absent, so a task leaving its epic gains an
/// owner and a task joining one loses theirs. A patch route that could set
/// `epic_id` alone would produce a task on two boards or on none.
///
/// `owner` is what the task takes when it lands on a user board, and is ignored
/// when `epic_id` names an epic.
#[spacetimedb::reducer]
pub fn set_task_epic(
    ctx: &ReducerContext,
    id: i64,
    epic_id: i64,
    owner: String,
) -> Result<(), String> {
    let Some(task) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    let was_in = task.epic_id;
    if was_in == epic_id {
        return Ok(());
    }
    write_task(
        ctx,
        Task {
            epic_id,
            owner: if epic_id == 0 { owner } else { String::new() },
            updated_at: now(ctx),
            ..task
        },
    )?;
    // Both, and in that order. The one it left may now be complete; the one it
    // joined may no longer be.
    recalculate_epic_chain(ctx, was_in);
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

/// `PhoenixRespawn`, atomic: inserts the successor and clears the
/// predecessor's `phoenix` flag in one transaction. Either both happen or
/// neither does — `TheFlagIsTheReceipt` holds literally, so a retry after a
/// failure cannot create a second successor. Mirrors
/// `src/db/queries/tasks.rs::respawn_phoenix_successor` exactly.
///
/// No internal recalculation: the caller (`PhoenixRespawn` in
/// `src/service/tasks/crud.rs`) already calls the already-routed
/// `recalculate_epic_status` itself on success, the same as every feed
/// ingestion call site below does for its own writes.
#[spacetimedb::reducer]
pub fn respawn_phoenix_successor(
    ctx: &ReducerContext,
    predecessor: i64,
    successor: Task,
) -> Result<(), String> {
    let Some(predecessor_row) = ctx.db.tasks().id().find(predecessor) else {
        return Err(format!("predecessor task {predecessor} not found"));
    };
    write_task(ctx, Task { id: 0, ..successor })?;
    // Nothing between the check above and here can touch `predecessor_row` —
    // reducers run one at a time, and the write above only inserts a NEW
    // successor row — so the row already in hand is still current; no need
    // to re-fetch it.
    ctx.db.tasks().id().update(Task {
        phoenix: false,
        ..predecessor_row
    });
    Ok(())
}

/// Atomically set `sub_status` for many tasks. One reducer taking the whole
/// batch, so it applies whole or not at all — `TaskCrud::batch_patch_sub_status`'s
/// own doc comment already promises this shape.
///
/// No recalculation: `sub_status` is derived board state, not a
/// status/epic-linkage change (`TaskServiceApi::batch_patch_sub_status`'s doc
/// comment states this explicitly), so it carries no
/// `recalculate_epic_status` obligation on either backing.
#[spacetimedb::reducer]
pub fn batch_patch_sub_status(
    ctx: &ReducerContext,
    updates: Vec<SubStatusUpdate>,
) -> Result<(), String> {
    let stamp = now(ctx);
    for update in updates {
        let Some(row) = ctx.db.tasks().id().find(update.task_id) else {
            continue;
        };
        ctx.db.tasks().id().update(Task {
            sub_status: update.sub_status,
            updated_at: stamp.clone(),
            ..row
        });
    }
    Ok(())
}

/// One `(task_id, sub_status)` pair in a [`batch_patch_sub_status`] call.
#[derive(spacetimedb::SpacetimeType, Clone, Debug)]
pub struct SubStatusUpdate {
    pub task_id: i64,
    pub sub_status: String,
}

// -- Epics ------------------------------------------------------------------

/// Create an epic. Backlog, by `epics.allium: CreateEpic`.
#[spacetimedb::reducer]
pub fn create_epic(ctx: &ReducerContext, row: Epic) -> Result<(), String> {
    let parent = row.parent_epic_id;
    validate_status("epic", row.id, &row.status)?;
    ctx.db.epics().insert(Epic { id: 0, ..row });
    // The new epic is a child of its parent and a parent has no children rule
    // it can break by gaining a backlog one — but the parent may have been
    // done, in which case it is not any more.
    recalculate_epic_chain(ctx, parent);
    Ok(())
}

/// Change some fields of an epic, then recalculate it and its ancestors.
///
/// The recalculation runs AFTER the patch, so a manual status move is what the
/// derivation sees as "current" — which is how `epics.allium`'s "manual
/// placements are preserved" rule and its two auto-transitions coexist.
#[spacetimedb::reducer]
pub fn patch_epic(ctx: &ReducerContext, id: i64, patch: EpicPatch) -> Result<(), String> {
    let Some(mut row) = ctx.db.epics().id().find(id) else {
        return Ok(());
    };
    let prior_status = row.status.clone();
    let was_child_of = row.parent_epic_id;
    let caller_named_a_completion = patch.completed_at.is_some();

    apply_epic_patch(&mut row, patch);
    if !caller_named_a_completion && stamps_completion(&prior_status, &row.status) {
        row.completed_at = now(ctx);
    }
    row.updated_at = now(ctx);
    validate_status("epic", row.id, &row.status)?;
    ctx.db.epics().id().update(row);

    recalculate_epic_chain(ctx, id);
    if was_child_of != 0 {
        recalculate_epic_chain(ctx, was_child_of);
    }
    Ok(())
}

/// Delete an epic, its sub-epics and their tasks.
///
/// Recursive, and the recursion is the point: an epic's subtree is not
/// reachable any other way once its root is gone, so leaving it would strand
/// every descendant on a board that cannot draw them.
///
/// epics.allium: `DeleteEpic`'s retirement clause runs first, over the WHOLE
/// doomed subtree computed before anything is deleted — reading it after
/// would see nothing. Every doomed epic's own `retired_feed_items` rows are
/// then dropped with it: deleting a feed epic is a reset, not a prune, and
/// SQLite's `ON DELETE CASCADE` has no module-side equivalent to do this for
/// free.
///
/// `DeleteEpic`'s guard — every task anywhere in the subtree is `done` — is
/// checked HERE too, not only in `EpicService::delete_epic`
/// (`src/service/epics.rs`), which reads through a subscription view that can
/// be a strict subset of what the store holds. Before task #4971 an
/// incomplete view passing a guard the server would have refused cost an
/// archive that a later archive-then-recoverable mistake could undo;
/// permanent delete makes the same gap irreversible, so this reducer refuses
/// rather than trusting the caller. See `DeleteEpic`'s guidance for why both
/// checks stay: the client's is the responsive, explain-why path, this one is
/// the actual safety net.
#[spacetimedb::reducer]
pub fn delete_epic(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    let Some(row) = ctx.db.epics().id().find(id) else {
        return Ok(());
    };
    let parent = row.parent_epic_id;
    let doomed = validate_epic_deletable(ctx, id)?;
    retire_feed_tasks_before_epic_delete(ctx, &doomed);
    delete_epic_subtree(ctx, id, 0);
    drop_retired_feed_items_for_epics(ctx, &doomed);
    recalculate_epic_chain(ctx, parent);
    Ok(())
}

/// `tasks.allium: BatchDelete`'s atomic server-side counterpart to looping
/// [`delete_task`]/[`delete_epic`] once per selected item.
///
/// Before this reducer existed, `handle_batch_delete`
/// (src/tui/update/selection.rs) evaluated its `all_pass` guard over the
/// board's own, possibly stale, subscription view and then issued one
/// INDEPENDENT `delete_task`/`delete_epic` reducer call per item. Each of
/// those re-validates against this row's TRUE state on its own, so a view
/// stale for just one item let the rest be permanently deleted while that one
/// alone was refused — a partial batch, exactly what "one operation, or
/// nothing at all" (`tasks.allium: BatchDelete`) forbids. This reducer closes
/// that gap by validating EVERY selected task and epic against the true
/// state FIRST, mutating nothing until every one of them has passed, then
/// performing every delete together in this one transaction.
///
/// Every item in the batch — a plain task or an epic's whole subtree — is
/// exempt from `WorktreeReleaseIsGated`'s retry-on-failure pointer, the same
/// way [`delete_epic`] already is: bundling every row delete into one
/// transaction means there is no single surviving row left to hold a retry
/// pointer if one item's own teardown fails. `handle_batch_delete` therefore
/// fires every item's teardown best-effort and fire-and-forget
/// (`CleanupFollowUp::Nothing`) rather than gating this call on any of them
/// completing first.
///
/// A missing task or epic id is treated as already gone, same as
/// [`delete_task`]/[`delete_epic`]'s own no-op-on-missing-row behavior —
/// idempotent rather than a refusal, since a batch that raced a concurrent
/// delete of one of its own items has nothing left to refuse there.
#[spacetimedb::reducer]
pub fn batch_delete(
    ctx: &ReducerContext,
    task_ids: Vec<i64>,
    epic_ids: Vec<i64>,
) -> Result<(), String> {
    // Validate every epic's whole subtree FIRST, over the true state, mutating
    // nothing yet — see collect_epic_subtree_ids's doc comment for why the
    // subtree must be captured before anything downstream can change it.
    let mut epic_doomed: Vec<(i64, std::collections::HashSet<i64>)> = Vec::new();
    for &id in &epic_ids {
        if ctx.db.epics().id().find(id).is_none() {
            continue;
        }
        let doomed = validate_epic_deletable(ctx, id)?;
        epic_doomed.push((id, doomed));
    }
    // Validate every plain task next. A task already covered by one of the
    // doomed epic subtrees above is validated again here if the caller passed
    // it too — harmless, since epics.allium's guard already required it done.
    for &id in &task_ids {
        if let Some(row) = ctx.db.tasks().id().find(id) {
            if row.status != DONE {
                return Err(format!("task {id}: cannot delete because it is not done"));
            }
        }
    }

    // Nothing failed: perform every delete together, in this same
    // transaction. From here on this calls the same helpers delete_epic/
    // delete_task's own bodies do, just over the whole batch at once.
    let mut recalc: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for (id, doomed) in &epic_doomed {
        let Some(row) = ctx.db.epics().id().find(*id) else {
            // A nested selection: an earlier epic in this same batch already
            // deleted it as part of its own subtree.
            continue;
        };
        recalc.insert(row.parent_epic_id);
        retire_feed_tasks_before_epic_delete(ctx, doomed);
        delete_epic_subtree(ctx, *id, 0);
        drop_retired_feed_items_for_epics(ctx, doomed);
    }
    for &id in &task_ids {
        let Some(row) = ctx.db.tasks().id().find(id) else {
            // Already gone: one of the doomed epics above owned it too.
            continue;
        };
        let epic_id = row.epic_id;
        retire_task_if_feed_backed(ctx, epic_id, &row.external_id);
        ctx.db.tasks().id().delete(id);
        delete_task_side_effects(ctx, id);
        recalc.insert(epic_id);
    }
    for epic_id in recalc {
        recalculate_epic_chain(ctx, epic_id);
    }
    Ok(())
}

/// Every epic id in `root`'s subtree, itself included — computed BEFORE any
/// delete, so [`delete_epic`] knows which ids are "doomed" while the rows
/// describing them still exist. Mirrors
/// `src/db/queries/epics.rs::retire_feed_tasks_before_epic_delete`'s recursive
/// CTE.
pub(crate) fn collect_epic_subtree_ids(
    ctx: &ReducerContext,
    root: i64,
) -> std::collections::HashSet<i64> {
    let mut doomed = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !doomed.insert(id) {
            continue;
        }
        for child in ctx.db.epics().parent_epic_id().filter(&id) {
            stack.push(child.id);
        }
    }
    doomed
}

/// epics.allium: `DeleteEpic`'s guard, `epic.subtree_tasks.all(t => t.status
/// = done)`, evaluated against `doomed` — every epic id in the subtree,
/// itself included — rather than a single epic's own direct tasks, for the
/// same reason `collect_epic_subtree_ids` walks the whole tree: a nested
/// sub-epic's unfinished task must block the delete exactly as one of the
/// root's own would. Returns the first non-`done` task's id found, or `None`
/// when every task in the subtree qualifies (an empty subtree qualifies).
pub(crate) fn first_undone_task_in(
    ctx: &ReducerContext,
    doomed: &std::collections::HashSet<i64>,
) -> Option<i64> {
    for &epic_id in doomed {
        for task in ctx.db.tasks().epic_id().filter(&epic_id) {
            if task.status != DONE {
                return Some(task.id);
            }
        }
    }
    None
}

/// epics.allium: `DeleteEpic`'s guard, combining [`collect_epic_subtree_ids`]
/// and [`first_undone_task_in`] into the one check both [`delete_epic`] and
/// `batch_delete`'s epic-validation pass need: collect `id`'s doomed subtree
/// and refuse if any task in it is not done. Returns the doomed set on
/// success so callers reuse it rather than recomputing the subtree walk.
pub(crate) fn validate_epic_deletable(
    ctx: &ReducerContext,
    id: i64,
) -> Result<std::collections::HashSet<i64>, String> {
    let doomed = collect_epic_subtree_ids(ctx, id);
    if let Some(undone_task_id) = first_undone_task_in(ctx, &doomed) {
        return Err(format!(
            "epic {id}: cannot delete while task {undone_task_id} in its subtree is not done"
        ));
    }
    Ok(doomed)
}

/// How deep a delete or a recalculation will walk before it gives up.
///
/// A cycle in `parent_epic_id` is unreachable through any writer here, so this
/// is not a correctness mechanism — it is the thing that turns a corrupt store
/// into a refused reducer rather than a wasm stack overflow, which takes the
/// whole module down rather than one call.
pub(crate) const MAX_EPIC_DEPTH: usize = 64;

/// Recursively delete `id`'s sub-epics and their tasks, then `id` itself.
///
/// Each removed task owes [`delete_task_side_effects`] the same as any other
/// task removal — see that function's doc comment.
pub(crate) fn delete_epic_subtree(ctx: &ReducerContext, id: i64, depth: usize) {
    if depth > MAX_EPIC_DEPTH {
        return;
    }
    for child in ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&id)
        .collect::<Vec<_>>()
    {
        delete_epic_subtree(ctx, child.id, depth + 1);
    }
    for task in ctx.db.tasks().epic_id().filter(&id).collect::<Vec<_>>() {
        ctx.db.tasks().id().delete(task.id);
        delete_task_side_effects(ctx, task.id);
    }
    ctx.db.epics().id().delete(id);
}

/// Recalculate an epic's status and every ancestor's, from the WHOLE child set.
///
/// THE ONE PIECE OF LOGIC THE MIGRATION MOVED SERVER-SIDE, and the reason is
/// visible in the first two lines of the body: it reads every task and every
/// sub-epic whose parent is this epic, without reference to who owns them or
/// which board subscribes to them. No client can do that. A client deriving the
/// status from its own subscription derives it from a subset, two clients hold
/// different subsets, and the epic flips between their two correct-looking
/// answers for as long as both are up. See
/// `epics.allium: EpicStatusRecalculation` and
/// `sync.allium: TheStoreEnforcesTheSharedRules`.
///
/// Called by the mutations above rather than exposed as the only entry point,
/// so the recalculation is part of the same transaction as the change that
/// provoked it. An epic whose status lagged its children by one round trip
/// would be a board showing a column that is briefly wrong, on every machine.
#[spacetimedb::reducer]
pub fn recalculate_epic_status(ctx: &ReducerContext, epic_id: i64) -> Result<(), String> {
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

pub(crate) fn recalculate_epic_chain(ctx: &ReducerContext, epic_id: i64) {
    // Zero is the absent epic, not epic zero. A task on somebody's user board
    // reaches here with it and there is nothing to recalculate.
    let mut next = epic_id;
    for _ in 0..MAX_EPIC_DEPTH {
        if next == 0 {
            return;
        }
        let Some(epic) = ctx.db.epics().id().find(next) else {
            return;
        };

        recalculate_one(ctx, &epic);
        next = epic.parent_epic_id;
    }
}

/// Derive and write one epic's status, from every one of its children.
pub(crate) fn recalculate_one(ctx: &ReducerContext, epic: &Epic) {
    let children: Vec<String> = ctx
        .db
        .tasks()
        .epic_id()
        .filter(&epic.id)
        .map(|t| t.status)
        .chain(
            ctx.db
                .epics()
                .parent_epic_id()
                .filter(&epic.id)
                .map(|e| e.status),
        )
        .collect();

    if let Some(target) = derive_epic_status(&epic.status, &children) {
        let completed_at = if stamps_completion(&epic.status, target) {
            now(ctx)
        } else {
            // Left exactly as it was. The regression out of done does not
            // clear it: `completed_at` records the last completion, and
            // reopening work does not unmake one.
            epic.completed_at.clone()
        };
        ctx.db.epics().id().update(Epic {
            status: target.to_string(),
            completed_at,
            updated_at: now(ctx),
            ..epic.clone()
        });
    }
}

// -- The dispatch claim ------------------------------------------------------
//
// THE MOST SAFETY-CRITICAL RULE IN THE SYSTEM, and the one a shared store both
// endangers and fixes. `dispatch.allium: DispatchClaimExclusive` says one task
// is claimed by one dispatcher; on a single machine a single SQL statement made
// that true. Two machines racing for the same backlog subtask had no such
// statement, and the losing one would provision a second worktree over the
// winner's.
//
// A reducer is a transaction, so the read that chooses the row and the write
// that claims it cannot be interleaved by another host. That is the whole fix,
// and it is why these do not live on the client.
//
// WHAT A CLAIM WRITES is one decision and WHICH ROWS IT ACCEPTS is another —
// the same split `CLAIM_SET` and its per-caller `WHERE` make on the SQLite
// side. `apply_claim` is the first; the two reducers are the second.

/// Everything a claim writes to the row it wins.
pub(crate) fn apply_claim(ctx: &ReducerContext, task: Task) -> Task {
    Task {
        status: RUNNING.into(),
        sub_status: "active".into(),
        // Seeded so the dispatch watchdog measures from the claim rather than
        // from whenever the agent first says something. An unseeded claim looks
        // like a task that has been silent since the epoch.
        last_pre_tool_use_at: now(ctx),
        updated_at: now(ctx),
        ..task
    }
}

/// Whether this host may claim the task, by `dispatch.allium`'s
/// `requires: task.is_locally_owned`.
///
/// A foreign-owned backlog subtask is one whose worktree sits on another
/// machine. Claiming it here would dispatch an agent with nowhere to work.
/// Absent (`""`) is local: a task that has never been dispatched belongs to
/// whoever gets to it. `pub` so `MemoryReducerCaller`
/// (src/sync/memory_caller.rs) calls this directly instead of re-deriving the
/// same check — the same reuse `derive_epic_status` and friends already get.
pub fn claimable_by(task: &Task, host: &str) -> bool {
    task.host.is_empty() || task.host == host
}

// THERE IS NO `claim_next_backlog_task`, AND THAT IS DELIBERATE.
//
// The obvious design is a reducer that picks the next candidate and claims it
// in one transaction. It was written, and then removed, because a reducer
// returns no value: the chain's caller needs to know WHICH task it claimed —
// it is about to provision that task's worktree — and there is no channel to
// tell it.
//
// The named claim above is enough, because a board choosing a candidate for an
// epic is NOT choosing from a partial view. A subscription to an epic is
// `WHERE epic_id = N`, which returns every subtask of it regardless of who
// owns them, so a board that is chaining an epic can see all of that epic's
// direct children. The visibility problem that forced the status derivation
// server-side (`derive_epic_status`) is about ANCESTORS and descendants across
// epics; it does not reach one epic's own subtask list.
//
// So the client orders the candidates by `claim_order_key`'s rule and offers
// them one at a time, and this reducer arbitrates: a task another host already
// took is refused, and the client moves to the next. Exclusivity is still the
// store's, which is the part that had to move.

/// LOSING IS AN ERROR HERE, and that is how the caller finds out.
///
/// A reducer returns no value, so "did I win?" has to be carried by the one
/// channel a reducer does have: accepted or refused
/// (`sync.allium: EveryMutationIsAtomicAndAnswered`). Reading the row back
/// instead would not work — a claim does not stamp the winner's name on it, so
/// two hosts asking "is it running now?" after a race would both see `running`
/// and both believe they won, which is the exact failure this reducer exists to
/// prevent.
///
/// The refusal is therefore ORDINARY rather than exceptional. The client maps
/// it to "somebody else got there first" and moves to the next candidate; it is
/// a transport failure, not this, that means the store is down.
#[spacetimedb::reducer]
pub fn claim_backlog_task(ctx: &ReducerContext, id: i64, host: String) -> Result<(), String> {
    let Some(task) = ctx.db.tasks().id().find(id) else {
        return Err(format!("task {id} no longer exists"));
    };
    if task.status != BACKLOG {
        return Err(format!(
            "task {id} is {} rather than backlog, so it is already claimed",
            task.status
        ));
    }
    if !claimable_by(&task, &host) {
        return Err(format!(
            "task {id}'s worktree is on {}, so {host} cannot claim it",
            task.host
        ));
    }
    // THROUGH `write_task`, not around it. Neither this nor the release below
    // touches `epic_id` or `owner`, so nothing is broken by a direct update
    // today — but `write_task`'s whole claim is that it is the only path, and a
    // second path is one the next claim-adjacent reducer copies.
    write_task(ctx, apply_claim(ctx, task))
}

/// Undo a claim whose provisioning failed.
///
/// Guarded on `worktree` being absent, and that guard is the whole safety of
/// it: a claim that got as far as a worktree is a dispatch in progress, and
/// releasing it would put a running agent's task back in the backlog for
/// another host to claim underneath it.
#[spacetimedb::reducer]
pub fn release_backlog_claim(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    let Some(task) = ctx.db.tasks().id().find(id) else {
        return Err(format!("task {id} no longer exists"));
    };
    if task.status != RUNNING {
        return Err(format!("task {id} is not claimed"));
    }
    if !task.worktree.is_empty() {
        return Err(format!(
            "task {id} already has a worktree, so releasing it would put a running \
             agent's task back in the backlog"
        ));
    }
    write_task(
        ctx,
        Task {
            status: BACKLOG.into(),
            sub_status: "none".into(),
            last_pre_tool_use_at: String::new(),
            updated_at: now(ctx),
            ..task
        },
    )
}
