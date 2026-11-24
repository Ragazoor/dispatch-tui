use super::*;

// -- Plugin metadata --

#[test]
fn plugin_json_is_valid() {
    let content = PLUGIN_DIR
        .get_file(".claude-plugin/plugin.json")
        .expect("plugin.json must be embedded")
        .contents_utf8()
        .expect("plugin.json must be UTF-8");
    let value: Value = serde_json::from_str(content).expect("plugin.json is invalid JSON");
    assert_eq!(value["name"], "dispatch");
}

#[test]
fn plugin_embeds_required_files() {
    let required = [
        ".claude-plugin/plugin.json",
        "hooks/hooks.json",
        "hooks/scripts/task-status-hook",
        "hooks/scripts/pr-gate-hook",
        "skills/wrap-up/SKILL.md",
        "skills/pull-request/SKILL.md",
        "skills/retro/SKILL.md",
        "skills/decompose-review/SKILL.md",
        "skills/decompose-review/references/plan-template.md",
        "skills/learnings/SKILL.md",
        "skills/grill/SKILL.md",
        "skills/fix/SKILL.md",
        "skills/preference-review/SKILL.md",
        "skills/allium-loop/SKILL.md",
        "skills/allium-loop/prompt.md",
    ];
    for path in required {
        assert!(
            PLUGIN_DIR.get_file(path).is_some(),
            "{path} must be embedded in PLUGIN_DIR"
        );
    }
}

/// Pins SimplifyOnlyForLargeChanges of `WrapUpSession` in `docs/specs/wrap-up-skill.allium`.
#[test]
fn wrap_up_skill_uses_simplify_not_code_simplifier() {
    let content = skill_body("wrap-up");
    assert!(
        !content.contains("code-simplifier"),
        "wrap-up skill must not reference the old 'code-simplifier' skill"
    );
    assert!(
        content.contains("\"simplify\""),
        "wrap-up skill must reference the 'simplify' skill"
    );
}

/// Step 3's own section, isolated so a size-threshold regression doesn't
/// hide behind unrelated wording elsewhere in the skill.
fn wrap_up_step3_section() -> String {
    section_after(skill_body("wrap-up"), "## Step 3: Simplify code changes")
        .unwrap_or_else(|| panic!("wrap-up skill must have a Step 3 for simplify"))
}

/// Task #4973: retrospective analysis of ~30 sessions found `simplify`
/// found real fixes (a test that couldn't fail, an O(n log n) sort-key
/// bug, a blocking call on the TUI's frame loop) almost exclusively on
/// large diffs, and found nothing on small ones — while still costing
/// four review sub-agents every time it runs. Gating Step 3 on file
/// extension alone (whether *any* source line changed) spins those four
/// agents up for single-digit-line fixes that consistently came back
/// empty.
///
/// Pins SimplifyOnlyForLargeChanges of `WrapUpSession` in `docs/specs/wrap-up-skill.allium`.
#[test]
fn wrap_up_step3_gates_simplify_on_changed_line_count() {
    let section = wrap_up_step3_section();
    assert!(
        section.contains("numstat"),
        "Step 3 must measure the diff with `git diff --numstat` (or \
         equivalent) rather than gating on file extension alone"
    );
    assert!(
        section.contains("100"),
        "Step 3 must state a concrete changed-line threshold below which \
         simplify is skipped"
    );
}

/// The size gate is additional to the file-type filter, not a replacement
/// for it — a docs/config-only change must still skip Step 3 outright,
/// regardless of how many lines it touches.
///
/// Pins SimplifyOnlyForLargeChanges of `WrapUpSession` in `docs/specs/wrap-up-skill.allium`.
#[test]
fn wrap_up_step3_still_skips_docs_only_changes_regardless_of_size() {
    let section = wrap_up_step3_section().to_lowercase();
    assert!(
        section.contains("docs") && section.contains("configs"),
        "Step 3 must still skip entirely for docs/config-only changes, \
         independent of the new line-count gate"
    );
}

