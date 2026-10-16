use super::*;

/// A lowercased section of the retro skill body, via [`section_after`]:
/// from the first occurrence of `anchor` up to the next Markdown heading of
/// any depth. Anchors passed here must therefore be lowercase.
///
/// Scoped per-section deliberately. Retro repeats words like "task",
/// "spec" and "fix" across its steps, so a whole-document `contains` can
/// still pass after the instruction under test has been deleted. If you
/// reword an anchor heading, re-anchor it here.
fn retro_section(anchor: &str) -> String {
    let content = skill_text("retro").to_lowercase();
    section_after(&content, anchor)
        .unwrap_or_else(|| panic!("retro skill must contain the section anchored on {anchor:?}"))
}

#[test]
fn retro_admission_test_is_next_agent_benefit_not_doc_accuracy() {
    // The old Step 2 asked whether CLAUDE.md or a spec was "stale or
    // wrong" — a correctness question every trivial nit passes, which is
    // how retro came to file 38 one-line doc chores. The bar is now
    // whether the *next* agent would do better, and each finding must
    // trace to a concrete moment this session actually lost time on.
    let section = retro_section("## what is worth changing");
    assert!(
        section.contains("would the next agent do better"),
        "retro's admission test must be whether the next agent benefits, \
         not whether a sentence is inaccurate"
    );
    assert!(
        section.contains("concrete moment"),
        "retro must require every finding to trace to a concrete moment \
         from Step 1 rather than to a hypothetical"
    );
    assert!(
        !section.contains("stale or wrong"),
        "retro must not frame its check as a documentation-accuracy audit"
    );
}

#[test]
fn retro_first_step_asks_whether_user_corrected_or_steered_you() {
    // Task #4326: the old Step 1 was entirely "context turned out wrong"
    // shaped (a bad CLAUDE.md assumption, a convention found by reading
    // source, a stale spec, a guessed rule, a failing command) — none of
    // it about the user. #4316's code-review feedback (test suffix,
    // one-class-per-file, ADTs over `require()`, and more) went
    // uncaptured until the human explicitly asked whether it had been
    // saved as learnings. Step 1 now asks specifically whether the user
    // corrected or steered the agent, and must still say "nothing
    // notable" is a real answer. (The Step-1-to-Step-2 linkage is
    // asserted separately, by the "concrete moment" check in
    // `retro_admission_test_is_next_agent_benefit_not_doc_accuracy`.)
    let section = retro_section("## what to look for");
    assert!(
        section.contains("correct") && section.contains("steer"),
        "retro's first step must ask whether the user corrected or \
         steered the agent this session"
    );
    assert!(
        section.contains("nothing notable"),
        "retro must state that an empty reflection is a real answer, so a \
         smooth session is not pressured into inventing findings"
    );
}

#[test]
fn retro_first_step_scopes_out_self_discovered_gaps() {
    // A stale spec is `weed`'s job (spec-code alignment), and a command
    // that failed until the right invocation was found is a tooling
    // problem — neither is feedback from the user, so Step 1 must not
    // invite them back in as findings.
    let section = retro_section("## what to look for");
    assert!(
        section.contains("weed"),
        "retro's first step must route spec-code alignment to the weed \
         skill rather than treating a stale spec as its own finding"
    );
    assert!(
        section.contains("tooling problem"),
        "retro's first step must name a failing command as a tooling \
         problem, not a correction from the user"
    );
}

#[test]
fn retro_first_step_names_code_review_and_design_examples() {
    // Step 1's checklist must give the agent concrete shapes of user
    // feedback to look for, and must say this counts even when it cost
    // no time in the moment — design/style feedback rarely does.
    let section = retro_section("## what to look for");
    assert!(
        section.contains("code review"),
        "retro's first step must prompt for feedback given during a \
         code review pass"
    );
    assert!(
        section.contains("design choice") || section.contains("design decision"),
        "retro's first step must ask about a design choice the user \
         made or corrected this session"
    );
    assert!(
        section.contains("even if it didn't") || section.contains("even if it did not"),
        "retro must state this category counts even when it didn't cost \
         time in the moment, since design/style feedback often doesn't \
         slow the agent down"
    );
}

#[test]
fn retro_relationship_section_says_not_to_wait_to_be_asked_for_feedback() {
    // #4316's design/style feedback only became learnings after the
    // human asked "did you save learnings about the feedback I gave
    // you" — retro must not rely on being asked.
    let section = retro_section("## relationship to other skills");
    assert!(
        section.contains("don't wait") || section.contains("do not wait"),
        "retro's relationship-to-learnings note must say to record \
         design/style feedback without waiting to be asked"
    );
}

#[test]
fn retro_skill_permits_fixing_small_context_drift_in_session() {
    // The old Step 3 said "Do not edit files yourself", which turned every
    // one-line doc correction into a task + worktree + agent dispatch. The
    // agent that just did the work has the context and is already in a
    // worktree whose next step is a commit; it should make the fix.
    let content = skill_text("retro").to_lowercase();
    assert!(
        !content.contains("do not edit files yourself"),
        "retro must no longer ban editing outright — fixing small context \
         drift in place is now its job"
    );
    let section = retro_section("## fix it or file it");
    assert!(
        section.contains("fix it yourself"),
        "retro must tell the agent to fix small context drift in this session"
    );
    assert!(
        section.contains("small and self-evident"),
        "retro's edit licence must be bounded to small, self-evident \
         corrections that need no design judgement"
    );
}

