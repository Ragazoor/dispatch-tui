//! Shared helpers: timestamps, the status vocabulary and epic status derivation.

use super::*;

// Timestamps

/// SQLite's timestamp spelling, which both stores write.
pub(crate) const SQLITE_TIMESTAMP: &str = "%Y-%m-%d %H:%M:%S%.3f";

/// Format an instant the way the SQLite side does.
///
/// Takes micros rather than a [`Timestamp`] so it is testable without a store;
/// [`now`] is the one-line adapter the reducers use.
///
/// An instant outside chrono's representable range formats as the empty string
/// — the module's spelling of absent. It cannot happen with a real store clock,
/// and the alternative is a panic inside a reducer, which aborts the whole
/// transaction over a timestamp.
pub(crate) fn format_timestamp_micros(micros: i64) -> String {
    match chrono::DateTime::from_timestamp_micros(micros) {
        Some(dt) => dt.format(SQLITE_TIMESTAMP).to_string(),
        None => String::new(),
    }
}

/// The store's clock, in the store's timestamp format.
///
/// `ctx.timestamp` rather than any host clock: it is the same instant for every
/// row a reducer writes, which is what makes a multi-row mutation carry one
/// consistent `updated_at` instead of several that happen to be close.
pub(crate) fn now(ctx: &ReducerContext) -> String {
    format_timestamp_micros(ctx.timestamp.to_micros_since_unix_epoch())
}

// Epic status derivation

/// The statuses an epic or a task can hold, in the store's spelling.
///
/// Listed rather than parsed loosely, so an unrecognised one is a REFUSAL
/// rather than a value that falls through to a plausible branch. See
/// `derive_epic_status`'s unknown-status arm for why that matters here in
/// particular. There is no `archived` any more — `epics.allium:
/// ArchivedStatusMigration` retired it (task #4971): a finished epic either
/// stays in `done` or is deleted.
pub(crate) const DONE: &str = "done";

pub(crate) const BACKLOG: &str = "backlog";

pub(crate) const KNOWN_STATUSES: [&str; 4] = [BACKLOG, "running", "review", DONE];

/// Refuse a task or epic row whose status is not one of [`KNOWN_STATUSES`].
///
/// `spacetime-seed.allium: StoreRefusesUnknownStatus`. Called on the RESULTING
/// row of every write, so a patch that moves a legacy row to a known status
/// succeeds and one that leaves it unknown is refused. Deleting is not a write
/// and is not checked. `kind` is "task" or "epic".
pub(crate) fn validate_status(kind: &str, id: i64, status: &str) -> Result<(), String> {
    if KNOWN_STATUSES.contains(&status) {
        Ok(())
    } else {
        Err(format!(
            "{kind} {id}: unknown status {status:?}; expected one of {KNOWN_STATUSES:?}"
        ))
    }
}

/// Derive an epic's status from its children's, or `None` for "leave it alone".
///
/// The whole of `epics.allium: EpicStatusRecalculation`'s derivation, as a pure
/// function over the child statuses. It is pure on purpose: this is the one
/// piece of logic the migration moved server-side, and the argument for moving
/// it is about WHICH children are visible rather than about how the answer is
/// computed — so the computation is testable without a store, and the
/// visibility is what the reducer below supplies.
///
/// `children` carries every child's status, tasks and sub-epics alike.
///
/// `None` means no write. That is distinct from writing the same value back: a
/// write stamps `updated_at`, and on the forward arm `completed_at` too.
pub fn derive_epic_status(current: &str, children: &[String]) -> Option<&'static str> {
    // An unknown status anywhere is a refusal, not a guess. The realistic
    // producer is a board running a newer binary than this module, and both
    // guesses are wrong in a way nobody can see: treating it as done finishes
    // an epic that is not finished, and treating it as unfinished holds one
    // open forever. Declining to write leaves the epic where it is and leaves
    // the next recalculation — by then perhaps against an updated module — free
    // to get it right.
    if children
        .iter()
        .any(|s| !KNOWN_STATUSES.contains(&s.as_str()))
    {
        return None;
    }

    // No children is NOT all-done. A freshly created epic has none and must
    // not be born done.
    if children.is_empty() {
        return None;
    }
    if children.iter().all(|s| s == DONE) {
        return (current != DONE).then_some(DONE);
    }
    // The regression: a done epic with an unfinished child is not done.
    if current == DONE {
        return Some(BACKLOG);
    }
    None
}

/// Whether moving from `prior` to `next` stamps a fresh completion.
///
/// The module's copy of `models::completed_at_for_status_transition`, on the
/// same two rules: only the transition INTO done stamps one, and a regression
/// out of done leaves the old stamp standing because `completed_at` records the
/// last completion rather than the current state.
pub fn stamps_completion(prior: &str, next: &str) -> bool {
    prior != DONE && next == DONE
}

// -- Agent session state (Phase 6b) ------------------------------------------

/// The statuses/sub-statuses this section's reducers read or write, in the
/// store's spelling. Named for the same reason `DONE`/`BACKLOG` above are:
/// every other status/sub-status is passed in by the caller
/// (`sub_status` in `record_pre_tool_use`, `status` inside a full row), so
/// only the values these reducers themselves decide need a name here.
pub(crate) const RUNNING: &str = "running";

pub(crate) const REVIEW: &str = "review";

pub(crate) const ACTIVE: &str = "active";

pub(crate) const AWAITING_REVIEW: &str = "awaiting_review";

pub(crate) const NEEDS_INPUT: &str = "needs_input";
