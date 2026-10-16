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
        "hooks/scripts/pr-learnings-hook",
        "skills/wrap-up/SKILL.md",
        "skills/retro/SKILL.md",
        "skills/decompose-review/SKILL.md",
        "skills/decompose-review/references/plan-template.md",
        "skills/learnings/SKILL.md",
        "skills/grill/SKILL.md",
        "skills/fix/SKILL.md",
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
/// it is the only thing that catches a regression here: epic chaining is a
/// server-side effect of `exit_session` and there is no `dispatch_next` tool
/// left for the skill to name.
#[test]
fn wrap_up_skill_does_not_instruct_calling_dispatch_next() {
    let content = skill_body("wrap-up");
    assert!(
        !content.contains("dispatch_next"),
        "wrap-up skill must not tell the agent to call dispatch_next — \
         exit_session chains the next epic subtask automatically"
    );
}

/// A regression in this guidance is silent: an agent that stops at a refused
/// close leaves a live tmux window and a task stuck in its old status, when one
/// more call would have closed it. Since store-split M5 the close is one
/// all-or-nothing module step, so it is an ERROR that changed nothing.
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
#[test]
fn wrap_up_skill_tells_the_agent_to_call_exit_session_again_after_a_failed_close() {
    let section = failed_close_guidance();
    assert!(
        section.contains("exit_session") && section.contains("again"),
        "failed-close guidance must tell the agent to call exit_session again"
    );
    assert!(
        section.contains("still valid"),
        "failed-close guidance must say the exit token is still valid"
    );
    assert!(
        section.contains("wrap_up") && section.contains("do not"),
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

/// The "Draft the title and body" step of the PR path, isolated from its
/// neighbouring steps. Not built on `section_after`: that helper ends a
/// section at the next line starting with `#`, but this step's own
/// Markdown examples may contain `#`-prefixed lines as literal example text,
/// which would truncate the section before the part these tests need to
/// inspect.
fn pr_body_draft_section() -> String {
    let content = references_text("wrap-up");
    let (_, after) = content
        .split_once("### Draft the title and body")
        .expect("wrap-up skill must have a 'Draft the title and body' step");
    let (section, _) = after.split_once("### Push and create the draft PR").expect(
        "wrap-up skill must have a 'Push and create the draft PR' step \
             after drafting",
    );
    section.to_string()
}

/// The PR body guidance states principles, not a template: it must not
/// bring back the fixed `## Summary` + bold category labels shape, and it
/// must keep the reviewer-minute test that makes the principles checkable.
/// The phrases "no template" and "under a minute" are anchors: if you
/// reword them, re-anchor here, and keep the 500-character feed note.
#[test]
fn wrap_up_skill_pr_body_states_principles_not_a_template() {
    let section = pr_body_draft_section();
    for label in ["**Breaking Changes**", "**New Features**", "**Bug Fixes**"] {
        assert!(
            !section.contains(label),
            "PR body guidance must not prescribe category labels, found: {label}"
        );
    }
    assert!(
        !section.contains("## Summary"),
        "PR body guidance must not prescribe a Summary heading"
    );
    assert!(
        section.to_lowercase().contains("no template"),
        "PR body guidance must say plainly that there is no template"
    );
    assert!(
        section.contains("under a minute"),
        "PR body guidance must keep the reviewer-minute test"
    );
    assert!(
        section.contains("500 characters"),
        "PR body guidance must say the review feed shows only the first 500 \
         characters, which is why the opening carries the PR"
    );
}

/// Distilled from the shared PR-description knowledge base (learnings
/// #146 and #154): dispatch task IDs must never appear in a PR body —
/// GitHub auto-links `#N` to an unrelated issue/PR in the target repo.
/// The old template's `Implements #{task_id}.` line was a live
/// contradiction of both learnings until this test locked it out.
#[test]
fn wrap_up_skill_pr_body_omits_task_references() {
    let section = pr_body_draft_section();
    assert!(
        !section.contains("Implements #{task_id}"),
        "PR body template must not instruct the agent to write \
         \"Implements #{{task_id}}\" into the body — GitHub auto-links #N \
         to an unrelated issue/PR in this repo"
    );
    assert!(
        section
            .to_lowercase()
            .contains("do not reference the dispatch task"),
        "PR body template must explicitly warn against referencing the \
         dispatch task (task IDs, \"Implements #N\")"
    );
}

/// Distilled from the shared PR-description knowledge base (learning
/// #188): the Test plan section is opt-in for dangerous/breaking PRs,
/// not a default part of every PR body.
#[test]
fn wrap_up_skill_pr_body_test_plan_is_opt_in_not_default() {
    let section = pr_body_draft_section();
    assert!(
        !section.contains("- [ ] {how to verify"),
        "PR body template must not show an unconditional Test plan checklist \
         in its default example"
    );
    let lower = section.to_lowercase();
    assert!(
        lower.contains("test plan") && (lower.contains("dangerous") || lower.contains("breaking")),
        "PR body template must say the Test plan section is only for \
         dangerous or breaking PRs, omitted by default"
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
