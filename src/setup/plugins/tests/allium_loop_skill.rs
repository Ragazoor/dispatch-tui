use super::*;

/// A skill file with every run of whitespace folded to one space, so a
/// phrase the Markdown wraps across lines still matches.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// /allium-loop is driven by a session and run by fresh iteration agents.
/// Each guarantee of `AlliumLoopSession` in `docs/specs/allium-loop-skill.allium`
/// that the skill text can hold is pinned here by the words that hold it:
/// the driver's flow in `SKILL.md`, the result handling in
/// `references/handling-results.md`, and the iteration's own task in
/// `prompt.md`. The skill is followed by an agent, so the text is the
/// behaviour a test can pin. `reference_files.rs` pins
/// IterationBudgetAndModelDefault's sonnet pin, the no-fork rule of
/// IterationsAreFreshAgentsOnThePinnedModel and the deferred-work and
/// replacement-test halves of ConvergenceIsGated.
#[test]
fn allium_loop_text_holds_each_session_guarantee() {
    let skill = flat(skill_body("allium-loop"));
    let results = flat(skill_file("allium-loop", "references/handling-results.md"));
    let prompt = flat(skill_file("allium-loop", "prompt.md"));
    let in_skill = [
        (
            "NeverOverwritesAnActiveLoop",
            "Never silently overwrite an active state file.",
        ),
        (
            "NeverOverwritesAnActiveLoop",
            "`runs_completed` of `max_iterations`",
        ),
        (
            "NeverOverwritesAnActiveLoop",
            "do not reset `runs_completed`, `retry_count`, or `consecutive_no_change_runs`",
        ),
        ("NeverOverwritesAnActiveLoop", "**Resume**"),
        ("NeverOverwritesAnActiveLoop", "**Abandon**"),
        ("NeverOverwritesAnActiveLoop", "**Cancel**"),
        ("DesignDocumentIsResolved", "**Explicit arg**"),
        ("DesignDocumentIsResolved", "**Recent context**"),
        ("DesignDocumentIsResolved", "**Ask**"),
        (
            "VerifyCommandIsResolvedNotAssumed",
            "**Task/session context**",
        ),
        ("VerifyCommandIsResolvedNotAssumed", "**Project docs**"),
        ("BaseBranchIsNeverAssumed", "use that task's `base_branch`"),
        (
            "StateFileHoldsTheLoop",
            "immediately (see \"Each Iteration\" below)",
        ),
        ("StateFileHoldsTheLoop", "max_iterations: MAX_ITERATIONS"),
        (
            "DesignDocumentIsResolved",
            "An `.allium` file is a first-class input here",
        ),
        (
            "DesignDocumentIsResolved",
            "**tell the user which document was picked**",
        ),
        (
            "VerifyCommandIsResolvedNotAssumed",
            "it never assumes a particular test runner or toolchain",
        ),
        (
            "VerifyCommandIsResolvedNotAssumed",
            "ask the user for the command via AskUserQuestion before starting the loop",
        ),
        ("BaseBranchIsNeverAssumed", "rather than assuming `main`"),
        ("IterationBudgetAndModelDefault", "otherwise default to `6`"),
        (
            "IterationBudgetAndModelDefault",
            "otherwise default to `sonnet`",
        ),
        (
            "StateFileHoldsTheLoop",
            "**Abandon** — delete the state file and continue with a fresh kickoff",
        ),
        ("StateFileHoldsTheLoop", "active: true"),
        ("StateFileHoldsTheLoop", "runs_completed: 0"),
        ("StateFileHoldsTheLoop", "retry_count: 0"),
        ("StateFileHoldsTheLoop", "consecutive_no_change_runs: 0"),
        ("StateFileHoldsTheLoop", "design_doc: \"DESIGN_DOC_PATH\""),
        ("StateFileHoldsTheLoop", "base_branch: \"BASE_BRANCH\""),
        (
            "StateFileHoldsTheLoop",
            "verify_command: \"VERIFY_COMMAND\"",
        ),
        ("StateFileHoldsTheLoop", "model: \"MODEL\""),
        ("StateFileHoldsTheLoop", "started_at: \"TIMESTAMP\""),
        (
            "IterationsAreFreshAgentsOnThePinnedModel",
            "(do not pass `subagent_type: \"fork\"`)",
        ),
        (
            "IterationsAreFreshAgentsOnThePinnedModel",
            "passing `model` set to the state file's `model` value",
        ),
        (
            "IterationsAreFreshAgentsOnThePinnedModel",
            "dispatches one fresh subagent per iteration",
        ),
        (
            "IterationsAreFreshAgentsOnThePinnedModel",
            "The driver never does the rebase/tend/implement/verify/weed work itself.",
        ),
        (
            "IterationsAreFreshAgentsOnThePinnedModel",
            "Read `model` from the state file rather than remembering it",
        ),
    ];
    let in_results = [
        ("DriverKeepsTheBooks", "by exactly 1 and reset `retry_count` to `0`"),
        (
            "DriverKeepsTheBooks",
            "this run's `SUMMARY` reports no changes, increment it by 1; otherwise reset it to `0`",
        ),
        (
            "FailureIsRetriedOnce",
            "missing EITHER required label (`CONVERGED:` or `SUMMARY:`), not just `CONVERGED:`",
        ),
        (
            "FailureIsRetriedOnce",
            "If `retry_count == 0`: set it to `1`, keep `{{ITERATION_NUMBER}}` unchanged",
        ),
        (
            "FailureIsRetriedOnce",
            "failed twice and the loop has stopped",
        ),
        (
            "StopsAtBudgetOrStall",
            "`runs_completed >= max_iterations` or `consecutive_no_change_runs >= 2`",
        ),
        (
            "StopsAtBudgetOrStall",
            "Never emit a false convergence claim to exit early.",
        ),
        (
            "StopsAtBudgetOrStall",
            "delete the state file, summarize to the user exactly what's unresolved and why, and stop",
        ),
        (
            "StateFileHoldsTheLoop",
            "delete `.claude/allium-loop-state.local.md` and report success to the user, including the final `SUMMARY`",
        ),
        ("LoopContinues", "dispatch the next iteration (repeat step 1 above)"),
        (
            "FailureIsRetriedOnce",
            "delete the state file and tell the user this iteration",
        ),
    ];
    let in_prompt = [
        ("RebaseIsNeverSkipped", "Never skip the rebase step."),
        (
            "RebaseIsNeverSkipped",
            "never silently drop a clause, and call out exactly what you did in your final report",
        ),
        (
            "SpecAgentsAreNotOverridden",
            "Do NOT pass a `model` override to `allium:tend` (or to `allium:weed`",
        ),
        (
            "OpenQuestionsAreAskedAndWrittenDown",
            "STOP and resolve it with the user via AskUserQuestion",
        ),
        (
            "OpenQuestionsAreAskedAndWrittenDown",
            "write the resolution into the spec",
        ),
        (
            "OpenQuestionsAreAskedAndWrittenDown",
            "include that edit in this run's commit (step 8) before you end",
        ),
        (
            "OpenQuestionsAreAskedAndWrittenDown",
            "git status --porcelain -- docs/specs/",
        ),
        (
            "OpenQuestionsAreAskedAndWrittenDown",
            "Resolving an open question and committing that resolution is itself a complete, valid run",
        ),
        (
            "IterationFirstTendsTheSpec",
            "Do not pre-declare a target file",
        ),
        ("IterationImplements", "no magic numbers"),
        (
            "IterationWeeds",
            "for code bugs that contradict a correct spec, ask the user before fixing",
        ),
        (
            "RedCheckPrecedesImplementation",
            "confirm they FAIL. A new test that already passes signals redundancy or vacuity",
        ),
        (
            "RedCheckPrecedesImplementation",
            "Confirm new tests fail before implementing",
        ),
        (
            "GeneratedTestsAreNeverWeakened",
            "Never weaken or hand-edit generated tests.",
        ),
        ("FixCodeNotTheContract", "fix the CODE, not the tests"),
        (
            "FixCodeNotTheContract",
            "the spec is likely wrong: STOP and ask the user via AskUserQuestion",
        ),
        (
            "EveryIterationEndsWithACommit",
            "Never end with a dirty tree",
        ),
        (
            "EveryIterationEndsWithACommit",
            "State this plainly in your final report too",
        ),
        (
            "EveryIterationEndsWithACommit",
            "treat fixing this as its first priority",
        ),
        (
            "EveryIterationEndsWithACommit",
            "Ending with no new commit is acceptable only when the tree is genuinely clean",
        ),
        (
            "EveryIterationEndsWithACommit",
            "wip(allium-loop): iteration {{ITERATION_NUMBER}}, verify failing",
        ),
        (
            "StateFileIsNeverCommitted",
            "Never stage or commit the loop's own state file",
        ),
        (
            "ReportIsTheTwoLineBlock",
            "Always end with the exact two-line `CONVERGED:` / `SUMMARY:` block",
        ),
        (
            "ConvergenceIsGated",
            "Emit `CONVERGED: yes` ONLY when ALL hold:",
        ),
        ("ConvergenceIsGated", "`{{VERIFY_COMMAND}}` passes."),
        (
            "ConvergenceIsGated",
            "name the surviving test by function and file",
        ),
        (
            "ConvergenceIsGated",
            "confirm it asserts the same direction",
        ),
        (
            "ReportIsTheTwoLineBlock",
            "or \"no changes\" if this run genuinely produced none",
        ),
        (
            "ConvergenceIsGated",
            "`allium:weed` reports no spec-code divergence",
        ),
        (
            "ConvergenceIsGated",
            "Every spec touched this run has an empty `open questions` section",
        ),
    ];
    for (file, text, needles) in [
        ("SKILL.md", &skill, &in_skill[..]),
        ("references/handling-results.md", &results, &in_results[..]),
        ("prompt.md", &prompt, &in_prompt[..]),
    ] {
        for (guarantee, needle) in needles {
            assert!(
                text.contains(&flat(needle)),
                "allium-loop {file} must hold {guarantee}: missing {needle:?}"
            );
        }
    }
}

