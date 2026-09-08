use std::sync::Arc;

use crate::db;
use crate::models::{EpicId, Learning, RetrievalSource, Task, TaskId, TaskTag};
use crate::service::embeddings::{
    deserialize_candidate_rows, embed_text_for_query, rag_rank_learnings, EmbeddingService,
    RagRankParams,
};

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
}

impl EpicContext {
    /// Build epic context from the database for a task that belongs to an epic.
    pub async fn from_db(task: &Task, db: &dyn db::TaskReadStore) -> Option<Self> {
        let epic_id = task.epic_id?;
        let epic = db.get_epic(epic_id).await.ok()??;
        Some(EpicContext {
            epic_id,
            epic_title: epic.title,
        })
    }

    pub(super) fn prompt_section(&self) -> String {
        format!(
            "\n\nThis task is part of epic #{}: {}\n\
            To find other tasks in this epic, call list_tasks with epic_id={}.\n\
            To ask questions or send updates to a sibling agent, use ListAgents to find its \
            session (named task-<id>, matching that task's own id) and message it directly \
            with SendMessage.",
            self.epic_id, self.epic_title, self.epic_id
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
/// `ThePromptNamesNoToolMerelyToSayItExists` in `docs/specs/dispatch.allium`,
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
/// Allium-first, interview-driven sequence (elicit → spec → tests → code →
/// weed) that replaced the older `/brainstorming` design-doc-then-plan step in
/// task #4366.
///
/// Shared verbatim between the no-plan dispatch addendum and the quick-dispatch
/// addendum, so the design step cannot drift apart between the two. A
/// `docs/plans/` doc and a hand-off to `/allium-loop` are both named as the
/// agent's judgement call rather than requirements — the spec, not a plan, is
/// what this step is expected to produce.
///
/// Each step names its skill and stops, the same rule
/// [`brainstorm_instruction`] follows. Step 1 said "One question at a time"
/// until `allium:elicit` turned out to head a section with that exact rule,
/// which made the clause a paraphrase of the skill the step loads.
///
/// Framed as an intermediate step, not a stopping point;
/// `Research`/`Dependabot`/`PrReview` never reach this addendum (see
/// `DispatchMode::for_task` and `TaskTag::is_review`), so no per-tag branch is
/// needed here. Carries the same epic-decomposition carve-out as
/// `wrap_up_instruction` so the two stay consistent about what counts as done.
pub(super) fn spec_first_instruction() -> &'static str {
    "Design the solution spec-first, in this order:\n\
\n\
1. Interview the user with the `allium:elicit` skill until the intended behaviour is \
unambiguous.\n\
2. Capture what you agreed in the relevant `docs/specs/*.allium` file, via `allium:tend`.\n\
3. Generate tests from the spec with `allium:propagate` and confirm they fail before you \
write any code.\n\
4. Implement the minimum code that makes them pass.\n\
5. Confirm spec and code agree with `allium:weed`.\n\
\n\
Two things are your judgement call, not requirements: write a plan to docs/plans/ and \
attach it with update_task only if the implementation is big enough that its steps are \
worth recording; and for a large or stubborn convergence, hand steps 3-5 to the \
`/allium-loop` skill instead of running them inline.\n\
\n\
The spec is not the end of the task — implement it in this same session (or, for an \
epic-decomposition task, create work packages for its subtasks instead) and verify your \
work before wrapping up."
}

/// The design instruction for a no-plan task in a repo that keeps **no** Allium
/// specs (`docs/specs/*.allium` is absent or empty). Sending such an agent to
/// `allium:elicit` would ask it to tend a garden that does not exist, so the
/// design step is `superpowers:brainstorming` instead — see
/// `DesignStepMatchesTheReposSpecs` in `docs/specs/dispatch.allium`.
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
/// Emitted only on the plan path. Both design steps close by stating the same
/// rule ("The spec is not the end of the task — implement it in this same
/// session…"), so on their paths this is the restatement
/// `NoLineRestatesTheDesignStep` rules out. The plan path is the one with no
/// design step above it, and is also the case #4188 was actually about.
///
/// It names only the plan, not "a spec or a plan": no spec-writing step runs
/// on the path that emits it, so the narrower wording is the accurate one.
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
/// trailing lines would be restatements. Three variants because only three
/// states are reachable: a two-boolean signature admitted a fourth
/// (spec-first in a repo with no specs) that cannot occur, and left
/// `has_allium_specs` silently unread whenever spec-first was set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Preceding {
    /// `spec_first_instruction` — states test-first as its steps 3 and 4, and
    /// the tend/weed cycle as its steps 2 and 5.
    SpecFirst,
    /// A plan was attached, in a repo that keeps specs. The prompt names no
    /// design sequence, so every conditional trailing line carries.
    PlanWithSpecs,
    /// The repo keeps no Allium specs, so the design step is
    /// `brainstorm_instruction`, which names a skill and nothing else.
    NoSpecs,
}

impl Preceding {
    /// The design-step branch, given whether a plan is attached and whether the
    /// repo keeps specs. The single place the mapping lives, so the two
    /// builders cannot disagree about which branch they took.
    pub(super) fn resolve(has_plan: bool, has_allium_specs: bool) -> Self {
        match (has_plan, has_allium_specs) {
            (_, false) => Preceding::NoSpecs,
            (true, true) => Preceding::PlanWithSpecs,
            (false, true) => Preceding::SpecFirst,
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
/// `DesignStepMatchesTheReposSpecs` in `docs/specs/dispatch.allium`. In short:
/// the trailing block never repeats a rule the addendum above it already gave,
/// and it never points a spec-less repo at `docs/specs/`.
///
/// A prose summary of the emitted order was tried here and went stale the
/// first time a conditional line was added, because the order lived in a
/// `match` result plus a later conditional push and was stated nowhere in one
/// place. A sixth line is now one row, at the position it occupies, with its
/// predicate attached.
pub(super) fn trailing_block(preceding: Preceding) -> String {
    let plan_path = preceding == Preceding::PlanWithSpecs;
    [
        // Steps 3-4 of spec-first already state test-first, unconditionally.
        (tdd_instruction(), preceding != Preceding::SpecFirst),
        // Steps 2 and 5 state the tend/weed cycle; and telling an agent
        // `docs/specs/` is the source of truth is false in a repo with no such
        // directory, and would send it looking for one.
        (allium_instruction(), plan_path),
        (learning_tools_instruction(), true),
        // Immediately above the line whose subject it qualifies, so the rule
        // and the call it constrains read as one thought. Both design steps
        // state it themselves, leaving the plan path as the only one that
        // needs it.
        (plan_not_a_stopping_point_instruction(), plan_path),
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
    let addendum = match (ctx.tag, plan) {
        (Some(TaskTag::Dependabot), _) => {
            dependabot_review_addendum(task_id, title, description, ctx.pr_url, ctx.from_feed)
        }
        (Some(TaskTag::PrReview), _) => pr_review_addendum().to_string(),
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
        trailing_block(Preceding::resolve(plan.is_some(), ctx.has_allium_specs))
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
/// `AReviewRunbookCarriesOnlyTheBranchThatApplies` in `docs/specs/dispatch.allium`.
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
    let addendum = format!(
        "This is a quick-dispatched task with a placeholder title. Start by asking the user \
what they want to achieve. Once you understand the goal, call `update_task` with a \
descriptive `title` (and optionally `description`) to rename the task on the kanban board.\n\
\n\
Then, before making any changes:\n\
\n\
{design}",
        design = design_instruction(ctx.has_allium_specs),
    );

    let block = task_block(task_id, title, description, epic);
    render_task_prompt(
        "You are working interactively with the user.",
        IntroSpacing::BlankLine,
        &block,
        ctx,
        &addendum,
        // Quick dispatch never carries a plan, so it always asks for a design
        // step — spec-first whenever the repo keeps specs.
        &trailing_block(Preceding::resolve(false, ctx.has_allium_specs)),
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
and wait for further instructions. Do NOT call /wrap-up — that is for the user to \
decide.\n\
\n\
Do NOT make code changes.";

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
    /// `DesignStepMatchesTheReposSpecs` in `docs/specs/dispatch.allium`.
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
    /// `docs/specs/dispatch.allium`.
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

pub use crate::service::embeddings::RAG_SIMILARITY_THRESHOLD as DISPATCH_RAG_THRESHOLD;

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
    db: &dyn crate::db::TaskReadStore,
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
    db: &dyn crate::db::TaskReadStore,
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::models::{LearningKind, LearningScope};

    #[test]
    fn pr_rebase_preamble_targets_pr_branch_on_demand() {
        let text = pr_rebase_preamble("renovate/serde-1.x");
        assert!(
            text.contains("git fetch origin renovate/serde-1.x"),
            "should fetch the PR branch, got: {text}"
        );
        assert!(
            text.contains("git rebase origin/renovate/serde-1.x"),
            "should rebase onto origin/<pr-branch>, got: {text}"
        );
        assert!(
            !text.contains("rebase main"),
            "must not rebase onto the base branch, got: {text}"
        );
    }

    #[test]
    fn reused_preamble_targets_a_local_start_point() {
        let sp = StartPoint::Local {
            base: "develop".to_string(),
        };
        let text = reused_rebase_preamble(&sp);
        assert!(text.contains("git fetch origin develop"), "got: {text}");
        assert!(text.contains("git rebase develop"), "got: {text}");
        assert!(
            !text.contains("git rebase origin/develop"),
            "must not drag a local-based branch back onto origin: {text}"
        );
        assert!(
            text.contains("git status"),
            "tells the agent to inspect first"
        );
        assert!(!text.contains("main"), "no literal main: {text}");
    }

    #[test]
    fn reused_preamble_targets_a_remote_start_point() {
        let sp = StartPoint::Remote {
            base: "develop".to_string(),
        };
        let text = reused_rebase_preamble(&sp);
        assert!(text.contains("git fetch origin develop"), "got: {text}");
        assert!(text.contains("git rebase origin/develop"), "got: {text}");
    }

    #[test]
    fn select_preamble_is_empty_for_a_fresh_worktree() {
        let sp = StartPoint::Remote {
            base: "main".to_string(),
        };
        assert_eq!(select_preamble(None, Some(&sp), false), "");
    }

    #[test]
    fn select_preamble_uses_reuse_wording_for_a_reused_worktree() {
        let sp = StartPoint::Local {
            base: "main".to_string(),
        };
        let text = select_preamble(None, Some(&sp), true);
        assert!(
            text.contains("reused from a previous attempt"),
            "got: {text}"
        );
        assert!(
            text.contains("git rebase main"),
            "mirrors the start point: {text}"
        );
    }

    #[test]
    fn select_preamble_prefers_the_pr_branch_regardless_of_reuse() {
        let sp = StartPoint::Remote {
            base: "renovate/serde-1.x".to_string(),
        };
        for reused in [true, false] {
            let text = select_preamble(Some("renovate/serde-1.x"), Some(&sp), reused);
            assert!(
                text.contains("git rebase origin/renovate/serde-1.x"),
                "reused={reused}, got: {text}"
            );
            assert!(
                !text.contains("reused from a previous attempt"),
                "reused={reused}"
            );
        }
    }

    #[test]
    fn prompt_head_carries_the_warning_even_with_no_preamble() {
        // The fresh + no-origin-ref case: nothing to rebase onto, but the agent
        // must still be told its base is local-only.
        let head = compose_prompt_head("", Some("origin has no branch main"));
        assert!(
            head.contains("Note: origin has no branch main"),
            "got: {head}"
        );
        assert!(!head.starts_with('\n'), "no leading blank line: {head:?}");
    }

    #[test]
    fn prompt_head_is_empty_when_there_is_nothing_to_say() {
        assert_eq!(compose_prompt_head("", None), "");
    }

    #[test]
    fn prompt_head_combines_preamble_and_warning() {
        let head = compose_prompt_head("REBASE", Some("stale"));
        assert!(head.starts_with("REBASE"), "got: {head}");
        assert!(head.contains("Note: stale"), "got: {head}");
        assert!(head.ends_with("\n\n"), "separates from the body: {head:?}");
    }

    #[test]
    fn fresh_worktree_with_no_origin_ref_gets_the_note_and_no_preamble() {
        // The decoupled case: nothing to rebase onto, but the agent must still be
        // told its base is local-only. An earlier design attached the warning to
        // the preamble, which silently dropped it on exactly this row.
        let sp = StartPoint::Local {
            base: "main".to_string(),
        };
        let preamble = select_preamble(None, Some(&sp), false);
        let head = compose_prompt_head(&preamble, Some("origin has no branch main"));

        assert!(
            preamble.is_empty(),
            "fresh worktree emits no preamble: {preamble:?}"
        );
        assert!(
            head.contains("Note: origin has no branch main"),
            "got: {head}"
        );
        assert!(
            !head.contains("git rebase"),
            "nothing to rebase onto: {head}"
        );
        assert!(
            !head.contains("reused from a previous attempt"),
            "got: {head}"
        );
    }

    #[test]
    fn learning_instruction_references_learnings_skill() {
        let text = learning_tools_instruction();
        assert!(
            text.contains("/learnings"),
            "learning instruction should reference the /learnings skill, got: {text}"
        );
    }

    /// Every MCP tool this line used to name now carries the same WHEN in its
    /// own schema, so naming them here restates three descriptions the agent
    /// already has — see `ThePromptNamesNoToolMerelyToSayItExists` in
    /// `docs/specs/dispatch.allium`. The skill survives: a skill listing
    /// carries a description, not a nudge to invoke it.
    ///
    /// Scanned over the whole trailing block, not just this one line, so the
    /// guard also catches a tool name reintroduced into a neighbouring shared
    /// line. `/wrap-up` is the skill, spelled with a hyphen, and does not match
    /// the `wrap_up` tool; every registry name is snake_case, so none collides
    /// with ordinary prose.
    ///
    /// Derived from `TOOL_NAMES` — the registry `mcp_tools!` generates — not a
    /// hand-written list. A hand-written one covered 8 of the 23 tools and
    /// would have grown a blind spot with every tool added, which is the same
    /// staleness this line was rewritten to escape.
    #[test]
    fn prompt_trailing_lines_name_no_mcp_tool() {
        for preceding in [
            Preceding::SpecFirst,
            Preceding::PlanWithSpecs,
            Preceding::NoSpecs,
        ] {
            let text = trailing_block(preceding);
            for tool in crate::mcp::handlers::TOOL_NAMES {
                assert!(
                    !text.contains(tool),
                    "{preceding:?}: the trailing block must not name the {tool} \
tool — its own description carries what the prose would say, got: {text}"
                );
            }
        }
    }

    #[test]
    fn research_prompt_names_forbidden_wrap_up_tool() {
        let text = build_research_prompt(
            TaskId(7),
            "Research async runtimes",
            "Compare tokio vs async-std",
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("/wrap-up"),
            "research prompt should explicitly forbid /wrap-up by name, got: {text}"
        );
    }

    #[test]
    fn learning_instruction_omits_deleted_action_skills() {
        let text = learning_tools_instruction();
        for skill in [
            "/codebase-knowledge",
            "/code-conventions",
            "/test-conventions",
            "/pr-workflow",
            "/dispatch-workflow",
            "/troubleshoot",
            "/improvement",
        ] {
            assert!(
                !text.contains(skill),
                "learning instruction should no longer reference deleted skill {skill}, got: {text}"
            );
        }
    }

    #[test]
    fn learning_instruction_in_task_prompts_with_plan() {
        let text = build_prompt(
            TaskId(1),
            "title",
            "desc",
            Some("/path/to/plan.md"),
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("/learnings"),
            "build_prompt (with plan) should reference /learnings skill"
        );
    }

    #[test]
    fn build_prompt_with_plan_and_auto_run_skips_confirmation() {
        let ctx = PromptContext {
            auto_run_plan: true,
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(1),
            "title",
            "desc",
            Some("/path/to/plan.md"),
            None,
            &ctx,
        );
        assert!(
            !text.contains("Shall I proceed with implementation?"),
            "auto_run_plan should skip the ask-permission addendum, got: {text}"
        );
        assert!(
            text.contains("/path/to/plan.md"),
            "the plan path should still be referenced, got: {text}"
        );
    }

    #[test]
    fn build_prompt_with_plan_default_still_asks_permission() {
        let text = build_prompt(
            TaskId(1),
            "title",
            "desc",
            Some("/path/to/plan.md"),
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("ask the user to confirm"),
            "default (auto_run_plan: false) must keep asking, got: {text}"
        );
        assert!(
            text.contains("Make no changes until they do"),
            "the confirmation must gate changes, not just request a summary, got: {text}"
        );
    }

    #[test]
    fn spec_first_instruction_frames_the_spec_as_an_intermediate_step() {
        let text = spec_first_instruction();
        assert!(
            text.contains("not the end of the task"),
            "spec_first_instruction should make clear writing the spec \
doesn't finish the task, got: {text}"
        );
        assert!(
            text.contains("implement it"),
            "spec_first_instruction should instruct the agent to implement \
after agreeing the spec, got: {text}"
        );
    }

    /// The whole point of task #4366: the design step is an Allium spec built
    /// by interview, not a prose design doc produced by /brainstorming.
    #[test]
    fn spec_first_instruction_names_the_elicit_spec_test_implement_sequence() {
        let text = spec_first_instruction();
        for token in [
            "allium:elicit",
            "docs/specs/",
            "allium:propagate",
            "allium:weed",
        ] {
            assert!(
                text.contains(token),
                "spec_first_instruction should name {token}, got: {text}"
            );
        }
        assert!(
            !text.contains("/brainstorming"),
            "spec_first_instruction must not name the retired /brainstorming skill, got: {text}"
        );
        // The sequence is ordered: elicit before the spec, spec before tests,
        // tests before the alignment check.
        let idx = |needle: &str| text.find(needle).expect("token present");
        assert!(
            idx("allium:elicit") < idx("docs/specs/"),
            "interview comes before the spec, got: {text}"
        );
        assert!(
            idx("docs/specs/") < idx("allium:propagate"),
            "the spec comes before the tests it generates, got: {text}"
        );
        assert!(
            idx("allium:propagate") < idx("allium:weed"),
            "tests come before the alignment check, got: {text}"
        );
    }

    /// The sequence names each skill and stops — the same rule
    /// `brainstorm_instruction` follows, applied to the branch it was not
    /// written for. `allium:elicit`'s SKILL.md heads a section "Ask one
    /// question at a time", so restating it here is a paraphrase of the skill
    /// the step loads, and can only drift from it.
    #[test]
    fn spec_first_instruction_does_not_paraphrase_the_skills_it_names() {
        let text = spec_first_instruction();
        assert!(
            !text.contains("One question at a time"),
            "step 1 must not restate allium:elicit's own interview rule, got: {text}"
        );
        // The skill is still named — dropping the paraphrase must not drop the
        // step that loads it.
        assert!(
            text.contains("allium:elicit"),
            "step 1 must still name the skill, got: {text}"
        );
    }

    /// Both escape hatches are the agent's judgement call. A prompt that reads
    /// as *requiring* a plan doc is the behaviour this task removed.
    #[test]
    fn spec_first_instruction_makes_the_plan_doc_and_allium_loop_optional() {
        let text = spec_first_instruction();
        assert!(
            text.contains("judgement call") || text.contains("not requirements"),
            "spec_first_instruction should mark the optional steps as the agent's \
call, got: {text}"
        );
        // A plan is still described well enough to write one when it helps.
        assert!(
            text.contains("docs/plans/") && text.contains("update_task"),
            "spec_first_instruction should still say where an optional plan goes \
and how to attach it, got: {text}"
        );
        assert!(
            text.contains("only if"),
            "the plan clause should be conditional, not an instruction, got: {text}"
        );
        assert!(
            text.contains("/allium-loop"),
            "spec_first_instruction should offer /allium-loop for a large or \
stubborn convergence, got: {text}"
        );
    }

    // -----------------------------------------------------------------------
    // DesignStepMatchesTheReposSpecs (task #4409)
    //
    // A repo with no `docs/specs/*.allium` cannot be sent through an
    // Allium-first design step, so its prompts name
    // `superpowers:brainstorming` instead and drop `allium_instruction`.
    // -----------------------------------------------------------------------

    /// The fallback names the skill and states why it was chosen. It must not
    /// paraphrase brainstorming's own process — the behaviour belongs to the
    /// skill the agent loads, and a paraphrase can only drift from it.
    #[test]
    fn brainstorm_instruction_names_the_skill_and_says_why() {
        let text = brainstorm_instruction();
        assert!(
            text.contains("superpowers:brainstorming"),
            "the fallback should name the brainstorming skill, got: {text}"
        );
        assert!(
            text.contains("no Allium specs"),
            "the fallback should say why it is not the spec-first sequence, got: {text}"
        );
    }

    #[test]
    fn brainstorm_instruction_does_not_restate_the_skill_or_tdd() {
        let text = brainstorm_instruction();
        for token in [
            "allium:elicit",
            "allium:tend",
            "allium:propagate",
            "allium:weed",
            "docs/specs/",
            "/allium-loop",
        ] {
            assert!(
                !text.contains(token),
                "the fallback must not name {token} — there is no spec to work \
from, got: {text}"
            );
        }
        assert!(
            !text.contains("failing test") && !text.contains("TDD"),
            "TDD is in the trailing block either way; the fallback must not \
restate it, got: {text}"
        );
    }

    /// The two design steps differ only in the design artefact. Everything
    /// about what is optional and what is not is identical.
    #[test]
    fn brainstorm_instruction_shares_the_optional_plan_and_no_stopping_point_clauses() {
        let text = brainstorm_instruction();
        assert!(
            text.contains("judgement call") && text.contains("docs/plans/"),
            "the plan doc should still be offered as the agent's call, got: {text}"
        );
        assert!(
            text.contains("update_task"),
            "the fallback should say how an optional plan gets attached, got: {text}"
        );
        assert!(
            text.contains("not the end of the task"),
            "the design step must not read as a stopping point, got: {text}"
        );
        assert!(
            text.contains("work packages"),
            "the epic-decomposition carve-out should match spec_first_instruction, \
got: {text}"
        );
    }

    #[test]
    fn prompt_context_defaults_to_the_spec_first_branch() {
        assert!(
            PromptContext::default().has_allium_specs,
            "an unknown repo must not be silently downgraded to brainstorming"
        );
    }

    /// The whole point of task #4409: the no-plan design step follows the repo.
    #[test]
    fn no_plan_addendum_follows_the_repos_specs() {
        let with_specs = build_prompt(TaskId(1), "t", "d", None, None, &PromptContext::default());
        assert!(
            with_specs.contains("allium:elicit") && !with_specs.contains("brainstorming"),
            "a spec-keeping repo gets the spec-first sequence, got: {with_specs}"
        );

        let no_specs = PromptContext {
            has_allium_specs: false,
            ..PromptContext::default()
        };
        let text = build_prompt(TaskId(1), "t", "d", None, None, &no_specs);
        assert!(
            text.contains("superpowers:brainstorming"),
            "a repo with no specs gets the brainstorming design step, got: {text}"
        );
        assert!(
            !text.contains("allium:elicit"),
            "a repo with no specs must not be sent to elicit a spec, got: {text}"
        );
    }

    #[test]
    fn quick_dispatch_design_step_follows_the_repos_specs() {
        let no_specs = PromptContext {
            has_allium_specs: false,
            ..PromptContext::default()
        };
        let text = build_quick_dispatch_prompt(TaskId(1), "t", "d", None, &no_specs);
        assert!(
            text.contains("superpowers:brainstorming") && !text.contains("allium:elicit"),
            "quick dispatch should share the no-spec design step, got: {text}"
        );
        // The rename step is unchanged — only the design step swaps.
        assert!(
            text.contains("call `update_task` with a"),
            "quick dispatch should still ask the agent to rename the task, got: {text}"
        );
    }

    /// The whole `trailing_block` contract as one table: which of the two
    /// conditional lines each `Preceding` carries, plus the two that are
    /// unconditional. Stated once rather than as three tests each re-asserting
    /// the invariants, so a fourth state means adding a row instead of deciding
    /// which test owns what.
    ///
    /// The two omissions have different reasons. `NoSpecs` drops the Allium
    /// line because pointing an agent at `docs/specs/` is false in a repo with
    /// no such directory. `SpecFirst` drops BOTH because that sequence already
    /// states them as numbered steps, and the trailing wordings are the weaker
    /// of the two — see NoLineRestatesTheDesignStep in
    /// `docs/specs/dispatch.allium`.
    #[test]
    fn trailing_block_carries_each_line_exactly_where_it_is_not_a_restatement() {
        for (preceding, want_tdd, want_allium, want_stopping_point) in [
            (Preceding::NoSpecs, true, false, false),
            (Preceding::PlanWithSpecs, true, true, true),
            (Preceding::SpecFirst, false, false, false),
        ] {
            let text = trailing_block(preceding);
            assert_eq!(
                text.contains(tdd_instruction()),
                want_tdd,
                "{preceding:?}: tdd presence, got: {text}"
            );
            assert_eq!(
                text.contains(allium_instruction()),
                want_allium,
                "{preceding:?}: allium presence, got: {text}"
            );
            // Both design steps close by saying the design is not the end of
            // the task, so only the plan path — which has no design step —
            // carries this line. Asserted on the substring rather than the
            // whole line, so a reworded restatement is caught too.
            assert_eq!(
                text.contains("not a stopping point"),
                want_stopping_point,
                "{preceding:?}: stopping-point presence, got: {text}"
            );
            if want_stopping_point {
                assert!(
                    text.contains(plan_not_a_stopping_point_instruction()),
                    "{preceding:?}: should carry the line verbatim, got: {text}"
                );
            }
            // No path may point a spec-less repo at the spec directory.
            if preceding == Preceding::NoSpecs {
                assert!(
                    !text.contains("docs/specs/"),
                    "{preceding:?}: no line may point at docs/specs/, got: {text}"
                );
            }
            // The two unconditional lines, on every path.
            assert!(
                text.contains("/learnings"),
                "{preceding:?}: the knowledge-base nudge is unconditional, got: {text}"
            );
            assert!(
                text.contains(wrap_up_instruction()),
                "{preceding:?}: wrap-up is unconditional, got: {text}"
            );
        }
    }

    /// `Preceding::resolve` is the single place the design-step branch is
    /// derived, so the two builders cannot disagree about which one they took.
    #[test]
    fn preceding_resolves_the_design_branch_from_plan_and_specs() {
        assert_eq!(Preceding::resolve(false, true), Preceding::SpecFirst);
        assert_eq!(Preceding::resolve(true, true), Preceding::PlanWithSpecs);
        // A repo with no specs takes the brainstorm branch either way.
        assert_eq!(Preceding::resolve(false, false), Preceding::NoSpecs);
        assert_eq!(Preceding::resolve(true, false), Preceding::NoSpecs);
    }

    /// The de-duplication is conditional on spec-first actually being present.
    /// A repo with no specs gets `brainstorm_instruction`, which names neither
    /// TDD nor Allium, so dropping the trailing lines there would leave the
    /// prompt with no statement of either.
    #[test]
    fn brainstorm_path_keeps_the_tdd_line_spec_first_would_have_replaced() {
        let no_specs = PromptContext {
            has_allium_specs: false,
            ..PromptContext::default()
        };
        let text = build_prompt(TaskId(1), "t", "d", None, None, &no_specs);
        assert!(
            text.contains("superpowers:brainstorming"),
            "sanity: this is the brainstorm path, got: {text}"
        );
        assert!(
            text.contains(tdd_instruction()),
            "the brainstorm path has no other statement of TDD, got: {text}"
        );
    }

    /// A task WITH a plan attached never reaches the spec-first sequence, so
    /// both lines stay there too.
    #[test]
    fn with_plan_path_keeps_tdd_and_allium() {
        let text = build_prompt(
            TaskId(1),
            "t",
            "d",
            Some("/tmp/plan.md"),
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains(tdd_instruction()),
            "the with-plan path has no other statement of TDD, got: {text}"
        );
        assert!(
            text.contains(allium_instruction()),
            "the with-plan path has no other statement of the tend/weed cycle, got: {text}"
        );
    }

    /// The trailing block is dropped in BOTH plan states, so one repo never
    /// gets contradictory prompts depending on whether a plan is attached.
    #[test]
    fn with_plan_prompt_omits_the_allium_instruction_when_the_repo_has_no_specs() {
        let no_specs = PromptContext {
            has_allium_specs: false,
            ..PromptContext::default()
        };
        let text = build_prompt(TaskId(1), "t", "d", Some("/p.md"), None, &no_specs);
        assert!(
            !text.contains("source of truth"),
            "with-plan prompt should drop the allium instruction too, got: {text}"
        );
        // The plan addendum itself is untouched.
        assert!(
            text.contains("Plan: /p.md"),
            "the plan addendum is unchanged, got: {text}"
        );
        assert!(
            !text.contains("superpowers:brainstorming"),
            "a task with a plan needs no design step, got: {text}"
        );
    }

    /// Review addenda already emit a trimmed trailing block with no allium
    /// line and no design step, so they need no branch — and must not grow one.
    #[test]
    fn review_addenda_are_unaffected_by_the_repos_specs() {
        for tag in [TaskTag::Dependabot, TaskTag::PrReview] {
            let ctx = PromptContext {
                tag: Some(tag),
                has_allium_specs: false,
                ..PromptContext::default()
            };
            let text = build_prompt(TaskId(1), "t", "d", None, None, &ctx);
            assert!(
                !text.contains("brainstorming"),
                "{tag:?}: a review agent gets no design step, got: {text}"
            );
        }
    }

    /// Brainstorming must not surface in *any* prompt for a repo that keeps
    /// Allium specs — there it is the retired design step (task #4366), and the
    /// spec-first sequence is the only one named. The no-spec branch that *does*
    /// name it (task #4409) is covered by
    /// `no_plan_addendum_follows_the_repos_specs`. A single named test cannot be
    /// blanket-accepted the way an `INSTA_UPDATE=always` snapshot can.
    #[test]
    fn no_prompt_variant_for_a_spec_keeping_repo_names_brainstorming() {
        let plain = PromptContext::default();
        assert!(
            plain.has_allium_specs,
            "this test only speaks for the spec-keeping branch"
        );
        let dependabot = PromptContext {
            tag: Some(TaskTag::Dependabot),
            ..PromptContext::default()
        };
        let pr_review = PromptContext {
            tag: Some(TaskTag::PrReview),
            ..PromptContext::default()
        };
        let variants = [
            (
                "dispatch no-plan",
                build_prompt(TaskId(1), "t", "d", None, None, &plain),
            ),
            (
                "dispatch with-plan",
                build_prompt(TaskId(1), "t", "d", Some("/p.md"), None, &plain),
            ),
            (
                "quick dispatch",
                build_quick_dispatch_prompt(TaskId(1), "t", "d", None, &plain),
            ),
            (
                "research",
                build_research_prompt(TaskId(1), "t", "d", None, &plain),
            ),
            (
                "dependabot",
                build_prompt(TaskId(1), "t", "d", None, None, &dependabot),
            ),
            (
                "pr review",
                build_prompt(TaskId(1), "t", "d", None, None, &pr_review),
            ),
        ];
        for (variant, text) in variants {
            assert!(
                !text.contains("brainstorm"),
                "{variant} prompt must not mention brainstorming, got: {text}"
            );
        }
    }

    #[test]
    fn no_plan_addendum_instructs_implementation_for_every_working_tag() {
        // spec_first_instruction is reused verbatim for every tag that
        // reaches it with no plan — Bug, Feature, Chore, Fix, and no tag.
        // Research never reaches this addendum with no plan (DispatchMode
        // diverts it to build_research_prompt instead), so no per-tag branch
        // is needed here.
        for tag in [
            None,
            Some(TaskTag::Bug),
            Some(TaskTag::Feature),
            Some(TaskTag::Chore),
            Some(TaskTag::Fix),
        ] {
            let ctx = PromptContext {
                tag,
                ..PromptContext::default()
            };
            let text = build_prompt(TaskId(1), "Task", "Desc", None, None, &ctx);
            assert!(
                text.contains("not the end of the task"),
                "tag {tag:?}: no-plan prompt should instruct implementation to \
follow plan-attach, got: {text}"
            );
        }
    }

    /// The bug this guards: prose that lists "attaching a plan" alongside
    /// "finishing implementation" as equally valid stopping points reads as
    /// permission to stop at a plan for bug/feature/chore/fix tasks (see task
    /// #4188). The rule now has its own line, so this asserts on that line —
    /// and on the fact that the plan path is where it is emitted, since that
    /// is the path #4188 was about.
    #[test]
    fn a_plan_alone_is_never_a_sufficient_stopping_point() {
        let text = plan_not_a_stopping_point_instruction();
        assert!(
            text.contains("not a stopping point"),
            "the line should say attaching a plan alone is not a stopping \
point, got: {text}"
        );
        assert!(
            text.contains("same session"),
            "the line should require implementation in the same session, got: {text}"
        );
    }

    #[test]
    fn learning_instruction_in_task_prompts_no_plan() {
        let text = build_prompt(
            TaskId(1),
            "title",
            "desc",
            None,
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("/learnings"),
            "build_prompt (no plan) should reference /learnings skill"
        );
    }

    #[test]
    fn learning_instruction_in_quick_dispatch_prompt() {
        let text = build_quick_dispatch_prompt(
            TaskId(1),
            "title",
            "desc",
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("/learnings"),
            "quick dispatch prompt should reference /learnings skill"
        );
    }

    #[test]
    fn trailing_block_includes_knowledge_base_nudge() {
        let text = trailing_block(Preceding::PlanWithSpecs);
        assert!(
            text.contains("/learnings"),
            "trailing block should point at the /learnings skill, got: {text}"
        );
    }

    #[test]
    fn research_prompt_includes_knowledge_block_when_learnings_injected() {
        // Regression: build_research_prompt used to silently omit the
        // validated-knowledge block that build_prompt/build_quick_dispatch_prompt
        // both include — research tasks get RAG-injected learnings too, so the
        // block must appear here as well.
        let l = seed(20, LearningScope::Repo, 1);
        let ctx = PromptContext {
            learnings: LearningInjections { ranked: vec![&l] },
            ..PromptContext::default()
        };
        let text = build_research_prompt(
            TaskId(7),
            "Research async runtimes",
            "Compare tokio vs async-std",
            None,
            &ctx,
        );
        assert!(
            text.contains("## Validated knowledge for this task"),
            "research prompt should include the knowledge block when learnings are injected, got: {text}"
        );
        assert!(text.contains("[#20 repo, \u{2191}1]"));
    }

    #[test]
    fn research_prompt_content() {
        let text = build_research_prompt(
            TaskId(7),
            "Research async runtimes",
            "Compare tokio vs async-std",
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("research agent"),
            "research prompt should identify the agent role"
        );
        assert!(
            text.contains("present") || text.contains("findings"),
            "research prompt should instruct presenting findings"
        );
        assert!(
            text.contains("Do NOT make code changes")
                || text.contains("do not make code changes")
                || text.contains("no code changes"),
            "research prompt should prohibit code changes"
        );
    }

    fn seed(id: i64, scope: LearningScope, count: i64) -> Learning {
        use crate::models::{LearningId, LearningStatus};
        use chrono::{TimeZone, Utc};
        Learning {
            id: LearningId(id),
            kind: LearningKind::Pitfall,
            summary: format!("learning {id}"),
            detail: None,
            scope,
            scope_ref: match scope {
                LearningScope::User => None,
                _ => Some("ref".into()),
            },
            tags: vec![],
            status: LearningStatus::Approved,
            source_task_id: None,
            upvote_count: count,
            last_upvoted_at: None,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    #[test]
    fn render_validated_knowledge_block_omits_when_empty() {
        assert_eq!(render_validated_knowledge_block(&[]), String::new());
    }

    #[test]
    fn render_validated_knowledge_block_formats_entries() {
        let l = seed(7, LearningScope::Epic, 3);
        let out = render_validated_knowledge_block(&[&l]);
        assert!(out.contains("## Validated knowledge for this task"));
        assert!(out.contains("[#7 epic, \u{2191}3]"));
        assert!(out.contains("learning 7"));
        assert!(
            out.contains("rate_learning"),
            "validated-knowledge block should instruct rate_learning, got: {out}"
        );
        assert!(
            !out.contains("learning_verdicts"),
            "validated-knowledge block should no longer reference wrap_up verdicts, got: {out}"
        );
    }

    #[test]
    fn build_prompt_default_injections_unchanged() {
        // Regression: when no learnings are injected the prompt must not gain
        // any leading whitespace or knowledge-block headers.
        let text = build_prompt(
            TaskId(1),
            "title",
            "desc",
            None,
            None,
            &PromptContext::default(),
        );
        assert!(text.starts_with("Your task is:"));
        assert!(!text.contains("Validated knowledge for this task"));
    }

    #[test]
    fn build_prompt_with_injections_includes_knowledge_block() {
        let procedural_l = {
            let mut l = seed(10, LearningScope::User, 0);
            l.kind = LearningKind::Procedural;
            l.detail = Some("Always run tests before committing.".into());
            l
        };
        let convention_l = seed(11, LearningScope::Repo, 2);
        let injections = LearningInjections {
            ranked: vec![&procedural_l, &convention_l],
        };
        let ctx = PromptContext {
            learnings: injections,
            ..PromptContext::default()
        };
        let text = build_prompt(TaskId(1), "title", "desc", None, None, &ctx);
        // Procedural learnings no longer appear as a verbatim prefix — prompt
        // always starts with the task block.
        assert!(text.starts_with("Your task is:"));
        assert!(text.contains("## Validated knowledge for this task"));
        // Both learnings appear in the validated-knowledge block.
        assert!(text.contains("[#10 user, \u{2191}0]"));
        assert!(text.contains("[#11 repo, \u{2191}2]"));
    }

    #[test]
    fn build_quick_dispatch_prompt_default_injections_unchanged() {
        let text = build_quick_dispatch_prompt(
            TaskId(1),
            "title",
            "desc",
            None,
            &PromptContext::default(),
        );
        assert!(text.starts_with("You are working interactively with the user."));
        assert!(!text.contains("Validated knowledge for this task"));
    }

    #[test]
    fn build_prompt_with_dependabot_tag_includes_review_section() {
        let ctx = PromptContext {
            tag: Some(TaskTag::Dependabot),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "Bump serde from 1.0.0 to 1.0.1",
            "https://github.com/example/repo/pull/7",
            None,
            None,
            &ctx,
        );

        assert!(text.contains("Dependabot PR review"), "missing role line");
        // The shared steps, which every route reaches.
        assert!(text.contains("gh pr view"));
        assert!(text.contains("gh pr diff"));
        assert!(text.contains("gh pr checks"));
        // No pr_url on this context, so the find-it-yourself step survives.
        assert!(text.contains("update_task(task_id=42, url="));
        assert!(text.contains("url_type=\"pr\""));
        assert!(text.contains("needs_input"));
        // 1.0.0 -> 1.0.1 is a patch, so this prompt takes the merge route and
        // states the verdict rather than asking for it.
        assert!(
            text.contains("Bump: patch — serde 1.0.0 → 1.0.1"),
            "the harness must state the bump, got: {text}"
        );
        assert!(text.contains("gh pr review"));
        assert!(text.contains("--approve"));
        assert!(text.contains("gh pr merge"));
        assert!(text.contains("--squash --auto"));
        // The standard trailing wrap-up instruction must not be present.
        assert!(
            !text.contains("use the /wrap-up skill"),
            "dependabot prompt must omit the standard wrap-up instruction"
        );
        // No TDD / allium — this agent doesn't edit code.
        assert!(
            !text.contains("Always use TDD"),
            "dependabot prompt must omit the TDD instruction"
        );
        // The standard spec-first addendum must be replaced.
        assert!(
            !text.contains("allium:elicit"),
            "dependabot prompt must omit the spec-first design addendum"
        );
    }

    #[test]
    fn build_prompt_without_dependabot_tag_omits_review_section() {
        let text = build_prompt(
            TaskId(1),
            "title",
            "desc",
            None,
            None,
            &PromptContext::default(),
        );
        assert!(!text.contains("Dependabot PR review"));
        assert!(!text.contains("gh pr merge"));
    }

    #[test]
    fn build_prompt_with_pr_review_tag_includes_review_commands() {
        let ctx = PromptContext {
            tag: Some(TaskTag::PrReview),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "Review PR: Add new login flow",
            "https://github.com/example/repo/pull/99",
            None,
            None,
            &ctx,
        );

        assert!(
            text.contains("/code-review"),
            "pr-review prompt must name the review command, got: {text}"
        );
        // The diff-size branch is gone: the command it routes to takes a PR
        // target and an effort level of its own, so measuring the diff was a
        // proxy for a choice that command already makes. See
        // AReviewRunbookCarriesOnlyTheBranchThatApplies in dispatch.allium.
        assert!(
            !text.contains("wc -l"),
            "pr-review prompt must not measure the diff, got: {text}"
        );
        assert!(
            !text.contains("/review-pr"),
            "pr-review prompt must name one review command, not two, got: {text}"
        );
    }

    #[test]
    fn build_prompt_with_pr_review_tag_omits_the_spec_first_design_addendum() {
        let ctx = PromptContext {
            tag: Some(TaskTag::PrReview),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "Review PR: Add new login flow",
            "https://github.com/example/repo/pull/99",
            None,
            None,
            &ctx,
        );

        assert!(
            !text.contains("allium:elicit"),
            "pr-review prompt must NOT contain the spec-first design addendum"
        );
        assert!(
            !text.contains("implementation plan"),
            "pr-review prompt must NOT mention implementation plan"
        );
        assert!(
            !text.contains("docs/plans/"),
            "pr-review prompt must NOT reference docs/plans/"
        );
    }

    #[test]
    fn build_prompt_with_pr_review_tag_omits_tdd_and_allium_instructions() {
        let ctx = PromptContext {
            tag: Some(TaskTag::PrReview),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "Review PR: Add new login flow",
            "https://github.com/example/repo/pull/99",
            None,
            None,
            &ctx,
        );

        assert!(
            !text.contains("Always use TDD"),
            "pr-review prompt must NOT contain TDD instruction"
        );
        assert!(
            !text.contains("Allium specs"),
            "pr-review prompt must NOT contain allium instruction"
        );
    }

    #[test]
    fn build_prompt_with_pr_review_tag_omits_wrap_up() {
        let ctx = PromptContext {
            tag: Some(TaskTag::PrReview),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "Review PR: Add new login flow",
            "https://github.com/example/repo/pull/99",
            None,
            None,
            &ctx,
        );

        assert!(
            text.contains("do not write a plan or change code"),
            "pr-review prompt must still state its role, got: {text}"
        );
        assert!(
            !text.contains("use the /wrap-up skill"),
            "pr-review prompt must omit the standard wrap-up instruction"
        );
    }

    /// The rule moved from the prompt to the tool. `wrap_up` refuses a
    /// review-tagged task outright (`ReviewTasksAreNotWrappedUp` in
    /// `docs/specs/mcp-task-tools.allium`), so asking for it in prose is a
    /// rule stated where it cannot be enforced. Counted at zero rather than
    /// merely "not the old sentence": a reworded prohibition is the same
    /// prompt work coming back under another name.
    ///
    /// What the runbooks keep is the positive half — each terminal names the
    /// end state that makes wrap-up unnecessary, which is what an agent
    /// standing at that terminal actually needs.
    #[test]
    fn review_runbooks_no_longer_forbid_wrap_up() {
        for (label, tag, terminal_states) in [
            (
                "dependabot",
                TaskTag::Dependabot,
                ["auto-cleaned on merge", "wait for the user's reply"],
            ),
            (
                "pr-review",
                TaskTag::PrReview,
                ["wait for the user's instructions", "here to review the PR"],
            ),
        ] {
            let ctx = PromptContext {
                tag: Some(tag),
                ..PromptContext::default()
            };
            let text = build_prompt(
                TaskId(42),
                "Bump serde from 1.0.0 to 1.0.1",
                "https://x/pull/9",
                None,
                None,
                &ctx,
            );
            assert_eq!(
                text.matches("/wrap-up").count(),
                0,
                "{label}: wrap_up refuses a review task itself, so the prompt \
must not ask, got: {text}"
            );
            for state in terminal_states {
                assert!(
                    text.contains(state),
                    "{label}: the terminal branches must still name the end \
state that makes wrap-up unnecessary, missing {state:?}, got: {text}"
                );
            }
        }
    }

    /// The point of classifying in the harness: an agent is handed its own
    /// branch, not a table it has to walk. Each route is checked for what it
    /// must carry AND for the branches it must not — a leftover branch is
    /// exactly the decision table this replaced.
    #[test]
    fn a_dependabot_prompt_carries_only_the_branch_its_bump_takes() {
        struct Route {
            label: &'static str,
            title: &'static str,
            body: &'static str,
            present: &'static [&'static str],
            absent: &'static [&'static str],
        }
        let cases = [
            Route {
                label: "patch",
                title: "#25 Bump dbt-common from 1.37.2 to 1.37.3 in /venvs/dbt",
                body: "",
                present: &["Bump: patch — dbt-common 1.37.2 → 1.37.3", "gh pr merge"],
                absent: &["CHANGELOG", "BREAKING", "gh pr comment", "cannot be routed"],
            },
            Route {
                label: "minor",
                title: "#29 Bump requests from 2.32.4 to 2.33.0 in /venvs/basic",
                body: "",
                present: &[
                    "Bump: minor — requests",
                    "CHANGELOG",
                    "BREAKING",
                    "gh pr merge",
                ],
                absent: &["gh pr comment", "cannot be routed"],
            },
            Route {
                label: "major",
                title: "#47 fix(deps): update dependency deepdiff to v9",
                body: "",
                present: &[
                    "Bump: major — deepdiff → v9",
                    "gh pr comment",
                    "never merge",
                ],
                // The merge terminal is unreachable from here, so the branch
                // that says "never merge a major bump yourself" must not be
                // followed two lines later by the commands to do exactly that.
                absent: &["gh pr merge", "CHANGELOG", "cannot be routed"],
            },
            Route {
                label: "grouped non-major",
                title: "#79 fix(deps): update python (non-major)",
                body: "",
                present: &[
                    "Bump: non-major group — python",
                    "cannot be routed",
                    "several packages",
                ],
                absent: &["gh pr merge", "gh pr comment", "CHANGELOG"],
            },
            Route {
                label: "digest",
                title: "#500 fix(deps): update postgres:18 docker digest to 4ef4dbc",
                body: "| postgres:18 | final | digest | `34f47c4` → `4ef4dbc` |",
                // It says WHY there is nothing to read rather than reusing the
                // unclassified wording — the tag did not move, so no changelog
                // exists that would clear it.
                present: &[
                    "Bump: digest re-pin — postgres:18 34f47c4 → 4ef4dbc",
                    "no changelog",
                ],
                absent: &[
                    "gh pr merge",
                    "gh pr comment",
                    "CHANGELOG",
                    "kind could not be read",
                ],
            },
            Route {
                label: "unclassifiable",
                title: "#3 chore: tidy the release workflow",
                body: "",
                present: &["kind could not be read", "cannot be routed"],
                // An unclassified bump is usually a SINGLE package. Telling it
                // the group reason states something about the PR that is not
                // true, which is exactly what the ask branch must not do.
                absent: &[
                    "gh pr merge",
                    "gh pr comment",
                    "CHANGELOG",
                    "several packages",
                    "grouped",
                ],
            },
        ];

        for case in cases {
            let ctx = PromptContext {
                tag: Some(TaskTag::Dependabot),
                ..PromptContext::default()
            };
            let text = build_prompt(TaskId(42), case.title, case.body, None, None, &ctx);
            let label = case.label;
            for needle in case.present {
                assert!(
                    text.contains(needle),
                    "{label}: missing {needle:?}, got: {text}"
                );
            }
            for needle in case.absent {
                assert!(
                    !text.contains(needle),
                    "{label}: carries {needle:?}, which belongs to another \
branch, got: {text}"
                );
            }
        }
    }

    /// A rendered prompt never ships a `{{KEY}}` the renderer failed to fill.
    /// Asserted across every builder, because the failure is silent: a
    /// mistyped or newly-added placeholder reaches the agent as literal braces,
    /// and only a human reading the prompt would notice.
    #[test]
    fn no_rendered_prompt_carries_an_unsubstituted_placeholder() {
        let with_pr = PromptContext {
            tag: Some(TaskTag::Dependabot),
            pr_url: Some("https://github.com/o/r/pull/7"),
            ..PromptContext::default()
        };
        let variants = [
            build_prompt(TaskId(42), "t", "d", None, None, &PromptContext::default()),
            build_prompt(
                TaskId(42),
                "t",
                "d",
                Some("/p/plan.md"),
                None,
                &PromptContext::default(),
            ),
            build_prompt(
                TaskId(42),
                "Bump foo from 1.0.0 to 2.0.0",
                "d",
                None,
                None,
                &PromptContext {
                    tag: Some(TaskTag::Dependabot),
                    ..PromptContext::default()
                },
            ),
            build_prompt(
                TaskId(42),
                "Bump foo from 1.0.0 to 1.0.1",
                "d",
                None,
                None,
                &with_pr,
            ),
            build_prompt(
                TaskId(42),
                "t",
                "d",
                None,
                None,
                &PromptContext {
                    tag: Some(TaskTag::PrReview),
                    ..PromptContext::default()
                },
            ),
            build_quick_dispatch_prompt(TaskId(42), "t", "d", None, &PromptContext::default()),
            build_research_prompt(TaskId(42), "t", "d", None, &PromptContext::default()),
        ];
        for text in variants {
            assert!(
                !text.contains("{{"),
                "an unsubstituted placeholder reached the agent: {text}"
            );
        }
    }

    /// The PR URL is already on the task, so the runbook states it instead of
    /// asking the agent to dig it out of a description the feed truncated to
    /// 500 characters. `gh` takes a URL wherever it takes a number, and the URL
    /// identifies the repo, so `--repo <owner/repo>` goes with it.
    #[test]
    fn a_recorded_pr_url_replaces_the_extract_it_yourself_step() {
        let ctx = PromptContext {
            tag: Some(TaskTag::Dependabot),
            pr_url: Some("https://github.com/example/repo/pull/42"),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(7),
            "Bump serde from 1.0.0 to 1.0.1",
            "some truncated body",
            None,
            None,
            &ctx,
        );

        assert!(
            text.contains("PR: https://github.com/example/repo/pull/42"),
            "the runbook must state the PR it already has, got: {text}"
        );
        assert!(
            !text.contains("url_type="),
            "nothing left to record, so the update_task(url=…) step must go, got: {text}"
        );
        assert!(
            !text.contains("--repo <owner/repo>"),
            "the URL identifies the repo, got: {text}"
        );
    }

    /// Only a pr-typed url reaches the runbook. A security-alert url handed to
    /// `gh pr view` would fail five times over, so the absence of one has to
    /// leave the find-it-yourself step in place.
    #[test]
    fn without_a_recorded_pr_url_the_runbook_still_asks_the_agent_to_find_it() {
        let ctx = PromptContext {
            tag: Some(TaskTag::Dependabot),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(7),
            "Bump serde from 1.0.0 to 1.0.1",
            "d",
            None,
            None,
            &ctx,
        );
        assert!(
            text.contains("update_task(task_id=7, url=<URL>, url_type=\"pr\")"),
            "got: {text}"
        );
    }

    /// Every route reaches the two guard failures and the ask terminal, so
    /// those are shared rather than branch-local. Asserted separately from
    /// the exclusivity test above so a regression says which half broke.
    #[test]
    fn every_dependabot_route_keeps_the_shared_steps_and_the_ask_terminal() {
        for title in [
            "Bump foo from 1.0.0 to 1.0.1",
            "Bump foo from 1.0.0 to 1.1.0",
            "fix(deps): update dependency foo to v9",
            "fix(deps): update python (non-major)",
            "chore: something else",
        ] {
            let ctx = PromptContext {
                tag: Some(TaskTag::Dependabot),
                ..PromptContext::default()
            };
            let text = build_prompt(TaskId(42), title, "", None, None, &ctx);
            for needle in ["gh pr view", "gh pr checks", "ASK THE USER", "needs_input"] {
                assert!(
                    text.contains(needle),
                    "{title:?}: every route needs {needle:?}, got: {text}"
                );
            }
        }
    }

    /// Everything `TheDepOnlyAllowlistAdmitsOnlyDeclarativeDependencyFiles`
    /// claims is a claim about ONE rendered line, so the assertions below read
    /// that line rather than the whole prompt — a path that appears anywhere
    /// else in the runbook must not pass for an allowlist entry.
    ///
    /// One render serves all three claims because the line is static markdown:
    /// no branch and no bump kind varies it, so a second render under a
    /// different title would assert the same bytes while implying the title
    /// decides which paths are admitted.
    #[test]
    fn the_dep_only_allowlist_admits_declarative_dependency_files_and_nothing_executable() {
        const PREFIX: &str = "Every changed file path must match one of:";
        let ctx = PromptContext {
            tag: Some(TaskTag::Dependabot),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "Bump serde from 1.0.0 to 1.0.1",
            "",
            None,
            None,
            &ctx,
        );
        let line = text
            .lines()
            .find(|line| line.contains(PREFIX))
            .unwrap_or_else(|| panic!("no dep-only allowlist line, got: {text}"));

        // Gradle. A Renovate PR touching only the version catalog is a
        // dependency bump and nothing else, so it must clear the guard rather
        // than escalate. Both files are declarative, which is what earns them
        // the place.
        for needle in ["gradle/libs.versions.toml", "gradle.properties"] {
            assert!(
                line.contains(needle),
                "the allowlist must admit {needle:?}, got: {line}"
            );
        }

        // The guard never reads the diff, so a path match is its whole
        // evidence. A build script's path says nothing about what the diff
        // did, so admitting one would let the agent auto-merge arbitrary build
        // logic unseen.
        for needle in [
            "build.gradle",
            "gradlew",
            "gradle/wrapper",
            "project/plugins.sbt",
            "project/build.properties",
        ] {
            assert!(
                !line.contains(needle),
                "{needle:?} carries executable build logic and must stay off the allowlist, \
got: {line}"
            );
        }
        assert!(
            line.contains(".github/workflows/*"),
            "the one stated exception must survive, got: {line}"
        );

        // Adding an ecosystem must not drop one. The allowlist is a single
        // rendered line, so an edit to it can silently lose an entry that
        // nothing else asserts.
        for needle in [
            "Cargo.toml",
            "Cargo.lock",
            "package.json",
            "package-lock.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "requirements*.txt",
            "pyproject.toml",
            "uv.lock",
            "go.mod",
            "go.sum",
            "Gemfile",
            "Gemfile.lock",
            "composer.json",
            "composer.lock",
        ] {
            assert!(
                line.contains(needle),
                "the allowlist must still admit {needle:?}, got: {line}"
            );
        }
    }

    #[test]
    fn build_prompt_with_pr_review_tag_includes_the_learning_instruction() {
        let ctx = PromptContext {
            tag: Some(TaskTag::PrReview),
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "Review PR: Add new login flow",
            "https://github.com/example/repo/pull/99",
            None,
            None,
            &ctx,
        );

        assert!(
            text.contains("/learnings"),
            "pr-review prompt must include the knowledge-base line"
        );
    }

    /// No prompt variant may render a "## Verification" section — see the
    /// unified prompt skeleton in `docs/specs/dispatch.allium`.
    ///
    /// The builders take no verify input, so this can only regress through
    /// hardcoded prompt copy. That is exactly the case the snapshots don't
    /// catch: `INSTA_UPDATE=always` silently accepts a reintroduced section,
    /// whereas a named test cannot be blanket-accepted.
    #[test]
    fn no_prompt_variant_renders_a_verification_section() {
        let ctx = PromptContext::default();
        let variants = [
            (
                "dispatch",
                build_prompt(TaskId(1), "t", "d", None, None, &ctx),
            ),
            (
                "quick dispatch",
                build_quick_dispatch_prompt(TaskId(1), "t", "d", None, &ctx),
            ),
            (
                "research",
                build_research_prompt(TaskId(1), "t", "d", None, &ctx),
            ),
        ];
        for (variant, text) in variants {
            assert!(
                !text.contains("## Verification"),
                "{variant} prompt must not render a verification section"
            );
            assert!(
                !text.contains("Before declaring work complete"),
                "{variant} prompt must not carry the verification instruction"
            );
        }
    }

    // -- The author check is omitted only where a feed did the filtering --
    // (task #4728; see AReviewRunbookCarriesOnlyTheBranchThatApplies)

    const AUTHOR_CLAIM: &str = "Do not re-check the PR author";

    #[test]
    fn a_feed_created_dependabot_prompt_omits_the_author_check() {
        let ctx = PromptContext {
            tag: Some(TaskTag::Dependabot),
            pr_url: Some("https://github.com/o/r/pull/42"),
            from_feed: true,
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "#42 Bump serde from 1.0.0 to 1.0.1",
            "",
            None,
            None,
            &ctx,
        );
        assert!(
            text.contains(AUTHOR_CLAIM),
            "a feed listed this PR by bot author, so the check can only agree: {text}"
        );
    }

    /// No feed created this task, so no author filter ever ran. The sentence
    /// asserts a fact, and asserting it here would be asserting a falsehood —
    /// the agent checks the author itself instead.
    #[test]
    fn a_hand_created_dependabot_prompt_makes_no_claim_about_a_filter() {
        let ctx = PromptContext {
            tag: Some(TaskTag::Dependabot),
            from_feed: false,
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "#42 Bump serde from 1.0.0 to 1.0.1",
            "",
            None,
            None,
            &ctx,
        );
        assert!(
            !text.contains(AUTHOR_CLAIM),
            "a task no feed created must not be told a feed filtered it: {text}"
        );
        assert!(
            !text.contains("already passed that filter"),
            "no half of the claim may survive: {text}"
        );
        // The rest of step 1 must be intact — only the one bullet goes.
        assert!(
            text.contains("Verify the PR touches only dependency files"),
            "dropping the bullet must not drop its step: {text}"
        );
        assert!(
            text.contains("Check CI"),
            "dropping the bullet must not disturb the step after it: {text}"
        );
    }

    /// Both halves are required. A feed created this task, but nothing on it
    /// names a PR — so there is no PR whose author a filter could have vetted,
    /// and the claim is not rendered. Reachable two ways: a task from a feed
    /// that is not a PR feed (the CVE feed sets external_id too) and was later
    /// retagged `dependabot` over MCP, and a feed-created review task whose
    /// url was cleared or retyped afterwards.
    #[test]
    fn a_feed_created_task_that_names_no_pr_makes_no_claim_about_a_filter() {
        let ctx = PromptContext {
            tag: Some(TaskTag::Dependabot),
            pr_url: None,
            from_feed: true,
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "#42 Bump serde from 1.0.0 to 1.0.1",
            "",
            None,
            None,
            &ctx,
        );
        assert!(
            !text.contains(AUTHOR_CLAIM),
            "with no PR recorded there is no filtered author to stand on: {text}"
        );
    }

    /// Provenance is read from external_id, not from having a PR url — a
    /// hand-created task can carry one, since update_task takes a url and the
    /// dependabot tag together.
    #[test]
    fn a_pr_url_alone_does_not_make_a_task_feed_created() {
        let ctx = PromptContext {
            tag: Some(TaskTag::Dependabot),
            pr_url: Some("https://github.com/o/r/pull/42"),
            from_feed: false,
            ..PromptContext::default()
        };
        let text = build_prompt(
            TaskId(42),
            "#42 Bump serde from 1.0.0 to 1.0.1",
            "",
            None,
            None,
            &ctx,
        );
        assert!(
            text.contains("PR: https://github.com/o/r/pull/42"),
            "the url is still rendered — it is recorded, whoever recorded it: {text}"
        );
        assert!(
            !text.contains(AUTHOR_CLAIM),
            "but a recorded url is not evidence a feed filtered the author: {text}"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod rag_dispatch_tests {
    use std::sync::Arc;

    use crate::db::{
        CreateLearningRow, CreateTaskRequest, Database, LearningRetrievalStore, LearningStore,
        TaskCrud, TaskRead,
    };
    use crate::models::{LearningKind, LearningScope, TaskStatus};
    use crate::service::embeddings::{serialize_embedding, EmbeddingService};

    use super::{
        build_and_record_injections, list_learnings_for_dispatch_rag, DISPATCH_INJECTION_CAP,
    };

    // The test EmbeddingService returns vec![0.1f32; 384]. Use the same dimensionality
    // for stored embeddings so cosine similarity is computed correctly.
    fn fake_emb_bytes() -> Vec<u8> {
        serialize_embedding(&vec![0.1f32; 384])
    }

    async fn seed_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().await.unwrap())
    }

    async fn make_task(db: &Arc<Database>) -> crate::models::Task {
        let id = db
            .create_task(CreateTaskRequest {
                title: "test task",
                description: "test description",
                repo_path: "/repo/test",
                plan: None,
                status: TaskStatus::Backlog,
                base_branch: "main",
                epic_id: None,
                sort_order: None,
                tag: None,
                wrap_up_mode: None,
                auto_run_plan: false,
                phoenix: false,
            })
            .await
            .unwrap();
        db.get_task(id).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn dispatch_injection_includes_procedural_learnings_without_prioritizing_them() {
        let db = seed_db().await;
        let task = make_task(&db).await;
        let emb = fake_emb_bytes();

        let proc_id = db
            .create_learning(CreateLearningRow {
                kind: LearningKind::Procedural,
                summary: "always run clippy",
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: &[],
                source_task_id: None,
                embedding: Some(&emb),
            })
            .await
            .unwrap();

        for i in 0..2 {
            db.create_learning(CreateLearningRow {
                kind: LearningKind::Convention,
                summary: &format!("convention {i}"),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: &[],
                source_task_id: None,
                embedding: Some(&emb),
            })
            .await
            .unwrap();
        }

        let emb_svc = EmbeddingService::new_test();
        // threshold=0.0 so all candidates pass the cosine filter
        let results = list_learnings_for_dispatch_rag(&*db, &task, &emb_svc, 0.0).await;

        assert!(!results.is_empty(), "should return at least one learning");
        // Procedural learnings are still included — just not artificially first.
        let ids: Vec<_> = results.iter().map(|l| l.id).collect();
        assert!(
            ids.contains(&proc_id),
            "procedural learning must be in results"
        );
    }

    #[tokio::test]
    async fn dispatch_injection_excludes_task_scoped_learnings() {
        let db = seed_db().await;
        let task = make_task(&db).await;
        let emb = fake_emb_bytes();

        // Task-scoped learning — should be excluded by list_all_approved_non_task_learnings
        db.create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "task-scoped learning",
            detail: None,
            scope: LearningScope::Task,
            scope_ref: Some(&task.id.0.to_string()),
            tags: &[],
            source_task_id: Some(task.id),
            embedding: Some(&emb),
        })
        .await
        .unwrap();

        let emb_svc = EmbeddingService::new_test();
        let results = list_learnings_for_dispatch_rag(&*db, &task, &emb_svc, 0.0).await;

        assert!(
            results.iter().all(|l| l.scope != LearningScope::Task),
            "task-scoped learnings must not appear in dispatch injection"
        );
    }

    #[tokio::test]
    async fn dispatch_injection_respects_cap_of_5() {
        let db = seed_db().await;
        let task = make_task(&db).await;
        let emb = fake_emb_bytes();

        // Seed 8 approved non-task learnings with embeddings
        for i in 0..8 {
            db.create_learning(CreateLearningRow {
                kind: LearningKind::Convention,
                summary: &format!("convention {i}"),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: &[],
                source_task_id: None,
                embedding: Some(&emb),
            })
            .await
            .unwrap();
        }

        let emb_svc = EmbeddingService::new_test();
        let results = list_learnings_for_dispatch_rag(&*db, &task, &emb_svc, 0.0).await;

        assert_eq!(
            results.len(),
            DISPATCH_INJECTION_CAP,
            "should return at most DISPATCH_INJECTION_CAP ({DISPATCH_INJECTION_CAP}) learnings"
        );
    }

    #[tokio::test]
    async fn dispatch_injection_excludes_learnings_without_embeddings() {
        let db = seed_db().await;
        let task = make_task(&db).await;
        let emb = fake_emb_bytes();

        // One learning with embedding, one without
        let with_emb_id = db
            .create_learning(CreateLearningRow {
                kind: LearningKind::Convention,
                summary: "has embedding",
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: &[],
                source_task_id: None,
                embedding: Some(&emb),
            })
            .await
            .unwrap();

        let no_emb_id = db
            .create_learning(CreateLearningRow {
                kind: LearningKind::Convention,
                summary: "no embedding",
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: &[],
                source_task_id: None,
                embedding: None,
            })
            .await
            .unwrap();

        let emb_svc = EmbeddingService::new_test();
        let results = list_learnings_for_dispatch_rag(&*db, &task, &emb_svc, 0.0).await;

        assert!(
            results.iter().any(|l| l.id == with_emb_id),
            "learning with embedding should be included"
        );
        assert!(
            results.iter().all(|l| l.id != no_emb_id),
            "learning without embedding should be excluded"
        );
    }

    #[tokio::test]
    async fn build_and_record_injections_records_all_as_prompt_injection() {
        let db = seed_db().await;
        let task = make_task(&db).await;
        let emb = fake_emb_bytes();

        let proc_id = db
            .create_learning(CreateLearningRow {
                kind: LearningKind::Procedural,
                summary: "always run tests",
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: &[],
                source_task_id: None,
                embedding: Some(&emb),
            })
            .await
            .unwrap();

        let conv_id = db
            .create_learning(CreateLearningRow {
                kind: LearningKind::Convention,
                summary: "use Arc for shared state",
                detail: None,
                scope: LearningScope::Repo,
                scope_ref: Some("/repo/test"),
                tags: &[],
                source_task_id: None,
                embedding: Some(&emb),
            })
            .await
            .unwrap();

        let emb_svc = EmbeddingService::new_test();
        let injected = build_and_record_injections(&*db, &task, &emb_svc).await;

        assert_eq!(injected.len(), 2);
        let ids: Vec<_> = injected.iter().map(|l| l.id).collect();
        assert!(ids.contains(&proc_id));
        assert!(ids.contains(&conv_id));

        // All retrievals recorded as PromptInjection regardless of kind.
        let rows = db.list_retrievals_for_task(task.id).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|r| matches!(r.source, crate::models::RetrievalSource::PromptInjection)));
    }
}