#[test]
fn retro_skill_forbids_speccing_unimplemented_behaviour() {
    // A spec edit describing behaviour the session already implemented is
    // documentation catching up. One describing behaviour the code lacks is
    // a design change, and this repo runs those spec -> tests -> code with
    // their own dispatch — so retro must file it, not write it.
    let section = retro_section("## fix it or file it");
    assert!(
        section.contains("already implemented"),
        "retro may only edit a spec to describe behaviour this session \
         already implemented"
    );
    assert!(
        section.contains("spec → tests → code"),
        "retro must route a spec change for not-yet-implemented behaviour \
         to a task, naming the spec -> tests -> code loop as the reason"
    );
}

#[test]
fn retro_skill_does_not_file_feature_tasks() {
    // Every archived retro-created task was a speculative refactor dressed
    // as an enhancement — "this invariant is enforced by convention, so the
    // same omission could recur", "this could be a single atomic insert".
    // feature leaves retro's vocabulary entirely.
    let section = retro_section("### what you may file");
    assert!(
        section.contains("never file a `feature`"),
        "retro must explicitly refuse to file feature tasks"
    );
    assert!(
        section.contains("speculative refactor"),
        "retro must name speculative refactors as a non-finding, since that \
         is the shape of every retro task that got archived"
    );
    // Scoped to the whole skill body, not just this section: the string
    // this guards against previously lived in Step 3's tag list, a
    // section this assertion does not otherwise cover. A future agent
    // restoring "`feature` for an enhancement idea" to Step 3 is the
    // likeliest regression, and no other section legitimately contains
    // this string, so widening the scope here is safe.
    let whole_skill = skill_text("retro").to_lowercase();
    assert!(
        !whole_skill.contains("`feature` for"),
        "retro must not still describe when to use the feature tag, in any section"
    );
}

#[test]
fn retro_skill_requires_a_duplicate_check_before_filing() {
    // Two findings were each filed twice: one stale sentence that appeared
    // in two documents, and one recurring shape nobody recognised. Nothing
    // in the skill told the agent to look first.
    let section = retro_section("### before you file");
    assert!(
        section.contains("list_tasks"),
        "retro must check for an existing task with list_tasks before filing"
    );
    assert!(
        section.contains("one task per finding"),
        "retro must collapse a finding that spans several files into one task"
    );
}

#[test]
fn retro_skill_states_zero_findings_is_the_normal_outcome() {
    // The old skill buried this under three steps of checklist-shaped
    // instructions, which read as a quota to fill rather than a bar to
    // clear.
    let section = retro_section("### before you file");
    assert!(
        section.contains("zero tasks is the normal outcome"),
        "retro must state outright that filing nothing is the expected result"
    );
}

#[test]
fn retro_step_2_asks_whether_root_cause_is_outside_this_repo() {
    // Task #4256: retro found the Bash sandbox blocks Gradle daemon
    // startup, wrote a workaround note to user-scala's CLAUDE.md, and
    // stopped — the sandbox defect itself was never filed. Nothing in
    // Step 2 asked where the root cause actually lived, so this passed
    // retro's own rubric as closed. Step 2 must now ask that question for
    // every finding, in addition to the next-agent-benefit test.
    let section = retro_section("## what is worth changing");
    assert!(
        section.contains("root cause"),
        "retro's Step 2 must ask whether a finding's root cause lies \
         outside this repo"
    );
    assert!(
        section.contains("sandbox") && section.contains("claude code"),
        "retro's root-cause question must name the sandbox, dispatch \
         itself, and claude code as tool/environment loci, not just \
         gesture at 'somewhere else'"
    );
}

#[test]
fn retro_root_cause_subsection_requires_filing_not_just_a_workaround() {
    // A local doc workaround treats the symptom, not the defect. Retro
    // must not let it read as full closure when the root cause is the
    // tool/environment running the agent rather than this repo — and
    // must route the filing itself based on who owns that root cause:
    // this task's own repo (file directly) or a different one (flag to
    // the user rather than filing unprompted onto a foreign board).
    let section = retro_section("### when the root cause is the tool or environment");
    let flattened = section.replace('\n', " ");
    assert!(
        section.contains("workaround"),
        "the new subsection must address the local workaround explicitly"
    );
    assert!(
        flattened.contains("is not") && flattened.contains("sufficient closure"),
        "the new subsection must state that a workaround alone is not \
         sufficient closure for a tool/environment root cause"
    );
    assert!(
        section.contains("create_task"),
        "retro must file a same-repo root-cause defect with create_task"
    );
    assert!(
        section.contains("not file") || section.contains("don't file silently"),
        "retro must not file a cross-repo root-cause defect silently"
    );
    assert!(
        section.contains("flag"),
        "retro must flag a cross-repo root-cause defect explicitly instead"
    );
}

#[test]
fn retro_output_template_has_a_root_cause_line() {
    // The #4256 gap wasn't just a missing filing rule — retro's own
    // output never said anything, so nothing forced the question in
    // front of the user before the session closed.
    //
    // Not scoped via retro_section("## step 4:"): the output template is
    // a fenced code block containing its own "## Session Retrospective"
    // heading, which retro_section's "next heading of any depth" cutoff
    // would treat as the section boundary and truncate before this line.
    let content = skill_text("retro").to_lowercase();
    assert!(
        content.contains("root-cause issues flagged"),
        "retro's output template must include a line surfacing \
         root-cause issues flagged elsewhere"
    );
}
