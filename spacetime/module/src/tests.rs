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
                alias: Some(MARK.into()),
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
        assert_eq!(row.alias, MARK);
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

    // Which pairs are legal is pinned pair by pair by the board's model tests
    // (`src/models/tasks/model_tests.rs`), which read the board's `SubStatus`
    // spellings through `validate_sub_status`; here, only the refusal's wording.

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
        assert!(validate_dispatch_request(&backlog_task(), HOST_A, "user", false).is_ok());
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
            let why = validate_dispatch_request(&task, HOST_A, "user", false).unwrap_err();
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
        assert!(validate_dispatch_request(&task, HOST_A, "user", false).is_ok());
        let why = validate_dispatch_request(&task, HOST_B, "user", false).unwrap_err();
        assert!(why.contains(HOST_A), "{why}");
    }

    /// `OneOpenDispatchRequestPerTask`: a task moved back to backlog by hand
    /// during provisioning still holds its open request, and a second one is
    /// refused rather than written beside it.
    #[test]
    fn a_task_with_an_open_request_is_refused_even_in_backlog() {
        let why = validate_dispatch_request(&backlog_task(), HOST_A, "user", true).unwrap_err();
        assert!(why.contains("open dispatch request"), "{why}");
    }

    /// `epics.allium: PhoenixIsNeverChained`, enforced by the request: the
    /// epic chain's request for a phoenix task is refused with that reason,
    /// and every other origin may take it.
    #[test]
    fn the_epic_chain_may_not_request_a_phoenix_task() {
        let phoenix = Task {
            phoenix: true,
            ..backlog_task()
        };
        let why = validate_dispatch_request(&phoenix, HOST_A, "epic_chain", false).unwrap_err();
        assert!(why.contains("phoenix"), "{why}");
        for origin in ["user", "mcp", "quick"] {
            assert!(
                validate_dispatch_request(&phoenix, HOST_A, origin, false).is_ok(),
                "{origin}"
            );
        }
        assert!(validate_dispatch_request(&backlog_task(), HOST_A, "epic_chain", false).is_ok());
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
            false,
            LATER,
        );
        assert_eq!(row.state, "dispatched");
        assert_eq!(row.finished_at, LATER);
        assert_eq!(row.worktree, "/wt/1-task");
        assert_eq!(row.tmux_window, "task-1");
        assert_eq!(row.reason, "");
    }

    /// The host's report of whether provisioning reused the worktree is kept
    /// on the row (`DispatchRequest.reused_worktree`).
    #[test]
    fn a_dispatched_report_records_whether_the_worktree_was_reused() {
        let reused = dispatched_request_row(
            request("provisioning", HOST_A),
            "/wt",
            "task-1",
            true,
            LATER,
        );
        assert!(reused.reused_worktree);
        let created = dispatched_request_row(
            request("provisioning", HOST_A),
            "/wt",
            "task-1",
            false,
            LATER,
        );
        assert!(!created.reused_worktree);
        assert!(
            !request("requested", HOST_A).reused_worktree,
            "false until then"
        );
    }

    /// D4: a dispatched report is refused once the row's deadline has passed
    /// by the module's clock; before it, it is in time.
    #[test]
    fn a_dispatched_report_past_the_deadline_is_refused() {
        let row = DispatchRequest {
            deadline: dispatch_request_deadline(NOW),
            ..request("provisioning", HOST_A)
        };
        assert!(validate_dispatched_report_in_time(&row, LATER).is_ok());
        let why = validate_dispatched_report_in_time(&row, "2026-10-10 12:01:00.000").unwrap_err();
        assert!(why.contains("deadline"), "{why}");
        assert!(validate_dispatched_report_in_time(&row, "2026-10-10 12:05:00.000").is_err());
    }

    /// `HostRepeatsDispatchedReport`: the same report on a row it already
    /// dispatched is a repeat; any other report is not.
    #[test]
    fn a_repeat_of_the_dispatched_report_is_recognised() {
        let dispatched = dispatched_request_row(
            request("provisioning", HOST_A),
            "/wt",
            "task-1",
            false,
            LATER,
        );
        assert!(dispatch_report_is_repeat(
            &dispatched,
            HOST_A,
            "/wt",
            "task-1"
        ));
        assert!(!dispatch_report_is_repeat(
            &dispatched,
            HOST_B,
            "/wt",
            "task-1"
        ));
        assert!(!dispatch_report_is_repeat(
            &dispatched,
            HOST_A,
            "/other",
            "task-1"
        ));
        assert!(!dispatch_report_is_repeat(
            &dispatched,
            HOST_A,
            "/wt",
            "task-2"
        ));
        let failed = DispatchRequest {
            worktree: "/wt".into(),
            tmux_window: "task-1".into(),
            ..request("failed", HOST_A)
        };
        assert!(!dispatch_report_is_repeat(&failed, HOST_A, "/wt", "task-1"));
        assert!(!dispatch_report_is_repeat(
            &request("provisioning", HOST_A),
            HOST_A,
            "",
            ""
        ));
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
            false,
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
        assert_eq!(
            validate_host_registration(Some(&host("")), "", "c0ffee"),
            Ok(())
        );
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
        assert!(
            why.contains(HOST_B) && why.contains("not registered"),
            "{why}"
        );
    }
}

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
mod session_requests {
    use super::*;

    const HOST_A: &str = "host-a";
    const HOST_B: &str = "host-b";
    const NOW: &str = "2026-10-10 12:00:00.000";
    const LATER: &str = "2026-10-10 12:00:20.000";
    const PAST_TEARDOWN_DEADLINE: &str = "2026-10-10 12:00:30.000";
    const PAST_REBASE_DEADLINE: &str = "2026-10-10 12:02:00.000";

    /// A running task on `HOST_A` with a worktree and a live window.
    fn running_task() -> Task {
        Task {
            id: 1,
            status: "running".into(),
            sub_status: "active".into(),
            worktree: "/wt/1-t".into(),
            tmux_window: "task-1".into(),
            host: HOST_A.into(),
            epic_id: 5,
            ..blank_task()
        }
    }

