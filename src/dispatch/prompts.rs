use std::collections::HashSet;
use std::sync::Arc;

use crate::embeddings::{
    deserialize_candidate_rows, embed_text_for_query, rag_rank_learnings, EmbeddingService,
    RagRankParams,
};
use crate::models::{EpicId, FeedRole, Learning, RetrievalSource, Task, TaskId, TaskTag};
use crate::store;

use crate::claude_paths::{claude_dir_name, plugin_dir_rel, statusline_settings_name};

use super::bump::{self, BumpKind};
use super::worktree::StartPoint;

/// Flags added to all Claude agent invocations. `--plugin-dir` so dispatched
/// agents discover the dispatch plugin's skills and commands (e.g. /wrap-up);
/// `--settings` so every session reports its subscription budget windows via
/// the `dispatch statusline` decorator (see docs/specs/dispatch.allium:
/// TokenBudgetIndicator).
///
/// Both paths are fixed literals on purpose: a `const` has no runtime source,
/// so these flags cannot go missing, and `claude` refuses to start without the
/// settings file. Runtime paths live inside that file, written by
/// `src/setup/statusline.rs`.
///
/// The path *segments* are not written out here — they expand from
/// `crate::claude_paths`, which is also where the writing side gets them and
/// where the reasoning lives. See docs/specs/dispatch.allium:
/// `SpawnSitesAndStartupNameTheSameConfigurationDirectory`.
///
/// Built with `concat!` rather than a backslash line-continuation inside the
/// literal: a `\` at end-of-line inside a Rust string swallows the newline
/// *and* all leading whitespace on the next line, which can silently collapse
/// or duplicate the space between the two flags.
pub(super) const DISPATCH_PLUGIN_DIR: &str = concat!(
    "--plugin-dir ~/",
    claude_dir_name!(),
    "/",
    plugin_dir_rel!(),
    " --settings ~/",
    claude_dir_name!(),
    "/",
    statusline_settings_name!()
);

/// Epic context passed to prompt builders so agents know about their epic.
pub struct EpicContext {
    pub epic_id: EpicId,
    pub epic_title: String,
    /// Does this task hang under the managed CVE feed root — its own epic
    /// carrying `feed_role = cve`, or any ancestor of it?
    ///
    /// The routing key for the CVE runbook (see
    /// `CveRemediationSkipsTheDesignStep` in `docs/specs/dispatch-prompt.allium`).
    /// It lives on the epic context rather than on [`PromptContext`] because
    /// answering it needs the database, and this is the one prompt input
    /// already assembled from it.
    ///
    /// **Ancestry, not one row.** With `group_by_repo` on, a CVE feed's tasks
    /// land on a `repo-group` sub-epic whose own `feed_role` is `None`, so
    /// reading `task.epic_id`'s row alone answers `false` for every task on a
    /// grouped CVE board.
    pub under_cve_feed: bool,
}

impl EpicContext {
    /// Build epic context from the database for a task that belongs to an epic.
    pub async fn from_db(task: &Task, db: &dyn store::TaskReadStore) -> Option<Self> {
        let epic_id = task.epic_id?;
        let epic = db.get_epic(epic_id).await.ok()??;
        Some(Self::from_epic(epic, db).await)
    }

    /// Build epic context from an epic row already in hand.
    ///
    /// The one place [`EpicContext::under_cve_feed`] is answered, so a caller
    /// that skips [`from_db`](Self::from_db)'s re-read cannot skip the ancestry
    /// walk with it — which is exactly what a hand-written struct literal at
    /// such a call site did before this existed.
    pub async fn from_epic(epic: crate::models::Epic, db: &dyn store::TaskReadStore) -> Self {
        let under_cve_feed = Self::walk_to_cve_root(&epic, db).await;
        EpicContext {
            epic_id: epic.id,
            epic_title: epic.title,
            under_cve_feed,
        }
    }

    /// True when `epic` or any ancestor of it carries `FeedRole::Cve`.
    ///
    /// A read failure mid-walk answers `false` rather than propagating: the
    /// consequence is a CVE task that gets the ordinary design step, which is
    /// the same answer it got before this branch existed. Failing the dispatch
    /// over it would be worse.
    ///
    /// The visited set is the cycle guard, mirroring
    /// `models::epics::ancestor_titles` — the service layer prevents a cycle,
    /// but a dispatch must not hang on one that got in anyway. A depth cap
    /// would do the same job approximately, and would put an arbitrary number
    /// in the spec that only exists because the guard was the weaker of the
    /// two.
    async fn walk_to_cve_root(epic: &crate::models::Epic, db: &dyn store::TaskReadStore) -> bool {
        let mut seen: HashSet<EpicId> = HashSet::new();
        let mut cursor = epic.id;
        let mut role = epic.feed_role;
        let mut parent = epic.parent_epic_id;
        loop {
            if !seen.insert(cursor) {
                return false; // cycle guard
            }
            if role == FeedRole::Cve {
                return true;
            }
            let Some(id) = parent else { return false };
            let Ok(Some(row)) = db.get_epic(id).await else {
                return false;
            };
            cursor = row.id;
            role = row.feed_role;
            parent = row.parent_epic_id;
        }
    }

    pub(super) fn prompt_section(&self) -> String {
        format!(
            "\n\nThis task is part of epic #{}: {}\n\
            Sibling agents run as sessions named task-<id>, matching that task's own id. \
            Find one with ListAgents and message it with SendMessage.",
            self.epic_id, self.epic_title
        )
    }
}

