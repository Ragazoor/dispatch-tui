use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// The store's spelling of "absent", named so the assertions below read as
    /// the domain statements they are rather than as bare literals. See
    /// "Absence is a sentinel, not a null" in this file's header.
    const NO_EPIC: i64 = 0;
    const NO_OWNER: &str = "";

    /// `spacetime-seed.allium: StoreRefusesUnknownStatus`. The module is the
    /// one layer no client can bypass, so it names the four statuses it
    /// holds and refuses the rest, `archived` included.
    /// `work-packages.allium: OneLinkPerPackageAndItem` rests on this key: a
    /// task and an epic sharing an id are different items, so the kind is
    /// part of it.
    #[test]
    fn a_work_package_link_key_names_package_kind_and_item() {
        assert_eq!(
            work_package_link_id(1, WORK_PACKAGE_ITEM_TASK, 3),
            "1/task/3"
        );
        assert_ne!(
            work_package_link_id(1, WORK_PACKAGE_ITEM_TASK, 3),
            work_package_link_id(1, WORK_PACKAGE_ITEM_EPIC, 3)
        );
    }

    #[test]
    fn the_four_known_statuses_are_accepted() {
        for status in ["backlog", "running", "review", "done"] {
            assert!(validate_status("task", 1, status).is_ok(), "{status}");
        }
    }

    #[test]
    fn the_retired_archived_status_is_refused_naming_status_and_id() {
        let why = validate_status("epic", 100, "archived").unwrap_err();
        assert!(why.contains("archived"), "{why}");
        assert!(why.contains("100"), "{why}");
        assert!(why.contains("epic"), "{why}");
    }

    #[test]
    fn an_empty_or_misspelled_status_is_refused() {
        for status in ["", "Done", "in_progress", " done"] {
            assert!(validate_status("task", 1, status).is_err(), "{status:?}");
        }
    }

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

/// The data rules of store-split.allium step M2 (task #53255), as the pure
/// functions the reducers call. See `rules.rs`.
#[cfg(test)]
mod rules {
    use super::*;
    use std::collections::HashMap;

    // Which pairs are legal is checked pair by pair against the board's own
    // `SubStatus::is_valid_for` in `src/sync/rules_gate.rs`; here, only the
    // refusal's wording.

    /// The wording reaches MCP callers unchanged from the service that used to
    /// produce it.
    #[test]
    fn an_illegal_sub_status_is_refused_naming_both_values() {
        assert_eq!(
            validate_sub_status("backlog", "active").unwrap_err(),
            "sub_status 'active' is not valid for status 'backlog'"
        );
    }

    #[test]
    fn an_unknown_sub_status_is_refused() {
        assert!(validate_sub_status("running", "bogus").is_err());
    }

    fn chain(edges: &[(i64, i64)]) -> impl Fn(i64) -> Option<i64> {
        let map: HashMap<i64, i64> = edges.iter().copied().collect();
        move |id| map.get(&id).copied()
    }

    const CYCLE: &str = "Setting this parent would create a cycle in the epic hierarchy";

    #[test]
    fn an_epic_cannot_be_its_own_parent() {
        assert_eq!(validate_epic_parent(1, 1, chain(&[])).unwrap_err(), CYCLE);
    }

    #[test]
    fn an_epic_cannot_move_under_its_own_descendant() {
        // 3 -> 2 -> 1: moving 1 under 3 closes the loop.
        let parents = chain(&[(3, 2), (2, 1)]);
        assert_eq!(validate_epic_parent(1, 3, parents).unwrap_err(), CYCLE);
    }

    #[test]
    fn an_epic_can_move_under_an_unrelated_epic() {
        assert!(validate_epic_parent(1, 3, chain(&[(3, 2)])).is_ok());
    }

    /// A stored `parent_epic_id` of 0 is a root, not epic 0.
    #[test]
    fn a_zero_parent_ends_the_walk() {
        assert!(validate_epic_parent(1, 3, chain(&[(3, 0), (0, 1)])).is_ok());
    }

