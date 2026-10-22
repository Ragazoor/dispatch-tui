use super::*;

#[test]
fn decompose_review_skill_defaults_wrap_up_mode_to_rebase() {
    // Review work packages are small and land on main — one draft PR per
    // package is noise. The skill pre-sets wrap_up_mode purely to skip
    // wrap-up's AskUserQuestion step, and 'rebase' is the right value.
    let content = skill_body("decompose-review");
    assert!(
        content.contains("`wrap_up_mode`: `\"rebase\"`"),
        "decompose-review skill must set wrap_up_mode to \"rebase\""
    );
    assert!(
        !content.contains("`wrap_up_mode`: `\"pr\"`"),
        "decompose-review skill must not set wrap_up_mode to \"pr\" — \
         a decomposed review epic would open one draft PR per work package"
    );
}

#[test]
fn decompose_review_skill_does_not_pass_repo_path_to_create_epic() {
    // Epics carry no repo_path — create_epic rejects the field outright
    // ("unknown field `repo_path`"). Step 6's subtasks are where the repo
    // path belongs, so this assertion is scoped to the Step 5 section:
    // a whole-document check would trip over Step 6's legitimate uses.
    let content = skill_body("decompose-review");
    let step5 = content
        .split("## Step 5: Create Epic")
        .nth(1)
        .expect("decompose-review skill must have a 'Step 5: Create Epic' section")
        .split("## Step 6")
        .next()
        .expect("split always yields at least one element");
    assert!(
        !step5.contains("`repo_path`:"),
        "decompose-review must not tell the agent to pass repo_path to \
         create_epic — the tool rejects it (found in Step 5: {step5:?})"
    );
}

/// A preset `wrap_up_mode` is the user's answer, given earlier. Suppressing
/// the question without carrying the wrap-up through leaves the agent idle
/// with the question it was told not to ask unanswered, and nothing reaches
/// base_branch until a human notices (task #4505, observed twice).
#[test]
fn wrap_up_skill_carries_a_preset_mode_through_without_asking_or_stopping() {
    let content = skill_body("wrap-up");
    let step_2 = content
        .split("## Step 2")
        .nth(1)
        .and_then(|s| s.split("## Step 3").next())
        .expect("wrap-up skill must have a Step 2 that reads the task");
    assert!(
        step_2.contains("Wrap-up mode:"),
        "Step 2 must read the preset mode off the line get_task actually \
         prints (`Wrap-up mode: <mode>`) — the response is prose, not JSON, \
         so an agent told to look for a `wrap_up_mode` field can find \
         nothing and fall through to the question it was meant to skip"
    );
    assert!(
        step_2.contains("Step 4"),
        "Step 2 must say what a preset mode does to the question step \
         (Step 4), or the agent has to infer it"
    );
    let lowered = step_2.to_lowercase();
    assert!(
        lowered.contains("do not stop") || lowered.contains("don't stop"),
        "a preset mode must come with an explicit instruction not to stop \
         part-way — suppressing the question alone is what left agents idle \
         at a blank prompt"
    );
}

#[test]
fn retro_skill_tells_agent_to_resume_the_caller() {
    // wrap-up invokes retro pre-commit, before its commit step. Without an
    // explicit instruction to resume the caller's remaining steps, an
    // agent that just finished following retro's own steps has nothing
    // telling it to continue — that's how wrap-up gets stuck after retro
    // and never reaches the commit, the user's action choice, or
    // exit_session. Retro's own edits are among what that commit carries,
    // so stopping here loses them.
    let content = skill_body("retro");
    assert!(
        content.contains("do not stop here") || content.contains("Do not stop here"),
        "retro skill must explicitly instruct the agent to resume the \
         calling skill's next step instead of stopping"
    );
}