/// Preamble for a worktree reused from a previous attempt.
///
/// Reuse is the only non-PR case where a rebase does real work: a fresh
/// worktree's branch *is* its start point, so rebasing onto that ref can only
/// report "up to date". The rebase targets whichever ref provisioning chose —
/// pointing a local-based branch at `origin/<base>` would replay local `<base>`'s
/// unpushed commits under new SHAs, which then collide with the wrap-up rebase
/// onto local `<base>`.
pub(super) fn reused_rebase_preamble(start_point: &StartPoint) -> String {
    format!(
        "This worktree was reused from a previous attempt and may contain \
         uncommitted changes or commits from that run. Check `git status` and \
         `git log` first, then bring the branch up to date:\n\
         ```\n\
         git fetch origin {base}\n\
         git rebase {target}\n\
         ```\n\
         If the rebase reports unstaged changes, commit or stash them first.",
        base = start_point.base(),
        target = start_point.git_ref(),
    )
}

/// Which rebase preamble — if any — a dispatch gets. The whole rule, in one
/// pure function, evaluated *after* provisioning so it can see the resolved ref.
///
/// Takes no fetch warning: the `Note:` is composed separately by
/// [`compose_prompt_head`], which is what keeps this a three-row table.
pub(super) fn select_preamble(
    pr_branch: Option<&str>,
    start_point: Option<&StartPoint>,
    reused: bool,
) -> String {
    if let Some(branch) = pr_branch {
        return pr_rebase_preamble(branch);
    }
    match start_point {
        Some(sp) if reused => reused_rebase_preamble(sp),
        _ => String::new(),
    }
}

/// Everything that precedes the "Always work from this worktree folder" line:
/// the preamble (possibly empty) and the fetch `Note:` (possibly absent), each
/// separated from what follows by a blank line.
///
/// The two are independent — a fresh worktree based on a local-only branch has
/// a warning worth surfacing and no preamble to attach it to.
pub(super) fn compose_prompt_head(preamble: &str, fetch_warning: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !preamble.is_empty() {
        parts.push(preamble.to_string());
    }
    if let Some(warning) = fetch_warning {
        parts.push(format!("Note: {warning}"));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("{}\n\n", parts.join("\n\n"))
}

/// Preamble for review tasks whose worktree is based on a PR branch.
///
/// The worktree already starts from the PR's code, so this is an on-demand
/// refresh: run it whenever you (or the user) want to pull in commits pushed to
/// the PR after dispatch. It rebases the worktree branch onto the latest
/// `origin/<branch>` rather than onto the repo's base branch.
pub(super) fn pr_rebase_preamble(branch: &str) -> String {
    format!(
        "This worktree is based on the PR branch `{branch}`. To pull in the \
         latest commits pushed to the PR, rebase onto it (do this whenever you \
         want to refresh the PR's code):\n\
         ```\n\
         git fetch origin {branch}\n\
         git rebase origin/{branch}\n\
         ```"
    )
}

/// Returns `(epic_id_line, epic_section)` for embedding in agent prompts.
pub(super) fn epic_preamble(epic: Option<&EpicContext>) -> (String, String) {
    let id_line = epic.map_or(String::new(), |e| format!("\n  EpicId: {}", e.epic_id));
    let section = epic.map_or(String::new(), |e| e.prompt_section());
    (id_line, section)
}

/// Standard task identification block shared by all task agent prompts.
pub(super) fn task_block(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
) -> String {
    let (epic_id_line, epic_section) = epic_preamble(epic);
    format!(
        "Task:\n  ID: {task_id}\n  Title: {title}\n  Description: {description}\
         {epic_id_line}{epic_section}"
    )
}

/// TDD instruction line, shared across all agents.
pub(super) fn tdd_instruction() -> &'static str {
    "Always use TDD: express intended behaviour as tests first, then implement the minimum code to make them pass."
}

// There is deliberately no "the dispatch MCP tools are available (get_task,
// update_task)" line here any more. Both tools reach the agent as real tool
// schemas, and naming them again in prompt prose shadows that list: it states
// no fact the schema lacks, and it goes stale the moment the tool set changes.
// `get_task`'s own description now carries what the prose was standing in for —
// what the response looks like and which of its lines matter.

/// One-line knowledge-base nudge for dispatched agents. The earlier
/// seven-skill checkpoint list saw <2 invocations each across hundreds
/// of dispatches — replaced with a direct prompt to query the KB
/// whenever anything is unclear.
///
/// It names the `/learnings` skill and no MCP tool. The three it used to name
/// each ended up stating the same WHEN in their own schema — `query_learnings`
/// closes with "Call it when something is unclear, before guessing or asking",
/// which was this line's first clause word for word — so naming them here
/// restated three descriptions the agent already has, and went stale whenever
/// one was reworded. A skill is the one thing left with no schema to shadow: a
/// skill listing carries a description, not a nudge to invoke it. See
/// `ThePromptNamesNoToolMerelyToSayItExists` in `docs/specs/dispatch-prompt.allium`,
/// and `prompt_trailing_lines_name_no_mcp_tool` which gates it.
///
/// Rating is not mentioned here either — the validated-knowledge block names
/// the full `rate_learning` call, and it renders exactly when there is
/// something surfaced to rate. See `AugmentDispatchPromptWithLearnings` in
/// `docs/specs/learnings.allium`.
pub(super) fn learning_tools_instruction() -> &'static str {
    "Knowledge base: the `/learnings` skill manages it — check it when something is \
unclear, and record what you find."
}