    #[test]
    fn a_walk_that_meets_an_existing_loop_terminates() {
        // 2 <-> 3 already loop; moving 1 under 2 must not spin forever.
        let parents = chain(&[(2, 3), (3, 2)]);
        assert!(validate_epic_parent(1, 2, parents).is_ok());
    }

    #[test]
    fn append_only_is_refused_with_grouping_or_a_feed_role() {
        assert!(validate_feed_append_only(true, true, "none").is_err());
        assert!(validate_feed_append_only(true, false, "reviews").is_err());
        assert!(validate_feed_append_only(true, false, "none").is_ok());
        assert!(validate_feed_append_only(false, true, "reviews").is_ok());
    }

    #[test]
    fn the_append_only_refusal_says_what_to_do_instead() {
        let why = validate_feed_append_only(true, true, "none").unwrap_err();
        assert!(
            why.starts_with("feed_append_only cannot be combined"),
            "{why}"
        );
        assert!(why.contains("one flat append-only epic per repo"), "{why}");
    }

    #[test]
    fn only_a_grouped_non_feed_epic_auto_groups() {
        assert!(can_auto_group(true, ""));
        assert!(!can_auto_group(true, "gh pr list"));
        assert!(!can_auto_group(false, ""));
    }

    #[test]
    fn a_repo_group_is_named_after_the_final_path_component() {
        assert_eq!(repo_name_from_path("/home/u/dispatch"), "dispatch");
        assert_eq!(repo_name_from_path("/home/u/dispatch/"), "dispatch");
        assert_eq!(repo_name_from_path(""), "other");
        assert_eq!(repo_name_from_path("/"), "other");
    }

    fn sig(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// `feeds.allium: RouteSignals`. Total, first match wins.
    #[test]
    fn signals_route_by_first_match() {
        let cases: &[(&[&str], &str)] = &[
            (&["direct-request"], ROLE_MY_REVIEWS),
            (&["team-request"], ROLE_TEAM_REVIEWS),
            (&["reviewed"], ROLE_MY_REVIEWS),
            (&["commented"], ROLE_MY_REVIEWS),
            (&["author-bot"], ROLE_BOTS),
            // engagement beats the bot rule
            (&["reviewed", "author-bot"], ROLE_MY_REVIEWS),
            // but the user's own comment on their own PR is not engagement
            (&["commented", "author-me", "author-bot"], ROLE_BOTS),
            (&["team-request", "reviewed"], ROLE_MY_REVIEWS),
            (&[], ROLE_MY_REVIEWS),
            (&["org-review"], ROLE_MY_REVIEWS),
            (&["org-review", "author-bot"], ROLE_BOTS),
            (&["org-review", "author-bot", "reviewed"], ROLE_MY_REVIEWS),
        ];
        for (signals, role) in cases {
            assert_eq!(route_signals(&sig(signals)), *role, "{signals:?}");
        }
    }

    /// `feeds.allium: ExcludeFromReviews`.
    #[test]
    fn own_authored_and_settled_approved_prs_are_excluded() {
        let others = [
            "direct-request",
            "team-request",
            "reviewed",
            "commented",
            "author-bot",
            "org-review",
        ];
        assert!(!excluded_from_reviews(&[]));
        assert!(excluded_from_reviews(&sig(&["author-me"])));
        for s in others.iter().chain(["approved"].iter()) {
            assert!(
                !excluded_from_reviews(&sig(&[s])) || *s == "approved",
                "{s}"
            );
            assert!(
                excluded_from_reviews(&sig(&["author-me", s])),
                "author-me + {s}"
            );
        }
        // approved is excluded unless a request rescues it
        assert!(excluded_from_reviews(&sig(&["approved"])));
        for s in ["reviewed", "commented", "author-bot", "org-review"] {
            assert!(excluded_from_reviews(&sig(&["approved", s])), "{s}");
        }
        for s in ["direct-request", "team-request"] {
            assert!(!excluded_from_reviews(&sig(&["approved", s])), "{s}");
            assert!(
                !excluded_from_reviews(&sig(&["approved", "reviewed", s])),
                "{s}"
            );
        }
        // an own-authored PR is never rescued
        assert!(excluded_from_reviews(&sig(&[
            "author-me",
            "approved",
            "direct-request"
        ])));
    }

    #[test]
    fn a_feed_item_url_names_its_repo_group() {
        assert_eq!(
            repo_name_from_url("https://github.com/org/my-repo/pull/42"),
            "my-repo"
        );
        assert_eq!(
            repo_name_from_url("https://github.com/org/my-repo"),
            "my-repo"
        );
        assert_eq!(repo_name_from_url(""), "other");
        assert_eq!(repo_name_from_url("https://gitlab.com/org/repo"), "other");
        assert_eq!(extract_github_repo("https://github.com/org"), None);
    }

    #[test]
    fn the_rules_version_is_positive() {
        // 0 is what a module that predates the gate reads as.
        assert!(RULES_VERSION > 0);
    }
}

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
mod dispatch_requests {
    use super::*;