    /// A session row for task 1 on `HOST_A`.
    fn session(id: i64, kind: &str, action: &str, state: &str) -> SessionRequest {
        SessionRequest {
            id,
            task_id: 1,
            kind: kind.into(),
            action: action.into(),
            host: HOST_A.into(),
            state: state.into(),
            requested_at: NOW.into(),
            ..blank_session_request()
        }
    }

    /// A done wrap_up row, finished at `finished_at`: a candidate exit token.
    fn done_wrap_up(id: i64, action: &str, finished_at: &str) -> SessionRequest {
        SessionRequest {
            finished_at: finished_at.into(),
            ..session(id, "wrap_up", action, "done")
        }
    }

    /// The exit_session row that consumed `token`.
    fn consumed(id: i64, token: i64) -> SessionRequest {
        SessionRequest {
            wrap_up_request: token,
            ..session(id, "exit_session", "rebase", "requested")
        }
    }

    fn dispatched(finished_at: &str) -> DispatchRequest {
        DispatchRequest {
            id: 1,
            task_id: 1,
            state: "dispatched".into(),
            finished_at: finished_at.into(),
            ..blank_dispatch_request()
        }
    }

    // -- config (store-split.allium: config) ------------------------------

    #[test]
    fn the_session_timeouts_are_120_and_30_seconds_with_a_ten_second_grace() {
        assert_eq!(WRAP_UP_REBASE_TIMEOUT_SECS, 120);
        assert_eq!(EXIT_SESSION_TEARDOWN_TIMEOUT_SECS, 30);
        assert_eq!(SESSION_REQUEST_BACKSTOP_GRACE_SECS, 10);
    }

    /// `SessionRequest.deadline`: ONE deadline from `requested_at`, which
    /// depends on the kind.
    #[test]
    fn the_deadline_is_requested_at_plus_the_timeout_of_the_kind() {
        assert_eq!(
            session_request_deadline("wrap_up", "2026-09-19 12:34:56.789"),
            "2026-09-19 12:36:56.789"
        );
        assert_eq!(
            session_request_deadline("exit_session", "2026-09-19 12:34:56.789"),
            "2026-09-19 12:35:26.789"
        );
        assert_eq!(session_request_deadline("wrap_up", "yesterday"), "");
    }

    /// `SessionRequestTimesOut` fires at deadline + grace, not at the
    /// deadline: a live host's own stop must land first.
    #[test]
    fn the_backstop_timer_is_due_after_the_deadline_plus_the_grace() {
        let requested = 1_789_821_296_789_000_i64;
        assert_eq!(
            session_request_backstop_due_micros("wrap_up", requested),
            requested + 130 * 1_000_000
        );
        assert_eq!(
            session_request_backstop_due_micros("exit_session", requested),
            requested + 40 * 1_000_000
        );
    }

    #[test]
    fn requested_and_running_are_open_and_the_finished_states_are_not() {
        assert!(session_request_is_open("requested"));
        assert!(session_request_is_open("running"));
        assert!(!session_request_is_open("done"));
        assert!(!session_request_is_open("failed"));
    }

    #[test]
    fn the_failure_reasons_and_refusals_are_the_specs_wording() {
        assert_eq!(SESSION_REASON_TIMED_OUT, "timed out");
        assert_eq!(SESSION_REASON_HOST_RESTARTED, "host restarted");
        assert_eq!(SESSION_REASON_TASK_DELETED, "task deleted");
        assert_eq!(EXIT_TOKEN_MISSING, "no exit token — call wrap_up first");
        assert_eq!(EXIT_TOKEN_INVALID, "invalid exit token");
    }

    // -- RequestWrapUp ------------------------------------------------------

    #[test]
    fn only_the_three_wrap_up_actions_are_actions() {
        for action in ["rebase", "done", "pr"] {
            assert!(validate_wrap_up_action(action).is_ok(), "{action}");
        }
        for action in ["", "merge", "Rebase"] {
            assert!(validate_wrap_up_action(action).is_err(), "{action:?}");
        }
    }

    #[test]
    fn a_running_or_review_task_with_a_worktree_on_the_host_may_be_wrapped_up() {
        for status in ["running", "review"] {
            let task = Task {
                status: status.into(),
                ..running_task()
            };
            assert!(
                validate_wrap_up_request(&task, HOST_A, false).is_ok(),
                "{status}"
            );
        }
    }

    #[test]
    fn a_task_that_is_not_running_or_review_or_has_no_worktree_is_refused() {
        for status in ["backlog", "done"] {
            let task = Task {
                status: status.into(),
                ..running_task()
            };
            let why = validate_wrap_up_request(&task, HOST_A, false).unwrap_err();
            assert!(
                why.contains("cannot be wrapped up") && why.contains("Requires Running or Review"),
                "{status}: {why}"
            );
        }
        let bare = Task {
            worktree: String::new(),
            ..running_task()
        };
        let why = validate_wrap_up_request(&bare, HOST_A, false).unwrap_err();
        assert!(why.contains("cannot be wrapped up"), "{why}");
    }

    /// `ReviewTasksAreNotWrappedUp`, the module's by decision: the refusal
    /// carries the tag, that a review task ends at its PR, and the retag way
    /// out.
    #[test]
    fn a_review_tagged_task_is_refused_with_the_retag_way_out() {
        for tag in ["pr-review", "dependabot", "release-please"] {
            let task = Task {
                tag: tag.into(),
                ..running_task()
            };
            let why = validate_wrap_up_request(&task, HOST_A, false).unwrap_err();
            assert!(
                why.contains(&format!("`{tag}`")) && why.contains("retag"),
                "{tag}: {why}"
            );
        }
        let chore = Task {
            tag: "chore".into(),
            ..running_task()
        };
        assert!(validate_wrap_up_request(&chore, HOST_A, false).is_ok());
    }

    /// The rebase runs where the worktree is, so only the task's own host may
    /// ask.
    #[test]
    fn a_task_whose_worktree_is_on_another_host_is_refused() {
        let why = validate_wrap_up_request(&running_task(), HOST_B, false).unwrap_err();
        assert!(why.contains(HOST_A) && why.contains(HOST_B), "{why}");
        let unhosted = Task {
            host: String::new(),
            ..running_task()
        };
        assert!(validate_wrap_up_request(&unhosted, HOST_A, false).is_err());
    }