#[test]
fn wrap_up_skill_runs_retro_between_the_action_choice_and_the_commit() {
    // Retro is bracketed on both sides, and each bound fixes a real defect:
    //
    //  • After the action choice, because retro decides fix-vs-file from the
    //    action. On `done` there is no rebase and no push, so a fix would be
    //    stranded — and retro cannot know that while the action is unsettled.
    //  • Before the commit, because that commit is what carries the fixes it
    //    does make. Invoked after wrap_up instead, they are stranded anyway:
    //    the rebase path has already fast-forwarded base_branch, so a later
    //    commit sits on a branch nobody merges, and the PR path has pushed.
    let content = skill_body("wrap-up");
    let choice_at = content
        .find("## Step 4: Ask the user to choose")
        .expect("wrap-up skill must have an action-choice step to anchor retro after");
    let retro_at = content
        .find("Skill({ skill: \"retro\" })")
        .expect("wrap-up skill must invoke the retro skill");
    let commit_at = content
        .find("## Step 6: Verify, then commit")
        .expect("wrap-up skill must have a commit step to anchor retro before");
    assert!(
        choice_at < retro_at,
        "wrap-up must settle the action before invoking retro, so retro can \
         tell whether a fix it makes can reach the base branch at all"
    );
    assert!(
        retro_at < commit_at,
        "wrap-up must invoke retro before its commit step, so retro's \
         context fixes are committed with the session's work"
    );
    assert_eq!(
        content.matches("Skill({ skill: \"retro\" })").count(),
        1,
        "wrap-up must invoke retro exactly once — a leftover call in the \
         closing sequence would run it twice and re-file its findings"
    );
    assert!(
        !content.to_lowercase().contains("run `/retro`"),
        "wrap-up must not still invoke retro from the closing sequence \
         between wrap_up and exit_session"
    );
}

/// The "Do NOT record" list must name the internal-code-citation failure
/// mode explicitly (task #4152 — learning #401 carried a stale
// allow-phantom-symbol: the actual stale citation learning #401 carried
/// `src/feed/cycle.rs::run_feed_cycle` citation that no gate ever caught).
/// Scoped to that one section: the rest of the skill mentions plenty of
/// backticked identifiers (tool names) that would make a whole-document
/// check pass even with this specific rule missing.
#[test]
fn learnings_skill_forbids_code_citations() {
    let section = section_after(&skill_text("learnings"), "### Do NOT record:")
        .expect("learnings skill must have a 'Do NOT record' section");
    assert!(
        section.contains("path.rs::symbol") || section.contains("path.rs"),
        "the Do NOT record list must name the path.rs::symbol citation shape: {section}"
    );
    assert!(
        section.to_lowercase().contains("rot"),
        "the rule must explain WHY (silent rot, no re-check) not just state a ban: {section}"
    );
}

/// The two skills that both tell an agent to rate a learning must not
/// disagree about *when*. `learnings` says at the moment you act on the
/// entry, explicitly "not deferred to wrap-up"; wrap-up's own copy used to
/// permit "at the latest before you wrap up", which is that deferral. An
/// agent reads whichever it loaded, so the looser wording wins whenever
/// wrap-up is the one in context — and rating a batch at the end is the
/// behaviour the immediate rule exists to prevent.
#[test]
fn wrap_up_and_learnings_agree_that_rating_is_not_deferred() {
    let learnings = skill_text("learnings");
    assert!(
        learnings.contains("not deferred to wrap-up"),
        "the learnings skill must keep the rate-immediately rule this test pins wrap-up to"
    );
    let wrap_up = skill_body("wrap-up");
    assert!(
        !wrap_up.contains("at the latest before you wrap up"),
        "wrap-up must not permit the deferral the learnings skill forbids"
    );
}

