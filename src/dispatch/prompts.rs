use crate::db;
use crate::models::{EpicId, Learning, LearningKind, LearningScope, ProjectId, Task, TaskId};

/// Plugin dir flag added to all Claude agent invocations so dispatched agents
/// discover the dispatch plugin's skills and commands (e.g. /wrap-up).
pub(super) const DISPATCH_PLUGIN_DIR: &str = "--plugin-dir ~/.claude/plugins/local/dispatch";

/// Epic context passed to prompt builders so agents know about their epic.
pub struct EpicContext {
    pub epic_id: EpicId,
    pub epic_title: String,
}

impl EpicContext {
    /// Build epic context from the database for a task that belongs to an epic.
    pub async fn from_db(task: &Task, db: &dyn db::TaskStore) -> Option<Self> {
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
            To ask questions or send updates to sibling agents, use send_message with their task ID.",
            self.epic_id, self.epic_title, self.epic_id
        )
    }
}

/// Project context passed to prompt builders so agents know which project to
/// assign sub-tasks to. The MCP `create_task` tool requires `project_id`; the
/// agent reads it from the `ProjectId:` line in the task block.
pub struct ProjectContext {
    pub project_id: ProjectId,
    pub project_name: String,
}

impl ProjectContext {
    /// Build project context for a task. Looks up the task's project_id; falls
    /// back to a synthetic name if the project record is missing (should not
    /// happen in practice — every task is FK-bound to a real project).
    pub async fn from_db(task: &Task, db: &dyn db::TaskStore) -> Self {
        let lookup = db
            .list_projects()
            .await
            .ok()
            .and_then(|projects| projects.into_iter().find(|p| p.id == task.project_id));
        match lookup {
            Some(p) => ProjectContext {
                project_id: p.id,
                project_name: p.name,
            },
            None => ProjectContext {
                project_id: task.project_id,
                project_name: format!("project #{}", task.project_id),
            },
        }
    }

    pub(super) fn prompt_section(&self) -> String {
        format!(
            "\n\nThis task is in project #{}: {}.\n\
            When creating sub-tasks via the create_task MCP tool, pass project_id={} \
            so the new task lands in the same project as this one.",
            self.project_id, self.project_name, self.project_id
        )
    }
}

pub(super) fn build_tmux_window_name(task_id: TaskId) -> String {
    format!("task-{task_id}")
}

pub(super) fn rebase_preamble(target: &str) -> String {
    format!(
        "Before starting work, rebase your branch from {target}:\n\
         ```\n\
         git rebase {target}\n\
         ```"
    )
}

/// Returns `(epic_id_line, epic_section)` for embedding in agent prompts.
pub(super) fn epic_preamble(epic: Option<&EpicContext>) -> (String, String) {
    let id_line = epic.map_or(String::new(), |e| format!("\n  EpicId: {}", e.epic_id));
    let section = epic.map_or(String::new(), |e| e.prompt_section());
    (id_line, section)
}

/// Returns `(project_id_line, project_section)` for embedding in agent prompts.
pub(super) fn project_preamble(project: Option<&ProjectContext>) -> (String, String) {
    let id_line = project.map_or(String::new(), |p| {
        format!("\n  ProjectId: {}", p.project_id)
    });
    let section = project.map_or(String::new(), |p| p.prompt_section());
    (id_line, section)
}

/// Standard task identification block shared by all task agent prompts.
pub(super) fn task_block(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
) -> String {
    let (epic_id_line, epic_section) = epic_preamble(epic);
    let (project_id_line, project_section) = project_preamble(project);
    format!(
        "Task:\n  ID: {task_id}\n  Title: {title}\n  Description: {description}\
         {project_id_line}{epic_id_line}{epic_section}{project_section}"
    )
}

/// TDD instruction line, shared across all agents.
pub(super) fn tdd_instruction() -> &'static str {
    "Always use TDD: express intended behaviour as tests first, then implement the minimum code to make them pass."
}

/// MCP tools availability notice, shared across all task agents.
pub(super) fn mcp_tools_instruction() -> &'static str {
    "The dispatch MCP tools are available — use them to query and update this task (get_task, update_task)."
}