/// AlliumLoop.stage: kickoff checks for an active loop before it resolves
/// anything, and the state file is written before the first dispatch.
#[test]
fn allium_loop_kickoff_checks_for_an_active_loop_before_resolving() {
    let skill = skill_body("allium-loop");
    let order = [
        "Check for an existing active loop first.",
        "**Resolve the input design/spec document**",
        "**Resolve the verify command**",
        "**Resolve the base branch**",
        "**Resolve `max_iterations`**",
        "**Resolve `model`**",
        "**Create the loop state file**",
        "### Each Iteration",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|n| {
            skill
                .find(n)
                .unwrap_or_else(|| panic!("allium-loop skill must have {n:?}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "allium-loop kickoff must follow the AlliumLoop kickoff order: {order:?}"
    );
}

/// One iteration's steps, in the order the spec's IterationRebases ..
/// IterationReports rules name them: the red check sits before implementing,
/// the commit after weed, and the report is last.
#[test]
fn allium_loop_iteration_steps_run_in_spec_order() {
    let prompt = skill_file("allium-loop", "prompt.md");
    let order = [
        "### 1. Rebase",
        "### 2. Advance the spec(s)",
        "### 3. Propagate tests",
        "### 4. Red check",
        "### 5. Implement",
        "### 6. Verify",
        "### 7. Weed",
        "### 8. Commit",
        "### 9. Report",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|n| {
            prompt
                .find(n)
                .unwrap_or_else(|| panic!("allium-loop prompt must have {n:?}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "allium-loop iteration must run its steps in order: {order:?}"
    );
}

/// IterationFirstTendsTheSpec and LaterIterationsTendOnlyOnSpecError: the
/// spec is tended from the design document on iteration 1 only, and after
/// that only when the run's own work reveals a spec error.
#[test]
fn allium_loop_tends_from_the_design_doc_on_iteration_one_only() {
    let prompt = flat(skill_file("allium-loop", "prompt.md"));
    for needle in [
        "If `{{ITERATION_NUMBER}}` is `1`, use the Agent tool with `subagent_type: \"allium:tend\"`",
        "On later iterations, only re-invoke `allium:tend` if this run's work reveals a spec error",
    ] {
        assert!(
            prompt.contains(needle),
            "allium-loop prompt must keep the tend rule: missing {needle:?}"
        );
    }
}

/// DispatchIteration: the iteration number is derived from the state file,
/// so a resume or a context compaction cannot lose it, and a retry reuses it.
#[test]
fn allium_loop_derives_the_iteration_number_from_the_state_file() {
    let skill = flat(skill_body("allium-loop"));
    assert!(
        skill.contains("always derived from the state file as `runs_completed + 1`"),
        "the iteration number must be derived from the state file"
    );
    assert!(
        skill.contains(
            "`{{DESIGN_DOC}}`, `{{VERIFY_COMMAND}}`, `{{BASE_BRANCH}}`, and `{{ITERATION_NUMBER}}`"
        ),
        "the dispatch must fill all four prompt placeholders"
    );
}
