use super::*;

/// A loop iteration does rebase, tend, propagate, red check, implement,
/// verify and weed, and repeats up to the loop's configured maximum — so an
/// iteration agent left on the session model (Opus) multiplies that cost by
/// the iteration count. Nothing in the loop's own output reveals which model
/// ran, so dropping this instruction would regress silently.
#[test]
fn allium_loop_dispatches_iteration_agents_on_sonnet() {
    let section = allium_loop_dispatch_instruction();
    assert!(
        section.contains("sonnet"),
        "allium-loop's dispatch step must name the sonnet model for \
         iteration agents"
    );
}

/// The no-fork rule and the model override are load-bearing together: a
/// `fork` ignores `model` entirely and runs on the session model, so losing
/// the no-fork constraint would silently undo the sonnet pin even with the
/// override still written down.
#[test]
fn allium_loop_dispatch_still_forbids_fork() {
    let section = allium_loop_dispatch_instruction();
    assert!(
        section.contains("fork"),
        "allium-loop's dispatch step must keep forbidding `fork` — a fork \
         ignores the model override and runs on the session model"
    );
}

/// The allium-loop skill's per-iteration dispatch instruction: the "Each
/// Iteration" section.
///
/// Scoped to that one section deliberately — the kickoff section also names
/// the model (it resolves and records the loop parameter), so a
/// whole-document check would still pass with the dispatch step's override
/// deleted. Re-anchor here if the heading is reworded.
fn allium_loop_dispatch_instruction() -> String {
    section_after(&skill_text("allium-loop"), "### Each Iteration")
        .expect("allium-loop skill must have an 'Each Iteration' section")
}

/// Both halves of the loop's convergence gate, added after two iterations of
/// one run reported `CONVERGED: yes` while their own prose named an
/// unresolved item. The driver acts on the label, not the prose, so a
/// self-contradicting report ends the loop with work still outstanding —
/// and nothing in the loop's output reveals that, which is what makes
/// losing this copy a silent regression rather than a visible one.
#[test]
fn allium_loop_convergence_gate_rejects_deferred_and_uncovered_work() {
    let section = section_after(
        skill_file("allium-loop", "prompt.md"),
        "Emit `CONVERGED: yes` ONLY when ALL hold:",
    )
    .expect("allium-loop prompt must state its CONVERGED criteria");
    assert!(
        section.contains("pending a decision"),
        "the convergence gate must keep disqualifying work left pending a \
         later run — a flagged-but-unresolved item is divergence"
    );
    assert!(
        section.contains("name the surviving test"),
        "the convergence gate must keep requiring a deleted test's \
         replacement to be named, not asserted in the abstract"
    );
}

/// "Skill copy" (mcp-task-tools.allium): a SKILL.md carries the flow every
/// invocation walks; detail only one branch needs lives in a references/
/// file beside it, and the branch names that file. Each of these skills
/// moves its long tail that way, so each has a references/ directory and
/// SKILL.md names every file in it — an unnamed reference is never loaded.
#[test]
fn agent_skills_keep_branch_detail_in_references_the_skill_names() {
    for skill in ["wrap-up", "retro", "allium-loop", "learnings"] {
        let refs = skill_references(skill);
        assert!(
            !refs.is_empty(),
            "{skill}: branch detail must live in a references/ file beside SKILL.md, \
             found none"
        );
        let body = skill_body(skill);
        for (name, _) in refs {
            assert!(
                body.contains(&format!("references/{name}")),
                "{skill}: SKILL.md must name references/{name} at the branch that \
                 needs it, or it is never loaded"
            );
        }
    }
}

/// pr-workflow.allium WrapUpPr: the pr path's steps live in the wrap-up
/// skill's pr reference file, named when the pr action is chosen, so an
/// agent on the rebase or done path never loads them.
#[test]
fn wrap_up_pr_path_lives_in_a_reference_named_from_the_branch() {
    let body = skill_body("wrap-up");
    for step in [
        "### Draft the title and body",
        "### Push and create the draft PR",
    ] {
        assert!(
            !body.contains(step),
            "wrap-up SKILL.md must not carry the pr path's {step:?} step — it belongs \
             in the pr reference file"
        );
    }
    let (name, reference) =
        reference_containing("wrap-up", "### Draft the title and body", "the pr path");
    assert!(
        reference.contains("gh pr create"),
        "the pr reference must carry the PR-creation step"
    );
    assert!(
        body.contains(&format!("references/{name}")),
        "wrap-up SKILL.md must name references/{name} where the pr path is taken"
    );
}

/// pr-workflow.allium ExitSession: the exit_session error handling — the
/// "has no active session" case treated as success, and the error that
/// reports a close which did not take effect — lives in a reference file the
/// skill names at the exit_session step.
#[test]
fn wrap_up_exit_session_errors_live_in_a_reference_named_at_the_step() {
    let body = skill_body("wrap-up");
    assert!(
        !body.contains("has no active session"),
        "wrap-up SKILL.md must not carry the exit_session error cases — they belong \
         in the exit_session-errors reference file"
    );
    let (name, reference) = reference_containing(
        "wrap-up",
        "has no active session",
        "the exit_session error handling",
    );
    assert!(
        reference.contains("did not take effect"),
        "the exit_session-errors reference must carry the failed-close reaction beside \
         the has-no-active-session case"
    );
    assert!(
        wrap_up_closing_sequence().contains(&format!("references/{name}")),
        "wrap-up's closing sequence must name references/{name} at the exit_session step"
    );
}

/// TheNextCallIsNamedJustInTime: exit_session owns the epic chain and its
/// description says so, so the skill carries no section telling the agent
/// not to dispatch the next subtask itself.
#[test]
fn wrap_up_skill_has_no_dont_dispatch_the_next_subtask_section() {
    let lower = skill_text("wrap-up").to_lowercase();
    assert!(
        !lower.contains("next subtask yourself"),
        "wrap-up must not carry a don't-dispatch-the-next-subtask section — \
         exit_session owns the chain"
    );
}

/// The other half of the same guarantee: the skill still names exit_session
/// as the call after wrap_up, in its closing sequence.
#[test]
fn wrap_up_skill_names_exit_session_in_its_closing_sequence() {
    assert!(
        wrap_up_closing_sequence().contains("exit_session"),
        "wrap-up's closing sequence must name exit_session as the next call"
    );
}