/// The design instruction for every task that arrives without a plan: an
/// Allium-first sequence (elicit, tend, propagate, implement, weed), stated as
/// one sentence that names the skills, which replaced the older
/// `/brainstorming` design-doc-then-plan step in task #4366.
///
/// Shared verbatim between the no-plan dispatch addendum and the quick-dispatch
/// addendum, so the design step cannot drift apart between the two. A
/// `docs/plans/` doc is named as the agent's judgement call rather than a
/// requirement — the spec, not a plan, is what this step is expected to produce.
///
/// It names each skill and stops, the same rule [`brainstorm_instruction`]
/// follows: the skills carry their own process (who to interview, how to
/// converge), and a restatement here can only drift from them.
///
/// Framed as an intermediate step, not a stopping point;
/// `Research`/`Dependabot`/`PrReview` never reach this addendum (see
/// `DispatchMode::for_task` and `TaskTag::is_review`), so no per-tag branch is
/// needed here. Carries the same epic-decomposition carve-out as
/// `wrap_up_instruction` so the two stay consistent about what counts as done.
pub(super) fn spec_first_instruction() -> &'static str {
    "Design the solution spec-first: elicit the intended behaviour with `allium:elicit`, \
capture it in docs/specs/ with `allium:tend`, generate tests with `allium:propagate` and \
confirm they fail before you write any code, implement the minimum that makes them pass, then \
check spec and code agree with `allium:weed`.\n\
\n\
Writing a plan to docs/plans/ and attaching it with update_task is your judgement call, \
not a requirement — do it only if the implementation is big enough that its steps are \
worth recording.\n\
\n\
The spec is not the end of the task — implement it in this same session (or, for an \
epic-decomposition task, create work packages for its subtasks instead) and verify your \
work before wrapping up."
}

/// The design instruction for a no-plan task in a repo that keeps **no** Allium
/// specs (`docs/specs/*.allium` is absent or empty). Sending such an agent to
/// `allium:elicit` would ask it to tend a garden that does not exist, so the
/// design step is `superpowers:brainstorming` instead — see
/// `DesignStepMatchesTheReposSpecs` in `docs/specs/dispatch-prompt.allium`.
///
/// It names the skill and stops. Brainstorming's own process is deliberately
/// not paraphrased here: the agent loads the skill, and a prompt-side
/// restatement can only drift from it. TDD is not restated either — it is in
/// the trailing block either way.
///
/// The optional-plan and not-a-stopping-point clauses are the same commitments
/// `spec_first_instruction` makes, including the epic-decomposition carve-out,
/// so the two branches differ only in the design artefact.
pub(super) fn brainstorm_instruction() -> &'static str {
    "Design the solution with the `superpowers:brainstorming` skill before you write any \
code. This repo has no Allium specs, so brainstorming is the design step.\n\
\n\
Writing a plan to docs/plans/ and attaching it with update_task is your judgement call, \
not a requirement — do it only if the implementation is big enough that its steps are \
worth recording.\n\
\n\
The design is not the end of the task — implement it in this same session (or, for an \
epic-decomposition task, create work packages for its subtasks instead) and verify your \
work before wrapping up."
}

/// The design step for a no-plan task, chosen by what the repo actually holds.
///
/// The single decision point, so the two builders that ask for a design step
/// (`build_prompt`'s no-plan arm and `build_quick_dispatch_prompt`) cannot
/// disagree about which branch a given repo takes.
pub(super) fn design_instruction(has_allium_specs: bool) -> &'static str {
    if has_allium_specs {
        spec_first_instruction()
    } else {
        brainstorm_instruction()
    }
}

/// A design artefact is not a stopping point on its own — the rule task #4188
/// added, after a bug/feature/chore-tagged agent read "write and attach a
/// plan" plus the then-universal wrap-up wording as licence to stop at a
/// plan-only state.
///
/// Emitted whenever a plan is attached. Both design steps close by stating the
/// same rule ("The spec is not the end of the task — implement it in this same
/// session…"), so on their paths this is the restatement
/// `NoLineRestatesTheDesignStep` rules out. The plan paths are the ones with no
/// design step above them, and are also the case #4188 was actually about.
///
/// Whether the repo keeps Allium specs does not enter into it: this wording
/// names no spec directory, so both [`Preceding::PlanWithSpecs`] and
/// [`Preceding::PlanWithoutSpecs`] carry it. Gating it on the former alone
/// reinstated the #4188 regression in every repo with no `docs/specs/`.
///
/// It names only the plan, not "a spec or a plan": no spec-writing step runs
/// on the paths that emit it, so the narrower wording is the accurate one.
pub(super) fn plan_not_a_stopping_point_instruction() -> &'static str {
    "Attaching a plan for your own task is not a stopping point on its own — \
implement it in the same session first."
}

/// Wrap-up instruction shared by every dispatched task agent, so the whole
/// task lifecycle terminates the same way regardless of mode.
///
/// What is left here is the part no design step states: which states are
/// terminal, and the call that ends the session. Creating work packages on an
/// epic is a legitimate terminal state for a decomposition task, since that
/// task's job is delegation, not implementation.
///
/// The stopping-point rule that used to open this line moved to
/// [`plan_not_a_stopping_point_instruction`]. The epic carve-out still appears
/// in both, deliberately: there it qualifies how the task may *finish*, here it
/// qualifies when to *call the skill*.
pub(super) fn wrap_up_instruction() -> &'static str {
    "When your work is done — finishing implementation, or (for an \
epic-decomposition task) creating work packages for its subtasks — use the \
/wrap-up skill to commit any remaining changes and finalise the task."
}

/// Allium spec instruction — shared across all agents that may touch domain behaviour.
pub(super) fn allium_instruction() -> &'static str {
    "The Allium specs in `docs/specs/` are the source of truth for domain logic. \
Consult them before changing core behaviour. If your implementation changes domain behaviour, \
update the spec using the `allium:tend` skill and verify alignment with `allium:weed`."
}