/// The validator only catches shapes prose never produces — a bare
/// PascalCase or short snake_case name walks straight through it (task
/// #4402: no regex separates `TuiRuntime` from `GitHub`). The skill copy
/// is therefore the only thing keeping those out, so it has to state the
/// rule in full rather than deferring to what `record_learning` rejects.
#[test]
fn learnings_skill_forbids_naming_implementation_detail_a_validator_misses() {
    let section = section_after(&skill_text("learnings"), "### Do NOT record:")
        .expect("learnings skill must have a 'Do NOT record' section");
    for term in ["function", "type", "macro", "fixture", "file"] {
        assert!(
            section.contains(term),
            "the Do NOT record list must name '{term}' among the things an entry \
             may not name — the validator does not catch all of them: {section}"
        );
    }
    assert!(
        section.contains("rejects only"),
        "the list must tell the agent the validator is narrower than the rule, \
         or an agent will read a successful call as approval: {section}"
    );
}

/// Ported from the `kognic-knowledge` plugin's capture triage (task
/// #4402). Without it, "Do NOT record" is a list of categories an agent
/// has to pattern-match against; with it there is one question that
/// decides, and it routes a whole class of would-be entries to a lint
/// instead of to prose.
#[test]
fn learnings_skill_carries_the_machine_check_triage() {
    let content = skill_text("learnings").to_lowercase();
    assert!(
        content.contains("failing check"),
        "the skill must ask whether a failing check could be written from the \
         source alone: {content}"
    );
    assert!(
        content.contains("lint"),
        "the triage's yes-branch must route to a lint rule rather than prose: {content}"
    );
    assert!(
        content.contains("smell"),
        "the triage's third branch — wrongly-shaped code is a smell, and the fix \
         is a refactor not a sentence — must survive: {content}"
    );
}

/// `record_learning` enforces only that a procedural entry HAS a detail.
/// What the detail must contain — the case where the agent stops and asks
/// a human — is convention, and this copy is where it lives (task #4402,
/// ported from OKF's required `# Escalate` section).
#[test]
fn learnings_skill_requires_a_boundary_on_procedural_entries() {
    let content = skill_text("learnings").to_lowercase();
    assert!(
        content.contains("procedural") && content.contains("ask a human"),
        "the skill must say a procedural entry's detail names when to stop and \
         ask a human: {content}"
    );
}

/// The summary-writing guidance used to illustrate "name the specific
/// thing" with an example naming a Rust type — the exact habit task #4402
/// exists to break. An example teaches harder than a rule, so a stale one
/// here would quietly undo the section above it.
#[test]
fn learnings_skill_summary_guidance_names_no_symbol() {
    let section = section_after(&skill_text("learnings"), "### Writing a good summary")
        .expect("learnings skill must have a summary-writing section");
    assert!(
        !section.contains("TaskPatch"),
        "the summary example must not name a type — it models the banned habit: {section}"
    );
}

/// The scope table must not offer `project` — LearningScope has only
/// user/repo/epic/task (task #4152: the row was stale, and an agent
/// passing scope="project" gets a deserialization error).
#[test]
fn learnings_skill_scope_table_has_no_project_row() {
    let content = skill_text("learnings");
    assert!(
        !content.contains("| `project` |"),
        "the scope table must not offer a project scope row: {content}"
    );
}

/// The `wrong` verdict bullet must not claim a human-review step exists
/// (learnings.allium: no human gate, no needs_review state, no status
/// change on either verdict).
#[test]
fn learnings_skill_wrong_verdict_does_not_claim_human_review() {
    // The correct text says "there is no human review step", which itself
    // contains the substring "human review" — so this checks for the old
    // false CLAIM (routing an entry TO review) rather than the substring.
    let content = skill_text("learnings").to_lowercase();
    assert!(
        !content.contains("routes an approved entry"),
        "the learnings skill must not claim a wrong verdict routes an entry \
         for human review — learnings.allium: no human gate, no status \
         change on either verdict: {content}"
    );
    assert!(
        content.contains("no human review"),
        "the learnings skill should state plainly that there is no human \
         review step: {content}"
    );
}

