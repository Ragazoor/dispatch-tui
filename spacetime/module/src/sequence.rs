//! The id-sequence burn: keeping `#[auto_inc]` ahead of rows restored with explicit ids.

use super::*;

// The sequence burn

/// Leave `table`'s id counter strictly past `ceiling`.
///
/// `#[auto_inc]` assigns a value only when the column is zero, so restoring
/// rows with explicit ids leaves the counter exactly where it started — at 1 —
/// and the next created task collides with the oldest restored one. Nothing
/// sets a counter, so the only way to move it is to make the store generate
/// values and throw them away.
///
/// # This must run BEFORE the rows are loaded
///
/// The throwaway inserts ask for ids 1, 2, 3 and so on, which are precisely the
/// ids a restore is about to write. Called against a table that already holds
/// them, the first insert violates the primary key and the whole reducer
/// aborts. There is no way to advance the counter past a row without generating
/// that row's id, so burning a loaded table is not slow, it is impossible.
///
/// Against an empty table it is neither: one insert and one delete per id, once,
/// inside one transaction. The counter's block pre-allocation is reserved
/// storage, not a shortcut past generating each value, so it does not shorten
/// this loop — a board whose highest task id is twenty thousand burns twenty
/// thousand of them. That is affordable because it happens at seed or recovery
/// time and never again.
///
/// Gaps in the id space are expected and harmless: nothing in the domain reads
/// an id as a count or a position.
#[spacetimedb::reducer]
pub fn burn_id_sequence(ctx: &ReducerContext, table: String, ceiling: i64) -> Result<(), String> {
    // One arm per generating table, each identical but for the accessor and the
    // blank row. Written as a macro so a seventh generating table is one line
    // rather than six, and so the six cannot drift apart — this is the one code
    // path whose job is to not lose anything.
    //
    // Tasks are the one table with a write seam (`write_task`) and this does not
    // use it, on purpose. The seam upserts and returns a Result; the burn needs
    // the generated id back, and inserts a row it deletes in the next statement,
    // so the row never becomes state any invariant is about. Giving the macro a
    // task-shaped special case would cost the uniformity that is its whole
    // point. What keeps it honest instead is `blank_task`'s SCRATCH_OWNER, which
    // `the_blank_task_the_burn_throws_away_satisfies_the_invariant` pins.
    macro_rules! burn_table {
        ($accessor:ident, $blank:expr) => {
            burn(ceiling, || {
                let id = ctx.db.$accessor().insert($blank).id;
                ctx.db.$accessor().id().delete(id);
                id
            })
        };
    }

    match table.as_str() {
        "tasks" => burn_table!(tasks, blank_task()),
        "epics" => burn_table!(epics, blank_epic()),
        "todos" => burn_table!(todos, blank_todo()),
        "task_watchers" => burn_table!(task_watchers, blank_watcher()),
        "repo_paths" => burn_table!(repo_paths, blank_repo_path()),
        "repo_base_branches" => burn_table!(repo_base_branches, blank_repo_base_branch()),
        // Found in passing while adding the two arms below: `poll_owners` DOES
        // generate ids (`SharedTable::id_column` says so, and
        // `restore.rs::burn_id_sequences` filters on exactly that), but had no
        // arm here — a restore carrying any poll_owners row would call this
        // with table="poll_owners" and hit the `unknown table` error below.
        // Pre-existing since Phase 7 (task #4865); fixed here rather than only
        // reported, since it is the same one-line shape as every other arm.
        "poll_owners" => burn_table!(poll_owners, blank_poll_owner()),
        "learnings" => burn_table!(learnings, blank_learning()),
        "learning_retrievals" => burn_table!(learning_retrievals, blank_learning_retrieval()),
        "usage_events" => burn_table!(usage_events, blank_usage_event()),
        // Same gap as the `poll_owners` one above, same fix: `retired_feed_items`
        // (task #4971) generates ids and had no arm here, so a restore or seed
        // carrying any of its rows would hit the `unknown table` error below —
        // caught by `seeding_a_board_puts_its_rows_on_that_persons_store_backed_board`
        // once task #4916's seed path started burning every generating table.
        "retired_feed_items" => burn_table!(retired_feed_items, blank_retired_feed_item()),
        // `task_shells`, `task_subagents`, `hosts`, `subscriptions`, `settings`
        // and `filter_presets` generate no ids, so there is nothing to burn.
        // Accepted rather than rejected so a caller can loop over every shared
        // table without a special case.
        "task_shells" | "task_subagents" | "hosts" | "subscriptions" | "settings"
        | "filter_presets" => {}
        other => return Err(format!("unknown table {other}")),
    }
    Ok(())
}

/// Insert a blank task asking the store for an id, and leave it there.
///
/// The verification tool for a restore, and the only way to answer the question
/// that matters: **does the next task created collide with a restored one?**
/// Asserting that the burn loop ran proves the code calls itself; asserting on
/// the id this hands back proves the property. The caller reads the id back out
/// of `tasks` and deletes the row.
#[spacetimedb::reducer]
pub fn probe_generated_task_id(ctx: &ReducerContext) -> Result<(), String> {
    write_task(
        ctx,
        Task {
            title: "probe".into(),
            ..blank_task()
        },
    )
}

/// Generate and discard until the counter is strictly past `ceiling`.
///
/// `generate` returns the id it was handed, so the loop can stop on the value
/// rather than on a count — a counter that started above zero, or one that
/// skipped a block, both end this loop correctly.
pub(crate) fn burn(ceiling: i64, mut generate: impl FnMut() -> i64) {
    // Strictly greater, not equal: a counter sitting exactly on the ceiling
    // hands that id out next, which is the same collision one iteration later.
    while generate() < ceiling {}
}