    /// `OneOpenSessionRequestPerTask`.
    #[test]
    fn a_task_with_an_open_session_request_is_refused() {
        let why = validate_wrap_up_request(&running_task(), HOST_A, true).unwrap_err();
        assert!(why.contains("open session request"), "{why}");
    }

    /// A rebase row is written requested, for its host to claim, with the
    /// wrap-up deadline.
    #[test]
    fn a_rebase_wrap_up_row_is_requested_with_a_deadline() {
        let row = new_wrap_up_request(1, HOST_A, "rebase", NOW);
        assert_eq!(row.task_id, 1);
        assert_eq!(row.kind, "wrap_up");
        assert_eq!(row.action, "rebase");
        assert_eq!(row.host, HOST_A);
        assert_eq!(row.state, "requested");
        assert_eq!(row.requested_at, NOW);
        assert_eq!(row.claimed_at, "");
        assert_eq!(row.finished_at, "");
        assert_eq!(row.deadline, "2026-10-10 12:02:00.000");
        assert_eq!(row.wrap_up_request, 0);
        assert_eq!(row.tmux_window, "");
        assert_eq!(row.chained_task, 0);
        assert!(session_request_has_host_work(&row));
    }

    /// done and pr need no host work: written already done in the same call,
    /// with no claim, no deadline and so no timer.
    #[test]
    fn a_done_or_pr_wrap_up_row_is_written_already_done() {
        for action in ["done", "pr"] {
            let row = new_wrap_up_request(1, HOST_A, action, NOW);
            assert_eq!(row.state, "done", "{action}");
            assert_eq!(row.finished_at, NOW, "{action}");
            assert_eq!(row.claimed_at, "", "{action}");
            assert_eq!(row.deadline, "", "{action}");
            assert!(!session_request_has_host_work(&row), "{action}");
        }
    }

    // -- HostClaimsSessionRequest -----------------------------------------

    #[test]
    fn the_rows_own_host_claims_a_requested_row() {
        let row = SessionRequest {
            deadline: session_request_deadline("wrap_up", NOW),
            ..session(7, "wrap_up", "rebase", "requested")
        };
        assert!(validate_session_request_claim(&row, HOST_A, LATER).is_ok());
        let claimed = claim_session_request_row(row, LATER);
        assert_eq!(claimed.state, "running");
        assert_eq!(claimed.claimed_at, LATER);
        assert_eq!(claimed.requested_at, NOW);
    }

    #[test]
    fn another_host_cannot_claim_the_row() {
        let row = session(7, "wrap_up", "rebase", "requested");
        assert!(validate_session_request_claim(&row, HOST_B, LATER).is_err());
    }

    /// Of two boards, or a row delivered twice, exactly one claim is accepted.
    #[test]
    fn a_row_that_is_not_requested_cannot_be_claimed() {
        for state in ["running", "done", "failed"] {
            let row = session(7, "wrap_up", "rebase", state);
            assert!(
                validate_session_request_claim(&row, HOST_A, LATER).is_err(),
                "{state}"
            );
        }
    }

    /// The host enforces the deadline, so a row past it is not started; the
    /// backstop fails it.
    #[test]
    fn a_row_at_or_past_its_deadline_cannot_be_claimed() {
        let rebase = SessionRequest {
            deadline: session_request_deadline("wrap_up", NOW),
            ..session(7, "wrap_up", "rebase", "requested")
        };
        assert!(validate_session_request_claim(&rebase, HOST_A, PAST_REBASE_DEADLINE).is_err());
        let teardown = SessionRequest {
            deadline: session_request_deadline("exit_session", NOW),
            ..session(8, "exit_session", "rebase", "requested")
        };
        assert!(
            validate_session_request_claim(&teardown, HOST_A, "2026-10-10 12:00:29.999").is_ok()
        );
        assert!(validate_session_request_claim(&teardown, HOST_A, PAST_TEARDOWN_DEADLINE).is_err());
    }

    /// Today's optimistic clear, moved into the claim: a Conflict sub_status
    /// is cleared to the status's default when a wrap_up row is claimed.
    #[test]
    fn claiming_a_wrap_up_clears_a_conflict() {
        let conflicted = Task {
            sub_status: "conflict".into(),
            ..running_task()
        };
        let cleared = clear_conflict_on_claim("wrap_up", conflicted, LATER).expect("was conflict");
        assert_eq!(cleared.sub_status, "active");
        assert_eq!(cleared.updated_at, LATER);

        let in_review = Task {
            status: "review".into(),
            sub_status: "conflict".into(),
            ..running_task()
        };
        let cleared = clear_conflict_on_claim("wrap_up", in_review, LATER).expect("was conflict");
        assert_eq!(cleared.sub_status, "awaiting_review");
    }

    #[test]
    fn claiming_leaves_a_task_without_a_conflict_alone() {
        assert!(clear_conflict_on_claim("wrap_up", running_task(), LATER).is_none());
    }

    /// Only a wrap_up claim clears it: a teardown claim never touches the
    /// task.
    #[test]
    fn claiming_a_teardown_never_clears_a_conflict() {
        let conflicted = Task {
            sub_status: "conflict".into(),
            ..running_task()
        };
        assert!(clear_conflict_on_claim("exit_session", conflicted, LATER).is_none());
    }

    #[test]
    fn the_default_sub_status_follows_the_status() {
        assert_eq!(default_sub_status("backlog"), "none");
        assert_eq!(default_sub_status("running"), "active");
        assert_eq!(default_sub_status("review"), "awaiting_review");
        assert_eq!(default_sub_status("done"), "none");
    }

    // -- HostReportsSessionRequestDone / Repeats / Failed -----------------

    #[test]
    fn the_rows_host_reports_on_a_running_row() {
        let row = session(7, "wrap_up", "rebase", "running");
        assert!(validate_session_report(&row, HOST_A).is_ok());
        assert!(validate_session_report(&row, HOST_B).is_err());
    }