/// What the addendum above the trailing block already said, which decides which
/// trailing lines would be restatements.
///
/// One variant per combination of the two questions — a plan attached, and the
/// repo keeping specs — so neither question shadows the other. It covers the
/// same ground as the pair of booleans and is kept for the naming, not for
/// narrowing: a trailing line's predicate is then a statement about what the
/// agent has already been told, and the two builders cannot disagree about
/// which branch they took.
///
/// A three-variant form collapsed the two spec-less states into one, on the
/// grounds that a two-boolean signature admits a fourth state that cannot
/// occur. The unreachable combination is spec-first *without* specs; a plan
/// without specs is routine, and collapsing it onto the no-plan state cost it
/// [`plan_not_a_stopping_point_instruction`] — see `enum Preceding` in
/// `docs/specs/dispatch-prompt.allium`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Preceding {
    /// No plan, and the repo keeps specs: `spec_first_instruction` — states
    /// test-first as its steps 3 and 4, and the tend/weed cycle as its steps 2
    /// and 5.
    SpecFirst,
    /// No plan, and the repo keeps no specs, so the design step is
    /// `brainstorm_instruction`, which names a skill and nothing else.
    Brainstorm,
    /// A plan was attached, in a repo that keeps specs. The prompt names no
    /// design sequence, so every conditional trailing line carries.
    PlanWithSpecs,
    /// A plan was attached, in a repo that keeps no specs. As
    /// [`Preceding::PlanWithSpecs`], minus `allium_instruction`: there is no
    /// `docs/specs/` to name as the source of truth.
    PlanWithoutSpecs,
    /// No plan, and the task is CVE remediation: the addendum is
    /// [`cve_runbook`], which rules test-first OUT rather than in. Whether the
    /// repo keeps specs does not enter into it — see
    /// `CveRemediationSkipsTheDesignStep` in `docs/specs/dispatch-prompt.allium`.
    CveRunbook,
    /// A plan was attached to a CVE task. As [`Preceding::CveRunbook`] plus
    /// `plan_not_a_stopping_point_instruction`: the plan replaces the runbook
    /// as the addendum, and a plan path still needs telling that a plan is not
    /// a finish line.
    CveWithPlan,
}

impl Preceding {
    /// The design-step branch, given whether a plan is attached and whether the
    /// repo keeps specs. The single place the mapping lives, so the two
    /// builders cannot disagree about which branch they took.
    ///
    /// Order-independent: every input pair has a state of its own, so no arm
    /// answers one question at the cost of leaving the other unread.
    ///
    /// `is_cve` COLLAPSES `has_allium_specs` rather than multiplying by it:
    /// both CVE states drop `allium_instruction`, which is the only line
    /// `has_allium_specs` reaches, so the eight combinations resolve to six
    /// states. That is a deliberate collapse, not the order-sensitivity above —
    /// the specs question has no line left to decide once `is_cve` holds.
    pub(super) fn resolve(has_plan: bool, has_allium_specs: bool, is_cve: bool) -> Self {
        // One match over all three questions rather than a CVE guard clause
        // above the original pair. The `_` in the specs position is where the
        // collapse is stated — a guard clause states it by leaving the
        // argument unread, which is the order-sensitive shape this function
        // exists to avoid.
        match (is_cve, has_plan, has_allium_specs) {
            (true, false, _) => Preceding::CveRunbook,
            (true, true, _) => Preceding::CveWithPlan,
            (false, false, true) => Preceding::SpecFirst,
            (false, false, false) => Preceding::Brainstorm,
            (false, true, true) => Preceding::PlanWithSpecs,
            (false, true, false) => Preceding::PlanWithoutSpecs,
        }
    }

    /// Every state, for tests that must speak for all of them.
    ///
    /// Hand-writing the list in each test loop is the blind spot
    /// `prompt_trailing_lines_name_no_mcp_tool` already rejected for tool
    /// names: a state added later joins no loop, and nothing looks wrong.
    #[cfg(test)]
    pub(super) const ALL: [Preceding; 6] = [
        Preceding::SpecFirst,
        Preceding::Brainstorm,
        Preceding::PlanWithSpecs,
        Preceding::PlanWithoutSpecs,
        Preceding::CveRunbook,
        Preceding::CveWithPlan,
    ];

    /// Whether a plan is attached.
    ///
    /// The three predicates below are exhaustive matches rather than `==` or
    /// `matches!` on purpose. Each has no implicit false arm, so a fifth state
    /// cannot compile until someone answers all three questions for it — the
    /// bug this enum was split to fix was a new case silently inheriting
    /// whatever a comparison happened to say.
    pub(super) fn has_plan(self) -> bool {
        match self {
            Preceding::PlanWithSpecs | Preceding::PlanWithoutSpecs | Preceding::CveWithPlan => true,
            Preceding::SpecFirst | Preceding::Brainstorm | Preceding::CveRunbook => false,
        }
    }

    /// Whether the task's repo keeps Allium specs, and so has a `docs/specs/`
    /// a prompt may name — see `DesignStepMatchesTheReposSpecs`.
    pub(super) fn keeps_specs(self) -> bool {
        match self {
            Preceding::SpecFirst | Preceding::PlanWithSpecs => true,
            // The CVE states answer false whatever the repo actually holds:
            // `resolve` never reads the specs question for them, so there is no
            // honest answer to give, and the one line this predicate gates is
            // dropped on both of them anyway.
            Preceding::Brainstorm
            | Preceding::PlanWithoutSpecs
            | Preceding::CveRunbook
            | Preceding::CveWithPlan => false,
        }
    }

    /// Whether the prompt above the trailing block has already settled both
    /// the test-first question and the spec-tending question, leaving neither
    /// trailing line anything to add.
    ///
    /// Two ways to settle them, and the predicate covers both. `spec_first`
    /// states them IN, as its own numbered steps 2-5 — see
    /// `NoLineRestatesTheDesignStep`. The CVE runbook rules them OUT by name —
    /// see `CveRemediationSkipsTheDesignStep`; `CveWithPlan` answers true too,
    /// because a plan does not put a version bump back in scope for TDD.
    ///
    /// `brainstorm_instruction` names a skill and stops, and the two ordinary
    /// plan states name no design step at all, so all three keep the lines.
    pub(super) fn addendum_settles_tdd_and_allium(self) -> bool {
        match self {
            Preceding::SpecFirst | Preceding::CveRunbook | Preceding::CveWithPlan => true,
            Preceding::Brainstorm | Preceding::PlanWithSpecs | Preceding::PlanWithoutSpecs => false,
        }
    }
}

