use super::*;

#[cfg(test)]
mod task_rows;

/// The task and epic data rules (`tasks.allium`, `epics.allium`), as the pure
/// functions the reducers call. See `rules.rs`.
#[cfg(test)]
mod rules;

/// The dispatch-request rules of store-split.allium (D1-D12), as
/// the pure functions the reducers call (`dispatch_request.rs`).
///
/// A unit test inside this crate cannot build a `ReducerContext`, so each
/// reducer's DECISION is pinned here as a pure function over rows. The table
/// writes, the transaction and the scheduled timer are pinned end to end by the
/// in-memory reducer tests in `spacetime/module-tests/tests/dispatch_requests.rs`,
/// which advance `TestClock` past the deadline and run `expire_dispatch_request`,
/// and by the live-store tests in `tests/spacetime_module.rs` (the dispatch request tests).
/// What the backstop does to a row whose deadline has passed is also pinned HERE,
/// on the pure pieces it composes ([`dispatch_request_is_open`] and
/// [`failed_request_row`]), and by the in-memory double's stand-in
/// (`MemoryReducerCaller::expire_dispatch_request`).
#[cfg(test)]
mod dispatch_requests;

/// The session-request rules of store-split.allium (W1-W13), as
/// the pure functions the reducers call (`session_request.rs`).
///
/// As for `dispatch_requests` above: a unit test inside this crate cannot
/// build a `ReducerContext`, so each reducer's DECISION is pinned here as a pure
/// function over rows. The table writes, the transaction and the scheduled timer
/// are pinned end to end by `spacetime/module-tests/tests/session_requests.rs`
/// (which time-travels the backstop with `TestClock`) and by the live-store tests
/// in `tests/spacetime_module.rs` (the session request tests). What the backstop does to a
/// row whose deadline has passed is also pinned HERE, on the body
/// `expire_session_request` runs ([`expire_session_request_row`]).
#[cfg(test)]
mod session_requests;

/// The feed-request rules (`feed_request.rs`) as pure functions over rows.
#[cfg(test)]
mod feed_requests;

/// The agent rules of agents.allium step A1 (task #90115), generic over
/// `AgentStore` and run here over a plain in-test store (`agent.rs`).
#[cfg(test)]
mod agents;

/// The scheduled-task rules of schedules.allium (task #98394): the cron
/// grammar, arming and clearing, the dispatch-request checks, and what a
/// launch's outcome and a phoenix respawn do to the schedule.
#[cfg(test)]
mod schedules;

/// `admin-surfaces.allium: UnseenItemListing`, as the pure function the
/// `unseen_items` view runs (`visibility.rs`).
#[cfg(test)]
mod unseen_items;

/// `docs/architecture.md`, "Read enforcement: per-caller views", as the functions the views run
/// (`views.rs`), over a plain in-test store.
#[cfg(test)]
mod views;