    /// A LATE REPORT IS REFUSED: on a row that is no longer running.
    #[test]
    fn a_report_on_a_row_that_is_not_running_is_refused() {
        for state in ["requested", "done", "failed"] {
            let row = session(7, "wrap_up", "rebase", state);
            assert!(validate_session_report(&row, HOST_A).is_err(), "{state}");
        }
    }

    /// A rebase reported done after its deadline, by the module's clock, is
    /// too late and issues no token. A teardown is not held to it: the close
    /// stands whatever the teardown does.
    #[test]
    fn a_rebase_done_report_past_the_deadline_is_refused_but_a_teardown_is_not() {
        let rebase = SessionRequest {
            deadline: session_request_deadline("wrap_up", NOW),
            ..session(7, "wrap_up", "rebase", "running")
        };
        assert!(validate_session_done_in_time(&rebase, LATER).is_ok());
        assert!(validate_session_done_in_time(&rebase, PAST_REBASE_DEADLINE).is_err());

        let teardown = SessionRequest {
            deadline: session_request_deadline("exit_session", NOW),
            ..session(8, "exit_session", "rebase", "running")
        };
        assert!(validate_session_done_in_time(&teardown, PAST_REBASE_DEADLINE).is_ok());
    }

    #[test]
    fn a_done_report_records_the_finish_on_the_row() {
        let row = done_session_request_row(session(7, "wrap_up", "rebase", "running"), LATER);
        assert_eq!(row.state, "done");
        assert_eq!(row.finished_at, LATER);
        assert_eq!(row.reason, "");
    }

    /// `HostRepeatsSessionRequestDone`: only a row this host claimed and
    /// reported; a wrap_up row written done had no report to repeat.
    #[test]
    fn a_repeat_of_the_done_report_is_recognised() {
        let reported = SessionRequest {
            claimed_at: NOW.into(),
            finished_at: LATER.into(),
            ..session(7, "wrap_up", "rebase", "done")
        };
        assert!(session_report_is_repeat(&reported, HOST_A));
        assert!(!session_report_is_repeat(&reported, HOST_B));

        let written_done = done_wrap_up(8, "done", NOW);
        assert!(!session_report_is_repeat(&written_done, HOST_A));

        let failed = session(9, "wrap_up", "rebase", "failed");
        assert!(!session_report_is_repeat(&failed, HOST_A));
    }

    #[test]
    fn a_failed_report_records_the_reason_on_the_row() {
        let row = failed_session_request_row(
            session(7, "wrap_up", "rebase", "running"),
            "conflict in src/a.rs",
            LATER,
        );
        assert_eq!(row.state, "failed");
        assert_eq!(row.finished_at, LATER);
        assert_eq!(row.reason, "conflict in src/a.rs");
    }

    /// Only a rebase can conflict.
    #[test]
    fn only_a_wrap_up_can_report_a_conflict() {
        let wrap_up = session(7, "wrap_up", "rebase", "running");
        assert!(validate_session_failure_report(&wrap_up, true).is_ok());
        assert!(validate_session_failure_report(&wrap_up, false).is_ok());
        let teardown = session(8, "exit_session", "rebase", "running");
        assert!(validate_session_failure_report(&teardown, false).is_ok());
        assert!(validate_session_failure_report(&teardown, true).is_err());
    }

    /// `FinishTaskConflict`: Conflict is legal only under running and review,
    /// so a task moved on meanwhile is left alone.
    #[test]
    fn a_conflict_sets_the_sub_status_only_while_running_or_review() {
        let running = apply_rebase_conflict(running_task(), LATER).expect("running");
        assert_eq!(running.sub_status, "conflict");
        assert_eq!(running.status, "running");
        assert_eq!(running.updated_at, LATER);

        let review = Task {
            status: "review".into(),
            sub_status: "awaiting_review".into(),
            ..running_task()
        };
        assert_eq!(
            apply_rebase_conflict(review, LATER)
                .expect("review")
                .sub_status,
            "conflict"
        );

        for status in ["backlog", "done"] {
            let moved = Task {
                status: status.into(),
                sub_status: "none".into(),
                ..running_task()
            };
            assert!(apply_rebase_conflict(moved, LATER).is_none(), "{status}");
        }
    }

    // -- SessionRequestTimesOut (the scheduled reducer's body) ------------

    #[test]
    fn the_backstop_fails_an_open_row_as_timed_out() {
        for state in ["requested", "running"] {
            let failed = expire_session_request_row(
                session(7, "wrap_up", "rebase", state),
                PAST_REBASE_DEADLINE,
            )
            .unwrap_or_else(|| panic!("{state} is open and must be failed"));
            assert_eq!(failed.state, "failed");
            assert_eq!(failed.reason, "timed out");
            assert_eq!(failed.finished_at, PAST_REBASE_DEADLINE);
        }
    }

    #[test]
    fn the_backstop_leaves_a_finished_row_alone() {
        for state in ["done", "failed"] {
            assert!(
                expire_session_request_row(session(7, "wrap_up", "rebase", state), LATER).is_none(),
                "{state}"
            );
        }
    }

    // -- HostRestartFailsInterruptedSessionRequests -----------------------