/// Trailing metadata shared by every dispatched task agent prompt, separated
/// by blank lines. Each `format!` in a builder ends with `{trailing}` where
/// this helper plugs in.
///
/// The table below IS the summary: source order is output order, and each
/// line's condition sits beside it. Three of the five lines are conditional,
/// and `Preceding` says why — see `NoLineRestatesTheDesignStep` and
/// `DesignStepMatchesTheReposSpecs` in `docs/specs/dispatch-prompt.allium`. In short:
/// the trailing block never repeats a rule the addendum above it already gave,
/// and it never points a spec-less repo at `docs/specs/`.
///
/// A prose summary of the emitted order was tried here and went stale the
/// first time a conditional line was added, because the order lived in a
/// `match` result plus a later conditional push and was stated nowhere in one
/// place. A sixth line is now one row, at the position it occupies, with its
/// predicate attached.
pub(super) fn trailing_block(preceding: Preceding) -> String {
    // Each predicate spells out the reason its line is conditional, rather
    // than naming the states that happen to satisfy it. One shared flag
    // covering two reasons at once is what dropped the stopping-point line
    // from every spec-less repo.
    [
        // Steps 3-4 of spec-first already state test-first, unconditionally.
        (
            tdd_instruction(),
            !preceding.addendum_settles_tdd_and_allium(),
        ),
        // Two independent reasons, and this is the only line either reaches:
        // steps 2 and 5 state the tend/weed cycle, and telling an agent
        // `docs/specs/` is the source of truth is false in a repo with no such
        // directory and would send it looking for one. It is also the only
        // line that names a spec directory at all.
        (
            allium_instruction(),
            !preceding.addendum_settles_tdd_and_allium() && preceding.keeps_specs(),
        ),
        (learning_tools_instruction(), true),
        // Immediately above the line whose subject it qualifies, so the rule
        // and the call it constrains read as one thought. Both design steps
        // state it themselves, leaving the plan paths as the only ones that
        // need it — spec-keeping or not, since the wording names no spec
        // directory.
        (
            plan_not_a_stopping_point_instruction(),
            preceding.has_plan(),
        ),
        (wrap_up_instruction(), true),
    ]
    .into_iter()
    .filter_map(|(line, keep)| keep.then_some(line))
    .collect::<Vec<_>>()
    .join("\n\n")
}

/// Render the tiered-knowledge block placed between the task block and the
/// addendum in a dispatch prompt. Returns an empty string when `picked` is
/// empty so existing prompts are byte-identical when no learnings are injected.
pub(super) fn render_validated_knowledge_block(picked: &[&Learning]) -> String {
    if picked.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "## Validated knowledge for this task\n\n\
The following knowledge has been validated by previous agents. Apply it where relevant. \
When you act on an entry, call `rate_learning(learning_id, task_id, verdict)` — `helped` if it \
applied, `wrong` if it misled you.\n\n",
    );
    for l in picked {
        out.push_str(&format!(
            "- [#{} {}, \u{2191}{}] {}\n",
            l.id.0,
            l.scope.as_str(),
            l.upvote_count,
            l.summary
        ));
    }
    out.push('\n');
    out
}

/// Whether a blank line separates the intro line from the task block.
/// `build_prompt` uses a single newline; the other two builders use a blank
/// line — a pre-existing inconsistency this enum makes explicit instead of
/// encoding it as raw `"\n"` vs `"\n\n"` bytes in the `intro` string.
enum IntroSpacing {
    SingleNewline,
    BlankLine,
}

impl IntroSpacing {
    fn as_separator(&self) -> &'static str {
        match self {
            IntroSpacing::SingleNewline => "\n",
            IntroSpacing::BlankLine => "\n\n",
        }
    }
}

/// Shared skeleton for every `build_*_prompt` variant:
/// `{intro}{spacing}{block}\n\n{knowledge}{addendum}\n\n{trailing}`.
///
/// Callers build `block` themselves (via `task_block`) since its inputs
/// aren't otherwise needed here — keeps this under clippy's argument-count
/// limit.
///
/// Each builder computes its own `intro`/`block`/`addendum`/`trailing` and
/// passes them here, so the knowledge plumbing stays in one place — a variant
/// can no longer silently drop the knowledge block (the research-prompt drift
/// this fixed) by forgetting to wire it in.
fn render_task_prompt(
    intro: &str,
    spacing: IntroSpacing,
    block: &str,
    ctx: &PromptContext<'_>,
    addendum: &str,
    trailing: &str,
) -> String {
    let knowledge = render_validated_knowledge_block(&ctx.learnings.ranked);
    let sep = spacing.as_separator();
    format!("{intro}{sep}{block}\n\n{knowledge}{addendum}\n\n{trailing}")
}

pub(super) fn build_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    plan: Option<&str>,
    epic: Option<&EpicContext>,
    ctx: &PromptContext<'_>,
) -> String {
    // Dependabot and PR-review tasks are review-only: they skip the plan /
    // implementation flow and use a trimmed trailing block.
    let is_review = ctx.tag.is_some_and(|t| t.is_review());
    let is_cve = is_cve_task(ctx.tag, epic);
    let addendum = match (ctx.tag, plan) {
        (Some(TaskTag::Dependabot), _) => {
            dependabot_review_addendum(task_id, title, description, ctx.pr_url, ctx.from_feed)
        }
        (Some(TaskTag::PrReview), _) => pr_review_addendum().to_string(),
        // A CVE task names no design step: the runbook takes the addendum the
        // design step would have had, and only that one. `is_cve` is read from
        // the epic context because answering it needs the epic's ancestry.
        (_, None) if is_cve => cve_runbook().to_string(),
        (_, None) => design_instruction(ctx.has_allium_specs).to_string(),
        (_, Some(path)) => {
            let tail = if ctx.auto_run_plan {
                " and begin implementing it right away — the plan has already \
been reviewed and confirmed, so no summary or confirmation step is needed."
            } else {
                ".\n\
\n\
Read the plan, then summarise the approach you intend to take and ask the user to \
confirm it. Make no changes until they do."
            };
            format!("Plan: {path}\nRead this file for the full implementation plan{tail}")
        }
    };
    let trailing = if is_review {
        learning_tools_instruction().to_string()
    } else {
        trailing_block(Preceding::resolve(
            plan.is_some(),
            ctx.has_allium_specs,
            is_cve,
        ))
    };

    let block = task_block(task_id, title, description, epic);
    render_task_prompt(
        "Your task is:",
        IntroSpacing::SingleNewline,
        &block,
        ctx,
        &addendum,
        &trailing,
    )
}