    const HOST_A: &str = "host-a";
    const HOST_B: &str = "host-b";
    const NOW: &str = "2026-10-10 12:00:00.000";
    const LATER: &str = "2026-10-10 12:00:30.000";

    fn backlog_task() -> Task {
        Task {
            id: 1,
            ..blank_task()
        }
    }

    /// A request row in `state`, for task 1, provisioned by `host`.
    fn request(state: &str, host: &str) -> DispatchRequest {
        DispatchRequest {
            id: 7,
            task_id: 1,
            host: host.into(),
            state: state.into(),
            requested_at: NOW.into(),
            ..blank_dispatch_request()
        }
    }

    // -- config (store-split.allium: config) ------------------------------

    #[test]
    fn the_request_timeout_is_sixty_seconds_and_the_backstop_grace_ten() {
        assert_eq!(DISPATCH_REQUEST_TIMEOUT_SECS, 60);
        assert_eq!(DISPATCH_REQUEST_BACKSTOP_GRACE_SECS, 10);
    }

    /// `DispatchRequest.deadline`: ONE deadline, from `requested_at`, in the
    /// store's timestamp spelling so the host reads it straight off the row.
    #[test]
    fn the_deadline_is_requested_at_plus_the_timeout() {
        assert_eq!(
            dispatch_request_deadline("2026-09-19 12:34:56.789"),
            "2026-09-19 12:35:56.789"
        );
    }

    /// `DispatchRequestTimesOut` fires at deadline + grace, not at the deadline:
    /// a live host's own stop must land first.
    #[test]
    fn the_backstop_timer_is_due_after_the_deadline_plus_the_grace() {
        let requested = 1_789_821_296_789_000_i64;
        assert_eq!(
            dispatch_request_backstop_due_micros(requested),
            requested + 70 * 1_000_000
        );
    }

    // -- the row's lifecycle ---------------------------------------------

    #[test]
    fn requested_and_provisioning_are_open_and_the_finished_states_are_not() {
        assert!(dispatch_request_is_open("requested"));
        assert!(dispatch_request_is_open("provisioning"));
        assert!(!dispatch_request_is_open("dispatched"));
        assert!(!dispatch_request_is_open("failed"));
    }

    /// `RequestDispatch`'s `DispatchRequest.created(...)`: kind dispatch, the
    /// requester as host, state requested, requested_at from the module clock,
    /// and every result column absent.
    #[test]
    fn a_new_request_is_requested_on_the_requesting_host() {
        let row = new_dispatch_request(1, HOST_A, "research", "mcp", NOW);
        assert_eq!(row.task_id, 1);
        assert_eq!(row.kind, "dispatch");
        assert_eq!(row.host, HOST_A);
        assert_eq!(row.mode, "research");
        assert_eq!(row.origin, "mcp");
        assert_eq!(row.state, "requested");
        assert_eq!(row.requested_at, NOW);
        assert_eq!(row.claimed_at, "");
        assert_eq!(row.finished_at, "");
        assert_eq!(row.reason, "");
        assert_eq!(row.worktree, "");
        assert_eq!(row.tmux_window, "");
    }