    /// Only RUNNING rows, over rows already read by the restarting host's
    /// index: a requested row is picked up by the starting board.
    #[test]
    fn a_restart_interrupts_only_running_rows() {
        let rows = vec![
            session(1, "wrap_up", "rebase", "running"),
            session(2, "wrap_up", "rebase", "requested"),
            session(3, "exit_session", "rebase", "running"),
            session(4, "wrap_up", "rebase", "done"),
            session(5, "wrap_up", "rebase", "failed"),
        ];
        let ids: Vec<i64> = interrupted_session_requests(rows)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![1, 3]);
    }

    // -- the exit token (W3) ----------------------------------------------

    #[test]
    fn the_most_recent_done_wrap_up_row_is_a_valid_token() {
        let rows = vec![done_wrap_up(10, "rebase", LATER)];
        let token = check_exit_token(1, 10, &rows, &[]).expect("valid");
        assert_eq!(token.id, 10);
        assert_eq!(token.action, "rebase");
    }

    /// No done wrap_up row at all: the agent never wrapped up.
    #[test]
    fn no_done_wrap_up_row_means_no_exit_token() {
        assert_eq!(
            check_exit_token(1, 10, &[], &[]).unwrap_err(),
            EXIT_TOKEN_MISSING
        );
        let not_done = vec![
            session(10, "wrap_up", "rebase", "requested"),
            session(11, "wrap_up", "rebase", "running"),
            session(12, "wrap_up", "rebase", "failed"),
        ];
        for token in [10, 11, 12] {
            assert_eq!(
                check_exit_token(1, token, &not_done, &[]).unwrap_err(),
                EXIT_TOKEN_MISSING,
                "{token}"
            );
        }
    }

    /// Each token is consumed once: the exit_session row that names it ends
    /// its validity, and a second call is told to wrap up again.
    #[test]
    fn a_consumed_token_means_no_exit_token() {
        let rows = vec![done_wrap_up(10, "rebase", LATER), consumed(11, 10)];
        assert_eq!(
            check_exit_token(1, 10, &rows, &[]).unwrap_err(),
            EXIT_TOKEN_MISSING
        );
    }

    /// A token that names some other row is "invalid", not "missing".
    #[test]
    fn a_token_naming_another_row_is_invalid() {
        let rows = vec![
            done_wrap_up(10, "rebase", LATER),
            SessionRequest {
                task_id: 2,
                ..done_wrap_up(20, "rebase", LATER)
            },
            consumed(11, 99),
        ];
        // No such row, another task's row, and an exit_session row.
        for token in [999, 20, 11] {
            assert_eq!(
                check_exit_token(1, token, &rows, &[]).unwrap_err(),
                EXIT_TOKEN_INVALID,
                "{token}"
            );
        }
    }

    /// Only the most recent done wrap_up row is valid, as a newer token
    /// replaces an older one today; ties on requested_at go to the higher id.
    #[test]
    fn a_newer_wrap_up_row_supersedes_an_older_token() {
        let newer = SessionRequest {
            requested_at: LATER.into(),
            ..done_wrap_up(11, "done", LATER)
        };
        let rows = vec![done_wrap_up(10, "rebase", NOW), newer];
        assert_eq!(
            check_exit_token(1, 10, &rows, &[]).unwrap_err(),
            EXIT_TOKEN_INVALID
        );
        assert!(check_exit_token(1, 11, &rows, &[]).is_ok());

        let tied = vec![
            done_wrap_up(10, "rebase", LATER),
            done_wrap_up(11, "done", LATER),
        ];
        assert_eq!(
            check_exit_token(1, 10, &tied, &[]).unwrap_err(),
            EXIT_TOKEN_INVALID
        );
        assert!(check_exit_token(1, 11, &tied, &[]).is_ok());
    }

    /// A LATER DISPATCH VOIDS THE TOKEN: a dispatched request of the task that
    /// finished at or after the token's row.
    #[test]
    fn a_later_dispatch_voids_the_token() {
        let rows = vec![done_wrap_up(10, "rebase", LATER)];
        for finished in [LATER, PAST_TEARDOWN_DEADLINE] {
            assert_eq!(
                check_exit_token(1, 10, &rows, &[dispatched(finished)]).unwrap_err(),
                EXIT_TOKEN_INVALID,
                "{finished}"
            );
        }
    }

    /// An earlier dispatch is the session the token closes, and a dispatch
    /// that failed never started a new one.
    #[test]
    fn an_earlier_or_failed_dispatch_leaves_the_token_valid() {
        let rows = vec![done_wrap_up(10, "rebase", LATER)];
        assert!(check_exit_token(1, 10, &rows, &[dispatched(NOW)]).is_ok());
        let failed = DispatchRequest {
            state: "failed".into(),
            ..dispatched(PAST_TEARDOWN_DEADLINE)
        };
        assert!(check_exit_token(1, 10, &rows, &[failed]).is_ok());
    }

    // -- RequestExitSession: the refusals ----------------------------------

    fn token(action: &str) -> SessionRequest {
        done_wrap_up(10, action, LATER)
    }

    #[test]
    fn a_matching_action_and_a_live_window_are_accepted() {
        assert!(validate_exit_session(&running_task(), &token("rebase"), "rebase", "").is_ok());
        assert!(validate_exit_session(&running_task(), &token("done"), "done", "").is_ok());
        assert!(validate_exit_session(
            &running_task(),
            &token("pr"),
            "pr",
            "https://github.com/acme/repo/pull/7"
        )
        .is_ok());
    }

    /// The error names both actions: it stops an agent doing one thing and
    /// closing as another.
    #[test]
    fn an_action_that_differs_from_the_tokens_is_refused_naming_both() {
        let why = validate_exit_session(&running_task(), &token("rebase"), "pr", "u").unwrap_err();
        assert_eq!(
            why,
            "exit token was issued for wrap_up(action=\"rebase\"), but exit_session was called with action=\"pr\""
        );
    }

    #[test]
    fn a_task_with_no_window_has_no_active_session() {
        let closed = Task {
            tmux_window: String::new(),
            ..running_task()
        };
        let why = validate_exit_session(&closed, &token("done"), "done", "").unwrap_err();
        assert_eq!(why, "task #1 has no active session");
    }

    #[test]
    fn a_pr_close_requires_a_pr_url() {
        for url in ["", "  "] {
            let why = validate_exit_session(&running_task(), &token("pr"), "pr", url).unwrap_err();
            assert_eq!(
                why, "pr_url is required for action 'pr' — pass the URL returned by `gh pr create`",
                "{url:?}"
            );
        }
    }

    /// The checks run in the order the spec lists them: action, window, url.
    #[test]
    fn the_refusals_come_in_the_specs_order() {
        let closed = Task {
            tmux_window: String::new(),
            ..running_task()
        };
        let why = validate_exit_session(&closed, &token("rebase"), "pr", "").unwrap_err();
        assert!(why.contains("exit token was issued"), "{why}");
        let why = validate_exit_session(&closed, &token("pr"), "pr", "").unwrap_err();
        assert!(why.contains("no active session"), "{why}");
    }

    #[test]
    fn an_open_session_request_refuses_a_close() {
        let why = validate_no_open_session_request(1, true).unwrap_err();
        assert!(why.contains("open session request"), "{why}");
        assert!(validate_no_open_session_request(1, false).is_ok());
    }

    // -- ExitSession: the close's writes -------------------------------------

    #[test]
    fn a_rebase_or_done_close_takes_the_task_to_done() {
        for action in ["rebase", "done"] {
            let closed = apply_session_close(
                Task {
                    stop_pending: true,
                    ..running_task()
                },
                action,
                "",
                LATER,
            );
            assert_eq!(closed.status, "done", "{action}");
            assert_eq!(closed.sub_status, "none", "{action}");
            assert_eq!(closed.tmux_window, "", "{action}");
            assert_eq!(closed.completed_at, LATER, "{action}");
            assert_eq!(closed.updated_at, LATER, "{action}");
            assert!(!closed.stop_pending, "{action}");
            assert_eq!(
                closed.worktree, "/wt/1-t",
                "the worktree survives the close"
            );
            assert_eq!(closed.url, "");
        }
    }

    /// pr lands in review with a pr-typed url, and does not complete the task.
    #[test]
    fn a_pr_close_takes_the_task_to_review_with_the_url() {
        let closed = apply_session_close(
            running_task(),
            "pr",
            "https://github.com/acme/repo/pull/7",
            LATER,
        );
        assert_eq!(closed.status, "review");
        assert_eq!(closed.sub_status, "awaiting_review");
        assert_eq!(closed.tmux_window, "");
        assert_eq!(closed.url, "https://github.com/acme/repo/pull/7");
        assert_eq!(closed.url_type, "pr");
        assert_eq!(closed.completed_at, "");
        assert_eq!(closed.updated_at, LATER);
    }

    /// Both branches leave running, so both clear a deferred Stop; a close
    /// from review has none to clear and does not touch the flag.
    #[test]
    fn the_deferred_stop_is_cleared_only_when_the_close_leaves_running() {
        let reviewing = Task {
            status: "review".into(),
            sub_status: "awaiting_review".into(),
            stop_pending: true,
            ..running_task()
        };
        let closed = apply_session_close(reviewing, "done", "", LATER);
        assert!(closed.stop_pending);
    }

    /// pr_url is ignored for rebase and done.
    #[test]
    fn a_done_close_ignores_a_pr_url() {
        let closed = apply_session_close(running_task(), "done", "https://x/pull/1", LATER);
        assert_eq!(closed.url, "");
        assert_eq!(closed.url_type, "");
    }

    // -- ExitSession: the exit_session row -----------------------------------

    #[test]
    fn an_exit_session_row_names_the_token_window_host_and_chained_subtask() {
        let row = new_exit_session_request(&running_task(), 10, "pr", "https://x/pull/1", 42, NOW);
        assert_eq!(row.task_id, 1);
        assert_eq!(row.kind, "exit_session");
        assert_eq!(row.action, "pr");
        assert_eq!(row.host, HOST_A);
        assert_eq!(row.state, "requested");
        assert_eq!(row.requested_at, NOW);
        assert_eq!(row.deadline, "2026-10-10 12:00:30.000");
        assert_eq!(row.pr_url, "https://x/pull/1");
        assert_eq!(row.wrap_up_request, 10);
        assert_eq!(row.tmux_window, "task-1");
        assert_eq!(row.chained_task, 42);
        assert_eq!(row.claimed_at, "");
        assert_eq!(row.finished_at, "");
        assert!(session_request_has_host_work(&row));
    }

    /// pr_url is recorded only for action pr; no chained subtask is 0.
    #[test]
    fn an_exit_session_row_records_the_pr_url_only_for_pr() {
        let row = new_exit_session_request(&running_task(), 10, "done", "https://x/pull/1", 0, NOW);
        assert_eq!(row.pr_url, "");
        assert_eq!(row.chained_task, 0);
    }

    // -- PhoenixRespawn through the close -------------------------------------

    fn phoenix_task() -> Task {
        Task {
            title: "weekly".into(),
            description: "d".into(),
            repo_path: "/repo".into(),
            plan_path: "/plans/p.md".into(),
            tag: "chore".into(),
            labels: "[\"x\"]".into(),
            base_branch: "develop".into(),
            wrap_up_mode: "rebase".into(),
            auto_run_plan: true,
            phoenix: true,
            ..running_task()
        }
    }

    #[test]
    fn a_phoenix_task_respawns_when_a_close_takes_it_into_done() {
        let prior = phoenix_task();
        let closed = apply_session_close(prior.clone(), "done", "", LATER);
        assert!(should_respawn_phoenix(&prior.status, &closed));
    }

    /// pr lands in review, and the respawn waits for the PR to merge. A feed
    /// row is exempt, and so is a task that was already done.
    #[test]
    fn a_phoenix_task_does_not_respawn_in_the_other_cases() {
        let prior = phoenix_task();
        let to_review = apply_session_close(prior.clone(), "pr", "u", LATER);
        assert!(!should_respawn_phoenix(&prior.status, &to_review));

        let feed = Task {
            external_id: "ext-1".into(),
            ..phoenix_task()
        };
        let closed = apply_session_close(feed.clone(), "done", "", LATER);
        assert!(!should_respawn_phoenix(&feed.status, &closed));

        let ordinary = apply_session_close(running_task(), "done", "", LATER);
        assert!(!should_respawn_phoenix("running", &ordinary));

        let closed = apply_session_close(prior, "done", "", LATER);
        assert!(!should_respawn_phoenix("done", &closed));
    }

    /// WhatTheSuccessorInherits: everything the operator configured and
    /// nothing the previous run produced.
    #[test]
    fn the_successor_inherits_the_configuration_and_none_of_the_run() {
        let prior = Task {
            url: "https://x/pull/1".into(),
            url_type: "pr".into(),
            completed_at: LATER.into(),
            sort_order: Some(3),
            last_pre_tool_use_at: LATER.into(),
            live_subagents: 2,
            ..phoenix_task()
        };
        let next = phoenix_successor(&prior, 9, NOW, "creator");
        assert_eq!(next.id, 0);
        assert_eq!(next.title, "weekly");
        assert_eq!(next.description, "d");
        assert_eq!(next.repo_path, "/repo");
        assert_eq!(next.plan_path, "/plans/p.md");
        assert_eq!(next.tag, "chore");
        assert_eq!(next.labels, "[\"x\"]");
        assert_eq!(next.base_branch, "develop");
        assert_eq!(next.wrap_up_mode, "rebase");
        assert!(next.auto_run_plan);
        assert!(next.phoenix);
        assert_eq!(next.epic_id, 9);
        assert_eq!(next.status, "backlog");
        assert_eq!(next.sub_status, "none");
        assert_eq!(next.created_at, NOW);
        assert_eq!(next.updated_at, NOW);
        assert_eq!(next.created_by, "creator");
        assert_eq!(next.sort_order, None);
        for empty in [
            &next.worktree,
            &next.tmux_window,
            &next.url,
            &next.url_type,
            &next.completed_at,
            &next.last_pre_tool_use_at,
            &next.external_id,
            &next.host,
        ] {
            assert_eq!(empty, "");
        }
        assert_eq!(next.live_subagents, 0);
    }

    /// A task on a user board (no epic) keeps its owner; one in an epic has
    /// none (`OwnerTracksUserBoardTask`).
    #[test]
    fn the_successor_owner_follows_the_epic() {
        let owned = Task {
            epic_id: 0,
            owner: "alice".into(),
            ..phoenix_task()
        };
        assert_eq!(phoenix_successor(&owned, 0, NOW, "c").owner, "alice");
        assert_eq!(phoenix_successor(&owned, 9, NOW, "c").owner, "");
    }

    // -- AutoDispatchNextSubtask: candidate selection --------------------------

    fn subtask(id: i64, status: &str, sort_order: Option<i64>) -> Task {
        Task {
            id,
            status: status.into(),
            sort_order,
            ..blank_task()
        }
    }

    /// First backlog subtask ordered by sort_order (nulls fall back to the
    /// task id), then by id.
    #[test]
    fn candidates_are_the_backlog_subtasks_in_board_order() {
        let tasks = vec![
            subtask(3, "backlog", None),
            subtask(1, "backlog", Some(5)),
            subtask(2, "backlog", Some(1)),
            subtask(4, "running", Some(0)),
            subtask(5, "done", None),
            subtask(6, "backlog", Some(3)),
        ];
        let ids: Vec<i64> = chain_candidates(tasks).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![2, 3, 6, 1]);
    }

    /// A phoenix subtask is a candidate in the list and refused by the request
    /// itself (`PhoenixIsNeverChained`), so the chain passes over it.
    #[test]
    fn the_chains_request_refuses_a_phoenix_candidate_and_one_on_another_host() {
        let phoenix = Task {
            phoenix: true,
            host: String::new(),
            ..subtask(1, "backlog", None)
        };
        assert!(validate_dispatch_request(&phoenix, HOST_A, "epic_chain", false).is_err());
        let elsewhere = Task {
            host: HOST_B.into(),
            ..subtask(2, "backlog", None)
        };
        assert!(validate_dispatch_request(&elsewhere, HOST_A, "epic_chain", false).is_err());
        assert!(validate_dispatch_request(
            &subtask(3, "backlog", None),
            HOST_A,
            "epic_chain",
            false
        )
        .is_ok());
    }

    // -- PR state (store-split.allium: ReportPrStatus, ReportPrUnreadable) ----

    fn pr_task(sub_status: &str) -> Task {
        Task {
            id: 1,
            status: "review".into(),
            sub_status: sub_status.into(),
            url: "https://github.com/acme/repo/pull/7".into(),
            url_type: "pr".into(),
            tmux_window: "task-1".into(),
            epic_id: 5,
            ..blank_task()
        }
    }

    #[test]
    fn a_merged_pr_takes_the_task_to_done() {
        let done = apply_pr_status(pr_task("approved"), "MERGED", "APPROVED", LATER).unwrap();
        assert_eq!(done.status, "done");
        assert_eq!(done.sub_status, "none");
        assert_eq!(done.tmux_window, "");
        assert_eq!(done.completed_at, LATER);
        assert_eq!(done.updated_at, LATER);
    }

    #[test]
    fn gh_letter_case_does_not_matter() {
        assert!(validate_pr_status_report("merged", "approved").is_ok());
        assert_eq!(
            apply_pr_status(pr_task("approved"), "merged", "", LATER)
                .unwrap()
                .status,
            "done"
        );
    }

    #[test]
    fn a_closed_pr_marks_pr_closed_and_overrides_conflict() {
        for from in ["awaiting_review", "conflict"] {
            let closed = apply_pr_status(pr_task(from), "CLOSED", "", LATER).unwrap();
            assert_eq!(closed.status, "review", "{from}");
            assert_eq!(closed.sub_status, "pr_closed", "{from}");
            assert_eq!(closed.updated_at, LATER);
        }
        assert!(apply_pr_status(pr_task("pr_closed"), "CLOSED", "", LATER).is_none());
    }

    #[test]
    fn an_open_pr_follows_the_review_decision() {
        for (decision, expected) in [
            ("APPROVED", "approved"),
            ("CHANGES_REQUESTED", "changes_requested"),
            ("REVIEW_REQUIRED", "awaiting_review"),
            ("", "awaiting_review"),
        ] {
            let got = apply_pr_status(pr_task("pr_unreachable"), "OPEN", decision, LATER).unwrap();
            assert_eq!(got.sub_status, expected, "{decision:?}");
        }
        assert!(apply_pr_status(pr_task("approved"), "OPEN", "APPROVED", LATER).is_none());
    }

    #[test]
    fn an_open_pr_never_overwrites_conflict() {
        assert!(apply_pr_status(pr_task("conflict"), "OPEN", "APPROVED", LATER).is_none());
    }

    #[test]
    fn a_report_for_a_task_outside_review_or_without_a_pr_changes_nothing() {
        let running = Task {
            status: "running".into(),
            sub_status: "active".into(),
            ..pr_task("")
        };
        assert!(apply_pr_status(running, "MERGED", "", LATER).is_none());
        let issue = Task {
            url_type: "issue".into(),
            ..pr_task("awaiting_review")
        };
        assert!(apply_pr_status(issue, "MERGED", "", LATER).is_none());
    }

    #[test]
    fn unknown_raw_strings_are_refused() {
        assert!(validate_pr_status_report("DRAFT", "").is_err());
        assert!(validate_pr_status_report("", "").is_err());
        assert!(validate_pr_status_report("OPEN", "MAYBE").is_err());
        assert!(validate_pr_status_report("OPEN", "").is_ok());
    }

    #[test]
    fn an_unreadable_pr_marks_pr_unreachable_once() {
        let got = apply_pr_unreadable(pr_task("awaiting_review"), LATER).unwrap();
        assert_eq!(got.sub_status, "pr_unreachable");
        assert_eq!(got.updated_at, LATER);
        assert!(apply_pr_unreadable(pr_task("pr_unreachable"), LATER).is_none());
        let backlog = Task {
            status: "done".into(),
            sub_status: "none".into(),
            ..pr_task("")
        };
        assert!(apply_pr_unreadable(backlog, LATER).is_none());
    }

    // -- RequestCloseTask (store-split.allium, M7 X1) --------------------------

    #[test]
    fn a_close_row_has_host_work_like_an_exit_session_row() {
        assert!(session_request_has_host_work(&session(
            1,
            "close",
            "done",
            "requested"
        )));
        assert_eq!(
            session_request_deadline("close", "2026-09-19 12:34:56.789"),
            "2026-09-19 12:35:26.789"
        );
        assert_eq!(
            session_request_backstop_due_micros("close", 1_000_000),
            1_000_000 + 40 * 1_000_000
        );
    }

    #[test]
    fn a_done_close_of_a_running_task_stamps_completion_and_clears_window_and_stop() {
        let closed = apply_done_close(
            Task {
                stop_pending: true,
                ..running_task()
            },
            LATER,
        );
        assert_eq!(closed.status, "done");
        assert_eq!(closed.sub_status, "none");
        assert_eq!(closed.tmux_window, "");
        assert_eq!(closed.completed_at, LATER);
        assert_eq!(closed.updated_at, LATER);
        assert!(!closed.stop_pending);
    }

    /// A done -> done re-close is no transition: completed_at is kept.
    #[test]
    fn a_done_to_done_reclose_keeps_completed_at() {
        let done = Task {
            status: "done".into(),
            sub_status: "none".into(),
            completed_at: NOW.into(),
            tmux_window: "task-1".into(),
            ..running_task()
        };
        let closed = apply_done_close(done, LATER);
        assert_eq!(closed.completed_at, NOW);
        assert_eq!(closed.updated_at, LATER);
        assert_eq!(closed.tmux_window, "");
    }

    #[test]
    fn a_close_needs_a_teardown_only_for_a_window_or_a_pr_url() {
        assert!(close_has_host_work(&running_task()));
        let no_window = Task {
            tmux_window: String::new(),
            ..running_task()
        };
        assert!(!close_has_host_work(&no_window));
        let with_pr = Task {
            url: "https://github.com/a/b/pull/1".into(),
            url_type: "pr".into(),
            ..no_window.clone()
        };
        assert!(close_has_host_work(&with_pr));
        let other_url = Task {
            url: "https://example.com".into(),
            url_type: "issue".into(),
            ..no_window
        };
        assert!(!close_has_host_work(&other_url));
    }

    #[test]
    fn a_close_row_carries_the_window_read_and_no_token_chain_or_pr_url() {
        let row = new_close_request(&running_task(), HOST_B, NOW);
        assert_eq!(row.kind, "close");
        assert_eq!(row.action, "done");
        assert_eq!(row.host, HOST_A, "the task's own host wins");
        assert_eq!(row.state, "requested");
        assert_eq!(row.tmux_window, "task-1");
        assert_eq!(row.wrap_up_request, 0);
        assert_eq!(row.chained_task, 0);
        assert_eq!(row.pr_url, "");
        assert_eq!(row.requested_at, NOW);
        assert_eq!(row.deadline, session_request_deadline("close", NOW));
    }

    #[test]
    fn a_close_row_falls_back_to_the_callers_host_when_the_task_has_none() {
        let hostless = Task {
            host: String::new(),
            tmux_window: String::new(),
            url: "https://github.com/a/b/pull/1".into(),
            url_type: "pr".into(),
            ..running_task()
        };
        let row = new_close_request(&hostless, HOST_B, NOW);
        assert_eq!(row.host, HOST_B);
        assert_eq!(row.tmux_window, "");
    }

    /// A close row consumes no token: neither a close row nor the open-ness of
    /// one changes what check_exit_token answers.
    #[test]
    fn a_close_row_is_not_an_exit_token_consumer() {
        let rows = vec![
            done_wrap_up(3, "rebase", LATER),
            SessionRequest {
                id: 9,
                ..session(9, "close", "done", "requested")
            },
        ];
        assert!(check_exit_token(1, 3, &rows, &[]).is_ok());
    }

    // -- patch_task's transition rules (M7 X2) -----------------------------------

    #[test]
    fn leaving_running_clears_a_pending_stop_unless_the_patch_names_it() {
        let mut row = Task {
            status: "review".into(),
            stop_pending: true,
            ..running_task()
        };
        apply_patch_status_rules("running", false, &mut row);
        assert!(!row.stop_pending);

        let mut named = Task {
            status: "review".into(),
            stop_pending: true,
            ..running_task()
        };
        apply_patch_status_rules("running", true, &mut named);
        assert!(named.stop_pending);

        let mut stays = Task {
            stop_pending: true,
            ..running_task()
        };
        apply_patch_status_rules("running", false, &mut stays);
        assert!(stays.stop_pending);

        let mut from_review = Task {
            status: "done".into(),
            stop_pending: true,
            ..running_task()
        };
        apply_patch_status_rules("review", false, &mut from_review);
        assert!(
            from_review.stop_pending,
            "only a move out of running clears"
        );
    }

    // -- the rules version (sync.allium: config.rules_version) ----------------

    /// W12: M5 moves the close and the wrap-up request rules into the module,
    /// so the version is 4. The board's own copy is pinned equal to this one
    /// by `src/sync/rules_gate.rs`'s test.
    #[test]
    fn the_rules_version_is_six_for_m7() {
        assert_eq!(RULES_VERSION, 6);
    }
}
