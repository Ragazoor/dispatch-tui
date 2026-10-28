//! The startup version gate: does the store enforce the data rules this board
//! was built to expect?
//!
//! Spec: `docs/specs/sync.allium`'s `AStoreWithOtherRulesFailsTheAttempt`; the
//! decision and its alternatives are in `docs/specs/store-split.allium`.
//!
//! The comparison is one integer for equality. The board's side is
//! [`BOARD_RULES_VERSION`], below; the store's is the `rules_version` its
//! module stamps into its `rules_version` table from `RULES_VERSION` in
//! `spacetime/module/src/rules.rs`. The board cannot link the module crate (a
//! release build deliberately does not pull in the SpacetimeDB dependency
//! tree), so the two constants are separate and a test compares them: bumping
//! one without the other fails `cargo test`, which is the same coupling a shared
//! constant would give.

/// The data-rules version this board build expects the store to enforce.
///
/// **Hand-bumped, in the commit that moves a rule into the module, together with
/// `RULES_VERSION` in the module crate** — see that constant for what does and
/// does not warrant a bump.
pub const BOARD_RULES_VERSION: i64 = 16;

/// What the operator reads when the store and the board disagree.
///
/// Says which side is behind, because the fix differs: a stale board is
/// updated, a stale module is republished. `0` is what a module published
/// before the gate existed reads as.
pub fn rules_version_mismatch_message(store: i64, board: i64) -> String {
    if store < board {
        format!(
            "the store's module enforces data rules version {store}, but this board \
             expects version {board}: the module is out of date. Republish the module \
             this board was built with (a managed store does this on launch)."
        )
    } else {
        format!(
            "the store's module enforces data rules version {store}, but this board \
             only knows version {board}: this board is out of date. Update dispatch \
             before connecting, or it would apply rules the store has moved on from."
        )
    }
}

/// What the operator reads when the store refuses to say which rules it
/// enforces: a module published before the version gate has no table to read.
pub fn unreadable_rules_version_message(store_error: &str) -> String {
    format!(
        "could not read the store's data rules version ({store_error}). A module \
         published before the version gate has none: {}",
        rules_version_mismatch_message(0, BOARD_RULES_VERSION)
    )
}

/// Refuse a store whose published rules version is not this board's.
pub fn check_rules_version(store: i64) -> Result<(), String> {
    if store == BOARD_RULES_VERSION {
        Ok(())
    } else {
        Err(rules_version_mismatch_message(store, BOARD_RULES_VERSION))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The board's number and the module's are bumped together, or this fails.
    #[test]
    fn the_board_and_the_module_agree_on_the_rules_version() {
        assert_eq!(
            BOARD_RULES_VERSION,
            dispatch_spacetime_module::RULES_VERSION,
            "bump both when a rule moves into the module"
        );
    }

    #[test]
    fn a_store_on_the_same_rules_version_is_accepted() {
        assert!(check_rules_version(BOARD_RULES_VERSION).is_ok());
    }

    #[test]
    fn a_store_behind_the_board_is_refused_and_told_to_republish() {
        let why = check_rules_version(BOARD_RULES_VERSION - 1).unwrap_err();
        assert!(why.contains("module is out of date"), "{why}");
        assert!(why.contains("Republish"), "{why}");
    }

    #[test]
    fn a_store_ahead_of_the_board_is_refused_and_told_to_update_the_board() {
        let why = check_rules_version(BOARD_RULES_VERSION + 1).unwrap_err();
        assert!(why.contains("board is out of date"), "{why}");
        assert!(why.contains("Update dispatch"), "{why}");
    }

    /// A module published before the gate existed publishes no version, which
    /// the column reads as 0. No build expects 0, so it is refused.
    #[test]
    fn a_module_that_predates_the_gate_is_refused() {
        const { assert!(BOARD_RULES_VERSION > 0) };
        assert!(check_rules_version(0).is_err());
    }

    #[test]
    fn an_unreadable_version_says_why_and_what_to_do() {
        let why = unreadable_rules_version_message("no such table: `rules_version`");
        assert!(why.contains("no such table"), "{why}");
        assert!(
            why.contains("predates") || why.contains("before the version gate"),
            "{why}"
        );
        assert!(why.contains("Republish"), "{why}");
    }

    #[test]
    fn the_message_names_both_versions() {
        let why = rules_version_mismatch_message(3, 7);
        assert!(why.contains('3') && why.contains('7'), "{why}");
    }

    // -- the board's models and the module's rule functions ------------------
    //
    // `repo_name_from_path` names a feed's repo groups on the board, and the
    // module's `repo_name_from_url` names them in the store; the two are
    // checked against each other so they cannot drift. Sub-status legality has
    // no board twin: the board's tests read the module's `validate_sub_status`
    // directly.

    #[test]
    fn the_module_and_the_boards_models_agree_on_repo_group_names() {
        for path in [
            "/home/u/dispatch",
            "/home/u/dispatch/",
            "dispatch",
            "",
            "/",
            "//",
            "/a/b c/d.e",
            "/home/u/dispatch/.",
            "/home/u/dispatch/./",
            "./dispatch",
            ".",
            "..",
            "/home/u/..",
        ] {
            assert_eq!(
                dispatch_spacetime_module::repo_name_from_path(path),
                crate::models::repo_name_from_path(path),
                "{path:?}"
            );
        }
    }
}