    /// The deadline is STORED on the row, computed by the module from the
    /// row's own `requested_at`, so the host reads it rather than re-deriving
    /// it from a copy of the config (D4).
    #[test]
    fn a_new_request_stores_its_deadline() {
        let row = new_dispatch_request(1, HOST_A, "dispatch", "user", NOW);
        assert_eq!(row.deadline, "2026-10-10 12:01:00.000");
        assert_eq!(row.deadline, dispatch_request_deadline(&row.requested_at));
    }

    #[test]
    fn an_unparseable_requested_at_has_no_deadline() {
        assert_eq!(dispatch_request_deadline(""), "");
        assert_eq!(dispatch_request_deadline("yesterday"), "");
    }

    #[test]
    fn only_the_four_launch_paths_are_origins() {
        for origin in ["user", "mcp", "epic_chain", "quick"] {
            assert!(validate_dispatch_origin(origin).is_ok(), "{origin}");
        }
        for origin in ["", "tui", "Quick"] {
            assert!(validate_dispatch_origin(origin).is_err(), "{origin:?}");
        }
    }

    // -- RequestDispatch: the refusals -----------------------------------

    #[test]
    fn a_backlog_task_with_no_open_request_may_be_requested() {
        assert!(validate_dispatch_request(&backlog_task(), HOST_A, false).is_ok());
    }

    /// `DispatchClaimExclusive`, through the request reducer: a claimed task is
    /// refused, and the refusal says so.
    #[test]
    fn a_task_that_is_not_in_backlog_is_refused() {
        for status in ["running", "review", "done"] {
            let task = Task {
                status: status.into(),
                ..backlog_task()
            };
            let why = validate_dispatch_request(&task, HOST_A, false).unwrap_err();
            assert!(why.contains("already claimed"), "{status}: {why}");
        }
    }

    /// `requires: task.host = null or task.host = requester`.
    #[test]
    fn a_task_whose_worktree_is_on_another_host_is_refused() {
        let task = Task {
            host: HOST_A.into(),
            ..backlog_task()
        };
        assert!(validate_dispatch_request(&task, HOST_A, false).is_ok());
        let why = validate_dispatch_request(&task, HOST_B, false).unwrap_err();
        assert!(why.contains(HOST_A), "{why}");
    }

    /// `OneOpenDispatchRequestPerTask`: a task moved back to backlog by hand
    /// during provisioning still holds its open request, and a second one is
    /// refused rather than written beside it.
    #[test]
    fn a_task_with_an_open_request_is_refused_even_in_backlog() {
        let why = validate_dispatch_request(&backlog_task(), HOST_A, true).unwrap_err();
        assert!(why.contains("open dispatch request"), "{why}");
    }

    // -- RequestDispatch: the mode (D3) ----------------------------------

    #[test]
    fn a_named_mode_is_kept() {
        for mode in ["dispatch", "research", "quick"] {
            assert_eq!(
                choose_dispatch_mode(&backlog_task(), mode).unwrap(),
                mode,
                "{mode}"
            );
        }
    }

    #[test]
    fn an_unknown_mode_is_refused() {
        assert!(choose_dispatch_mode(&backlog_task(), "brainstorm").is_err());
    }

    /// Omitted (`""`) derives: a plan wins, then the research tag, else dispatch.
    #[test]
    fn an_omitted_mode_is_derived_by_todays_rule() {
        assert_eq!(
            choose_dispatch_mode(&backlog_task(), "").unwrap(),
            "dispatch"
        );

        let research = Task {
            tag: "research".into(),
            ..backlog_task()
        };
        assert_eq!(choose_dispatch_mode(&research, "").unwrap(), "research");

        let planned_research = Task {
            plan_path: "/plans/p.md".into(),
            ..research
        };
        assert_eq!(
            choose_dispatch_mode(&planned_research, "").unwrap(),
            "dispatch"
        );
    }