/// Substitute every `{{KEY}}` placeholder in a prompt template loaded via
/// `include_str!`, in one pass. Trims the trailing newline added by editors so
/// the inlined block composes cleanly with surrounding `format!` blocks.
///
/// One pass rather than a chain of single-key calls, because a chain is
/// order-dependent in a way nothing about it shows: substituting `TASK_ID`
/// first and then splicing in a fragment means a fragment that legitimately
/// wants `{{TASK_ID}}` ships the literal braces to the agent. Here a value is
/// never rescanned, so the pairs can be given in any order.
fn render_template(template: &str, pairs: &[(&str, &str)]) -> String {
    let template = template.trim_end_matches('\n');
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open..].find("}}").map(|i| open + i) else {
            break;
        };
        let key = &rest[open + 2..close];
        match pairs.iter().find(|(k, _)| *k == key) {
            Some((_, value)) => {
                out.push_str(&rest[..open]);
                out.push_str(value);
            }
            // An unknown key is left verbatim rather than blanked, so a typo
            // shows up in the rendered prompt (and in the snapshot) instead of
            // silently deleting a step.
            None => out.push_str(&rest[..close + 2]),
        }
        rest = &rest[close + 2..];
    }
    out.push_str(rest);
    out
}

/// The two fragments the dependabot runbook is assembled from for a given bump:
/// the one decision body, and the merge terminal — empty when that terminal is
/// unreachable from this route.
///
/// Both come out of one match rather than a body plus a reachability flag: a
/// flag is a second encoding of the same branch, and a new [`BumpKind`] could
/// pick a body and forget it. A route that can never merge does not get shown
/// how to, which matters most on the major branch — rendering the merge
/// commands beside "never merge a major bump yourself" would put the exact call
/// the branch forbids two lines under the prohibition. See
/// `AReviewRunbookCarriesOnlyTheBranchThatApplies` in `docs/specs/dispatch-prompt.allium`.
fn dependabot_decision(kind: BumpKind) -> (&'static str, &'static str) {
    // MERGE is a const because two arms share it. Every decision body is
    // inlined, so the fragment a route renders is readable off its own arm.
    const MERGE: &str = include_str!("prompts/dependabot/merge.md");
    match kind {
        BumpKind::Patch => (include_str!("prompts/dependabot/patch.md"), MERGE),
        BumpKind::Minor => (include_str!("prompts/dependabot/minor.md"), MERGE),
        BumpKind::Major => (include_str!("prompts/dependabot/major.md"), ""),
        // Reaches the same ask terminal as an unclassified bump, but says WHY
        // there is nothing to read rather than that nothing was read.
        BumpKind::Digest => (include_str!("prompts/dependabot/digest.md"), ""),
        // Two ask bodies, not one shared body: each states WHY the bump is
        // unroutable, and neither reason is true of the other. Sharing one
        // told every unclassified PR it was a group.
        BumpKind::NonMajor => (include_str!("prompts/dependabot/grouped.md"), ""),
        BumpKind::Unknown => (include_str!("prompts/dependabot/unclassified.md"), ""),
    }
}

/// PR review guidance, loaded from `prompts/pr-review.md`.
///
/// Find the PR, run the review command on it, present the findings and wait.
/// One command: the diff-size branch that used to pick between two was a proxy
/// for an effort level the command takes itself. Nothing here forbids
/// `/wrap-up` — `wrap_up` refuses a review-tagged task (`Task::wrap_up_block`).
fn pr_review_addendum() -> &'static str {
    include_str!("prompts/pr-review.md").trim_end_matches('\n')
}

/// CVE remediation guidance, loaded from `prompts/cve.md`.
///
/// Replaces the DESIGN step and nothing else, so it is selected only on the
/// no-plan arm: an attached plan still wins the addendum, unlike the two review
/// runbooks, which ignore the plan outright. Silently not reading a plan
/// someone wrote for this task would be a worse failure than a missing runbook.
///
/// Unlike those runbooks it does not trim the trailing block. A CVE task
/// authors its own PR and finishes through `/wrap-up`, so `wrap_up_instruction`
/// is owed to it — and for the same reason `TaskTag::is_review` is untouched.
/// See `CveRemediationSkipsTheDesignStep` in `docs/specs/dispatch-prompt.allium`.
fn cve_runbook() -> &'static str {
    include_str!("prompts/cve.md").trim_end_matches('\n')
}

/// Whether this prompt takes the CVE branch: the task hangs under the managed
/// CVE feed root, and no tag claims it first.
///
/// The single decision point, so `build_prompt` and
/// `build_quick_dispatch_prompt` cannot disagree about which branch a given
/// task took. Written against `tag` rather than against what each builder
/// happens to carry: quick dispatch never sets a tag today, and a guard that
/// relies on that is a guard resting on a property nothing checks.
///
/// The three excluded tags are `NameTheDesignStep`'s own list, and deliberately
/// the same list. `Dependabot` and `PrReview` review someone else's PR, which
/// is different work from remediating the alert. `Research` is normally
/// diverted to `build_research_prompt` before any addendum is selected — but
/// only while it has no plan, since `DispatchMode::for_task` routes ANY planned
/// task to `Dispatch`. Naming it here closes that corner rather than resting on
/// a divert that does not always happen.
fn is_cve_task(tag: Option<TaskTag>, epic: Option<&EpicContext>) -> bool {
    let claimed_by_tag = tag.is_some_and(|t| t.is_review() || t == TaskTag::Research);
    !claimed_by_tag && epic.is_some_and(|e| e.under_cve_feed)
}

