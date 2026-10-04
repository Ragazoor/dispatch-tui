use super::*;

// -----------------------------------------------------------------------
// Companion pane identity (docs/specs/agent-tree.allium: HideAgentTreePane)
// -----------------------------------------------------------------------

/// The regression the active/inactive heuristic was replaced for: with focus in
/// the companion pane, "the window's single inactive pane" is the *agent's*, so
/// the toggle killed the user's live claude session.
#[test]
fn toggle_kills_the_tree_pane_even_when_the_tree_pane_is_active() {
    let mock = MockProcessRunner::new(vec![
        // pane_ids_with_option_value: %2 carries the agent-tree role, and it is
        // the active pane — which the listing does not even report, because
        // identity no longer depends on it.
        MockProcessRunner::ok_with_stdout(b"%1 \n%2 agent_tree\n"),
        MockProcessRunner::ok(), // kill-pane
    ])
    .with_windows(&["task-42"]);

    toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(calls[1].1, vec!["kill-pane", "-t", "%2"]);
}

/// Hiding takes the diff pane with the tree. A diff pane outliving its tree is
/// orphaned: nothing drives its open set, nothing refreshes it, and this toggle
/// does not act on it — see KillAgentTreeDiffPaneWithItsTree in
/// docs/specs/agent-tree.allium.
#[test]
fn toggle_with_a_diff_pane_open_kills_both_panes() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1 \n%2 agent_tree\n%3 diff\n"),
        MockProcessRunner::ok(), // kill-pane: the diff pane
        MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options @dispatch_dir
        MockProcessRunner::ok(), // kill-pane: the tree
    ])
    .with_windows(&["task-42"]);

    toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap();

    let calls = mock.recorded_calls();
    let killed: Vec<&str> = calls
        .iter()
        .filter(|(_, args)| args[0] == "kill-pane")
        .map(|(_, args)| args[2].as_str())
        .collect();
    // The diff pane first, so it cannot briefly outlive its tree.
    assert_eq!(killed, vec!["%3", "%2"]);
}

/// The agent's own pane carries no role, so it is never touched however many
/// panes dispatch has put beside it. Presence-matching rather than
/// value-matching would have killed it here.
#[test]
fn toggle_never_kills_a_pane_dispatch_did_not_create() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1 \n%2 agent_tree\n"),
        MockProcessRunner::ok(), // kill-pane
    ])
    .with_windows(&["task-42"]);

    toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|(_, args)| args.contains(&"%1".to_string())),
        "the agent's own pane must be untouched; calls: {calls:?}"
    );
}

/// With nothing open there is no set to clear, so the toggle must not pay a
/// `show-options` round-trip resolving the worktree. This is the common case.
#[test]
fn toggle_with_no_diff_pane_costs_only_a_lookup_and_a_kill() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1 \n%2 agent_tree\n"),
        MockProcessRunner::ok(), // kill-pane
    ])
    .with_windows(&["task-42"]);

    toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap();

    assert_eq!(mock.recorded_calls().len(), 2);
}

/// A pane dispatch did not create carries no role, whatever it is running — so
/// the toggle re-splits rather than killing it. The lookup this replaced had to
/// defend against the *contents* of such a pane's command line: an editor opened
/// on docs/specs/agent-tree.allium matched a substring test.
#[test]
fn toggle_ignores_an_unmarked_pane_and_splits_a_tree_pane() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1 \n%3 \n"),
        MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options @dispatch_dir
        MockProcessRunner::ok_with_stdout(b"%4\n"),  // split-window
        MockProcessRunner::ok(),                     // set-option
    ])
    .with_windows(&["task-42"]);

    toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(calls[2].1[0], "split-window", "calls: {calls:?}");
    assert!(
        !calls.iter().any(|(_, args)| args[0] == "kill-pane"),
        "must not kill a pane it did not create; calls: {calls:?}"
    );
}

#[test]
fn toggle_splits_a_tree_pane_when_the_window_has_none() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1 \n"),
        MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options @dispatch_dir
        MockProcessRunner::ok_with_stdout(b"%2\n"),  // split-window
        MockProcessRunner::ok(),                     // set-option
    ])
    .with_windows(&["task-42"]);

    toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap();

    assert_eq!(mock.recorded_calls()[2].1[0], "split-window");
}