/// Knowledge base skill checkpoints for dispatched agents.
pub(super) fn learning_tools_instruction() -> &'static str {
    "Knowledge base: Invoke domain-specific skills at action checkpoints — \
not just at task start:\n\
- Exploring unfamiliar code → `/codebase-knowledge`\n\
- Before writing code → `/code-conventions`\n\
- Before writing tests → `/test-conventions`\n\
- Before creating/updating a PR → `/pr-workflow`\n\
- When using dispatch MCP tools → `/dispatch-workflow`\n\
- When hitting a build or test failure → `/troubleshoot`\n\
- When noticing an improvement opportunity (and before wrapping up) → `/improvement`\n\
- Before wrapping up → `/lint`\n\
Use `/learnings` to record new entries or upvote entries that proved useful."
}

/// Instructions for writing a plan and attaching it to the task via MCP.
pub(super) fn plan_and_attach_instruction() -> &'static str {
    "Use /brainstorming to design the solution, then save the plan to docs/plans/ \
and call update_task to attach it."
}

/// Dispatch instruction for no-plan tasks: conditionally suggests brainstorming
/// based on agent judgment of task description clarity.
pub(super) fn plan_or_brainstorm_instruction() -> &'static str {
    "Use /brainstorming to design the solution if the task description is vague or \
underspecified. Otherwise write an implementation plan directly, save it to docs/plans/ \
and call update_task to attach it."
}

/// Wrap-up instruction shared by every dispatched task agent. Wording is
/// intentionally universal — the same line covers attaching a plan,
/// creating work packages on an epic, and finishing implementation.
pub(super) fn wrap_up_instruction() -> &'static str {
    "When your work is done — attaching a plan, creating work packages, \
or finishing implementation — use the /wrap-up skill to commit any \
remaining changes and finalise the task."
}

/// Allium spec instruction — shared across all agents that may touch domain behaviour.
pub(super) fn allium_instruction() -> &'static str {
    "The Allium specs in `docs/specs/` are the source of truth for domain logic. \
Consult them before changing core behaviour. If your implementation changes domain behaviour, \
update the spec using the `allium:tend` skill and verify alignment with `allium:weed`."
}

/// Trailing metadata shared by every dispatched task agent prompt:
/// `tdd + allium + mcp + learning + wrap_up`, separated by blank lines.
/// Each `format!` in a builder ends with `{trailing}` where this helper plugs in.
pub(super) fn trailing_block() -> String {
    format!(
        "{tdd}\n\
\n\
{allium}\n\
\n\
{mcp}\n\
\n\
{learning}\n\
\n\
{wrap_up}",
        tdd = tdd_instruction(),
        allium = allium_instruction(),
        mcp = mcp_tools_instruction(),
        learning = learning_tools_instruction(),
        wrap_up = wrap_up_instruction(),
    )
}