/// The embedded copy of the wrap-up skill is what agents actually read, so
/// it is the only thing that catches a regression here: there is no
/// `dispatch_next` tool for the skill to name.
#[test]
fn wrap_up_skill_does_not_instruct_calling_dispatch_next() {
    let content = skill_body("wrap-up");
    assert!(
        !content.contains("dispatch_next"),
        "wrap-up skill must not tell the agent to call dispatch_next, which does not exist"
    );
}

/// A regression in this guidance is silent: an agent that stops at a refused
/// close leaves a live tmux window and a task stuck in its old status, when one
/// more call would have closed it. The close is one
/// all-or-nothing module step, so it is an ERROR that changed nothing.
///
/// Pins FailedCloseReadsTheReference of `WrapUpSession` in `docs/specs/wrap-up-skill.allium`.
#[test]
fn wrap_up_skill_says_a_failed_close_is_an_error_that_changed_nothing() {
    let section = failed_close_guidance();
    assert!(
        section.contains("error") && section.contains("changes nothing"),
        "failed-close guidance must say the close errored and changed nothing"
    );
}

/// The one reaction that must survive any rewording of the failed-close
/// guidance: the exit token is NOT spent by a failed close (the close is one
/// transaction), so calling `exit_session` again is the way out, and a fresh
/// `wrap_up` is not. Asserted positively — a negative check would also pass if
/// the guidance were deleted outright, which is the regression that matters.
///
/// Pins FailedCloseReadsTheReference of `WrapUpSession` in `docs/specs/wrap-up-skill.allium`.
#[test]
fn wrap_up_skill_tells_the_agent_to_call_exit_session_again_after_a_failed_close() {
    let section = failed_close_guidance();
    assert!(
        section.contains("session_exit") && section.contains("again"),
        "failed-close guidance must tell the agent to call session_exit again"
    );
    assert!(
        section.contains("still valid"),
        "failed-close guidance must say the exit token is still valid"
    );
    assert!(
        section.contains("session_wrap-up") && section.contains("do not"),
        "failed-close guidance must tell the agent not to call wrap_up again for a \
         fresh token"
    );
}

/// Regression guard for the stale instruction that survived unnoticed until
/// task #3769: the skill told the agent to record the PR URL via
/// `update_task`, when the URL actually travels with `exit_session`. It
/// drifted precisely because nothing pinned it. Scoped to lines that pair
/// `update_task` with a URL, so an unrelated future use of the tool is not
/// blocked.
///
/// Pins PrPathIsAuthoredByTheAgent of `WrapUpSession` in `docs/specs/wrap-up-skill.allium`.
#[test]
fn wrap_up_skill_does_not_record_the_pr_url_via_update_task() {
    for line in skill_text("wrap-up").lines() {
        let lower = line.to_lowercase();
        assert!(
            !(lower.contains("update_task") && lower.contains("url")),
            "wrap-up skill must not tell the agent to record the PR URL via \
             update_task — the URL travels with exit_session: {line}"
        );
    }
}

/// pr.md carries no copy of the PR rules; it sends the agent to the
/// /pull-request skill, by both routes, so the rules have one source.
///
/// Pins PrFollowsThePullRequestSkill of `WrapUpSession` in `docs/specs/wrap-up-skill.allium`.
#[test]
fn wrap_up_pr_path_points_to_the_pull_request_skill() {
    let refs = references_text("wrap-up");
    assert!(
        refs.contains("dispatch:pull-request") && refs.contains("skill://pull-request/SKILL.md"),
        "references/pr.md must name the pull-request skill by both routes"
    );
    for copied in ["500 characters", "under a minute", "Implements #"] {
        assert!(
            !refs.contains(copied),
            "references/pr.md must not carry a copy of the skill's PR rules, found: {copied}"
        );
    }
    assert!(
        refs.contains("`--base`"),
        "references/pr.md must still open the PR against the task's base branch"
    );
    assert!(
        refs.contains("pr_url"),
        "references/pr.md must still say the PR URL goes to session_exit as pr_url"
    );
}