/// Dependabot PR review guidance, assembled from `prompts/dependabot.md` and
/// the fragments under `prompts/dependabot/`.
///
/// The bump is classified from the task's own title and description before the
/// template is filled, so the rendered runbook states which bump this is and
/// carries only the branch that bump takes. The PR URL is rendered too, when
/// the task has one: `gh` accepts a URL wherever it accepts a number, so
/// rendering it removes both the extract-the-number step and the
/// `--repo <owner/repo>` the agent would otherwise have to spell five times.
///
/// What is left for the agent is the work that needs the network or a
/// judgement: the dep-only file check, CI status, fetching a changelog, and
/// writing the breaking-change summary.
///
/// It does NOT tell the agent to avoid /wrap-up — `wrap_up` refuses a
/// review-tagged task itself (`ReviewTasksAreNotWrappedUp` in
/// `docs/specs/mcp-task-tools.allium`), so the rule no longer needs asking for.
fn dependabot_review_addendum(
    task_id: TaskId,
    title: &str,
    description: &str,
    pr_url: Option<&str>,
    from_feed: bool,
) -> String {
    let bump = bump::classify(title, description);
    let (decision, merge) = dependabot_decision(bump.kind);
    // Both fragments are trimmed and the paragraph break is added here, so no
    // fragment's trailing blank line is load-bearing file bytes an editor or a
    // whitespace hook could silently eat.
    let merge = match merge.trim_end() {
        "" => String::new(),
        body => format!("{body}\n\n"),
    };
    const AUTHOR: &str = include_str!("prompts/dependabot/author.md");
    let task_id_str = task_id.0.to_string();
    // One match decides both, because the author bullet is only ever true of a
    // PR this task actually names. Rendering it from the same arm makes that
    // structural: the `not recorded` arm cannot produce the bullet, so nothing
    // has to assert that the two agree.
    //
    // The bullet states a fact about how this PR reached the board, so both
    // halves must hold — a feed created the task (only a feed sets
    // external_id) AND the task names a PR whose author that feed could have
    // filtered. The CVE feed sets external_id too and its task can be retagged
    // `dependabot` over MCP, so provenance alone is not enough.
    //
    // What goes when it goes is a PROHIBITION, and nothing replaces it:
    // dropping it leaves the agent free to check the author, which is what it
    // should do when nothing vouched for it.
    let (pr, author) = match pr_url {
        Some(url) => (
            format!(
                "PR: {url}\n   Pass this URL to every `gh` command below — it identifies the \
repo too, so none of them need `--repo`."
            ),
            // The fragment carries its own leading newline via this join, so
            // omitting it leaves no blank bullet behind.
            if from_feed {
                format!("\n{}", AUTHOR.trim_end())
            } else {
                String::new()
            },
        ),
        // No url means the feed never recorded one, so the agent does have to
        // find the PR itself. That is the only case the extract-it-yourself
        // instruction survives for — and with no PR named, there is no
        // filtered author to stand on either.
        None => (
            format!(
                "PR: not recorded on this task. Find its URL in the task description, then call \
update_task(task_id={task_id_str}, url=<URL>, url_type=\"pr\") before going on."
            ),
            String::new(),
        ),
    };
    render_template(
        include_str!("prompts/dependabot.md"),
        &[
            ("TASK_ID", &task_id_str),
            ("BUMP", &bump.prompt_line()),
            ("PR", &pr),
            ("AUTHOR", &author),
            ("DECISION", decision.trim_end()),
            ("MERGE", &merge),
        ],
    )
}

pub(super) fn build_quick_dispatch_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
    ctx: &PromptContext<'_>,
) -> String {
    // A placeholder task created inside the CVE epic is CVE work too, so the
    // same branch applies here — through the same guard, not a second copy of
    // it that assumes quick dispatch carries no tag.
    let is_cve = is_cve_task(ctx.tag, epic);
    let addendum = format!(
        "This is a quick-dispatched task with a placeholder title. Start by asking the user \
what they want to achieve. Once you understand the goal, call `update_task` with a \
descriptive `title` (and optionally `description`) to rename the task on the kanban board.\n\
\n\
Then, before making any changes:\n\
\n\
{design}",
        design = if is_cve {
            cve_runbook()
        } else {
            design_instruction(ctx.has_allium_specs)
        },
    );

    let block = task_block(task_id, title, description, epic);
    render_task_prompt(
        "You are working interactively with the user.",
        IntroSpacing::BlankLine,
        &block,
        ctx,
        &addendum,
        // Quick dispatch never carries a plan, so it always asks for a design
        // step — spec-first whenever the repo keeps specs, unless the task sits
        // under the CVE epic.
        &trailing_block(Preceding::resolve(false, ctx.has_allium_specs, is_cve)),
    )
}

/// The research prompt's opening line. Routing tests across the runtime and
/// service layers assert on it to prove `DispatchMode::Research` reached this
/// builder, so it is a shared constant rather than a literal they each repeat.
pub(crate) const RESEARCH_AGENT_INTRO: &str = "You are a research agent.";

pub(super) fn build_research_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
    ctx: &PromptContext<'_>,
) -> String {
    let addendum = "Investigate the topic described above. You may read the codebase, \
documentation, and external resources.\n\
\n\
When you have gathered sufficient information, present your findings clearly to the user \
and wait for further instructions; this session is for investigation, so code changes and \
wrapping up are the user's call once they have read what you found.";

    let block = task_block(task_id, title, description, epic);
    render_task_prompt(
        RESEARCH_AGENT_INTRO,
        IntroSpacing::BlankLine,
        &block,
        ctx,
        addendum,
        learning_tools_instruction(),
    )
}

