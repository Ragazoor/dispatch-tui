use crate::db;
use crate::models::{EpicId, ProjectId, Task, TaskId};

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
    pub fn from_db(task: &Task, db: &dyn db::TaskStore) -> Option<Self> {
        let epic_id = task.epic_id?;
        let epic = db.get_epic(epic_id).ok()??;
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
    pub fn from_db(task: &Task, db: &dyn db::TaskStore) -> Self {
        let lookup = db
            .list_projects()
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

/// Wrap-up instruction for when implementation is complete.
pub(super) fn wrap_up_instruction() -> &'static str {
    "When implementation is complete, use the /wrap-up skill to commit remaining \
changes and ask the user whether to rebase onto main or create a PR."
}

/// Allium spec instruction — shared across all agents that may touch domain behaviour.
pub(super) fn allium_instruction() -> &'static str {
    "The Allium specs in `docs/specs/` are the source of truth for domain logic \
Consult them before changing core behaviour. If your implementation changes domain behaviour, \
update the spec using the `allium:tend` skill and verify alignment with `allium:weed`."
}

pub(super) fn build_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    plan: Option<&str>,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
) -> String {
    let block = task_block(task_id, title, description, epic, project);

    match plan {
        None => {
            // No plan yet — agent brainstorms (if vague) or writes a plan directly.
            format!(
                "Your task is:\n\
{block}\n\
\n\
{attach}\n\
\n\
{tdd}\n\
\n\
{allium}\n\
\n\
{mcp}",
                block = block,
                attach = plan_or_brainstorm_instruction(),
                tdd = tdd_instruction(),
                allium = allium_instruction(),
                mcp = mcp_tools_instruction(),
            )
        }
        Some(path) => {
            // Plan exists — review it and ask for permission before implementing.
            format!(
                "Your task is:\n\
{block}\n\
\n\
Plan: {path}\n\
Read this file for the full implementation plan.\n\
\n\
Review the plan carefully. Summarise your intended approach in 3–5 bullet points, \
then ask: 'Shall I proceed with implementation?' Wait for confirmation before \
making any changes.\n\
\n\
{tdd}\n\
\n\
{allium}\n\
\n\
{mcp}\n\
\n\
{wrap_up}",
                block = block,
                path = path,
                tdd = tdd_instruction(),
                allium = allium_instruction(),
                mcp = mcp_tools_instruction(),
                wrap_up = wrap_up_instruction(),
            )
        }
    }
}

pub(super) fn build_quick_dispatch_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
) -> String {
    let block = task_block(task_id, title, description, epic, project);

    format!(
        "You are working interactively with the user.\n\
\n\
{block}\n\
\n\
This is a quick-dispatched task with a placeholder title. Start by asking the user \
what they want to achieve. Once you understand the goal, call `update_task` with a \
descriptive `title` (and optionally `description`) to rename the task on the kanban board.\n\
\n\
Then write a focused plan before making any changes:\n\
\n\
{attach}\n\
\n\
{tdd}\n\
\n\
{allium}\n\
\n\
{mcp}",
        block = block,
        attach = plan_and_attach_instruction(),
        tdd = tdd_instruction(),
        allium = allium_instruction(),
        mcp = mcp_tools_instruction(),
    )
}

pub(super) fn build_brainstorm_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
) -> String {
    let block = task_block(task_id, title, description, epic, project);

    format!(
        "You are starting a brainstorming session.\n\
\n\
{block}\n\
\n\
{attach}\n\
\n\
{allium}\n\
\n\
{mcp}",
        block = block,
        attach = plan_and_attach_instruction(),
        allium = allium_instruction(),
        mcp = mcp_tools_instruction(),
    )
}

pub(super) fn build_plan_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
) -> String {
    let block = task_block(task_id, title, description, epic, project);

    format!(
        "You are starting a planning session.\n\
\n\
{block}\n\
\n\
{attach}\n\
\n\
{tdd}\n\
\n\
{allium}\n\
\n\
{mcp}",
        block = block,
        attach = plan_and_attach_instruction(),
        tdd = tdd_instruction(),
        allium = allium_instruction(),
        mcp = mcp_tools_instruction(),
    )
}

pub(super) fn build_epic_planning_prompt(
    epic_id: EpicId,
    title: &str,
    description: &str,
    project: &ProjectContext,
) -> String {
    let project_section = project.prompt_section();
    format!(
        "You are starting a planning session.\n\
\n\
Epic:\n\
  ID: {epic_id}\n\
  Title: {title}\n\
  Description: {description}\n\
  ProjectId: {project_id}\n\
{project_section}\n\
Your goal is to explore the codebase, write an implementation plan, and break \
it into work packages on the kanban board.\n\
\n\
Steps:\n\
1. Explore the codebase to understand what needs to change.\n\
2. Use the /brainstorm skill to write the plan. When done, attach it to the epic:\n\
   Call update_epic with epic_id={epic_id} and plan=<absolute path to plan file>\n\
3. Create work packages from the plan using create_task. Work packages are kanban \
tasks — do not confuse them with subtasks inside the plan document itself:\n\
   - Set epic_id={epic_id} on every work package\n\
   - Set project_id={project_id} on every work package\n\
   - Use sort_order to control execution order (1, 2, 3, \u{2026})\n\
   - Work packages at the same sort_order in different repositories run in parallel\n\
   - Work packages in the same repository must have different sort_order values\n\
   - Set repo_path to the absolute path of the repository each work package targets\n\
\n\
After creating the work packages, confirm with the user before doing anything further.\n\
\n\
{tdd}\n\
\n\
{allium}\n\
\n\
Use the dispatch MCP tools to query tasks and epics. Relevant tools: create_task, update_epic, list_tasks.\n\
\n\
IMPORTANT: Do NOT start implementing. Your job ends after creating the work packages.",
        epic_id = epic_id,
        title = title,
        description = description,
        project_id = project.project_id,
        project_section = project_section,
        tdd = tdd_instruction(),
        allium = allium_instruction(),
    )
}