/// The wrap-up skill's failed-close guidance block, lowercased: the heading
/// section telling the agent that an `exit_session` error can report that the
/// close did not take effect, and that the token is then still valid.
///
/// Scoped to that one section deliberately — "do not retry" also appears in
/// the neighbouring `exit_session` *errors* section, so a whole-document
/// check would still pass with this section's retry guidance deleted. The
/// section is anchored on the phrase below and ends at the next heading of
/// any depth (so promoting or demoting the heading cannot silently widen it
/// to the rest of the file); if you reword the heading, re-anchor it here.
fn failed_close_guidance() -> String {
    // pr-workflow.allium ExitSession: the reaction lives in the skill's
    // exit_session-errors reference file, not in SKILL.md.
    let content = references_text("wrap-up").to_lowercase();
    section_after(&content, "did not take effect").expect(
        "wrap-up skill must document that an exit_session error can report \
         that the close did not take effect",
    )
}

/// /wrap-up is the agent-side flow around `wrap_up` and `exit_session`. Each
/// guarantee of `WrapUpSession` in `docs/specs/wrap-up-skill.allium` that the
/// skill text can hold is pinned here by the words in `SKILL.md` that hold it;
/// the skill is followed by an agent, so the text is the behaviour a test can
/// pin. Guarantees with their own test carry a `Pins` line on it instead.
#[test]
fn wrap_up_skill_text_holds_each_session_guarantee() {
    let content = skill_body("wrap-up");
    // Line breaks in the source are not behaviour.
    let flat = content.split_whitespace().collect::<Vec<_>>().join(" ");
    for (guarantee, needle) in [
        (
            "AnnouncesItself",
            "I'm using the wrap-up skill to complete this task.",
        ),
        (
            "TaskIdComesFromTheBranch",
            "leading integer of the `{id}-{slug}` branch name",
        ),
        (
            "TaskIdComesFromTheBranch",
            "doesn't follow the dispatch naming convention",
        ),
        ("PathIsTheOperatorsChoice", "Ignore any other argument"),
        ("PathIsTheOperatorsChoice", "Otherwise ask and wait"),
        (
            "PathIsTheOperatorsChoice",
            "When your task's prompt names an orchestrator, send it this question as a message",
        ),
        (
            "PathIsTheOperatorsChoice",
            "Without one, or when it is not running, use `AskUserQuestion`",
        ),
        (
            "ReportsTheOutcomeToItsOrchestrator",
            "If your task's prompt names an orchestrator, send it a short outcome report",
        ),
        (
            "PathIsTheOperatorsChoice",
            "even work that looks like a PR may want a rebase",
        ),
        (
            "RefusedTaskStopsEarly",
            "`Wrap-up: refused — <reason>` line means the task cannot be wrapped up",
        ),
        (
            "RefusedTaskStopsEarly",
            "stop, before the retro or any commit",
        ),
        ("CancelCallsNothing", "exit without calling any tool"),
        (
            "PresetAnswerIsNotAPause",
            "do not stop before `session_exit` returns",
        ),
        (
            "PresetAnswerIsNotAPause",
            "Its summary is not a stopping point",
        ),
        (
            "VerifyBeforeWrapUp",
            "on the rebase and pr paths, run it in your worktree now",
        ),
        (
            "VerifyBeforeWrapUp",
            "On the done path, run no verify command",
        ),
        (
            "VerifyBeforeWrapUp",
            "exactly as printed, with no arguments added",
        ),
        (
            "VerifyBeforeWrapUp",
            "do not run it again after `session_wrap-up`",
        ),
        (
            "VerifyBeforeWrapUp",
            "Dispatch does not run it for you and does not check that you did",
        ),
        ("VerifyBeforeWrapUp", "keeps a red build out of the push"),
        (
            "VerifyFailureIsNotWrappedPast",
            "do not wrap up past it alone",
        ),
        (
            "VerifyFailureIsNotWrappedPast",
            "coordinate with other agents and the board",
        ),
        ("CommitBeforeWrapUp", "`git add` the relevant files by name"),
        ("CommitBeforeWrapUp", "review `git diff --cached`"),
        (
            "CommitBeforeWrapUp",
            "`session_wrap-up` refuses a dirty worktree",
        ),
        ("RateBeforeClosing", "Rate retrieved knowledge"),
        (
            "RateBeforeClosing",
            "every entry from \"## Validated knowledge for this task\" that did not fit this task `not_applicable`",
        ),
        ("ClosingSequenceRunsInOneTurn", "back to back in one turn"),
        ("RefusalIsShownAsWritten", "show the user the exact message"),
        ("RefusalIsShownAsWritten", "suggest resolution steps"),
        ("RefusalIsShownAsWritten", "show the exact message and stop"),
        (
            "ExitSessionNeedsAToken",
            "Do not call `session_exit` without a token",
        ),
        ("ExitSessionIsMandatoryAndLast", "mandatory on every path"),
        ("ExitSessionIsMandatoryAndLast", "Make it your final action"),
        (
            "ExitSessionIsMandatoryAndLast",
            "plus `pr_url` on the pr path only",
        ),
        (
            "FailedCloseReadsTheReference",
            "read `references/exit-session-errors.md` before reacting",
        ),
        (
            "BranchDetailLivesInReferences",
            "read `references/pr.md` when this path is chosen",
        ),
        ("PrPathIsAuthoredByTheAgent", "Write a real title and body"),
    ] {
        assert!(
            flat.contains(needle),
            "wrap-up skill must hold {guarantee}: missing {needle:?}"
        );
    }
    // BranchDetailLivesInReferences: the files SKILL.md names exist.
    for file in ["pr.md", "exit-session-errors.md"] {
        assert!(
            skill_references("wrap-up")
                .iter()
                .any(|(name, _)| name == file),
            "wrap-up must ship references/{file}"
        );
    }
    // The steps run in the order the spec gives: task, simplify, path, retro,
    // verify and commit, then rate, wrap_up and exit_session.
    let order = [
        "## Step 1: Get the task ID",
        "## Step 2: Get task details",
        "## Step 3: Simplify",
        "## Step 4: Ask which path",
        "## Step 5: Run the retro",
        "## Step 6: Verify, then commit",
        "## Step 7: The closing sequence",
        "Rate retrieved knowledge",
        "**`session_wrap-up(task_id, action)`.**",
        "**`session_exit(task_id, token, action)`**",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|n| {
            content
                .find(n)
                .unwrap_or_else(|| panic!("wrap-up skill must have {n}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "wrap-up must follow the spec's step order: {order:?}"
    );
}

/// The reference files are read from inside the flow, so a step number they
/// cite must be one SKILL.md has. The pr reference once sent the agent to
/// "Step 8" and "Step C", which do not exist.
///
/// Pins BranchDetailLivesInReferences of `WrapUpSession` in `docs/specs/wrap-up-skill.allium`.
#[test]
fn wrap_up_references_cite_only_steps_the_skill_has() {
    let body = skill_body("wrap-up");
    let steps: Vec<u32> = body
        .lines()
        .filter_map(|l| l.strip_prefix("## Step "))
        .filter_map(|l| l.split(':').next()?.parse().ok())
        .collect();
    assert!(!steps.is_empty(), "wrap-up skill must have numbered steps");
    for (name, text) in skill_references("wrap-up") {
        for (at, _) in text.match_indices("Step ") {
            let token: String = text[at + 5..]
                .chars()
                .take_while(|c| c.is_alphanumeric())
                .collect();
            let n: u32 = token.parse().unwrap_or_else(|_| {
                panic!("references/{name} cites \"Step {token}\", which is not a step number")
            });
            assert!(
                steps.contains(&n),
                "references/{name} cites Step {n}, which SKILL.md does not have"
            );
        }
    }
}