/// Render procedural-kind learnings as verbatim prompt-prefix instructions.
/// `procedurals` is expected to already be ordered by scope priority then
/// `upvote_count DESC`.
pub(super) fn render_procedural_prefix(procedurals: &[&Learning]) -> String {
    if procedurals.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for l in procedurals {
        let body = l.detail.as_deref().unwrap_or(l.summary.as_str());
        out.push_str(body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// Render the optional repo-map block injected between the validated-knowledge
/// section and the mode-specific addendum. Returns an empty string when the
/// map is `None` or empty, so prompts are byte-identical when generation is
/// disabled, fails, or yields no symbols.
///
/// See [`crate::dispatch::repo_map`] and `AugmentDispatchPromptWithRepoMap` in
/// `docs/specs/tasks.allium`.
pub(super) fn render_repo_map(map: Option<&str>) -> String {
    let body = map.unwrap_or("").trim();
    if body.is_empty() {
        return String::new();
    }
    format!(
        "## Repo map\n\n\
The following structural summary was generated by ctags over the worktree. \
Use it to orient before reading files; it is a best-effort starting point, \
not a source of truth.\n\n\
```\n\
{body}\n\
```\n\n"
    )
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
The following knowledge has been validated by previous agents. Apply it where relevant; \
return a verdict for each entry at wrap-up via the `learning_verdicts` argument of `wrap_up`.\n\n",
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

pub(super) fn build_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    plan: Option<&str>,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
    ctx: &PromptContext<'_>,
) -> String {
    let block = task_block(task_id, title, description, epic, project);
    let addendum = match plan {
        None => plan_or_brainstorm_instruction().to_string(),
        Some(path) => format!(
            "Plan: {path}\n\
Read this file for the full implementation plan.\n\
\n\
Review the plan carefully. Summarise your intended approach in 3–5 bullet points, \
then ask: 'Shall I proceed with implementation?' Wait for confirmation before \
making any changes."
        ),
    };
    let proc_prefix = render_procedural_prefix(&ctx.learnings.procedural);
    let knowledge = render_validated_knowledge_block(&ctx.learnings.tiered);
    let repo_map = render_repo_map(ctx.repo_map.as_deref());

    format!(
        "{proc_prefix}Your task is:\n\
{block}\n\
\n\
{knowledge}{repo_map}{addendum}\n\
\n\
{trailing}",
        trailing = trailing_block(),
    )
}

pub(super) fn build_quick_dispatch_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
    ctx: &PromptContext<'_>,
) -> String {
    let block = task_block(task_id, title, description, epic, project);
    let addendum = format!(
        "This is a quick-dispatched task with a placeholder title. Start by asking the user \
what they want to achieve. Once you understand the goal, call `update_task` with a \
descriptive `title` (and optionally `description`) to rename the task on the kanban board.\n\
\n\
Then write a focused plan before making any changes:\n\
\n\
{attach}",
        attach = plan_and_attach_instruction(),
    );
    let proc_prefix = render_procedural_prefix(&ctx.learnings.procedural);
    let knowledge = render_validated_knowledge_block(&ctx.learnings.tiered);
    let repo_map = render_repo_map(ctx.repo_map.as_deref());

    format!(
        "{proc_prefix}You are working interactively with the user.\n\
\n\
{block}\n\
\n\
{knowledge}{repo_map}{addendum}\n\
\n\
{trailing}",
        trailing = trailing_block(),
    )
}

pub(super) fn build_pr_review_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
    ctx: &PromptContext<'_>,
) -> String {
    let block = task_block(task_id, title, description, epic, project);
    let repo_map = render_repo_map(ctx.repo_map.as_deref());

    format!(
        "You are a PR reviewer.\n\
\n\
{block}\n\
\n\
{repo_map}1. Extract the PR URL or number from the task description.\n\
2. Run `gh pr diff <number> | wc -l` to assess the diff size.\n\
3. Run `/anthropic-review-pr:review-pr <number>` to perform a comprehensive review. \
This skill orchestrates security, code-quality, test-coverage, performance, and documentation \
sub-reviewers. The number of sub-reviewers launched scales with the diff size.\n\
4. When the review is complete, call wrap_up to finish this task.\n\
\n\
Do NOT make code changes. Your job is to review, not to implement.\n\
\n\
{mcp}",
        block = block,
        mcp = mcp_tools_instruction(),
    )
}

pub(super) fn build_research_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
    ctx: &PromptContext<'_>,
) -> String {
    let block = task_block(task_id, title, description, epic, project);
    let repo_map = render_repo_map(ctx.repo_map.as_deref());

    format!(
        "You are a research agent.\n\
\n\
{block}\n\
\n\
{repo_map}Investigate the topic described above. You may read the codebase, documentation, and \
external resources.\n\
\n\
When you have gathered sufficient information, present your findings clearly to the user \
and wait for further instructions. Do NOT wrap up autonomously — that is for the user to \
decide.\n\
\n\
Do NOT make code changes.\n\
\n\
{mcp}",
        block = block,
        mcp = mcp_tools_instruction(),
    )
}

pub(super) fn build_fix_task_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
    ctx: &PromptContext<'_>,
) -> String {
    let block = task_block(task_id, title, description, epic, project);
    let repo_map = render_repo_map(ctx.repo_map.as_deref());

    format!(
        "You are a security fix agent.\n\
\n\
{block}\n\
\n\
{repo_map}Research the CVE or vulnerability described above, then apply a minimal targeted fix.\n\
\n\
TDD approach:\n\
  1. Write a failing test that reproduces the vulnerability (if feasible)\n\
  2. Apply the minimal fix to make the test pass\n\
  3. Run the full test suite to verify nothing else breaks\n\
  4. Commit and open a PR: gh pr create\n\
\n\
Focus on the smallest safe change. Avoid broad refactors.\n\
\n\
{tdd}\n\
\n\
{allium}\n\
\n\
{learning}\n\
\n\
{mcp}\n\
\n\
{wrap_up}",
        block = block,
        tdd = tdd_instruction(),
        allium = allium_instruction(),
        learning = learning_tools_instruction(),
        mcp = mcp_tools_instruction(),
        wrap_up = wrap_up_instruction(),
    )
}

