use super::*;

#[cfg(test)]
mod task_rows;

/// The data rules of store-split.allium step M2 (task #53255), as the pure
/// functions the reducers call. See `rules.rs`.
#[cfg(test)]
mod rules;

/// The dispatch-request rules of store-split.allium step M4 (task #53257), as
/// the pure functions the reducers call (`dispatch_request.rs`).
///
/// A unit test cannot build a `ReducerContext`, so each reducer's DECISION is
/// pinned here as a pure function over rows, and the table writes, the
/// transaction and the scheduled timer are pinned end to end by the live-store
/// tests in `tests/spacetime_module.rs` ("store-split M4"). The backstop timer
/// in particular cannot be time-travelled on a live store without a wall-clock
/// wait, so what it does to a row whose deadline has passed is pinned HERE, on
/// the pure pieces `expire_dispatch_request` composes
/// ([`dispatch_request_is_open`] and [`failed_request_row`]), and end to end
/// by the in-memory double's stand-in (`MemoryReducerCaller::expire_dispatch_request`).
#[cfg(test)]
mod dispatch_requests;

/// The session-request rules of store-split.allium step M5 (task #53258), as
/// the pure functions the reducers call (`session_request.rs`).
///
/// As for M4's `dispatch_requests` above: a unit test cannot build a
/// `ReducerContext`, so each reducer's DECISION is pinned here as a pure
/// function over rows, and the table writes, the transaction and the scheduled
/// timer are pinned end to end by the live-store tests in
/// `tests/spacetime_module.rs` ("store-split M5"). The backstop cannot be
/// time-travelled on a live store, so what it does to a row whose deadline has
/// passed is pinned HERE, on the body `expire_session_request` runs
/// ([`expire_session_request_row`]).
#[cfg(test)]
mod session_requests;
