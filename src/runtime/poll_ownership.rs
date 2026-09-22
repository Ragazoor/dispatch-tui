//! The host-scoping decision shared by `PollPrStatus`'s host-less-task path
//! (`src/runtime/pr.rs::exec_check_status_if_owned`) and `FeedTick`'s per-epic
//! gate (`src/feed/mod.rs`). `core.allium: PollOwner`.
//!
//! Pure on purpose, the same reasoning `derive_epic_status` in the SpacetimeDB
//! module is pure for: the interesting question here is "given what the
//! store answered, what should THIS host do", not how the answer was
//! fetched — so the decision is testable without a live store or a mock of
//! one, and the two callers (task scope, epic scope) share one
//! implementation instead of two copies that could drift.

/// What a host should do this tick for a scope whose current `PollOwner` row
/// names `current_owner` (or names no one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PollAction {
    /// A different host already owns this scope. Do nothing: no claim call,
    /// no poll/feed-cycle, this tick or any future one — until a human or
    /// agent explicitly reassigns ownership
    /// (`pr-workflow.allium: OverridePrPollOwner`,
    /// `feeds.allium: OverrideFeedOwner`).
    Skip,
    /// No one owns this scope yet. Claim it, then act as this tick's owner.
    ClaimAndProceed,
    /// This host already owns it. Act, without claiming again.
    Proceed,
}

/// `current_owner` is the `Host.id` a `core/PollOwner` lookup returned for
/// this scope, or `None` if the scope is unclaimed. `host_id` is this
/// machine's own `Host.id`.
pub(crate) fn decide_poll_action(current_owner: Option<&str>, host_id: &str) -> PollAction {
    match current_owner {
        None => PollAction::ClaimAndProceed,
        Some(owner) if owner == host_id => PollAction::Proceed,
        Some(_) => PollAction::Skip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unclaimed_scope_is_claimed_and_proceeded_with() {
        assert_eq!(decide_poll_action(None, "me"), PollAction::ClaimAndProceed);
    }

    #[test]
    fn a_scope_this_host_already_owns_proceeds_without_reclaiming() {
        assert_eq!(decide_poll_action(Some("me"), "me"), PollAction::Proceed);
    }

    #[test]
    fn a_scope_another_host_owns_is_skipped() {
        assert_eq!(
            decide_poll_action(Some("someone-else"), "me"),
            PollAction::Skip
        );
    }
}