/// The spawn side writes the marker the lookup side reads — the whole mechanism
/// is these two halves agreeing, so the `set-option` is asserted verbatim.
#[test]
fn the_spawned_tree_pane_is_marked_with_its_role() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1 \n"),
        MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options @dispatch_dir
        MockProcessRunner::ok_with_stdout(b"%2\n"),  // split-window returns the new pane
        MockProcessRunner::ok(),                     // set-option
    ])
    .with_windows(&["task-42"]);

    toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(
        calls[3].1,
        vec![
            "set-option",
            "-p",
            "-t",
            "%2",
            tmux::PANE_ROLE_OPTION,
            tmux::PANE_ROLE_AGENT_TREE
        ],
        "calls: {calls:?}"
    );
}

/// The marker matters to the *next* toggle, not this one: the pane is already
/// open and rendering, so a failed write is logged rather than reported as a
/// failed toggle. Same accepted gap as the editor pane's own marker
/// (docs/specs/agent-tree.allium: OneEditorPanePerAgentWindow).
#[test]
fn a_failing_role_marker_write_does_not_fail_the_toggle() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1 \n"),
        MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options @dispatch_dir
        MockProcessRunner::ok_with_stdout(b"%2\n"),  // split-window
        MockProcessRunner::fail("bad option"),       // set-option: the marker write
    ])
    .with_windows(&["task-42"]);

    assert!(toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).is_ok());
    // The failure must land on the marker write, not on an earlier call — this
    // test is worthless if the split is what failed.
    let calls = mock.recorded_calls();
    assert_eq!(calls[3].1[0], "set-option", "calls: {calls:?}");
}

#[test]
fn toggle_is_a_no_op_for_a_window_that_is_not_a_task_window() {
    let mock = MockProcessRunner::new(vec![]).with_queued_window_lookup();
    toggle_agent_tree_pane(&test_tmux_window("TUI"), &mock).unwrap();
    assert!(mock.recorded_calls().is_empty());
}

/// Pinning moves only the agent's own pane out, so *every* pane dispatch put in
/// that window has to go with it — with an editor pane open the old
/// single-inactive-pane lookup was ambiguous and orphaned both.
#[test]
fn companion_pane_ids_returns_both_the_tree_and_the_editor_pane() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"%1 \n%2 agent_tree\n%3 editor\n",
    )])
    .with_windows(&["task-42"]);

    let found = companion_pane_ids(&test_tmux_window("task-42"), &mock).unwrap();

    assert_eq!(found, vec!["%2".to_string(), "%3".to_string()]);
}

/// One lookup, not one per role: "a pane dispatch created" is a single question
/// about the marker's presence, and asking it per role is what the three-call
/// version did — a cost paid on every pin, and a place for a future role to be
/// forgotten.
#[test]
fn companion_pane_ids_asks_tmux_once() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"%1 \n%2 agent_tree\n%3 editor\n",
    )])
    .with_windows(&["task-42"]);

    companion_pane_ids(&test_tmux_window("task-42"), &mock).unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1, "calls: {calls:?}");
    assert_eq!(calls[0].1[0], "list-panes");
}

#[test]
fn companion_pane_ids_is_empty_for_a_window_with_only_an_agent_pane() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%1 \n")])
        .with_windows(&["task-42"]);

    assert!(companion_pane_ids(&test_tmux_window("task-42"), &mock)
        .unwrap()
        .is_empty());
}

#[test]
fn build_prompt_includes_plan_path() {
    let prompt = build_prompt(
        TaskId(1),
        "Task",
        "Desc",
        Some("docs/plans/my-plan.md"),
        None,
        &PromptContext::default(),
    );
    assert!(prompt.contains("Plan: docs/plans/my-plan.md"));
}