pub(super) fn build_epic_planning_prompt(
    task_id: TaskId,
    task_title: &str,
    task_description: &str,
    epic: &EpicContext,
    project: &ProjectContext,
    ctx: &PromptContext<'_>,
) -> String {
    let block = task_block(
        task_id,
        task_title,
        task_description,
        Some(epic),
        Some(project),
    );
    let repo_map = render_repo_map(ctx.repo_map.as_deref());
    let addendum = format!(
        "Your goal is to explore the codebase, write an implementation plan, and break \
it into work packages on the kanban board.\n\
\n\
Steps:\n\
1. Explore the codebase to understand what needs to change.\n\
2. Use /brainstorming to write the plan. When done, attach it to the epic by calling \
`update_epic` with `epic_id={epic_id}` and `plan=<absolute path to plan file>`.\n\
3. Create work packages from the plan using `create_task`. Work packages are kanban \
tasks — do not confuse them with subtasks inside the plan document itself:\n\
   - Set `epic_id={epic_id}` on every work package\n\
   - Set `project_id={project_id}` on every work package\n\
   - Use `sort_order` to control execution order (1, 2, 3, \u{2026})\n\
   - Work packages at the same sort_order in different repositories run in parallel\n\
   - Work packages in the same repository must have different sort_order values\n\
   - Set `repo_path` to the absolute path of the repository each work package targets\n\
\n\
After creating the work packages, confirm with the user before doing anything further.\n\
\n\
IMPORTANT: Do NOT start implementing. Your job ends after creating the work packages.",
        epic_id = epic.epic_id,
        project_id = project.project_id,
    );

    format!(
        "You are planning an epic.\n\
\n\
{block}\n\
\n\
{repo_map}{addendum}\n\
\n\
{trailing}",
        trailing = trailing_block(),
    )
}

const TIER_ORDER: [LearningScope; 4] = [
    LearningScope::Epic,
    LearningScope::Repo,
    LearningScope::Project,
    LearningScope::User,
];
const PER_TIER: usize = 2;
/// Maximum total learnings injected into a dispatch prompt across all scope tiers.
pub const INJECTION_CAP: usize = 8;

/// Push-injection groups for a dispatch prompt. Both lists hold borrowed
/// `Learning`s so the caller keeps ownership of the active list returned by
/// `LearningStore::list_learnings_for_dispatch`.
#[derive(Default)]
pub struct LearningInjections<'a> {
    /// Procedural-kind learnings, rendered verbatim near the top of the prompt.
    /// Expected to be ordered by scope priority then `upvote_count DESC`.
    pub procedural: Vec<&'a Learning>,
    /// Non-procedural learnings selected by `select_tiered_learnings`.
    pub tiered: Vec<&'a Learning>,
}

/// Bundle of all push-injected context for a dispatch prompt. Threaded through
/// every `build_*_prompt` so individual builders never grow more positional
/// parameters when a new context source lands.
#[derive(Default)]
pub struct PromptContext<'a> {
    pub learnings: LearningInjections<'a>,
    pub repo_map: Option<String>,
}

impl<'a> PromptContext<'a> {
    pub fn with_map(learnings: LearningInjections<'a>, repo_map: Option<String>) -> Self {
        Self {
            learnings,
            repo_map,
        }
    }
}

pub async fn build_and_record_injections(
    db: &dyn crate::db::TaskStore,
    task: &crate::models::Task,
) -> (Vec<Learning>, Vec<Learning>) {
    use crate::models::RetrievalSource;
    let active = db
        .list_learnings_for_dispatch(Some(task.project_id), &task.repo_path, task.epic_id)
        .await
        .unwrap_or_default();
    let (procedural, non_procedural): (Vec<_>, Vec<_>) = active
        .into_iter()
        .partition(|l| l.kind == LearningKind::Procedural);
    let tiered: Vec<Learning> = select_tiered_learnings(&non_procedural, INJECTION_CAP)
        .into_iter()
        .cloned()
        .collect();
    let pairs = procedural
        .iter()
        .map(|l| (l.id, RetrievalSource::Procedural))
        .chain(
            tiered
                .iter()
                .map(|l| (l.id, RetrievalSource::PromptInjection)),
        );
    for (lid, source) in pairs {
        if let Err(e) = db.record_retrieval(task.id, lid, source).await {
            tracing::warn!(
                task_id = task.id.0,
                learning_id = lid.0,
                error = ?e,
                "failed to record learning retrieval"
            );
        }
    }
    (procedural, tiered)
}