    /// quick is only ever named by its caller, never derived.
    #[test]
    fn quick_is_never_derived() {
        for tag in ["", "research", "chore", "quick"] {
            let task = Task {
                tag: tag.into(),
                ..backlog_task()
            };
            assert_ne!(choose_dispatch_mode(&task, "").unwrap(), "quick", "{tag}");
        }
    }

    // -- RequestDispatch: the claim --------------------------------------

    /// The claim's writes, as `RequestDispatch` ensures them.
    #[test]
    fn the_request_claims_the_task() {
        let task = Task {
            live_subagents: 3,
            stop_pending: true,
            ..backlog_task()
        };
        let claimed = apply_dispatch_claim(task, NOW);
        assert_eq!(claimed.status, "running");
        // core/SubStatus.default_for(running)
        assert_eq!(claimed.sub_status, "active");
        assert_eq!(claimed.last_pre_tool_use_at, NOW);
        assert_eq!(claimed.live_subagents, 0);
        assert!(!claimed.stop_pending);
        assert_eq!(claimed.id, 1);
    }

    // -- HostClaimsDispatch -----------------------------------------------

    #[test]
    fn the_rows_own_host_claims_a_requested_row() {
        assert!(validate_dispatch_request_claim(&request("requested", HOST_A), HOST_A).is_ok());
        let claimed = claim_dispatch_request_row(request("requested", HOST_A), LATER);
        assert_eq!(claimed.state, "provisioning");
        assert_eq!(claimed.claimed_at, LATER);
        assert_eq!(claimed.requested_at, NOW);
    }

    #[test]
    fn another_host_cannot_claim_the_row() {
        assert!(validate_dispatch_request_claim(&request("requested", HOST_A), HOST_B).is_err());
    }

    /// Of two boards, or of a row delivered twice, exactly one claim is
    /// accepted: the second finds the row no longer requested.
    #[test]
    fn a_row_that_is_not_requested_cannot_be_claimed() {
        for state in ["provisioning", "dispatched", "failed"] {
            assert!(
                validate_dispatch_request_claim(&request(state, HOST_A), HOST_A).is_err(),
                "{state}"
            );
        }
    }

    // -- HostReportsDispatched / HostReportsDispatchFailed ---------------

    #[test]
    fn the_rows_host_reports_on_a_provisioning_row() {
        assert!(validate_dispatch_report(&request("provisioning", HOST_A), HOST_A).is_ok());
    }

    #[test]
    fn a_report_from_another_host_is_refused() {
        assert!(validate_dispatch_report(&request("provisioning", HOST_A), HOST_B).is_err());
    }

    /// A LATE REPORT IS REFUSED: after the timer (or a deletion) failed the
    /// row, nothing the host says lands on it or on the task.
    #[test]
    fn a_report_on_a_row_that_is_not_provisioning_is_refused() {
        for state in ["requested", "dispatched", "failed"] {
            assert!(
                validate_dispatch_report(&request(state, HOST_A), HOST_A).is_err(),
                "{state}"
            );
        }
    }

    #[test]
    fn a_dispatched_report_records_the_result_on_the_row() {
        let row = dispatched_request_row(
            request("provisioning", HOST_A),
            "/wt/1-task",
            "task-1",
            LATER,
        );
        assert_eq!(row.state, "dispatched");
        assert_eq!(row.finished_at, LATER);
        assert_eq!(row.worktree, "/wt/1-task");
        assert_eq!(row.tmux_window, "task-1");
        assert_eq!(row.reason, "");
    }

    /// The writes today's post-provision patch made: worktree, window, and the
    /// host paired with the worktree (`HostTracksWorktree`).
    #[test]
    fn a_dispatched_report_writes_the_tasks_worktree_window_and_host() {
        let running = apply_dispatch_claim(backlog_task(), NOW);
        let row = dispatched_request_row(
            request("provisioning", HOST_A),
            "/wt/1-task",
            "task-1",
            LATER,
        );
        let task = apply_dispatched_to_task(running, &row);
        assert_eq!(task.worktree, "/wt/1-task");
        assert_eq!(task.tmux_window, "task-1");
        assert_eq!(task.host, HOST_A);
        assert_eq!(task.status, "running");
    }