/// /fix files the issue as a task in the repo that owns it and dispatches it.
/// It must never edit that repo itself, must resolve the main repo root (a
/// worktree path as `repo_path` makes dispatch branch from the wrong place),
/// and must get the user's confirmation before anything is created: dispatch
/// launches an agent in auto mode, and a dispatched agent can invoke /fix too.
#[test]
fn fix_skill_confirms_then_creates_and_dispatches_a_fix_task_in_the_owning_repo() {
    let content = skill_body("fix");
    for needle in [
        "`create_task`",
        "`dispatch_task`",
        "`tag`: `\"fix\"`",
        "--git-common-dir",
        "never a\nworktree path",
        "`epic_id`",
        "`list_tasks`",
        "Do not edit",
    ] {
        assert!(content.contains(needle), "fix skill must mention {needle}");
    }
    let at = |needle: &str| {
        content
            .find(needle)
            .unwrap_or_else(|| panic!("fix skill must mention {needle}"))
    };
    assert!(
        at("`list_tasks`") < at("`AskUserQuestion`")
            && at("`AskUserQuestion`") < at("`create_task`")
            && at("`create_task`") < at("`dispatch_task`"),
        "fix skill must check duplicates, then confirm, then create, then dispatch"
    );
}

/// The section of `skill` from `start` up to the next `## Step` heading.
fn step_section(skill: &str, start: &str) -> &'static str {
    skill_body(skill)
        .split(start)
        .nth(1)
        .unwrap_or_else(|| panic!("{skill} skill must have a '{start}' section"))
        .split("\n## Step")
        .next()
        .expect("split always yields at least one element")
}

/// Low findings are small issues the repo wants fixed in passing, not only
/// reported. The 2026-10-22 review (epic 69651) dropped 10 of them until the
/// user asked; Step 1 must take every finding and Step 2 must give Low ones
/// a package rather than leave them out.
#[test]
fn decompose_review_skill_keeps_low_findings() {
    let step1 = step_section("decompose-review", "## Step 1: Extract Review Findings");
    assert!(
        !step1.contains("blocker, high or medium"),
        "Step 1 must not filter findings to blocker/high/medium: {step1:?}"
    );
    assert!(
        step1.contains("Low"),
        "Step 1 must say Low findings are included: {step1:?}"
    );
    let step2 = step_section("decompose-review", "## Step 2: Group into Work Packages");
    assert!(
        step2.contains("Cleanups"),
        "Step 2 must group Low findings with no other home into a \
         \"Cleanups\" package: {step2:?}"
    );
}

#[test]
fn decompose_review_skill_confirmation_names_left_out_findings() {
    let step3 = step_section("decompose-review", "## Step 3: Confirm the Work Packages");
    assert!(
        step3.contains("Not included"),
        "Step 3's confirmation list must name every finding left out, and \
         why: {step3:?}"
    );
}

/// `auto_run_plan` skips the "proceed?" question, not a decision only the
/// user can make. A finding that needs one must reach the plan as a
/// decision item the executing agent asks about, instead of guessing.
#[test]
fn decompose_review_skill_carries_decision_items_into_the_plan() {
    let step2 = step_section("decompose-review", "## Step 2: Group into Work Packages");
    assert!(
        step2.contains("decision item"),
        "Step 2 must say how to recognise a finding that needs a user \
         decision: {step2:?}"
    );
    let step6 = step_section(
        "decompose-review",
        "## Step 6: Write Plan Files and Create Tasks",
    );
    assert!(
        step6.contains("decision item"),
        "Step 6 must write decision items into the plan and say \
         auto_run_plan does not skip them: {step6:?}"
    );
    let template = skill_file("decompose-review", "references/plan-template.md");
    assert!(
        template.contains("## Decisions"),
        "plan-template.md must have a '## Decisions' section"
    );
    assert!(
        template.contains("AskUserQuestion"),
        "the template's decision items must tell the executing agent to ask \
         the user with AskUserQuestion"
    );
}