/// Maximum total learnings injected into a dispatch prompt via RAG.
pub const DISPATCH_INJECTION_CAP: usize = 5;

/// Push-injection groups for a dispatch prompt.
#[derive(Default, Clone)]
pub struct LearningInjections<'a> {
    pub ranked: Vec<&'a Learning>,
}

impl<'a> From<&'a [Learning]> for LearningInjections<'a> {
    fn from(v: &'a [Learning]) -> Self {
        Self {
            ranked: v.iter().collect(),
        }
    }
}

/// Bundle of all push-injected context for a dispatch prompt. Threaded through
/// every `build_*_prompt` so individual builders never grow more positional
/// parameters when a new context source lands.
pub struct PromptContext<'a> {
    pub learnings: LearningInjections<'a>,
    pub tag: Option<TaskTag>,
    pub auto_run_plan: bool,
    /// Does the task's repository keep Allium specs? Decides which design step
    /// the prompt names and whether `allium_instruction` is emitted — see
    /// `DesignStepMatchesTheReposSpecs` in `docs/specs/dispatch-prompt.allium`.
    /// Computed per dispatch by `super::allium_specs::repo_has_allium_specs`.
    pub has_allium_specs: bool,
    /// The task's PR URL, when it has one of `url_type = pr`.
    ///
    /// Read only by the dependabot runbook, which threads a PR through five
    /// `gh` calls. The feed sets `url` at insert time, so the number and the
    /// owner/repo the runbook used to ask the agent to extract from the
    /// description were already on the task — and the description it was told
    /// to extract them from is the 500-character truncation.
    pub pr_url: Option<&'a str>,
    /// Did a feed create this task? Read from `Task.external_id` being set,
    /// which only a feed does.
    ///
    /// Read only by the dependabot runbook, to decide whether to tell the
    /// agent the PR author was already filtered. Deliberately NOT derived from
    /// `pr_url`: `update_task` takes a url and the dependabot tag together, so
    /// a hand-created task can carry a PR without any feed having filtered
    /// anything. See `AReviewRunbookCarriesOnlyTheBranchThatApplies` in
    /// `docs/specs/dispatch-prompt.allium`.
    pub from_feed: bool,
}

/// `Default` is hand-written for one field: `has_allium_specs` defaults to
/// `true`, not `bool::default()`. A context assembled without the filesystem
/// check must not silently downgrade a spec-keeping repo to brainstorming —
/// the detection helper is the only thing entitled to answer `false`.
impl Default for PromptContext<'_> {
    fn default() -> Self {
        Self {
            learnings: LearningInjections::default(),
            tag: None,
            auto_run_plan: false,
            has_allium_specs: true,
            pr_url: None,
            from_feed: false,
        }
    }
}

pub use crate::embeddings::RAG_SIMILARITY_THRESHOLD as DISPATCH_RAG_THRESHOLD;

/// Build the learning injections for a dispatch prompt using the RAG pipeline.
///
/// Steps:
/// 1. Embeds the task title + description to form a query vector.
/// 2. Fetches all approved non-task-scoped learnings with embeddings from the DB.
/// 3. Ranks them by cosine similarity + scope/upvote boost (via `rag_rank_learnings`).
/// 4. Returns at most `DISPATCH_INJECTION_CAP` results; all go into the
///    validated-knowledge block regardless of `LearningKind`.
///
/// On embedding failure the function falls back to an empty list so a single
/// model error never blocks dispatch.
pub async fn list_learnings_for_dispatch_rag(
    db: &dyn crate::store::TaskReadStore,
    task: &Task,
    emb_svc: &Arc<EmbeddingService>,
    threshold: f32,
) -> Vec<Learning> {
    let query_text = embed_text_for_query(&task.title, &task.description);
    let query_vec = match emb_svc.embed(query_text).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                task_id = task.id.0,
                error = ?e,
                "dispatch RAG: embedding query failed, skipping injection"
            );
            return vec![];
        }
    };

    let rows = match db.list_all_approved_non_task_learnings().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                task_id = task.id.0,
                error = ?e,
                "dispatch RAG: failed to fetch learnings, skipping injection"
            );
            return vec![];
        }
    };

    let candidates = deserialize_candidate_rows(rows);

    let epic_id_str = task.epic_id.map(|e| e.0.to_string());
    let all_ranked = rag_rank_learnings(
        &candidates,
        &RagRankParams {
            query_vec: &query_vec,
            task_epic_id: epic_id_str.as_deref(),
            task_repo: Some(task.repo_path.as_str()),
            threshold,
            tag_filter: &[],
            limit: DISPATCH_INJECTION_CAP,
        },
    );

    all_ranked.into_iter().cloned().collect()
}

pub async fn build_and_record_injections(
    db: &dyn crate::store::TaskReadStore,
    task: &crate::models::Task,
    emb_svc: &Arc<EmbeddingService>,
) -> Vec<Learning> {
    let all = list_learnings_for_dispatch_rag(db, task, emb_svc, DISPATCH_RAG_THRESHOLD).await;
    for l in &all {
        if let Err(e) = db
            .record_retrieval(task.id, l.id, RetrievalSource::PromptInjection)
            .await
        {
            tracing::warn!(
                task_id = task.id.0,
                learning_id = l.id.0,
                error = ?e,
                "failed to record learning retrieval"
            );
        }
    }
    all
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod rag_dispatch_tests;

/// `EpicContext::from_db`'s CVE answer, which is an ancestry walk rather than a
/// single row read. With `group_by_repo` on, a CVE feed's tasks land on a
/// repo-group SUB-epic whose own `feed_role` is `none`, so reading the task's
/// immediate epic alone answers false for every task on a grouped CVE board.
/// See `TheCveAnswerIsStructuralNotTextual` in `docs/specs/dispatch-prompt.allium`.
#[cfg(test)]
mod cve_epic_context_tests;