    #[test]
    fn a_failed_report_records_the_reason_on_the_row() {
        let row = failed_request_row(
            request("provisioning", HOST_A),
            "git worktree add failed",
            LATER,
        );
        assert_eq!(row.state, "failed");
        assert_eq!(row.finished_at, LATER);
        assert_eq!(row.reason, "git worktree add failed");
        assert_eq!(row.worktree, "");
        assert_eq!(row.tmux_window, "");
    }

    // -- ReleaseClaimOfFailedRequest -------------------------------------

    #[test]
    fn a_failure_releases_a_claim_that_never_got_a_worktree() {
        let claimed = apply_dispatch_claim(backlog_task(), NOW);
        let released = release_claim_after_failure(claimed).expect("still claimed");
        assert_eq!(released.status, "backlog");
        // core/SubStatus.default_for(backlog)
        assert_eq!(released.sub_status, "none");
        assert_eq!(released.last_pre_tool_use_at, "");
    }

    /// Conditional, as today: a task that was provisioned meanwhile is left
    /// alone rather than stomped back to backlog.
    #[test]
    fn a_failure_leaves_a_task_with_a_worktree_alone() {
        let provisioned = Task {
            worktree: "/wt/1-task".into(),
            ..apply_dispatch_claim(backlog_task(), NOW)
        };
        assert!(release_claim_after_failure(provisioned).is_none());
    }

    /// ...and one that moved on (back to backlog by hand, or into review).
    #[test]
    fn a_failure_leaves_a_task_that_is_not_running_alone() {
        for status in ["backlog", "review", "done"] {
            let moved = Task {
                status: status.into(),
                ..backlog_task()
            };
            assert!(release_claim_after_failure(moved).is_none(), "{status}");
        }
    }

    // -- DispatchRequestTimesOut (the scheduled reducer's body) ----------

    /// The scheduled reducer fires only once its row is due (deadline +
    /// grace), so its body is "fail the row 'timed out' if it is still open";
    /// a row the host already finished is left as it is (its own report won).
    #[test]
    fn the_backstop_fails_an_open_row_as_timed_out() {
        for state in ["requested", "provisioning"] {
            let row = request(state, HOST_A);
            assert!(dispatch_request_is_open(&row.state), "{state}");
            let failed = failed_request_row(row, DISPATCH_REASON_TIMED_OUT, LATER);
            assert_eq!(failed.state, "failed");
            assert_eq!(failed.reason, "timed out");
            assert_eq!(failed.finished_at, LATER);
        }
        for state in ["dispatched", "failed"] {
            assert!(!dispatch_request_is_open(state), "{state}");
        }
    }

    // -- HostRestartFailsInterrupted -------------------------------------

    /// Only the PROVISIONING rows of the host's own (the reducer reads them
    /// by its `host` index): a requested row is picked up by the starting
    /// board, and finished rows are history.
    #[test]
    fn a_restart_interrupts_only_provisioning_rows() {
        let rows = vec![
            DispatchRequest {
                id: 1,
                ..request("provisioning", HOST_A)
            },
            DispatchRequest {
                id: 2,
                ..request("requested", HOST_A)
            },
            DispatchRequest {
                id: 4,
                ..request("dispatched", HOST_A)
            },
            DispatchRequest {
                id: 5,
                ..request("failed", HOST_A)
            },
        ];
        let ids: Vec<i64> = interrupted_dispatch_requests(rows)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![1]);
    }

    #[test]
    fn the_failure_reasons_are_the_specs_wording() {
        assert_eq!(DISPATCH_REASON_TIMED_OUT, "timed out");
        assert_eq!(DISPATCH_REASON_HOST_RESTARTED, "host restarted");
        assert_eq!(DISPATCH_REASON_TASK_DELETED, "task deleted");
    }