pub(crate) fn select_tiered_learnings(learnings: &[Learning], cap: usize) -> Vec<&Learning> {
    use std::collections::HashSet;
    let mut picked: Vec<&Learning> = Vec::new();
    let mut seen: HashSet<i64> = HashSet::new();
    for tier in TIER_ORDER {
        let mut tier_items: Vec<&Learning> = learnings
            .iter()
            .filter(|l| l.scope == tier && l.kind != LearningKind::Procedural)
            .collect();
        tier_items.sort_by(|a, b| {
            b.upvote_count
                .cmp(&a.upvote_count)
                .then(b.updated_at.cmp(&a.updated_at))
        });
        for l in tier_items.into_iter().take(PER_TIER) {
            if picked.len() >= cap {
                return picked;
            }
            if seen.insert(l.id.0) {
                picked.push(l);
            }
        }
    }
    picked
}

#[cfg(test)]
mod tiered_selection_tests {
    use super::*;
    use crate::models::{Learning, LearningId, LearningKind, LearningScope, LearningStatus};
    use chrono::{TimeZone, Utc};

    fn seed(id: i64, scope: LearningScope, count: i64) -> Learning {
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

    fn seed_kind(id: i64, scope: LearningScope, kind: LearningKind) -> Learning {
        let mut l = seed(id, scope, 0);
        l.kind = kind;
        l
    }

    #[test]
    fn picks_top_two_per_scope_in_tier_order() {
        let learnings = vec![
            seed(1, LearningScope::Epic, 5),
            seed(2, LearningScope::Epic, 4),
            seed(3, LearningScope::Epic, 3),
            seed(4, LearningScope::Repo, 2),
            seed(5, LearningScope::Repo, 1),
            seed(6, LearningScope::Repo, 0),
            seed(7, LearningScope::User, 0),
        ];
        let picked = select_tiered_learnings(&learnings, 8);
        let ids: Vec<i64> = picked.iter().map(|l| l.id.0).collect();
        assert_eq!(ids, vec![1, 2, 4, 5, 7]);
    }

    #[test]
    fn caps_at_eight() {
        let mut learnings = vec![];
        let mut id = 1;
        for s in [
            LearningScope::Epic,
            LearningScope::Repo,
            LearningScope::Project,
            LearningScope::User,
        ] {
            for i in 0..3 {
                learnings.push(seed(id, s, i));
                id += 1;
            }
        }
        // 4 tiers * 3 entries = 12, take 2 each = 8, cap = 8.
        assert_eq!(select_tiered_learnings(&learnings, 8).len(), 8);
    }

    #[test]
    fn skips_procedural_entries() {
        let learnings = vec![
            seed_kind(1, LearningScope::Epic, LearningKind::Procedural),
            seed(2, LearningScope::Epic, 0),
        ];
        let picked = select_tiered_learnings(&learnings, 8);
        let ids: Vec<i64> = picked.iter().map(|l| l.id.0).collect();
        assert_eq!(ids, vec![2]);
    }

    #[test]
    fn dedups_by_id_across_tiers() {
        // Same id appearing twice in input (shouldn't happen in practice,
        // but if it did the function must not duplicate).
        let learnings = vec![
            seed(1, LearningScope::Epic, 5),
            seed(1, LearningScope::Repo, 5),
        ];
        let picked = select_tiered_learnings(&learnings, 8);
        let ids: Vec<i64> = picked.iter().map(|l| l.id.0).collect();
        assert_eq!(ids, vec![1]);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(select_tiered_learnings(&[], 8).is_empty());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn learning_instruction_references_learnings_skill() {
        let text = learning_tools_instruction();
        assert!(
            text.contains("/learnings"),
            "learning instruction should reference the /learnings skill, got: {text}"
        );
    }

    #[test]
    fn learning_instruction_mentions_checkpoints() {
        let text = learning_tools_instruction();
        assert!(
            text.contains("checkpoint") || text.contains("action"),
            "learning instruction should describe action checkpoints, got: {text}"
        );
    }

    #[test]
    fn learning_instruction_in_task_prompts_with_plan() {
        let text = build_prompt(
            TaskId(1),
            "title",
            "desc",
            Some("/path/to/plan.md"),
            None,
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("/learnings"),
            "build_prompt (with plan) should reference /learnings skill"
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
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("/learnings"),
            "quick dispatch prompt should reference /learnings skill"
        );
    }

    #[test]
    fn knowledge_base_checkpoints_include_all_skills() {
        let text = trailing_block();
        for skill in [
            "/pr-workflow",
            "/troubleshoot",
            "/improvement",
            "/codebase-knowledge",
        ] {
            assert!(text.contains(skill), "trailing block missing {skill}");
        }
    }

    #[test]
    fn pr_review_prompt_content() {
        let text = build_pr_review_prompt(
            TaskId(42),
            "Review my PR",
            "https://github.com/foo/bar/pull/99",
            None,
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("PR reviewer"),
            "pr_review prompt should identify the agent role"
        );
        assert!(
            text.contains("review-pr"),
            "pr_review prompt should reference the review-pr skill"
        );
        assert!(
            text.contains("wrap-up") || text.contains("wrap_up"),
            "pr_review prompt should instruct wrap up"
        );
        assert!(
            text.contains("Do NOT make code changes")
                || text.contains("do not make code changes")
                || text.contains("no code changes"),
            "pr_review prompt should prohibit code changes"
        );
    }

    #[test]
    fn research_prompt_content() {
        let text = build_research_prompt(
            TaskId(7),
            "Research async runtimes",
            "Compare tokio vs async-std",
            None,
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

    #[test]
    fn fix_task_prompt_content() {
        let text = build_fix_task_prompt(
            TaskId(5),
            "Fix CVE-2024-1234",
            "Heap overflow in serde",
            None,
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("security fix") || text.contains("fix agent"),
            "fix_task prompt should identify the agent role"
        );
        assert!(
            text.contains("minimal"),
            "fix_task prompt should emphasise minimal fix"
        );
        assert!(
            text.contains("gh pr create"),
            "fix_task prompt should instruct creating a PR"
        );
        assert!(
            text.contains("wrap-up") || text.contains("wrap_up"),
            "fix_task prompt should instruct wrap up"
        );
    }

    #[test]
    fn fix_task_prompt_has_learning_tools() {
        let text = build_fix_task_prompt(
            TaskId(5),
            "Fix CVE",
            "desc",
            None,
            None,
            &PromptContext::default(),
        );
        assert!(
            text.contains("/learnings"),
            "fix_task prompt should reference /learnings skill"
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
    }

    #[test]
    fn render_procedural_prefix_omits_when_empty() {
        assert_eq!(render_procedural_prefix(&[]), String::new());
    }

    #[test]
    fn render_procedural_prefix_emits_summary_when_no_detail() {
        let l = seed(1, LearningScope::User, 0);
        let out = render_procedural_prefix(&[&l]);
        assert!(out.contains("learning 1"));
        assert!(out.ends_with("\n\n"));
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
            None,
            &PromptContext::default(),
        );
        assert!(text.starts_with("Your task is:"));
        assert!(!text.contains("Validated knowledge for this task"));
    }

    #[test]
    fn build_prompt_with_injections_includes_knowledge_block() {
        let proc_l = {
            let mut l = seed(10, LearningScope::User, 0);
            l.kind = LearningKind::Procedural;
            l.detail = Some("Always run tests before committing.".into());
            l
        };
        let tier_l = seed(11, LearningScope::Repo, 2);
        let injections = LearningInjections {
            procedural: vec![&proc_l],
            tiered: vec![&tier_l],
        };
        let ctx = PromptContext {
            learnings: injections,
            repo_map: None,
        };
        let text = build_prompt(TaskId(1), "title", "desc", None, None, None, &ctx);
        assert!(text.starts_with("Always run tests before committing."));
        assert!(text.contains("## Validated knowledge for this task"));
        assert!(text.contains("[#11 repo, \u{2191}2]"));
    }

    #[test]
    fn build_quick_dispatch_prompt_default_injections_unchanged() {
        let text = build_quick_dispatch_prompt(
            TaskId(1),
            "title",
            "desc",
            None,
            None,
            &PromptContext::default(),
        );
        assert!(text.starts_with("You are working interactively with the user."));
        assert!(!text.contains("Validated knowledge for this task"));
    }

    // ---- repo-map injection ----

    fn ctx_with_map(map: &str) -> PromptContext<'static> {
        PromptContext {
            learnings: LearningInjections::default(),
            repo_map: Some(map.to_string()),
        }
    }

    const REPO_MAP_MARKER: &str = "## Repo map";
    const SAMPLE_MAP: &str = "src/a.rs\n  function foo, bar\n";

    #[test]
    fn build_prompt_includes_repo_map_when_provided() {
        let ctx = ctx_with_map(SAMPLE_MAP);
        let text = build_prompt(TaskId(1), "t", "d", None, None, None, &ctx);
        assert!(text.contains(REPO_MAP_MARKER));
        assert!(text.contains("src/a.rs"));
    }

    #[test]
    fn build_prompt_omits_repo_map_section_when_none() {
        let text = build_prompt(
            TaskId(1),
            "t",
            "d",
            None,
            None,
            None,
            &PromptContext::default(),
        );
        assert!(!text.contains(REPO_MAP_MARKER));
    }

    #[test]
    fn repo_map_appears_after_knowledge_before_addendum() {
        let proc_l = {
            let mut l = seed(20, LearningScope::User, 0);
            l.kind = LearningKind::Procedural;
            l.detail = Some("Procedural rule X.".into());
            l
        };
        let tier_l = seed(21, LearningScope::Repo, 1);
        let ctx = PromptContext {
            learnings: LearningInjections {
                procedural: vec![&proc_l],
                tiered: vec![&tier_l],
            },
            repo_map: Some(SAMPLE_MAP.to_string()),
        };
        let text = build_prompt(TaskId(1), "t", "d", Some("/p/plan.md"), None, None, &ctx);
        let knowledge_at = text
            .find("## Validated knowledge for this task")
            .expect("knowledge present");
        let map_at = text.find(REPO_MAP_MARKER).expect("map present");
        let plan_at = text
            .find("Plan: /p/plan.md")
            .expect("plan addendum present");
        assert!(
            knowledge_at < map_at,
            "knowledge ({knowledge_at}) must precede repo map ({map_at})"
        );
        assert!(
            map_at < plan_at,
            "repo map ({map_at}) must precede addendum ({plan_at})"
        );
    }

    #[test]
    fn build_quick_dispatch_prompt_includes_repo_map_when_provided() {
        let ctx = ctx_with_map(SAMPLE_MAP);
        let text = build_quick_dispatch_prompt(TaskId(1), "t", "d", None, None, &ctx);
        assert!(text.contains(REPO_MAP_MARKER));
    }

    #[test]
    fn build_pr_review_prompt_includes_repo_map_when_provided() {
        let ctx = ctx_with_map(SAMPLE_MAP);
        let text = build_pr_review_prompt(TaskId(1), "t", "d", None, None, &ctx);
        assert!(text.contains(REPO_MAP_MARKER));
    }

    #[test]
    fn build_research_prompt_includes_repo_map_when_provided() {
        let ctx = ctx_with_map(SAMPLE_MAP);
        let text = build_research_prompt(TaskId(1), "t", "d", None, None, &ctx);
        assert!(text.contains(REPO_MAP_MARKER));
    }

    #[test]
    fn build_fix_task_prompt_includes_repo_map_when_provided() {
        let ctx = ctx_with_map(SAMPLE_MAP);
        let text = build_fix_task_prompt(TaskId(1), "t", "d", None, None, &ctx);
        assert!(text.contains(REPO_MAP_MARKER));
    }

    #[test]
    fn build_epic_planning_prompt_includes_repo_map_when_provided() {
        let epic = EpicContext {
            epic_id: crate::models::EpicId(1),
            epic_title: "e".into(),
        };
        let project = ProjectContext {
            project_id: crate::models::ProjectId(1),
            project_name: "p".into(),
        };
        let ctx = ctx_with_map(SAMPLE_MAP);
        let text = build_epic_planning_prompt(TaskId(1), "t", "d", &epic, &project, &ctx);
        assert!(text.contains(REPO_MAP_MARKER));
    }

    #[test]
    fn render_repo_map_omits_when_empty_or_none() {
        assert!(render_repo_map(None).is_empty());
        assert!(render_repo_map(Some("")).is_empty());
        assert!(render_repo_map(Some("   \n  ")).is_empty());
    }
}
