use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// The store's spelling of "absent", named so the assertions below read as
    /// the domain statements they are rather than as bare literals. See
    /// "Absence is a sentinel, not a null" in this file's header.
    const NO_EPIC: i64 = 0;
    const NO_OWNER: &str = "";

    /// A task with no epic sits on somebody's user board, and the row has to
    /// say whose. Nothing else in it can answer.
    #[test]
    fn an_epicless_task_needs_an_owner() {
        assert!(validate_task_ownership(NO_EPIC, NO_OWNER).is_err());
        assert!(validate_task_ownership(NO_EPIC, "user-1").is_ok());
    }

    /// The other arm, and the one that is easy to leave out: an owner on a task
    /// that already has an epic is REJECTED, not ignored. Two answers to
    /// "whose board is this?" place the same card on two boards, and neither
    /// reader looks wrong locally.
    #[test]
    fn an_epic_task_must_not_carry_an_owner() {
        assert!(validate_task_ownership(7, "user-1").is_err());
        assert!(validate_task_ownership(7, NO_OWNER).is_ok());
    }

    /// Whitespace is not an identity either.
    ///
    /// The empty string is the sentinel, so the first assertion is really about
    /// the sentinel doing its job. The second is the one that earns its keep: a
    /// space passes every test for presence — non-empty, non-sentinel — while
    /// answering nothing.
    #[test]
    fn an_empty_owner_is_not_an_owner() {
        assert!(validate_task_ownership(NO_EPIC, "").is_err());
        assert!(validate_task_ownership(NO_EPIC, "   ").is_err());
    }

    /// The message names which arm failed. A seeding run that trips this is
    /// reading a snapshot taken before the field existed, and "invalid task"
    /// would leave the operator no way to tell which of the two arms to fix.
    #[test]
    fn the_refusal_says_which_arm_failed() {
        let missing = validate_task_ownership(NO_EPIC, NO_OWNER).unwrap_err();
        assert!(missing.contains("no epic"), "{missing}");

        let surplus = validate_task_ownership(7, "user-1").unwrap_err();
        assert!(surplus.contains("epic"), "{surplus}");
        assert_ne!(missing, surplus);
    }

    // -----------------------------------------------------------------
    // Epic status derivation (epics.allium: EpicStatusRecalculation)
    // -----------------------------------------------------------------

    fn children(statuses: &[&str]) -> Vec<String> {
        statuses.iter().map(|s| (*s).to_string()).collect()
    }

    /// The forward auto-transition. Every active child done means the epic is
    /// done, and this is the only way an epic reaches done without somebody
    /// moving it.
    #[test]
    fn all_active_children_done_makes_the_epic_done() {
        assert_eq!(
            derive_epic_status("backlog", &children(&["done", "done"])),
            Some("done")
        );
    }

    /// The backward one. A done epic that gains an unfinished child is not
    /// done, and saying so is what keeps the column honest when work reopens.
    #[test]
    fn a_done_epic_with_an_unfinished_child_regresses_to_backlog() {
        assert_eq!(
            derive_epic_status("done", &children(&["done", "running"])),
            Some("backlog")
        );
    }

    /// Neither transition fires, so nothing is written. The distinction between
    /// "no change" and "write the same value" is not cosmetic here: a write
    /// would stamp `updated_at` and, on the done arm, `completed_at`.
    #[test]
    fn a_running_child_leaves_a_backlog_epic_alone() {
        assert_eq!(derive_epic_status("backlog", &children(&["running"])), None);
    }

    /// Manual placements survive. `running` and `review` are never derived, so
    /// an epic a person moved there stays there until they move it back.
    #[test]
    fn a_manual_placement_survives_a_recalculation() {
        assert_eq!(derive_epic_status("review", &children(&["running"])), None);
        assert_eq!(derive_epic_status("running", &children(&["backlog"])), None);
    }

    /// ...but the forward transition still overrides one. A review epic whose
    /// children all finished is finished.
    #[test]
    fn all_done_overrides_even_a_manual_placement() {
        assert_eq!(
            derive_epic_status("review", &children(&["done"])),
            Some("done")
        );
    }

    /// No children is NOT "all children done". An epic with nothing in it
    /// keeps whatever status it has — a freshly created one is backlog and
    /// must not be born done.
    #[test]
    fn an_epic_with_no_children_keeps_its_status() {
        assert_eq!(derive_epic_status("backlog", &children(&[])), None);
        assert_eq!(derive_epic_status("running", &children(&[])), None);
    }

    /// THE TWO-HOST CASE, as a property of the function rather than of a
    /// deployment. Each board sees a subset; the store sees the union. The
    /// subsets disagree and the union is right, which is the whole argument for
    /// deriving this at the store (sync.allium: TheStoreEnforcesTheSharedRules).
    #[test]
    fn two_partial_views_disagree_and_the_whole_view_decides() {
        let host_a_sees = children(&["done"]);
        let host_b_sees = children(&["running"]);
        let the_store_sees = children(&["done", "running"]);

        assert_eq!(derive_epic_status("backlog", &host_a_sees), Some("done"));
        assert_eq!(derive_epic_status("backlog", &host_b_sees), None);
        assert_eq!(derive_epic_status("backlog", &the_store_sees), None);

        // And the same disagreement from the other side: a done epic that host
        // A would leave alone and host B would regress.
        assert_eq!(derive_epic_status("done", &host_a_sees), None);
        assert_eq!(derive_epic_status("done", &host_b_sees), Some("backlog"));
        assert_eq!(derive_epic_status("done", &the_store_sees), Some("backlog"));
    }

    /// An unknown status is not a silent "not done". A board running a newer
    /// binary can write a status this module does not know, and treating it as
    /// unfinished would hold an epic open forever with nothing saying why.
    #[test]
    fn an_unknown_child_status_is_refused_rather_than_guessed() {
        assert_eq!(
            derive_epic_status("backlog", &children(&["done", "quantum"])),
            None
        );
    }

    // -----------------------------------------------------------------
    // The dispatch claim (dispatch.allium: DispatchClaimExclusive)
    // -----------------------------------------------------------------

    /// A task nobody has dispatched belongs to whoever gets to it.
    #[test]
    fn an_unowned_task_is_claimable_by_anyone() {
        let task = blank_task();
        assert!(claimable_by(&task, "host-a"));
        assert!(claimable_by(&task, "host-b"));
    }

    /// A task whose worktree sits on another machine is not. Claiming it would
    /// dispatch an agent with nowhere to work.
    #[test]
    fn a_foreign_owned_task_is_passed_over() {
        let task = Task {
            host: "host-a".into(),
            ..blank_task()
        };
        assert!(claimable_by(&task, "host-a"));
        assert!(!claimable_by(&task, "host-b"));
    }

    // -----------------------------------------------------------------
    // Patches reach every field
    // -----------------------------------------------------------------
    //
    // THE GUARANTEE `apply_patch!` DOES NOT GIVE. Leaving a name out of an
    // invocation compiles: the patch field is `pub`, so there is no dead-field
    // warning, and that column just silently stops being patchable. Nothing
    // else in the suite would notice — a patch reducer that ignores one field
    // still returns Ok. So each of these builds a patch with EVERY field set to
    // a value the blank row does not hold, applies it, and asserts the blank is
    // gone everywhere. A column added to the struct and forgotten in the macro
    // fails here.

    /// A distinctive value for every `String` field, so "unchanged" is
    /// unmistakable.
    const MARK: &str = "patched";

    #[test]
    fn every_field_of_a_task_patch_reaches_the_row() {
        let mut row = blank_task();
        row.id = 7;
        let before = row.clone();

        apply_task_patch(
            &mut row,
            TaskPatch {
                title: Some(MARK.into()),
                description: Some(MARK.into()),
                repo_path: Some(MARK.into()),
                status: Some(DONE.into()),
                worktree: Some(MARK.into()),
                tmux_window: Some(MARK.into()),
                plan_path: Some(MARK.into()),
                epic_id: Some(9),
                sub_status: Some(MARK.into()),
                tag: Some(MARK.into()),
                sort_order: Some(Some(3)),
                base_branch: Some(MARK.into()),
                external_id: Some(MARK.into()),
                labels: Some("[\"x\"]".into()),
                last_pre_tool_use_at: Some(MARK.into()),
                last_notification_at: Some(MARK.into()),
                wrap_up_mode: Some(MARK.into()),
                url: Some(MARK.into()),
                url_type: Some(MARK.into()),
                pr_learnings_gate_shown_at: Some(MARK.into()),
                auto_run_plan: Some(true),
                live_subagents: Some(2),
                stop_pending: Some(true),
                stop_pending_at: Some(MARK.into()),
                last_peer_message_sent_at: Some(MARK.into()),
                last_peer_message_received_at: Some(MARK.into()),
                phoenix: Some(true),
                host: Some(MARK.into()),
                owner: Some(MARK.into()),
                completed_at: Some(MARK.into()),
            },
        );

        // The id is not patchable and must be untouched.
        assert_eq!(row.id, before.id);
        // Everything else moved.
        assert_eq!(row.title, MARK);
        assert_eq!(row.description, MARK);
        assert_eq!(row.repo_path, MARK);
        assert_eq!(row.status, DONE);
        assert_eq!(row.worktree, MARK);
        assert_eq!(row.tmux_window, MARK);
        assert_eq!(row.plan_path, MARK);
        assert_eq!(row.epic_id, 9);
        assert_eq!(row.sub_status, MARK);
        assert_eq!(row.tag, MARK);
        assert_eq!(row.sort_order, Some(3));
        assert_eq!(row.base_branch, MARK);
        assert_eq!(row.external_id, MARK);
        assert_eq!(row.labels, "[\"x\"]");
        assert_eq!(row.last_pre_tool_use_at, MARK);
        assert_eq!(row.last_notification_at, MARK);
        assert_eq!(row.wrap_up_mode, MARK);
        assert_eq!(row.url, MARK);
        assert_eq!(row.url_type, MARK);
        assert_eq!(row.pr_learnings_gate_shown_at, MARK);
        assert!(row.auto_run_plan);
        assert_eq!(row.live_subagents, 2);
        assert!(row.stop_pending);
        assert_eq!(row.stop_pending_at, MARK);
        assert_eq!(row.last_peer_message_sent_at, MARK);
        assert_eq!(row.last_peer_message_received_at, MARK);
        assert!(row.phoenix);
        assert_eq!(row.host, MARK);
        assert_eq!(row.owner, MARK);
        assert_eq!(row.completed_at, MARK);
        // created_at/updated_at are not patch fields; the reducer stamps them.
        assert_eq!(row.created_at, before.created_at);
    }

    #[test]
    fn every_field_of_an_epic_patch_reaches_the_row() {
        let mut row = blank_epic();
        row.id = 7;

        apply_epic_patch(
            &mut row,
            EpicPatch {
                title: Some(MARK.into()),
                description: Some(MARK.into()),
                status: Some(DONE.into()),
                plan_path: Some(MARK.into()),
                sort_order: Some(Some(3)),
                auto_dispatch: Some(true),
                parent_epic_id: Some(9),
                feed_command: Some(MARK.into()),
                feed_interval_secs: Some(60),
                group_by_repo: Some(true),
                feed_role: Some(MARK.into()),
                origin: Some(MARK.into()),
                feed_append_only: Some(true),
                completed_at: Some(MARK.into()),
            },
        );

        assert_eq!(row.id, 7);
        assert_eq!(row.title, MARK);
        assert_eq!(row.description, MARK);
        assert_eq!(row.status, DONE);
        assert_eq!(row.plan_path, MARK);
        assert_eq!(row.sort_order, Some(3));
        assert!(row.auto_dispatch);
        assert_eq!(row.parent_epic_id, 9);
        assert_eq!(row.feed_command, MARK);
        assert_eq!(row.feed_interval_secs, 60);
        assert!(row.group_by_repo);
        assert_eq!(row.feed_role, MARK);
        assert_eq!(row.origin, MARK);
        assert!(row.feed_append_only);
        assert_eq!(row.completed_at, MARK);
    }

    /// The other half: an EMPTY patch changes nothing at all. Without this, a
    /// macro arm that wrote unconditionally would pass the tests above.
    #[test]
    fn an_empty_patch_changes_no_field() {
        let mut row = blank_task();
        row.title = "original".into();
        row.epic_id = 4;
        row.host = "host-a".into();
        let before = row.clone();

        apply_task_patch(&mut row, TaskPatch::default());

        assert_eq!(row.title, before.title);
        assert_eq!(row.epic_id, before.epic_id);
        assert_eq!(row.host, before.host);
        assert_eq!(row.status, before.status);
        assert_eq!(row.sort_order, before.sort_order);
    }

    // -----------------------------------------------------------------
    // Timestamps
    // -----------------------------------------------------------------

    /// The store's instant, in SQLite's spelling.
    ///
    /// Every timestamp column in this module is TEXT in the format the SQLite
    /// side writes, because the two stores' rows have to compare equal — the
    /// schema parity test and the seed round-trip both depend on it. A format
    /// that drifted by one character would pass every test that reads a
    /// timestamp back through the same parser and fail the one that compares
    /// stores.
    #[test]
    fn a_timestamp_is_formatted_the_way_sqlite_writes_one() {
        // 2026-09-19T12:34:56.789Z
        assert_eq!(
            format_timestamp_micros(1_789_821_296_789_000),
            "2026-09-19 12:34:56.789"
        );
    }

    /// Milliseconds, always three of them, padded. A truncating formatter that
    /// dropped a trailing zero would sort wrong as text, which is exactly how
    /// the Done column reads these.
    #[test]
    fn the_milliseconds_are_always_three_digits() {
        assert_eq!(
            format_timestamp_micros(1_789_821_296_700_000),
            "2026-09-19 12:34:56.700"
        );
        assert_eq!(
            format_timestamp_micros(1_789_821_296_000_000),
            "2026-09-19 12:34:56.000"
        );
    }

    /// Sub-millisecond precision is dropped rather than rounded, matching the
    /// SQLite side's `trunc_subsecs(3)`.
    #[test]
    fn sub_millisecond_precision_is_truncated() {
        assert_eq!(
            format_timestamp_micros(1_789_821_296_789_999),
            "2026-09-19 12:34:56.789"
        );
    }

    // -----------------------------------------------------------------
    // completed_at
    // -----------------------------------------------------------------

    /// Only the transition INTO done stamps it, mirroring tasks.allium's
    /// ConfirmDone.
    #[test]
    fn only_entering_done_stamps_a_completion() {
        assert!(stamps_completion("backlog", "done"));
        assert!(!stamps_completion("done", "backlog"));
        assert!(!stamps_completion("backlog", "running"));
    }

    /// The regression does not clear it. `completed_at` records the last
    /// completion and a reopening does not unmake one.
    #[test]
    fn a_regression_does_not_clear_the_completion() {
        assert!(!stamps_completion("done", "backlog"));
    }

    /// A blank task is the row `burn_id_sequence` throws away and the row
    /// `probe_generated_task_id` writes. It must satisfy the invariant or
    /// neither works: a blank has no epic, so it needs an owner. This is what
    /// makes SCRATCH_OWNER load-bearing rather than decorative.
    #[test]
    fn the_blank_task_the_burn_throws_away_satisfies_the_invariant() {
        let blank = blank_task();
        assert!(validate_task_ownership(blank.epic_id, &blank.owner).is_ok());
    }
}