    // -- the rules version (sync.allium: config.rules_version) -----------

    /// D7: M4 moves the dispatch rules into the module, so the version is 3.
    /// The board's own copy is pinned equal to this one by
    /// `src/sync/rules_gate.rs`'s test.
    #[test]
    fn the_rules_version_is_three_for_m4() {
        assert_eq!(RULES_VERSION, 3);
    }

    // -- D11: who may act for a host --------------------------------------

    fn host(owner: &str) -> Host {
        Host {
            id: HOST_A.into(),
            label: String::new(),
            owner: owner.into(),
        }
    }

    /// The host's own owner may name it.
    #[test]
    fn the_hosts_owner_may_act_for_it() {
        assert_eq!(
            validate_host_acting_for(Some(&host("c0ffee")), HOST_A, "c0ffee"),
            Ok(())
        );
    }

    /// Another identity may not, and the refusal says whose host it is.
    #[test]
    fn another_identity_may_not_act_for_a_host() {
        let why = validate_host_acting_for(Some(&host("c0ffee")), HOST_A, "beef").unwrap_err();
        assert!(why.contains(HOST_A) && why.contains("beef"), "{why}");
    }

    /// A host with no owner yet is refused for everyone: there is no one it
    /// is known to belong to.
    #[test]
    fn nobody_may_act_for_an_ownerless_host() {
        let why = validate_host_acting_for(Some(&host("")), HOST_A, "c0ffee").unwrap_err();
        assert!(why.contains("no owner"), "{why}");
    }

    // -- LocalHostOwnerIsWrittenOnce, enforced by register_host -------------

    /// A caller may register a host as its own, and re-register it (a label
    /// change) as often as it likes.
    #[test]
    fn a_caller_may_register_and_re_register_its_own_host() {
        assert_eq!(validate_host_registration(None, "c0ffee", "c0ffee"), Ok(()));
        assert_eq!(
            validate_host_registration(Some(&host("c0ffee")), "c0ffee", "c0ffee"),
            Ok(())
        );
    }

    /// A NEW host row must name its owner: host ids are public, so an
    /// ownerless row would be adoptable by whichever caller registered the id
    /// next — a teammate could lock a new machine out of its own host.
    #[test]
    fn a_new_host_may_not_be_registered_ownerless() {
        let why = validate_host_registration(None, "", "c0ffee").unwrap_err();
        assert!(why.contains("owner"), "{why}");
    }

    /// A legacy row written before owners were enforced may still be adopted
    /// by the first caller naming itself, and an ownerless re-register of it
    /// changes no ownership.
    #[test]
    fn a_legacy_ownerless_row_may_still_be_adopted() {
        assert_eq!(
            validate_host_registration(Some(&host("")), "c0ffee", "c0ffee"),
            Ok(())
        );
        assert_eq!(validate_host_registration(Some(&host("")), "", "c0ffee"), Ok(()));
    }

    /// Nobody may name another identity as a host's owner.
    #[test]
    fn a_caller_may_not_name_another_identity_as_owner() {
        let why = validate_host_registration(None, "beef", "c0ffee").unwrap_err();
        assert!(why.contains("beef") && why.contains("c0ffee"), "{why}");
    }

    /// An owned host's owner never changes — not to the caller, not to empty.
    #[test]
    fn an_owned_hosts_owner_is_never_rewritten() {
        let why = validate_host_registration(Some(&host("beef")), "c0ffee", "c0ffee").unwrap_err();
        assert!(why.contains("beef"), "{why}");
        let why = validate_host_registration(Some(&host("c0ffee")), "", "c0ffee").unwrap_err();
        assert!(why.contains("c0ffee"), "{why}");
    }

    /// A host missing from the registry is refused.
    #[test]
    fn nobody_may_act_for_an_unregistered_host() {
        let why = validate_host_acting_for(None, HOST_B, "c0ffee").unwrap_err();
        assert!(why.contains(HOST_B) && why.contains("not registered"), "{why}");
    }
}