#[test]
fn build_prompt_without_plan_omits_plan_section() {
    let prompt = build_prompt(
        TaskId(1),
        "Task",
        "Desc",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(!prompt.contains("Plan:"));
}

#[test]
fn build_quick_dispatch_prompt_includes_the_spec_first_sequence() {
    let prompt = build_quick_dispatch_prompt(
        TaskId(42),
        "Quick task",
        "",
        None,
        &PromptContext::default(),
    );
    assert!(
        prompt.contains("allium:elicit") && prompt.contains("docs/specs/"),
        "quick dispatch prompt should send the agent through the spec-first design step"
    );
    assert!(
        prompt.contains("asking the user"),
        "quick dispatch prompt should still open by asking the user what they want"
    );
}

#[test]
fn build_quick_dispatch_prompt_contains_rename_instruction() {
    let prompt = build_quick_dispatch_prompt(
        TaskId(42),
        "Quick task",
        "",
        None,
        &PromptContext::default(),
    );
    assert!(prompt.contains("42"));
    assert!(prompt.contains("Quick task"));
    assert!(prompt.contains("update_task"));
    assert!(prompt.contains("title"));
    assert!(prompt.contains("placeholder"));
}

#[test]
fn build_quick_dispatch_prompt_names_update_task_for_the_rename() {
    let prompt =
        build_quick_dispatch_prompt(TaskId(1), "Quick task", "", None, &PromptContext::default());
    // update_task is named because the prompt asks for a specific call — the
    // rename off the placeholder title — not as a general tool notice.
    assert!(prompt.contains("update_task"));
    assert!(!prompt.contains("dispatch MCP tools"));
    assert!(!prompt.contains("add_note"));
}

#[test]
fn build_quick_dispatch_prompt_differs_from_regular() {
    let regular = build_prompt(
        TaskId(1),
        "Task",
        "Desc",
        None,
        None,
        &PromptContext::default(),
    );
    let quick =
        build_quick_dispatch_prompt(TaskId(1), "Task", "Desc", None, &PromptContext::default());
    assert!(quick.contains("placeholder"));
    assert!(!regular.contains("placeholder"));
}

#[test]
fn build_quick_dispatch_prompt_includes_epic_context() {
    let ctx = EpicContext {
        epic_id: EpicId(7),
        epic_title: "My Epic".to_string(),
        under_cve_feed: false,
    };
    let prompt = build_quick_dispatch_prompt(
        TaskId(42),
        "Quick task",
        "",
        Some(&ctx),
        &PromptContext::default(),
    );
    assert!(prompt.contains("EpicId: 7"), "should include epic ID");
    assert!(prompt.contains("My Epic"), "should include epic title");
    assert!(
        prompt.contains("SendMessage"),
        "should tell agent how to message sibling agents"
    );
}

#[test]
fn no_plan_prompts_reference_the_elicit_skill() {
    let standard = build_prompt(TaskId(1), "T", "D", None, None, &PromptContext::default());
    let quick = build_quick_dispatch_prompt(TaskId(1), "T", "D", None, &PromptContext::default());

    for (name, prompt) in [("standard-no-plan", standard), ("quick", quick)] {
        assert!(
            prompt.contains("allium:elicit"),
            "{name} prompt should reference the allium:elicit skill"
        );
        assert!(
            !prompt.contains("/brainstorming"),
            "{name} prompt should no longer reference the retired /brainstorming skill"
        );
    }
}

/// Lines every aligned prompt carries, whichever design step it took.
///
/// Deliberately shorter than it was. `TDD` and the `allium_instruction` wording
/// used to be here, but the spec-first path now states both as numbered steps
/// and drops the trailing restatements — so the acronym and that exact sentence
/// are no longer universal, while what they were standing in for still is. The
/// `dispatch MCP tools` prose notice is gone from every prompt.
///
/// Test-first is covered by `every_implementation_prompt_states_test_first`,
/// which knows the two wordings; pinning it here would need a literal no path
/// shares.
const SHARED_TRAILING_LINES: &[&str] = &[
    "docs/specs/", // spec_first_instruction's steps, or allium_instruction
    "/learnings",  // learning_tools_instruction (names the skill, no MCP tool)
    "/wrap-up",    // wrap_up_instruction (universal)
];

/// Text no prompt may carry. The dispatch MCP tools reach the agent as real
/// tool schemas, so prompt prose that lists them shadows that list: it states
/// no fact the schema lacks, and it goes stale the moment the tool set changes.
/// The needle is the short form deliberately — it also catches a reworded
/// reintroduction, which the full sentence would not.
const SHARED_ABSENT_LINES: &[&str] = &["dispatch MCP tools"];

fn all_aligned_prompts() -> [(&'static str, String); 3] {
    [
        (
            "standard-no-plan",
            build_prompt(
                TaskId(1),
                "Task",
                "Desc",
                None,
                None,
                &PromptContext::default(),
            ),
        ),
        (
            "standard-with-plan",
            build_prompt(
                TaskId(1),
                "Task",
                "Desc",
                Some("docs/plans/p.md"),
                None,
                &PromptContext::default(),
            ),
        ),
        (
            "quick-dispatch",
            build_quick_dispatch_prompt(
                TaskId(1),
                "Quick task",
                "",
                None,
                &PromptContext::default(),
            ),
        ),
    ]
}

#[test]
fn every_prompt_includes_shared_trailing_metadata() {
    for (name, prompt) in all_aligned_prompts() {
        for needle in SHARED_TRAILING_LINES {
            assert!(
                prompt.contains(needle),
                "{name} prompt missing shared trailing line: {needle}\n--- prompt ---\n{prompt}"
            );
        }
        for needle in SHARED_ABSENT_LINES {
            assert!(
                !prompt.contains(needle),
                "{name} prompt carries text no prompt may carry: {needle}\n--- prompt ---\n{prompt}"
            );
        }
    }
}

#[test]
fn every_prompt_uses_task_block_format() {
    for (name, prompt) in all_aligned_prompts() {
        assert!(
            prompt.contains("Task:"),
            "{name} prompt should open task block with `Task:` (no `Epic:` header)\n{prompt}"
        );
        assert!(prompt.contains("ID:"), "{name} prompt should have `ID:`");
        assert!(
            prompt.contains("Title:"),
            "{name} prompt should have `Title:`"
        );
        assert!(
            prompt.contains("Description:"),
            "{name} prompt should have `Description:`"
        );
    }
}

/// Quick dispatch and the no-plan variant share one design instruction, so the
/// design step cannot drift apart between them.
#[test]
fn quick_dispatch_embeds_the_shared_spec_first_instruction() {
    let prompt =
        build_quick_dispatch_prompt(TaskId(1), "Quick task", "", None, &PromptContext::default());
    assert!(
        prompt.contains(spec_first_instruction()),
        "quick-dispatch prompt should embed spec_first_instruction verbatim"
    );
    assert!(
        !prompt.contains("vague or"),
        "no prompt should retain the retired vague-vs-clear branch"
    );
}

/// The sole owner of this line's wording contract. It asserts both terminal
/// states rather than either-or, and that the stopping-point rule is NOT here —
/// that rule moved to its own conditional line, and restating it here would
/// repeat both design steps (`NoLineRestatesTheDesignStep` in
/// `docs/specs/dispatch-prompt.allium`).
#[test]
fn wrap_up_instruction_universal_wording() {
    let text = wrap_up_instruction();
    assert!(
        text.contains("/wrap-up"),
        "wrap_up_instruction should reference the /wrap-up skill"
    );
    assert!(
        text.contains("finishing implementation"),
        "wrap_up_instruction should name finishing implementation as terminal, got: {text}"
    );
    assert!(
        text.contains("creating work packages"),
        "wrap_up_instruction should name work-package creation as terminal for an \
epic-decomposition task, got: {text}"
    );
    assert!(
        !text.contains("not a stopping point"),
        "the stopping-point rule belongs to plan_not_a_stopping_point_instruction; \
keeping it here restates both design steps, got: {text}"
    );
}

/// The instruction is a numbered sequence, so it is necessarily longer than the
/// one-liner it replaced — but it must stay a compact runbook, not an essay.
#[test]
fn spec_first_instruction_stays_compact() {
    let instruction = spec_first_instruction();
    assert!(
        instruction.len() < 1200,
        "spec_first_instruction should stay a compact runbook (< 1200 chars), got {} chars",
        instruction.len()
    );
    assert!(instruction.contains("allium:elicit"));
    assert!(instruction.contains("update_task"));
    assert!(instruction.contains("docs/plans/"));
}

#[test]
fn epic_preamble_returns_empty_strings_for_none() {
    let (id_line, section) = epic_preamble(None);
    assert!(id_line.is_empty());
    assert!(section.is_empty());
}

#[test]
fn epic_preamble_returns_id_line_and_section_for_some() {
    let ctx = EpicContext {
        epic_id: EpicId(5),
        epic_title: "Auth Rework".to_string(),
        under_cve_feed: false,
    };
    let (id_line, section) = epic_preamble(Some(&ctx));
    assert!(id_line.contains("EpicId: 5"));
    assert!(section.contains("Auth Rework"));
    assert!(
        section.contains("SendMessage") && section.contains("ListAgents"),
        "should guide agent to use native cross-session messaging, got: {section}"
    );
    assert!(
        section.contains("task-<id>") || section.contains("task-{"),
        "should name the task-<id> session-naming convention, got: {section}"
    );
    assert!(
        !section.contains("send_message"),
        "the dispatch send_message MCP tool no longer exists, got: {section}"
    );
    assert!(
        !section.contains("Sibling tasks:"),
        "should not enumerate sibling tasks"
    );
}
