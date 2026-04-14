use anyhow::{Context, Result};
use std::fs;

use crate::db;
use crate::models::{
    expand_tilde, slugify, DispatchResult, EpicId, ResumeResult, ReviewDecision, Task, TaskId,
    TaskStatus,
};
use crate::process::ProcessRunner;
use crate::tmux;

/// Plugin dir flag added to all Claude agent invocations so dispatched agents
/// discover the dispatch plugin's skills and commands (e.g. /wrap-up).
const DISPATCH_PLUGIN_DIR: &str = "--plugin-dir ~/.claude/plugins/local/dispatch";

/// Epic context passed to prompt builders so agents know about their epic.
pub struct EpicContext {
    pub epic_id: EpicId,
    pub epic_title: String,
    pub sibling_summaries: Vec<String>,
}

impl EpicContext {
    /// Build epic context from the database for a task that belongs to an epic.
    pub fn from_db(task: &Task, db: &dyn db::TaskStore) -> Option<Self> {
        let epic_id = task.epic_id?;
        let epic = db.get_epic(epic_id).ok()??;
        let siblings = db
            .list_tasks_for_epic(epic_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.id != task.id && t.status != TaskStatus::Archived)
            .map(|t| format!("[{}] {} ({})", t.id, t.title, t.status.as_str()))
            .collect();
        Some(EpicContext {
            epic_id,
            epic_title: epic.title,
            sibling_summaries: siblings,
        })
    }

    fn prompt_section(&self) -> String {
        let mut section = format!(
            "\n\nThis task is part of epic #{}: {}\n\
            To find other tasks in this epic, call list_tasks with epic_id={}.",
            self.epic_id, self.epic_title, self.epic_id
        );
        if !self.sibling_summaries.is_empty() {
            section.push_str("\n\nSibling tasks:");
            for s in &self.sibling_summaries {
                section.push_str(&format!("\n- {s}"));
            }
        }
        section
    }
}

// ---------------------------------------------------------------------------
// dispatch_agent
// ---------------------------------------------------------------------------

struct ProvisionResult {
    worktree_path: String,
    tmux_window: String,
}

/// Create a git worktree and open a tmux window.
/// Shared by both `dispatch_agent` and `brainstorm_agent`.
fn provision_worktree(
    task: &Task,
    runner: &dyn ProcessRunner,
    base_branch: Option<&str>,
) -> Result<ProvisionResult> {
    let repo_path = expand_tilde(&task.repo_path);
    let slug = slugify(&task.title);
    let worktree_name = format!("{}-{slug}", task.id);
    let worktree_path = format!("{repo_path}/.worktrees/{worktree_name}");
    let tmux_window = build_tmux_window_name(task.id);

    tracing::info!(task_id = task.id.0, %worktree_path, ?base_branch, "provisioning worktree");

    fs::create_dir_all(format!("{repo_path}/.worktrees"))
        .context("failed to create .worktrees directory")?;

    if std::path::Path::new(&worktree_path).exists() {
        tracing::info!(task_id = task.id.0, %worktree_path, "worktree already exists, reusing");
    } else {
        let mut args = vec![
            "-C",
            &repo_path,
            "worktree",
            "add",
            &*worktree_path,
            "-B",
            &*worktree_name,
        ];
        if let Some(base) = base_branch {
            args.push(base);
        }
        let output = runner
            .run("git", &args)
            .context("failed to run git worktree add")?;
        anyhow::ensure!(
            output.status.success(),
            "git worktree add failed: {}",
            stderr_str(&output)
        );
    }

    tmux::new_window(&tmux_window, &worktree_path, runner)
        .context("failed to create tmux window")?;

    tmux::set_window_dispatch_dir(&tmux_window, &worktree_path, runner)
        .context("failed to set tmux window dispatch dir")?;
    tmux::ensure_split_hook(runner).context("failed to ensure tmux split hook")?;

    Ok(ProvisionResult {
        worktree_path,
        tmux_window,
    })
}

fn rebase_preamble(target: &str) -> String {
    format!(
        "Before starting work, rebase your branch from {target}:\n\
         ```\n\
         git rebase {target}\n\
         ```"
    )
}

#[derive(Clone, Copy)]
enum ClaudeMode {
    Plan,
}

impl ClaudeMode {
    fn as_flag(self) -> &'static str {
        match self {
            ClaudeMode::Plan => "plan",
        }
    }
}

/// Provision worktree, write prompt file, launch Claude via tmux.
/// The prompt file is deleted after Claude reads it.
/// Shared by all dispatch variants.
fn dispatch_with_prompt(
    task: &Task,
    prompt: &str,
    mode: ClaudeMode,
    runner: &dyn ProcessRunner,
    base_branch: Option<&str>,
) -> Result<DispatchResult> {
    let repo_path = expand_tilde(&task.repo_path);

    // Resolve the start-point once; reuse in both provision_worktree and rebase_preamble.
    let detected;
    let resolved = match base_branch {
        Some(b) => b,
        None => {
            detected = detect_default_branch(&repo_path, runner);
            &detected
        }
    };

    let provision = provision_worktree(task, runner, Some(resolved))?;

    let full_prompt = format!(
        "{}\n\n\
         Always work from this worktree folder — do not `cd` to the parent repo \
         or other directories.\n\n\
         {prompt}",
        rebase_preamble(resolved)
    );
    let prompt_file = format!("{}/.claude-prompt", provision.worktree_path);
    fs::write(&prompt_file, &full_prompt)
        .with_context(|| format!("failed to write {prompt_file}"))?;
    let claude_cmd = format!(
        "bash -c 'prompt=$(cat .claude-prompt) && rm -f .claude-prompt \
         && claude {DISPATCH_PLUGIN_DIR} --permission-mode {} \"$prompt\"'",
        mode.as_flag()
    );
    tmux::send_keys(&provision.tmux_window, &claude_cmd, runner)
        .context("failed to send keys to tmux window")?;

    tracing::info!(task_id = task.id.0, worktree = %provision.worktree_path, "agent dispatched");

    Ok(DispatchResult {
        worktree_path: provision.worktree_path,
        tmux_window: provision.tmux_window,
    })
}

pub fn dispatch_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
) -> Result<DispatchResult> {
    let prompt = build_prompt(
        task.id,
        &task.title,
        &task.description,
        task.plan_path.as_deref(),
        epic,
    );
    dispatch_with_prompt(
        task,
        &prompt,
        ClaudeMode::Plan,
        runner,
        Some(&task.base_branch),
    )
}

pub fn brainstorm_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
) -> Result<DispatchResult> {
    let prompt = build_brainstorm_prompt(task.id, &task.title, &task.description, epic);
    dispatch_with_prompt(
        task,
        &prompt,
        ClaudeMode::Plan,
        runner,
        Some(&task.base_branch),
    )
}

pub fn plan_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
) -> Result<DispatchResult> {
    let prompt = build_plan_prompt(task.id, &task.title, &task.description, epic);
    dispatch_with_prompt(
        task,
        &prompt,
        ClaudeMode::Plan,
        runner,
        Some(&task.base_branch),
    )
}

pub fn quick_dispatch_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
) -> Result<DispatchResult> {
    let prompt = build_quick_dispatch_prompt(task.id, &task.title, &task.description, epic);
    dispatch_with_prompt(
        task,
        &prompt,
        ClaudeMode::Plan,
        runner,
        Some(&task.base_branch),
    )
}

pub fn epic_planning_agent(
    task: &Task,
    epic_id: EpicId,
    epic_title: &str,
    epic_description: &str,
    runner: &dyn ProcessRunner,
) -> Result<DispatchResult> {
    let prompt = build_epic_planning_prompt(epic_id, epic_title, epic_description);
    dispatch_with_prompt(
        task,
        &prompt,
        ClaudeMode::Plan,
        runner,
        Some(&task.base_branch),
    )
}

// ---------------------------------------------------------------------------
// cleanup_task
// ---------------------------------------------------------------------------

/// Remove the tmux window (if it still exists) and the git worktree.
///
/// Errors are logged but not propagated for the tmux step so that the
/// worktree removal is always attempted.
pub fn cleanup_task(
    repo_path: &str,
    worktree_path: &str,
    tmux_window: Option<&str>,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    tracing::info!(worktree_path, "cleaning up task");

    if let Some(window) = tmux_window {
        match tmux::has_window(window, runner) {
            Ok(true) => {
                tmux::kill_window(window, runner)
                    .context("failed to kill tmux window during cleanup")?;
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!("could not check tmux window during cleanup: {e}");
            }
        }
    }

    let repo = expand_tilde(repo_path);
    let output = runner
        .run(
            "git",
            &["-C", &repo, "worktree", "remove", "--force", worktree_path],
        )
        .context("failed to run git worktree remove")?;
    if !output.status.success() {
        let stderr = stderr_str(&output);
        // If the worktree is already gone (manually removed or pruned), treat as success.
        if stderr.contains("is not a working tree") {
            tracing::info!(worktree_path, "worktree already removed, skipping");
        } else {
            anyhow::bail!(
                "git worktree remove failed for path {worktree_path}: {}",
                stderr
            );
        }
    }

    if let Some(branch) = std::path::Path::new(worktree_path)
        .file_name()
        .and_then(|n| n.to_str())
    {
        // Best-effort: ignore errors (branch may not exist).
        let _ = runner.run("git", &["-C", &repo, "branch", "-D", branch]);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// finish_task
// ---------------------------------------------------------------------------

/// Detect the default branch for a repo by inspecting `origin/HEAD`.
/// Falls back to `"main"` when there is no remote or the ref is missing.
fn detect_default_branch(repo_path: &str, runner: &dyn ProcessRunner) -> String {
    if let Ok(output) = runner.run(
        "git",
        &["-C", repo_path, "symbolic-ref", "refs/remotes/origin/HEAD"],
    ) {
        if output.status.success() {
            let refname = stdout_str(&output);
            // e.g. "refs/remotes/origin/master" → "master"
            if let Some(branch) = refname.rsplit('/').next() {
                if !branch.is_empty() {
                    return branch.to_string();
                }
            }
        }
    }
    "main".to_string()
}

/// Errors from the finish (rebase + cleanup) operation.
#[derive(Debug)]
pub enum FinishError {
    NotOnDefaultBranch { current: String, expected: String },
    RebaseConflict(String),
    Other(String),
}

impl std::fmt::Display for FinishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FinishError::NotOnDefaultBranch { current, expected } => write!(
                f,
                "Repo root is not on {expected} (currently on {current}) — checkout {expected} first"
            ),
            FinishError::RebaseConflict(branch) => {
                write!(f, "Rebase conflict on {branch} — resolve and try again")
            }
            FinishError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

/// Rebase the task branch onto `base_branch` and fast-forward it, then kill the tmux window.
/// The worktree is preserved — it will be cleaned up when the task is archived.
pub fn finish_task(
    repo_path: &str,
    worktree: &str,
    branch: &str,
    base_branch: &str,
    tmux_window: Option<&str>,
    runner: &dyn ProcessRunner,
) -> std::result::Result<(), FinishError> {
    let repo_path = &expand_tilde(repo_path);
    let worktree = &expand_tilde(worktree);

    // 1. Verify we're on the base branch
    let output = runner
        .run(
            "git",
            &["-C", repo_path, "rev-parse", "--abbrev-ref", "HEAD"],
        )
        .map_err(|e| FinishError::Other(format!("Failed to check current branch: {e}")))?;
    let current_branch = stdout_str(&output);
    if current_branch != base_branch {
        return Err(FinishError::NotOnDefaultBranch {
            current: current_branch,
            expected: base_branch.to_string(),
        });
    }

    // 2. Pull latest base branch (skip if no remote configured)
    let has_remote = runner
        .run("git", &["-C", repo_path, "remote", "get-url", "origin"])
        .map(|o| o.status.success())
        .unwrap_or(false);

    if has_remote {
        let output = runner
            .run("git", &["-C", repo_path, "pull", "origin", base_branch])
            .map_err(|e| FinishError::Other(format!("Failed to pull: {e}")))?;
        if !output.status.success() {
            return Err(FinishError::Other(format!(
                "Failed to pull {base_branch}: {}",
                stderr_str(&output)
            )));
        }
    }

    // 3. Rebase branch onto base branch (from worktree, where branch is checked out)
    let output = runner
        .run("git", &["-C", worktree, "rebase", base_branch])
        .map_err(|e| FinishError::Other(format!("Failed to run git rebase: {e}")))?;
    if !output.status.success() {
        let stderr = stderr_str(&output);
        let stdout = stdout_str(&output);
        let is_conflict = stderr.contains("CONFLICT")
            || stdout.contains("CONFLICT")
            || stderr.contains("could not apply")
            || stderr.contains("Merge conflict");

        let _ = runner.run("git", &["-C", worktree, "rebase", "--abort"]);

        if is_conflict {
            return Err(FinishError::RebaseConflict(branch.to_string()));
        }
        return Err(FinishError::Other(format!("Rebase failed: {}", stderr)));
    }

    // 4. Fast-forward base branch to the rebased branch
    let output = runner
        .run("git", &["-C", repo_path, "merge", "--ff-only", branch])
        .map_err(|e| FinishError::Other(format!("Failed to fast-forward {base_branch}: {e}")))?;
    if !output.status.success() {
        return Err(FinishError::Other(format!(
            "Fast-forward failed after rebase: {}",
            stderr_str(&output)
        )));
    }

    // 5. Kill tmux window (worktree is preserved for later archival)
    if let Some(window) = tmux_window {
        match tmux::has_window(window, runner) {
            Ok(true) => {
                tmux::kill_window(window, runner)
                    .map_err(|e| FinishError::Other(format!("Failed to kill tmux window: {e}")))?;
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!("could not check tmux window during finish: {e}");
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// resume_agent
// ---------------------------------------------------------------------------

/// Re-open a tmux window for an existing worktree and resume the most recent
/// Claude conversation with `claude --continue`.
///
/// This function is **synchronous** and should be called via
/// `tokio::task::spawn_blocking` from async contexts.
pub fn resume_agent(
    task_id: TaskId,
    worktree_path: &str,
    runner: &dyn ProcessRunner,
) -> Result<ResumeResult> {
    let tmux_window = build_tmux_window_name(task_id);

    tmux::new_window(&tmux_window, worktree_path, runner)
        .context("failed to create tmux window for resume")?;

    tmux::set_window_dispatch_dir(&tmux_window, worktree_path, runner)
        .context("failed to set tmux window dispatch dir")?;
    tmux::ensure_split_hook(runner).context("failed to ensure tmux split hook")?;

    tmux::send_keys(
        &tmux_window,
        &format!("claude {DISPATCH_PLUGIN_DIR} --continue"),
        runner,
    )
    .context("failed to send resume keys to tmux window")?;

    tracing::info!(task_id = task_id.0, %tmux_window, "agent resumed");

    Ok(ResumeResult { tmux_window })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_tmux_window_name(task_id: TaskId) -> String {
    format!("task-{task_id}")
}

/// Returns `(epic_id_line, epic_section)` for embedding in agent prompts.
fn epic_preamble(epic: Option<&EpicContext>) -> (String, String) {
    let id_line = epic.map_or(String::new(), |e| format!("\n  EpicId: {}", e.epic_id));
    let section = epic.map_or(String::new(), |e| e.prompt_section());
    (id_line, section)
}

/// Standard task identification block shared by all task agent prompts.
fn task_block(
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
fn tdd_instruction() -> &'static str {
    "Always use TDD: express intended behaviour as tests first, then implement the minimum code to make them pass."
}

/// MCP tools availability notice, shared across all task agents.
fn mcp_tools_instruction() -> &'static str {
    "The dispatch MCP tools are available — use them to query and update this task (get_task, update_task)."
}

/// Instructions for writing a plan and attaching it to the task via MCP.
fn plan_and_attach_instruction(_task_id: TaskId) -> String {
    "Use /brainstorming to design the solution, then save the plan to docs/plans/ \
and call update_task to attach it."
        .to_string()
}

/// Dispatch instruction for no-plan tasks: conditionally suggests brainstorming
/// based on agent judgment of task description clarity.
fn plan_or_brainstorm_instruction() -> &'static str {
    "Use /brainstorming to design the solution if the task description is vague or \
underspecified. Otherwise write an implementation plan directly, save it to docs/plans/ \
and call update_task to attach it."
}

/// Wrap-up instruction for when implementation is complete.
fn wrap_up_instruction() -> &'static str {
    "When implementation is complete, use the /wrap-up skill to commit remaining \
changes and ask the user whether to rebase onto main or create a PR."
}

/// Allium spec instruction — shared across all agents that may touch domain behaviour.
fn allium_instruction() -> &'static str {
    "The Allium specs in `docs/specs/` are the source of truth for domain logic \
(`core.allium` for the domain model, `tasks.allium` for task lifecycle, \
`epics.allium` for epic lifecycle). \
Consult them before changing core behaviour. If your implementation changes domain behaviour, \
update the spec using the `allium:tend` skill and verify alignment with `allium:weed`."
}

fn build_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    plan: Option<&str>,
    epic: Option<&EpicContext>,
) -> String {
    let block = task_block(task_id, title, description, epic);

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

fn build_quick_dispatch_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
) -> String {
    let block = task_block(task_id, title, description, epic);

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
        attach = plan_and_attach_instruction(task_id),
        tdd = tdd_instruction(),
        allium = allium_instruction(),
        mcp = mcp_tools_instruction(),
    )
}

fn build_brainstorm_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
) -> String {
    let block = task_block(task_id, title, description, epic);

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
        attach = plan_and_attach_instruction(task_id),
        allium = allium_instruction(),
        mcp = mcp_tools_instruction(),
    )
}

fn build_plan_prompt(
    task_id: TaskId,
    title: &str,
    description: &str,
    epic: Option<&EpicContext>,
) -> String {
    let block = task_block(task_id, title, description, epic);

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
        attach = plan_and_attach_instruction(task_id),
        tdd = tdd_instruction(),
        allium = allium_instruction(),
        mcp = mcp_tools_instruction(),
    )
}

fn build_epic_planning_prompt(epic_id: EpicId, title: &str, description: &str) -> String {
    format!(
        "You are starting a planning session.\n\
\n\
Epic:\n\
  ID: {epic_id}\n\
  Title: {title}\n\
  Description: {description}\n\
\n\
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
        tdd = tdd_instruction(),
        allium = allium_instruction(),
    )
}

// ---------------------------------------------------------------------------
// PR types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PrResult {
    pub pr_url: String,
}

#[derive(Debug)]
pub enum PrError {
    PushFailed(String),
    CreateFailed(String),
    Other(String),
}

impl std::fmt::Display for PrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrError::PushFailed(msg) => write!(f, "Push failed: {msg}"),
            PrError::CreateFailed(msg) => write!(f, "PR creation failed: {msg}"),
            PrError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug)]
pub struct PrStatus {
    pub state: PrState,
    pub review_decision: Option<ReviewDecision>,
}

// ---------------------------------------------------------------------------
// PR functions
// ---------------------------------------------------------------------------

/// Extract "owner/repo" from a git remote URL.
/// Handles both SSH (git@github.com:owner/repo.git) and HTTPS (https://github.com/owner/repo.git).
fn parse_repo_slug(remote_url: &str) -> Option<String> {
    let url = remote_url.trim().trim_end_matches(".git");
    // SSH: git@github.com:owner/repo
    if let Some(path) = url.strip_prefix("git@github.com:") {
        return Some(path.to_string());
    }
    // HTTPS: https://github.com/owner/repo
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        return Some(rest.to_string());
    }
    None
}

/// Push the branch to origin and create a GitHub PR using `gh`.
pub fn create_pr(
    repo_path: &str,
    branch: &str,
    title: &str,
    description: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> std::result::Result<PrResult, PrError> {
    let repo_path = &expand_tilde(repo_path);

    // 1. Push the branch
    let output = runner
        .run("git", &["-C", repo_path, "push", "-u", "origin", branch])
        .map_err(|e| PrError::PushFailed(format!("Failed to run git push: {e}")))?;
    if !output.status.success() {
        return Err(PrError::PushFailed(stderr_str(&output)));
    }

    // 2. Get the repo slug from git remote
    let remote_output = runner
        .run("git", &["-C", repo_path, "remote", "get-url", "origin"])
        .map_err(|e| PrError::Other(format!("Failed to get remote URL: {e}")))?;
    let remote_url = stdout_str(&remote_output);
    let repo_slug = parse_repo_slug(&remote_url)
        .ok_or_else(|| PrError::Other(format!("Could not parse repo slug from: {remote_url}")))?;

    // 3. Create the PR.
    // Use owner:branch format for --head so gh resolves the branch in the same repo as --repo.
    // Without the owner prefix, gh defaults to the authenticated user's namespace, causing
    // "Head sha can't be blank" errors when the user isn't the repo owner.
    let owner = repo_slug
        .split('/')
        .next()
        .ok_or_else(|| PrError::Other(format!("Invalid repo slug: {repo_slug}")))?;
    let head_ref = format!("{owner}:{branch}");
    let output = runner
        .run(
            "gh",
            &[
                "pr",
                "create",
                "--draft",
                "--title",
                title,
                "--body",
                description,
                "--head",
                &head_ref,
                "--base",
                base_branch,
                "--repo",
                &repo_slug,
            ],
        )
        .map_err(|e| PrError::Other(format!("Failed to run gh: {e}")))?;
    if !output.status.success() {
        return Err(PrError::CreateFailed(stderr_str(&output)));
    }

    // 4. Parse the PR URL from stdout
    let pr_url = stdout_str(&output);
    Ok(PrResult { pr_url })
}

/// Check the current status of a PR using `gh pr view`.
pub fn check_pr_status(pr_url: &str, runner: &dyn ProcessRunner) -> Result<PrStatus> {
    let output = runner
        .run(
            "gh",
            &[
                "pr",
                "view",
                pr_url,
                "--json",
                "state,reviewDecision",
                "-q",
                r#"[.state, .reviewDecision] | join("\n")"#,
            ],
        )
        .context("Failed to run gh pr view")?;
    if !output.status.success() {
        anyhow::bail!("gh pr view failed: {}", stderr_str(&output));
    }

    let stdout = stdout_str(&output);
    let mut lines = stdout.lines();
    let state_str = lines.next().unwrap_or("").to_uppercase();
    let review_str = lines.next().unwrap_or("").to_uppercase();

    let state = match state_str.as_str() {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ => PrState::Open,
    };

    let review_decision = ReviewDecision::parse(&review_str);

    Ok(PrStatus {
        state,
        review_decision,
    })
}

/// Merge a GitHub PR using `gh pr merge --merge`.
pub fn merge_pr(pr_url: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner
        .run("gh", &["pr", "merge", "--merge", pr_url])
        .context("Failed to run gh pr merge")?;
    if !output.status.success() {
        anyhow::bail!("{}", stderr_str(&output));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve a GitHub repo name (e.g. `"org/repo"`) to a local filesystem path
/// by matching against known repo paths.  Returns the first path whose
/// directory name equals the short repo name.
pub fn resolve_repo_path(github_repo: &str, known_paths: &[String]) -> Option<String> {
    let repo_short = github_repo.split('/').next_back().unwrap_or(github_repo);
    known_paths
        .iter()
        .find(|p| {
            std::path::Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|dir| dir == repo_short)
        })
        .cloned()
}

// ---------------------------------------------------------------------------
// Shared agent dispatch infrastructure
// ---------------------------------------------------------------------------

/// How to set up the git worktree for a dispatched agent.
enum WorktreeStrategy<'a> {
    /// Check out an existing remote branch (e.g. for PR reviews).
    CheckoutRemote { head_ref: &'a str },
    /// Create a new branch from the repo's default branch (e.g. for fixes).
    NewBranch { branch_name: String },
}

/// Configuration for dispatching an agent into an isolated worktree.
struct AgentDispatchConfig<'a> {
    repo_path: String,
    worktree_name: String,
    tmux_prefix: &'a str,
    number: i64,
    git_strategy: WorktreeStrategy<'a>,
    prompt: String,
}

/// Shared worktree provisioning + agent launch used by both review and fix dispatch.
fn provision_and_dispatch(
    config: AgentDispatchConfig,
    runner: &dyn ProcessRunner,
) -> Result<DispatchResult> {
    let repo_short = config
        .repo_path
        .split('/')
        .next_back()
        .unwrap_or(&config.repo_path);
    let worktree_path = format!("{}/.worktrees/{}", config.repo_path, config.worktree_name);
    let tmux_window = format!("{}-{}-{}", config.tmux_prefix, repo_short, config.number);

    // Check if tmux window already exists — focus it instead
    if tmux::has_window(&tmux_window, runner).unwrap_or(false) {
        return Ok(DispatchResult {
            worktree_path,
            tmux_window,
        });
    }

    std::fs::create_dir_all(format!("{}/.worktrees", config.repo_path))
        .context("failed to create .worktrees directory")?;

    // Prune stale worktree entries (directories deleted without `git worktree remove`)
    let _ = runner.run("git", &["-C", &config.repo_path, "worktree", "prune"]);

    // Set up the worktree according to the chosen strategy
    match &config.git_strategy {
        WorktreeStrategy::CheckoutRemote { head_ref } => {
            let fetch_output = runner
                .run(
                    "git",
                    &["-C", &config.repo_path, "fetch", "origin", head_ref],
                )
                .context("failed to fetch PR branch")?;
            anyhow::ensure!(
                fetch_output.status.success(),
                "git fetch failed: {}",
                stderr_str(&fetch_output)
            );

            if !std::path::Path::new(&worktree_path).exists() {
                let output = runner
                    .run(
                        "git",
                        &[
                            "-C",
                            &config.repo_path,
                            "worktree",
                            "add",
                            &worktree_path,
                            &format!("origin/{head_ref}"),
                        ],
                    )
                    .context("failed to create review worktree")?;
                anyhow::ensure!(
                    output.status.success(),
                    "git worktree add failed: {}",
                    stderr_str(&output)
                );
            }
        }
        WorktreeStrategy::NewBranch { branch_name } => {
            let head_output = runner
                .run(
                    "git",
                    &[
                        "-C",
                        &config.repo_path,
                        "symbolic-ref",
                        "refs/remotes/origin/HEAD",
                    ],
                )
                .context("failed to detect default branch")?;
            let head_stdout = String::from_utf8_lossy(&head_output.stdout);
            let default_branch = head_stdout
                .trim()
                .strip_prefix("refs/remotes/origin/")
                .unwrap_or("main");

            let _ = runner.run(
                "git",
                &["-C", &config.repo_path, "fetch", "origin", default_branch],
            );

            if !std::path::Path::new(&worktree_path).exists() {
                let output = runner
                    .run(
                        "git",
                        &[
                            "-C",
                            &config.repo_path,
                            "worktree",
                            "add",
                            "-b",
                            branch_name,
                            &worktree_path,
                            &format!("origin/{default_branch}"),
                        ],
                    )
                    .context("failed to create fix worktree")?;
                anyhow::ensure!(
                    output.status.success(),
                    "git worktree add failed: {}",
                    stderr_str(&output)
                );
            }
        }
    }

    tmux::new_window(&tmux_window, &worktree_path, runner)
        .context("failed to create tmux window")?;

    // Write prompt and launch Claude
    let prompt_file = format!("{worktree_path}/.claude-prompt");
    fs::write(&prompt_file, &config.prompt)
        .with_context(|| format!("failed to write {prompt_file}"))?;
    let claude_cmd = &format!("bash -c 'prompt=$(cat .claude-prompt) && rm -f .claude-prompt && claude {DISPATCH_PLUGIN_DIR} --permission-mode acceptEdits \"$prompt\"'");
    tmux::send_keys(&tmux_window, claude_cmd, runner)
        .context("failed to send keys to tmux window")?;

    Ok(DispatchResult {
        worktree_path,
        tmux_window,
    })
}

/// Dispatch a Claude agent to review a PR in an isolated worktree.
pub fn dispatch_review_agent(
    req: &crate::tui::ReviewAgentRequest,
    runner: &dyn ProcessRunner,
) -> Result<DispatchResult> {
    let prompt = if req.is_dependabot {
        format!(
            "Review dependency update PR #{} in {}.\n\n\
             Run `/anthropic-review-pr:review-pr {}` to review.\n\n\
             After the review completes, call the `update_review_status` MCP tool:\n\
             update_review_status(repo=\"{}\", number={}, status=\"findings_ready\")\n\n\
             Wait for the user.",
            req.number, req.github_repo, req.number, req.github_repo, req.number
        )
    } else {
        format!(
            "Review PR #{} in {}.\n\n\
             Run `/anthropic-review-pr:review-pr {}` to perform a comprehensive code review.\n\n\
             After the review completes, call the `update_review_status` MCP tool:\n\
             update_review_status(repo=\"{}\", number={}, status=\"findings_ready\")\n\n\
             Wait for the user.",
            req.number, req.github_repo, req.number, req.github_repo, req.number
        )
    };

    provision_and_dispatch(
        AgentDispatchConfig {
            repo_path: expand_tilde(&req.repo),
            worktree_name: format!("review-{}", req.number),
            tmux_prefix: "review",
            number: req.number,
            git_strategy: WorktreeStrategy::CheckoutRemote {
                head_ref: &req.head_ref,
            },
            prompt,
        },
        runner,
    )
}

/// Build the prompt for a fix agent based on the alert kind.
pub fn build_fix_prompt(req: &crate::tui::FixAgentRequest) -> String {
    let repo = &req.github_repo;
    let number = req.number;
    match req.kind {
        crate::models::AlertKind::Dependabot => {
            let pkg = req.package.as_deref().unwrap_or("unknown");
            let fix = req
                .fixed_version
                .as_deref()
                .map(|v| format!("A fix is available: upgrade to version {v}"))
                .unwrap_or_else(|| "No fixed version is available yet.".to_string());
            format!(
                "You are fixing security alert #{number} in `{repo}`.\n\n\
                 ## Vulnerability\n\n\
                 **{}**\n\
                 Package: `{pkg}`\n\
                 {fix}\n\n\
                 ## Instructions\n\n\
                 1. Find and update the dependency `{pkg}` to the fixed version\n\
                 2. Run the project's tests to verify nothing breaks\n\
                 3. Commit with a descriptive message referencing the alert\n\
                 4. Create a PR with `gh pr create`\n\n\
                 Focus on the minimal change needed to resolve the vulnerability.\n\n\
                 When done, call the `update_review_status` MCP tool:\n\
                 update_review_status(repo=\"{repo}\", number={number}, status=\"findings_ready\")\n\n\
                 Wait for the user.",
                req.title
            )
        }
        crate::models::AlertKind::CodeScanning => {
            format!(
                "You are fixing a code scanning alert #{number} in `{repo}`.\n\n\
                 ## Alert\n\n\
                 **{}**\n\
                 Location: `{}`\n\n\
                 ## Instructions\n\n\
                 1. Read the flagged code at the reported location\n\
                 2. Understand the vulnerability and apply the appropriate fix\n\
                 3. Run tests to verify the fix doesn't break anything\n\
                 4. Commit and create a PR with `gh pr create`\n\n\
                 Focus on the minimal change needed to resolve the vulnerability.\n\n\
                 When done, call the `update_review_status` MCP tool:\n\
                 update_review_status(repo=\"{repo}\", number={number}, status=\"findings_ready\")\n\n\
                 Wait for the user.",
                req.title, req.description
            )
        }
    }
}

/// Dispatch a Claude agent to fix a security vulnerability in an isolated worktree.
pub fn dispatch_fix_agent(
    req: crate::tui::FixAgentRequest,
    runner: &dyn ProcessRunner,
) -> Result<DispatchResult> {
    let prompt = build_fix_prompt(&req);
    let number = req.number;

    provision_and_dispatch(
        AgentDispatchConfig {
            repo_path: expand_tilde(&req.repo),
            worktree_name: format!("fix-vuln-{number}"),
            tmux_prefix: "fix",
            number,
            git_strategy: WorktreeStrategy::NewBranch {
                branch_name: format!("fix/vuln-{number}"),
            },
            prompt,
        },
        runner,
    )
}

/// A task can be wrapped up if it has a worktree and is either Running or Review.
pub fn is_wrappable(task: &Task) -> bool {
    task.worktree.is_some()
        && (task.status == TaskStatus::Review || task.status == TaskStatus::Running)
}

/// Extract the branch name from a worktree path (its last path component).
pub fn branch_from_worktree(worktree: &str) -> Option<String> {
    std::path::Path::new(worktree)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// Extract stderr from a process `Output` as a trimmed `String`.
fn stderr_str(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

/// Extract stdout from a process `Output` as a trimmed `String`.
fn stdout_str(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Validate that a repo path points to an existing directory.
///
/// Returns the expanded path on success, or an error message on failure.
pub fn validate_repo_path(path: &str) -> Result<String, String> {
    let expanded = expand_tilde(path);
    let p = std::path::Path::new(&expanded);
    if !p.exists() {
        return Err(format!("Directory does not exist: {expanded}"));
    }
    if !p.is_dir() {
        return Err(format!("Not a directory: {expanded}"));
    }
    Ok(expanded)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::models::{EpicId, Task, TaskId, TaskStatus};
    use crate::process::{exit_fail, MockProcessRunner};
    use chrono::Utc;
    use std::process::Output;

    // -----------------------------------------------------------------------
    // Shared helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn task_block_contains_id_title_description() {
        let block = task_block(TaskId(5), "My title", "My description", None);
        assert!(block.contains("5"));
        assert!(block.contains("My title"));
        assert!(block.contains("My description"));
    }

    #[test]
    fn task_block_includes_epic_section_when_present() {
        let ctx = EpicContext {
            epic_id: EpicId(3),
            epic_title: "Big Epic".to_string(),
            sibling_summaries: vec![],
        };
        let block = task_block(TaskId(1), "T", "D", Some(&ctx));
        assert!(block.contains("EpicId: 3"));
        assert!(block.contains("Big Epic"));
    }

    #[test]
    fn tdd_instruction_mentions_tests_first() {
        let instr = tdd_instruction();
        assert!(instr.contains("tests first") || instr.contains("behaviour as tests"));
    }

    #[test]
    fn mcp_tools_instruction_mentions_get_and_update() {
        let instr = mcp_tools_instruction();
        assert!(instr.contains("get_task"));
        assert!(instr.contains("update_task"));
    }

    #[test]
    fn plan_and_attach_instruction_mentions_docs_plans_and_update_task() {
        let instr = plan_and_attach_instruction(TaskId(9));
        assert!(instr.contains("docs/plans/"));
        assert!(instr.contains("update_task"));
    }

    #[test]
    fn wrap_up_instruction_mentions_wrap_up_skill() {
        let instr = wrap_up_instruction();
        assert!(instr.contains("/wrap-up"));
    }

    #[test]
    fn allium_instruction_mentions_spec_and_skills() {
        let instr = allium_instruction();
        assert!(instr.contains("docs/specs/"));
        assert!(instr.contains("allium:tend"));
        assert!(instr.contains("allium:weed"));
    }

    fn make_task(repo_path: &str) -> Task {
        Task {
            id: TaskId(42),
            title: "Fix bug".to_string(),
            description: "A nasty crash".to_string(),
            repo_path: repo_path.to_string(),
            status: TaskStatus::Backlog,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: None,
            sub_status: crate::models::SubStatus::None,
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn resolve_repo_path_matches_directory_name() {
        let paths = vec![
            "/home/user/projects/frontend".to_string(),
            "/home/user/projects/backend".to_string(),
        ];
        assert_eq!(
            resolve_repo_path("org/backend", &paths),
            Some("/home/user/projects/backend".to_string()),
        );
    }

    #[test]
    fn resolve_repo_path_returns_none_when_no_match() {
        let paths = vec!["/home/user/projects/frontend".to_string()];
        assert_eq!(resolve_repo_path("org/backend", &paths), None);
    }

    #[test]
    fn resolve_repo_path_handles_empty_paths() {
        assert_eq!(resolve_repo_path("org/repo", &[]), None);
    }

    #[test]
    fn build_prompt_contains_task_info() {
        let prompt = build_prompt(TaskId(42), "Fix bug", "A nasty crash", None, None);
        assert!(prompt.contains("42"));
        assert!(prompt.contains("Fix bug"));
        assert!(prompt.contains("A nasty crash"));
        assert!(prompt.contains("TDD"));
    }

    #[test]
    fn build_prompt_mentions_tdd() {
        let prompt = build_prompt(TaskId(7), "Title", "Desc", None, None);
        assert!(prompt.contains("TDD"));
        assert!(prompt.contains("behaviour as tests first"));
    }

    #[test]
    fn build_prompt_mentions_wrap_up_skill() {
        // wrap-up instruction only appears when a plan exists (agent is implementing)
        let prompt = build_prompt(TaskId(7), "Title", "Desc", Some("docs/plans/p.md"), None);
        assert!(
            prompt.contains("/wrap-up"),
            "with-plan prompt should tell agent to use /wrap-up skill"
        );
        assert!(
            prompt.contains("rebase") || prompt.contains("PR"),
            "with-plan prompt should mention rebase/PR choice"
        );
    }

    #[test]
    fn build_prompt_without_plan_omits_wrap_up() {
        let prompt = build_prompt(TaskId(7), "Title", "Desc", None, None);
        assert!(
            !prompt.contains("/wrap-up"),
            "no-plan prompt should not mention /wrap-up (agent isn't implementing yet)"
        );
    }

    #[test]
    fn build_prompt_without_plan_includes_planning_instruction() {
        let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None);
        assert!(
            prompt.contains("docs/plans/"),
            "no-plan prompt should instruct agent to write a plan"
        );
        assert!(
            prompt.contains("update_task"),
            "no-plan prompt should instruct agent to attach plan via MCP"
        );
        assert!(
            prompt.contains("ask") || prompt.contains("permission") || prompt.contains("proceed"),
            "no-plan prompt should ask for permission before implementing"
        );
    }

    #[test]
    fn build_prompt_without_plan_mentions_brainstorm_if_vague() {
        let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None);
        assert!(
            prompt.contains("/brainstorming"),
            "no-plan prompt should mention /brainstorming for vague descriptions"
        );
        assert!(
            prompt.contains("vague"),
            "no-plan prompt should mention vagueness as the condition for brainstorming"
        );
    }

    #[test]
    fn build_prompt_without_plan_mentions_direct_plan_alternative() {
        let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None);
        assert!(
            prompt.contains("implementation plan directly"),
            "no-plan prompt should offer writing a plan directly for clear descriptions"
        );
    }

    #[test]
    fn build_prompt_with_plan_asks_permission_before_implementing() {
        let prompt = build_prompt(TaskId(1), "Task", "Desc", Some("docs/plans/plan.md"), None);
        assert!(prompt.contains("docs/plans/plan.md"));
        assert!(
            prompt.contains("Shall I proceed")
                || prompt.contains("permission")
                || prompt.contains("proceed"),
            "with-plan prompt should ask for permission before implementing"
        );
        assert!(
            !prompt.contains("step by step"),
            "with-plan prompt should not say 'Follow it step by step' — agent reviews first"
        );
    }

    #[test]
    fn build_prompt_mentions_mcp_tools() {
        let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None);
        assert!(
            prompt.contains("dispatch MCP tools"),
            "standard dispatch prompt should mention MCP tools"
        );
    }

    #[test]
    fn is_wrappable_running_with_worktree() {
        let task = Task {
            status: TaskStatus::Running,
            worktree: Some("/tmp/wt".to_string()),
            ..make_task("/repo")
        };
        assert!(is_wrappable(&task));
    }

    #[test]
    fn is_wrappable_review_with_worktree() {
        let task = Task {
            status: TaskStatus::Review,
            worktree: Some("/tmp/wt".to_string()),
            ..make_task("/repo")
        };
        assert!(is_wrappable(&task));
    }

    #[test]
    fn is_wrappable_running_without_worktree() {
        let task = Task {
            status: TaskStatus::Running,
            worktree: None,
            ..make_task("/repo")
        };
        assert!(!is_wrappable(&task));
    }

    #[test]
    fn is_wrappable_backlog_with_worktree() {
        let task = Task {
            status: TaskStatus::Backlog,
            worktree: Some("/tmp/wt".to_string()),
            ..make_task("/repo")
        };
        assert!(!is_wrappable(&task));
    }

    #[test]
    fn validate_repo_path_existing_dir() {
        assert!(validate_repo_path("/tmp").is_ok());
    }

    #[test]
    fn validate_repo_path_nonexistent() {
        let result = validate_repo_path("/nonexistent/path");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn validate_repo_path_not_a_dir() {
        let result = validate_repo_path("/etc/hostname");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Not a directory"));
    }

    #[test]
    fn resume_window_name_matches_dispatch() {
        // The resume window name should use the same naming convention as dispatch
        assert_eq!(build_tmux_window_name(TaskId(42)), "task-42");
    }

    #[test]
    fn build_prompt_includes_plan_path() {
        let prompt = build_prompt(
            TaskId(1),
            "Task",
            "Desc",
            Some("docs/plans/my-plan.md"),
            None,
        );
        assert!(prompt.contains("Plan: docs/plans/my-plan.md"));
    }

    #[test]
    fn build_prompt_without_plan_omits_plan_section() {
        let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None);
        assert!(!prompt.contains("Plan:"));
    }

    #[test]
    fn build_quick_dispatch_prompt_includes_planning_instruction() {
        let prompt = build_quick_dispatch_prompt(TaskId(42), "Quick task", "", None);
        assert!(
            prompt.contains("docs/plans/") || prompt.contains("plan"),
            "quick dispatch prompt should instruct agent to write a plan before implementing"
        );
        assert!(
            prompt.contains("ask") || prompt.contains("permission") || prompt.contains("proceed"),
            "quick dispatch prompt should ask for permission before implementing"
        );
    }

    #[test]
    fn build_quick_dispatch_prompt_contains_rename_instruction() {
        let prompt = build_quick_dispatch_prompt(TaskId(42), "Quick task", "", None);
        assert!(prompt.contains("42"));
        assert!(prompt.contains("Quick task"));
        assert!(prompt.contains("update_task"));
        assert!(prompt.contains("title"));
        assert!(prompt.contains("placeholder"));
    }

    #[test]
    fn build_quick_dispatch_prompt_mentions_mcp() {
        let prompt = build_quick_dispatch_prompt(TaskId(1), "Quick task", "", None);
        assert!(prompt.contains("dispatch MCP tools"));
        assert!(prompt.contains("update_task"));
        assert!(!prompt.contains("add_note"));
    }

    #[test]
    fn build_quick_dispatch_prompt_differs_from_regular() {
        let regular = build_prompt(TaskId(1), "Task", "Desc", None, None);
        let quick = build_quick_dispatch_prompt(TaskId(1), "Task", "Desc", None);
        assert!(quick.contains("placeholder"));
        assert!(!regular.contains("placeholder"));
    }

    #[test]
    fn build_quick_dispatch_prompt_includes_epic_context() {
        let ctx = EpicContext {
            epic_id: EpicId(7),
            epic_title: "My Epic".to_string(),
            sibling_summaries: vec!["[2] Sibling (running)".to_string()],
        };
        let prompt = build_quick_dispatch_prompt(TaskId(42), "Quick task", "", Some(&ctx));
        assert!(prompt.contains("EpicId: 7"), "should include epic ID");
        assert!(prompt.contains("My Epic"), "should include epic title");
        assert!(
            prompt.contains("Sibling"),
            "should include sibling summaries"
        );
    }

    #[test]
    fn rebase_preamble_prepended_to_all_prompts() {
        let body = build_prompt(TaskId(1), "Task", "Desc", None, None);
        let full = format!(
            "{}\n\n\
             Always work from this worktree folder — do not `cd` to the parent repo \
             or other directories.\n\n\
             {body}",
            rebase_preamble("origin/main")
        );
        assert!(full.contains("rebase your branch from origin/main"));
        assert!(full.starts_with("Before starting work"));
        assert!(full.contains("Always work from this worktree folder"));
    }

    #[test]
    fn build_brainstorm_prompt_contains_task_info() {
        let prompt =
            build_brainstorm_prompt(TaskId(7), "Design auth", "Rework the auth flow", None);
        assert!(prompt.contains("7"));
        assert!(prompt.contains("Design auth"));
        assert!(prompt.contains("Rework the auth flow"));
        assert!(prompt.contains("brainstorm"));
        assert!(prompt.contains("update_task"));
    }

    #[test]
    fn build_plan_prompt_contains_task_info() {
        let prompt = build_plan_prompt(TaskId(8), "Add feature", "Small improvement", None);
        assert!(prompt.contains("8"));
        assert!(prompt.contains("Add feature"));
        assert!(prompt.contains("Small improvement"));
        assert!(prompt.contains("/plan"));
        assert!(prompt.contains("update_task"));
    }

    #[test]
    fn build_plan_prompt_differs_from_brainstorm() {
        let plan = build_plan_prompt(TaskId(1), "T", "D", None);
        let brainstorm = build_brainstorm_prompt(TaskId(1), "T", "D", None);
        assert_ne!(plan, brainstorm);
        assert!(plan.contains("planning"));
        assert!(brainstorm.contains("brainstorm"));
    }

    #[test]
    fn brainstorm_prompt_omits_tdd() {
        let prompt = build_brainstorm_prompt(TaskId(7), "Design auth", "Rework auth", None);
        assert!(
            !prompt.contains("TDD"),
            "brainstorm prompt should not include TDD — no code is written at design stage"
        );
    }

    #[test]
    fn brainstorm_prompt_omits_clarifying_questions_opener() {
        let prompt = build_brainstorm_prompt(TaskId(7), "Design auth", "Rework auth", None);
        assert!(
            !prompt.contains("clarifying questions"),
            "brainstorm prompt should not have a clarifying-questions opener — /brainstorming skill handles it"
        );
    }

    #[test]
    fn all_planning_prompts_reference_brainstorming_skill() {
        let brainstorm = build_brainstorm_prompt(TaskId(1), "T", "D", None);
        let plan = build_plan_prompt(TaskId(1), "T", "D", None);
        let standard = build_prompt(TaskId(1), "T", "D", None, None);
        let quick = build_quick_dispatch_prompt(TaskId(1), "T", "D", None);

        for (name, prompt) in [
            ("brainstorm", brainstorm),
            ("plan", plan),
            ("standard-no-plan", standard),
            ("quick", quick),
        ] {
            assert!(
                prompt.contains("/brainstorming"),
                "{name} prompt should reference /brainstorming skill"
            );
        }
    }

    #[test]
    fn plan_and_attach_instruction_is_concise() {
        let instruction = plan_and_attach_instruction(TaskId(42));
        assert!(
            instruction.len() < 200,
            "plan_and_attach_instruction should be concise (< 200 chars), got {} chars",
            instruction.len()
        );
        assert!(instruction.contains("/brainstorming"));
        assert!(instruction.contains("update_task"));
        assert!(instruction.contains("docs/plans/"));
    }

    #[test]
    fn epic_preamble_returns_empty_strings_for_none() {
        let (id_line, section) = epic_preamble(None);
        assert!(id_line.is_empty());
        assert!(section.is_empty());
    }

    #[test]
    fn epic_preamble_returns_id_line_and_section_for_some() {
        let ctx = EpicContext {
            epic_id: EpicId(5),
            epic_title: "Auth Rework".to_string(),
            sibling_summaries: vec!["[3] Setup DB (done)".to_string()],
        };
        let (id_line, section) = epic_preamble(Some(&ctx));
        assert!(id_line.contains("EpicId: 5"));
        assert!(section.contains("Auth Rework"));
        assert!(section.contains("Setup DB"));
    }

    // --- ProcessRunner-based tests ---

    #[test]
    fn dispatch_reuses_existing_worktree() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        // Pre-create worktree dir — simulates a re-dispatch where the worktree
        // already exists on disk from a previous dispatch cycle.
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            // No detect_default_branch call — task.base_branch is used directly
            // git worktree add is skipped (dir exists)
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let task = make_task(&repo_path);
        dispatch_agent(&task, &mock, None).unwrap();

        let calls = mock.recorded_calls();
        assert!(
            calls
                .iter()
                .all(|(prog, args)| !(prog == "git" && args.iter().any(|a| a == "worktree"))),
            "git worktree add should be skipped for existing worktree"
        );
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1[0], "new-window");
        assert_eq!(calls[1].0, "tmux");
        assert_eq!(calls[1].1[0], "set-option");
        assert_eq!(calls[2].0, "tmux");
        assert_eq!(calls[2].1[0], "set-hook");
    }

    #[test]
    fn dispatch_sends_claude_command() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            // No detect_default_branch call — task.base_branch is used directly
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
            MockProcessRunner::ok(), // tmux send-keys -l (the claude command)
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let task = make_task(&repo_path);
        dispatch_agent(&task, &mock, None).unwrap();

        let calls = mock.recorded_calls();
        // The literal send-keys call (index 3) carries the claude invocation
        assert!(
            calls[3].1.iter().any(|a| a.contains("claude")),
            "send-keys should include claude"
        );
        assert!(
            calls[3]
                .1
                .iter()
                .any(|a| a.contains("--permission-mode plan")),
            "dispatch_agent send-keys should use plan mode"
        );
    }

    #[test]
    fn dispatch_agent_uses_plan_mode() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            // No detect_default_branch call — task.base_branch is used directly
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let task = make_task(&repo_path);
        dispatch_agent(&task, &mock, None).unwrap();

        let calls = mock.recorded_calls();
        let send_keys_arg = calls[3].1.iter().find(|a| a.contains("claude")).unwrap();
        assert!(
            send_keys_arg.contains("--permission-mode plan"),
            "dispatch_agent should use plan mode, got: {send_keys_arg}"
        );
    }

    #[test]
    fn plan_agent_uses_plan_mode() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            // No detect_default_branch call — task.base_branch is used directly
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let task = make_task(&repo_path);
        plan_agent(&task, &mock, None).unwrap();

        let calls = mock.recorded_calls();
        let send_keys_arg = calls[3].1.iter().find(|a| a.contains("claude")).unwrap();
        assert!(
            send_keys_arg.contains("--permission-mode plan"),
            "plan_agent should use plan mode, got: {send_keys_arg}"
        );
    }

    #[test]
    fn provision_worktree_creates_new_when_dir_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        // Do NOT pre-create the worktree dir — test the "create" path

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // git worktree add
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        ]);

        let task = make_task(&repo_path);
        let result = provision_worktree(&task, &mock, None).unwrap();

        let calls = mock.recorded_calls();
        assert_eq!(calls[0].0, "git", "first call should be git worktree add");
        assert!(calls[0].1.contains(&"worktree".to_string()));
        assert!(calls[0].1.contains(&"add".to_string()));
        assert_eq!(calls[1].0, "tmux");
        assert_eq!(calls[1].1[0], "new-window");

        let expected_path = format!("{repo_path}/.worktrees/42-fix-bug");
        assert_eq!(result.worktree_path, expected_path);
    }

    #[test]
    fn provision_worktree_skips_git_when_dir_exists() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        ]);

        let task = make_task(&repo_path);
        let result = provision_worktree(&task, &mock, None).unwrap();

        let calls = mock.recorded_calls();
        assert!(
            calls.iter().all(|(prog, _)| prog != "git"),
            "git should be skipped"
        );
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1[0], "new-window");
        assert_eq!(result.worktree_path, worktree_dir.to_str().unwrap());
    }

    #[test]
    fn provision_worktree_with_base_branch_passes_start_point() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // git worktree add (with base branch)
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        ]);

        let task = make_task(&repo_path);
        let result = provision_worktree(&task, &mock, Some("99-prev-task")).unwrap();

        let calls = mock.recorded_calls();
        assert_eq!(calls[0].0, "git");
        // The base branch should be the last arg to git worktree add
        let git_args = &calls[0].1;
        assert_eq!(
            git_args.last().unwrap(),
            "99-prev-task",
            "base branch should be last git arg, got: {git_args:?}"
        );

        let expected_path = format!("{repo_path}/.worktrees/42-fix-bug");
        assert_eq!(result.worktree_path, expected_path);
    }

    #[test]
    fn rebase_preamble_with_base_branch() {
        let preamble = rebase_preamble("99-prev-task");
        assert!(
            preamble.contains("99-prev-task"),
            "should reference the base branch"
        );
        assert!(
            !preamble.contains("origin/main"),
            "should not reference origin/main"
        );
    }

    #[test]
    fn rebase_preamble_uses_given_target() {
        let preamble = rebase_preamble("origin/develop");
        assert!(
            preamble.contains("origin/develop"),
            "should use given target, got: {preamble}"
        );
        assert!(
            !preamble.contains("origin/main"),
            "should not contain origin/main"
        );
    }

    #[test]
    fn resume_skips_git_issues_tmux_continue() {
        let dir = tempfile::TempDir::new().unwrap();
        let worktree_path = dir.path().to_str().unwrap().to_string();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 5);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1[0], "new-window");
        assert_eq!(calls[1].1[0], "set-option");
        assert_eq!(calls[2].1[0], "set-hook");
        assert!(
            calls.iter().all(|(prog, _)| prog != "git"),
            "resume should make no git calls"
        );
        assert!(calls[3].1.iter().any(|a| a.contains("--continue")));
    }

    #[test]
    fn cleanup_kills_window_and_removes_worktree() {
        let mock = MockProcessRunner::new(vec![
            // has_window: list-windows returns the window name in stdout
            MockProcessRunner::ok_with_stdout(b"task-42\n"),
            MockProcessRunner::ok(), // tmux kill-window
            MockProcessRunner::ok(), // git worktree remove
            MockProcessRunner::ok(), // git branch -D (best-effort)
        ]);

        cleanup_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            Some("task-42"),
            &mock,
        )
        .unwrap();

        let calls = mock.recorded_calls();
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1[0], "list-windows");
        assert_eq!(calls[1].0, "tmux");
        assert_eq!(calls[1].1[0], "kill-window");
        assert_eq!(calls[2].0, "git");
        // git worktree remove is invoked with -C <repo>
        assert!(calls[2].1.contains(&"-C".to_string()));
        assert!(calls[2].1.contains(&"remove".to_string()));
    }

    #[test]
    fn cleanup_succeeds_when_worktree_already_removed() {
        // When git says "not a working tree" the archive should still succeed,
        // not surface an error to the user.
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::fail("fatal: '/repo/.worktrees/42-fix-bug' is not a working tree"),
            MockProcessRunner::ok(), // git branch -D (best-effort)
        ]);

        cleanup_task("/repo", "/repo/.worktrees/42-fix-bug", None, &mock).unwrap();
    }

    #[test]
    fn dispatch_uses_task_base_branch_in_prompt() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            // No detect_default_branch call — task.base_branch is used directly
            // git worktree add is skipped (dir exists)
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let mut task = make_task(&repo_path);
        task.base_branch = "master".to_string();
        dispatch_agent(&task, &mock, None).unwrap();

        // Verify the prompt uses task.base_branch directly — no symbolic-ref call needed
        let prompt_file = worktree_dir.join(".claude-prompt");
        let prompt = std::fs::read_to_string(prompt_file).unwrap();
        assert!(
            prompt.contains("rebase your branch from master"),
            "prompt should reference task.base_branch (master), got: {prompt}"
        );
        assert!(
            !prompt.contains("rebase your branch from main"),
            "prompt should not reference main when task.base_branch is master"
        );
    }

    #[test]
    fn dispatch_fails_fast_if_git_fails() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::fail("not a git repo"), // git worktree add fails
        ]);

        let task = make_task(&repo_path);
        let result = dispatch_agent(&task, &mock, None);
        assert!(result.is_err());
        let calls = mock.recorded_calls();
        assert_eq!(
            calls.len(),
            1,
            "only git worktree add should have been called (no detect_default_branch)"
        );
    }

    #[test]
    fn brainstorm_reuses_existing_worktree() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            // No detect_default_branch call — task.base_branch is used directly
            // git worktree add is skipped (dir exists)
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let task = make_task(&repo_path);
        brainstorm_agent(&task, &mock, None).unwrap();

        let calls = mock.recorded_calls();
        assert!(
            calls
                .iter()
                .all(|(prog, args)| !(prog == "git" && args.iter().any(|a| a == "worktree"))),
            "git worktree add should be skipped for existing worktree"
        );
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1[0], "new-window");
    }

    #[test]
    fn brainstorm_sends_brainstorm_prompt() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            // No detect_default_branch call — task.base_branch is used directly
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let task = make_task(&repo_path);
        brainstorm_agent(&task, &mock, None).unwrap();

        // Verify the prompt file was written with brainstorm content
        let prompt_file = worktree_dir.join(".claude-prompt");
        let prompt = std::fs::read_to_string(prompt_file).unwrap();
        assert!(
            prompt.contains("brainstorm"),
            "prompt should mention brainstorming"
        );
        assert!(
            prompt.contains("/brainstorming"),
            "prompt should reference /brainstorming skill"
        );
    }

    #[test]
    fn quick_dispatch_reuses_existing_worktree() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            // No detect_default_branch call — task.base_branch is used directly
            // git worktree add is skipped (dir exists)
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let task = make_task(&repo_path);
        quick_dispatch_agent(&task, &mock, None).unwrap();

        let calls = mock.recorded_calls();
        assert!(
            calls
                .iter()
                .all(|(prog, args)| !(prog == "git" && args.iter().any(|a| a == "worktree"))),
            "git worktree add should be skipped for existing worktree"
        );
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1[0], "new-window");
    }

    #[test]
    fn quick_dispatch_sends_rename_prompt() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            // No detect_default_branch call — task.base_branch is used directly
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let task = make_task(&repo_path);
        quick_dispatch_agent(&task, &mock, None).unwrap();

        let prompt_file = worktree_dir.join(".claude-prompt");
        let prompt = std::fs::read_to_string(prompt_file).unwrap();
        assert!(
            prompt.contains("placeholder"),
            "prompt should mention placeholder title"
        );
        assert!(
            prompt.contains("update_task"),
            "prompt should mention update_task for rename"
        );
    }

    // --- finish_task tests ---

    #[test]
    fn epic_planning_prompt_contains_epic_context() {
        let prompt =
            build_epic_planning_prompt(EpicId(42), "Redesign auth", "Rework the login flow");
        assert!(prompt.contains("42"));
        assert!(prompt.contains("Redesign auth"));
        assert!(prompt.contains("Rework the login flow"));
        assert!(prompt.contains("Do NOT start implementing"));
        // Work package instructions
        assert!(
            prompt.contains("create_task"),
            "prompt should instruct using create_task"
        );
        assert!(
            prompt.contains("sort_order"),
            "prompt should explain sort_order for ordering"
        );
        assert!(
            prompt.contains("update_epic"),
            "prompt should instruct attaching plan to epic"
        );
        assert!(
            prompt.contains("repo_path"),
            "prompt should explain repo_path for parallelization"
        );
        assert!(
            prompt.contains("epic_id=42"),
            "update_epic call should include the resolved epic id"
        );
        assert!(
            prompt.contains("/brainstorm"),
            "prompt should direct agent to use the brainstorm skill"
        );
        assert!(
            prompt.contains("work package"),
            "prompt should use 'work package' terminology"
        );
    }

    #[test]
    fn finish_task_happy_path() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
            MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // remote get-url origin
            MockProcessRunner::ok(), // git pull origin main
            MockProcessRunner::ok(), // git rebase main (from worktree)
            MockProcessRunner::ok(), // git merge --ff-only (fast-forward main)
            // Only tmux kill (worktree preserved for archival):
            MockProcessRunner::ok_with_stdout(b"task-42\n"), // tmux list-windows (has_window)
            MockProcessRunner::ok(),                         // tmux kill-window
        ]);

        finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "main",
            Some("task-42"),
            &mock,
        )
        .unwrap();

        let calls = mock.recorded_calls();
        assert!(calls.iter().any(|c| c.1.contains(&"rebase".to_string())));
        assert!(calls.iter().any(|c| c.1.contains(&"--ff-only".to_string())));
        // No worktree removal
        assert!(!calls.iter().any(|c| c.1.contains(&"remove".to_string())));
    }

    #[test]
    fn finish_task_with_master_default_branch() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"master\n"), // rev-parse HEAD
            MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // remote get-url origin
            MockProcessRunner::ok(), // git pull origin master
            MockProcessRunner::ok(), // git rebase master (from worktree)
            MockProcessRunner::ok(), // git merge --ff-only
        ]);

        finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "master",
            None,
            &mock,
        )
        .unwrap();

        let calls = mock.recorded_calls();
        // pull should reference "master" not "main"
        let pull_call = calls
            .iter()
            .find(|c| c.1.contains(&"pull".to_string()))
            .unwrap();
        assert!(pull_call.1.contains(&"master".to_string()));
        // rebase should reference "master"
        let rebase_call = calls
            .iter()
            .find(|c| c.1.contains(&"rebase".to_string()))
            .unwrap();
        assert!(rebase_call.1.contains(&"master".to_string()));
    }

    #[test]
    fn finish_task_not_on_default_branch() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"feature-branch\n"), // rev-parse HEAD
        ]);

        let result = finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "main",
            None,
            &mock,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, FinishError::NotOnDefaultBranch { .. }));
        assert!(err.to_string().contains("feature-branch"));
    }

    #[test]
    fn finish_task_rebase_conflict() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"main\n"),
            MockProcessRunner::fail(""),                         // remote get-url (no remote)
            Ok(Output {
                status: exit_fail(),
                stdout: b"".to_vec(),
                stderr: b"CONFLICT (content): Merge conflict in src/main.rs\nerror: could not apply abc1234\n".to_vec(),
            }),
            MockProcessRunner::ok(),                             // git rebase --abort
        ]);

        let result = finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "main",
            None,
            &mock,
        );
        assert!(matches!(
            result.unwrap_err(),
            FinishError::RebaseConflict(_)
        ));
        let calls = mock.recorded_calls();
        assert!(calls.last().unwrap().1.contains(&"--abort".to_string()));
    }

    #[test]
    fn finish_task_pull_fails() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"main\n"),
            MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // remote get-url origin
            MockProcessRunner::fail("fatal: unable to access remote"),           // git pull fails
        ]);

        let result = finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "main",
            None,
            &mock,
        );
        assert!(matches!(result.unwrap_err(), FinishError::Other(_)));
    }

    // --- create_pr tests ---

    #[test]
    fn create_pr_happy_path() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // git push
            MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url origin
            MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/42\n"), // gh pr create
        ]);

        let result = create_pr(
            "/repo",
            "42-fix-bug",
            "Fix bug",
            "A nasty crash",
            "main",
            &mock,
        )
        .unwrap();
        assert_eq!(result.pr_url, "https://github.com/org/repo/pull/42");

        let calls = mock.recorded_calls();
        assert_eq!(calls[0].0, "git");
        assert!(calls[0].1.contains(&"push".to_string()));
        assert!(calls[0].1.contains(&"-u".to_string()));
        assert_eq!(calls[1].0, "git");
        assert!(calls[1].1.contains(&"get-url".to_string()));
        assert_eq!(calls[2].0, "gh");
        assert!(calls[2].1.contains(&"create".to_string()));
        assert!(calls[2].1.contains(&"--draft".to_string()));
        assert!(calls[2].1.contains(&"org/repo".to_string()));
        // --head must include owner prefix to avoid gh resolving it in the wrong namespace
        assert!(
            calls[2].1.contains(&"org:42-fix-bug".to_string()),
            "--head must be owner:branch, got: {:?}",
            calls[2].1
        );
    }

    #[test]
    fn create_pr_head_ref_includes_owner_prefix() {
        // Regression: gh pr create --head branch (no owner) causes GitHub to resolve the
        // branch in the authenticated user's namespace instead of the --repo owner's namespace,
        // producing "Head sha can't be blank" errors. The fix is to always pass owner:branch.
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(),
            MockProcessRunner::ok_with_stdout(b"https://github.com/myorg/myrepo.git\n"),
            MockProcessRunner::ok_with_stdout(b"https://github.com/myorg/myrepo/pull/1\n"),
        ]);

        create_pr("/repo", "99-my-feature", "Feature", "desc", "main", &mock).unwrap();

        let calls = mock.recorded_calls();
        let gh_args = &calls[2].1;
        let head_idx = gh_args
            .iter()
            .position(|a| a == "--head")
            .expect("--head flag must be present");
        assert_eq!(
            gh_args[head_idx + 1],
            "myorg:99-my-feature",
            "--head value must be owner:branch"
        );
    }

    #[test]
    fn create_pr_with_master_base() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // git push
            MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url origin
            MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/42\n"), // gh pr create
        ]);

        create_pr("/repo", "42-fix-bug", "Fix bug", "desc", "master", &mock).unwrap();

        let calls = mock.recorded_calls();
        let gh_call = calls.iter().find(|c| c.0 == "gh").unwrap();
        assert!(gh_call.1.contains(&"master".to_string()));
    }

    #[test]
    fn create_pr_push_fails() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::fail("fatal: no remote"), // git push fails
        ]);

        let result = create_pr("/repo", "42-fix-bug", "Fix bug", "desc", "main", &mock);
        assert!(matches!(result, Err(PrError::PushFailed(_))));
    }

    #[test]
    fn create_pr_gh_create_fails() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // git push succeeds
            MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
            MockProcessRunner::fail("error: pull request already exists"),       // gh pr create
        ]);

        let result = create_pr("/repo", "42-fix-bug", "Fix bug", "desc", "main", &mock);
        assert!(matches!(result, Err(PrError::CreateFailed(_))));
    }

    // --- check_pr_status tests ---

    #[test]
    fn check_pr_status_open() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            b"OPEN\nREVIEW_REQUIRED\n",
        )]);
        let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
        assert_eq!(result.state, PrState::Open);
        assert_eq!(result.review_decision, Some(ReviewDecision::ReviewRequired));
    }

    #[test]
    fn check_pr_status_merged() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"MERGED\n")]);
        let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
        assert_eq!(result.state, PrState::Merged);
        assert_eq!(result.review_decision, None);
    }

    #[test]
    fn check_pr_status_closed() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"CLOSED\n")]);
        let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
        assert_eq!(result.state, PrState::Closed);
        assert_eq!(result.review_decision, None);
    }

    #[test]
    fn check_pr_status_open_approved() {
        let mock =
            MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"OPEN\nAPPROVED\n")]);
        let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
        assert_eq!(result.state, PrState::Open);
        assert_eq!(result.review_decision, Some(ReviewDecision::Approved));
    }

    #[test]
    fn check_pr_status_open_changes_requested() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            b"OPEN\nCHANGES_REQUESTED\n",
        )]);
        let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
        assert_eq!(result.state, PrState::Open);
        assert_eq!(
            result.review_decision,
            Some(ReviewDecision::ChangesRequested)
        );
    }

    // --- parse_repo_slug tests ---

    #[test]
    fn parse_repo_slug_ssh() {
        assert_eq!(
            parse_repo_slug("git@github.com:org/repo.git"),
            Some("org/repo".to_string()),
        );
    }

    #[test]
    fn parse_repo_slug_https() {
        assert_eq!(
            parse_repo_slug("https://github.com/org/repo.git"),
            Some("org/repo".to_string()),
        );
    }

    #[test]
    fn parse_repo_slug_no_git_suffix() {
        assert_eq!(
            parse_repo_slug("https://github.com/org/repo"),
            Some("org/repo".to_string()),
        );
    }

    #[test]
    fn finish_task_no_remote_skips_pull() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
            MockProcessRunner::fail(""),                  // remote get-url (no remote)
            MockProcessRunner::ok(),                      // git rebase main (from worktree)
            MockProcessRunner::ok(),                      // git merge --ff-only (fast-forward)
                                                          // No tmux window, no cleanup
        ]);

        finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "main",
            None,
            &mock,
        )
        .unwrap();
        let calls = mock.recorded_calls();
        // Should not have a "pull" call
        assert!(!calls.iter().any(|c| c.1.contains(&"pull".to_string())));
    }

    // --- new TDD tests for explicit base_branch ---

    #[test]
    fn finish_task_uses_explicit_base_branch_not_auto_detected() {
        // "develop" is passed explicitly; no symbolic-ref (detect_default_branch) call
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"develop\n"), // rev-parse HEAD → on develop
            MockProcessRunner::fail(""),                     // remote get-url (no remote)
            MockProcessRunner::ok(),                         // git rebase develop
            MockProcessRunner::ok(),                         // git merge --ff-only develop
        ]);

        finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "develop",
            None,
            &mock,
        )
        .unwrap();

        let calls = mock.recorded_calls();
        // No symbolic-ref call — branch was provided explicitly
        assert!(
            !calls
                .iter()
                .any(|c| c.0 == "git" && c.1.iter().any(|a| a == "symbolic-ref")),
            "symbolic-ref must not be called when base_branch is explicit"
        );
        // Rebase should target "develop"
        let rebase = calls
            .iter()
            .find(|c| c.1.contains(&"rebase".to_string()))
            .unwrap();
        assert!(rebase.1.contains(&"develop".to_string()));
    }

    #[test]
    fn create_pr_uses_explicit_base_branch_not_auto_detected() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // git push
            MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
            MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/1\n"), // gh pr create
        ]);

        create_pr("/repo", "42-fix-bug", "Fix bug", "desc", "develop", &mock).unwrap();

        let calls = mock.recorded_calls();
        // No symbolic-ref call
        assert!(
            !calls
                .iter()
                .any(|c| c.0 == "git" && c.1.iter().any(|a| a == "symbolic-ref")),
            "symbolic-ref must not be called when base_branch is explicit"
        );
        let gh_call = calls.iter().find(|c| c.0 == "gh").unwrap();
        assert!(
            gh_call.1.contains(&"develop".to_string()),
            "gh pr create should use explicit base_branch"
        );
    }

    #[test]
    fn dispatch_agent_uses_task_base_branch_in_prompt() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        // No detect_default_branch call expected — task.base_branch is used directly
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let mut task = make_task(&repo_path);
        task.base_branch = "develop".to_string();
        dispatch_agent(&task, &mock, None).unwrap();

        let prompt_file = worktree_dir.join(".claude-prompt");
        let prompt = std::fs::read_to_string(prompt_file).unwrap();
        assert!(
            prompt.contains("rebase your branch from develop"),
            "prompt should reference task.base_branch (develop), got: {prompt}"
        );
        // No symbolic-ref call
        let calls = mock.recorded_calls();
        assert!(
            !calls
                .iter()
                .any(|c| c.0 == "git" && c.1.iter().any(|a| a == "symbolic-ref")),
            "dispatch_agent must not call symbolic-ref when task.base_branch is set"
        );
    }

    // --- dispatch_review_agent tests ---

    fn review_req(
        repo_path: &str,
        number: i64,
        head_ref: &str,
        is_dependabot: bool,
    ) -> crate::tui::ReviewAgentRequest {
        crate::tui::ReviewAgentRequest {
            repo: repo_path.to_string(),
            github_repo: "acme/app".to_string(),
            number,
            head_ref: head_ref.to_string(),
            is_dependabot,
        }
    }

    #[test]
    fn review_agent_returns_early_when_window_exists() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let repo_short = dir.path().file_name().unwrap().to_str().unwrap();
        let tmux_window = format!("review-{repo_short}-99");

        let mock = MockProcessRunner::new(vec![
            // has_window: list-windows returns the window name
            MockProcessRunner::ok_with_stdout(tmux_window.as_bytes()),
        ]);

        let result =
            dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock)
                .unwrap();

        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1, "only list-windows should be called");
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1[0], "list-windows");
        assert_eq!(result.tmux_window, tmux_window);
        let expected_worktree = format!("{repo_path}/.worktrees/review-99");
        assert_eq!(result.worktree_path, expected_worktree);
    }

    #[test]
    fn review_agent_skips_worktree_add_when_dir_exists() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        // Pre-create the review worktree directory
        let worktree_dir = dir.path().join(".worktrees").join("review-99");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"other-window\n"), // has_window: no match
            MockProcessRunner::ok(),                              // git worktree prune
            MockProcessRunner::ok(),                              // git fetch origin feature-branch
            // git worktree add is skipped (dir exists)
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let result =
            dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock)
                .unwrap();

        let calls = mock.recorded_calls();
        assert!(
            calls
                .iter()
                .all(|(prog, args)| !(prog == "git" && args.iter().any(|a| a == "add"))),
            "git worktree add should be skipped when dir exists"
        );
        // git fetch should still happen
        assert_eq!(calls[2].0, "git");
        assert!(calls[2].1.contains(&"fetch".to_string()));
        assert!(calls[2].1.contains(&"feature-branch".to_string()));
        assert_eq!(result.worktree_path, worktree_dir.to_str().unwrap());
    }

    #[test]
    fn review_agent_happy_path_writes_prompt_and_launches_claude() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        // Pre-create worktree dir (simulates a previous fetch or existing
        // worktree — the mock git worktree add can't create dirs on disk)
        let worktree_dir = dir.path().join(".worktrees").join("review-99");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"other-window\n"), // has_window: no match
            MockProcessRunner::ok(),                              // git worktree prune
            MockProcessRunner::ok(),                              // git fetch origin feature-branch
            // git worktree add skipped (dir exists)
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let result =
            dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock)
                .unwrap();

        let calls = mock.recorded_calls();
        // Verify git fetch
        assert_eq!(calls[2].0, "git");
        assert!(calls[2].1.contains(&"fetch".to_string()));
        assert!(calls[2].1.contains(&"feature-branch".to_string()));
        // Verify tmux new-window
        assert_eq!(calls[3].0, "tmux");
        assert_eq!(calls[3].1[0], "new-window");
        // Verify send-keys includes claude command
        assert!(
            calls[4].1.iter().any(|a| a.contains("claude")),
            "send-keys should include claude command"
        );

        // Verify prompt file content
        let prompt_file = worktree_dir.join(".claude-prompt");
        let prompt = std::fs::read_to_string(prompt_file).unwrap();
        assert!(
            prompt.contains("PR #99"),
            "prompt should reference PR number"
        );
        assert!(
            prompt.contains("/anthropic-review-pr:review-pr 99"),
            "prompt should invoke fully qualified /anthropic-review-pr:review-pr skill"
        );
        assert!(
            prompt.contains("update_review_status"),
            "prompt should reference MCP tool"
        );

        let repo_short = dir.path().file_name().unwrap().to_str().unwrap();
        assert_eq!(result.tmux_window, format!("review-{repo_short}-99"));
    }

    #[test]
    fn review_agent_calls_worktree_add_when_dir_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        // Do NOT pre-create the review worktree directory

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"other-window\n"), // has_window: no match
            MockProcessRunner::ok(),                              // git worktree prune
            MockProcessRunner::ok(),                              // git fetch origin feature-branch
            MockProcessRunner::ok(),                              // git worktree add
            MockProcessRunner::ok(),                              // tmux new-window
                                                                  // fs::write will fail (mock worktree add doesn't create dir),
                                                                  // but we can still verify the calls made so far
        ]);

        // The function will error at fs::write since mock doesn't create the dir
        let result =
            dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock);
        assert!(result.is_err());

        let calls = mock.recorded_calls();
        // Verify git worktree add was called with correct args
        let wt_call = calls.iter().find(|(prog, args)| {
            prog == "git"
                && args.contains(&"add".to_string())
                && args.contains(&"worktree".to_string())
        });
        assert!(
            wt_call.is_some(),
            "git worktree add should be called when dir is missing"
        );
        let (_, args) = wt_call.unwrap();
        assert!(args.iter().any(|a| a == "origin/feature-branch"));
    }

    #[test]
    fn review_agent_fails_when_git_fetch_fails() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"\n"), // has_window: no match
            MockProcessRunner::ok(),                  // git worktree prune
            MockProcessRunner::fail("fatal: couldn't find remote ref"), // git fetch fails
        ]);

        let result =
            dispatch_review_agent(&review_req(&repo_path, 99, "nonexistent", false), &mock);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("git fetch failed"));
    }

    #[test]
    fn review_agent_uses_accept_edits_mode() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("review-99");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"\n"), // has_window: no match
            MockProcessRunner::ok(),                  // git worktree prune
            MockProcessRunner::ok(),                  // git fetch
            // worktree exists, skip add
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock).unwrap();

        let calls = mock.recorded_calls();
        let send_keys_arg = calls[4].1.iter().find(|a| a.contains("claude")).unwrap();
        assert!(
            send_keys_arg.contains("--permission-mode acceptEdits"),
            "review agent should use acceptEdits mode, got: {send_keys_arg}"
        );
    }

    // --- plugin-dir tests ---

    #[test]
    fn dispatch_agent_includes_plugin_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("42-fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            // No detect_default_branch call — task.base_branch is used directly
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        let task = make_task(&repo_path);
        dispatch_agent(&task, &mock, None).unwrap();

        let calls = mock.recorded_calls();
        let send_keys_arg = calls[3].1.iter().find(|a| a.contains("claude")).unwrap();
        assert!(
            send_keys_arg.contains("--plugin-dir"),
            "dispatch_agent should include --plugin-dir, got: {send_keys_arg}"
        );
        assert!(
            send_keys_arg.contains(".claude/plugins/local/dispatch"),
            "plugin-dir should point to local dispatch plugin, got: {send_keys_arg}"
        );
    }

    #[test]
    fn resume_agent_includes_plugin_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let worktree_path = dir.path().to_str().unwrap().to_string();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]);

        resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

        let calls = mock.recorded_calls();
        let send_keys_arg = calls[3].1.iter().find(|a| a.contains("claude")).unwrap();
        assert!(
            send_keys_arg.contains("--plugin-dir"),
            "resume_agent should include --plugin-dir, got: {send_keys_arg}"
        );
        assert!(
            send_keys_arg.contains(".claude/plugins/local/dispatch"),
            "plugin-dir should point to local dispatch plugin, got: {send_keys_arg}"
        );
    }

    #[test]
    fn review_agent_includes_plugin_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("review-99");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"\n"), // has_window: no match
            MockProcessRunner::ok(),                  // git worktree prune
            MockProcessRunner::ok(),                  // git fetch
            MockProcessRunner::ok(),                  // tmux new-window
            MockProcessRunner::ok(),                  // tmux send-keys -l
            MockProcessRunner::ok(),                  // tmux send-keys Enter
        ]);

        dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock).unwrap();

        let calls = mock.recorded_calls();
        let send_keys_arg = calls[4].1.iter().find(|a| a.contains("claude")).unwrap();
        assert!(
            send_keys_arg.contains("--plugin-dir"),
            "review_agent should include --plugin-dir, got: {send_keys_arg}"
        );
        assert!(
            send_keys_arg.contains(".claude/plugins/local/dispatch"),
            "plugin-dir should point to local dispatch plugin, got: {send_keys_arg}"
        );
    }

    // --- build_fix_prompt tests ---

    #[test]
    fn review_prompt_invokes_review_skill() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("review-42");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b""), // has_window: no match
            MockProcessRunner::ok(),                // git worktree prune
            MockProcessRunner::ok(),                // git fetch
            MockProcessRunner::ok(),                // tmux new-window
            MockProcessRunner::ok(),                // tmux send-keys -l
            MockProcessRunner::ok(),                // tmux send-keys Enter
        ]);
        dispatch_review_agent(&review_req(&repo_path, 42, "feature-branch", false), &mock).unwrap();

        let prompt = std::fs::read_to_string(worktree_dir.join(".claude-prompt")).unwrap();
        assert!(
            prompt.contains("/anthropic-review-pr:review-pr"),
            "prompt should invoke fully qualified /anthropic-review-pr:review-pr skill"
        );
        assert!(
            prompt.contains("update_review_status"),
            "prompt should reference MCP tool"
        );
        assert!(
            !prompt.contains("gh pr review"),
            "prompt should NOT tell agent to submit review directly"
        );
    }

    #[test]
    fn review_prompt_dependabot_mentions_dependency_update() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let worktree_dir = dir.path().join(".worktrees").join("review-42");
        std::fs::create_dir_all(&worktree_dir).unwrap();

        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b""), // has_window
            MockProcessRunner::ok(),                // git worktree prune
            MockProcessRunner::ok(),                // git fetch
            MockProcessRunner::ok(),                // tmux new-window
            MockProcessRunner::ok(),                // tmux send-keys -l
            MockProcessRunner::ok(),                // tmux send-keys Enter
        ]);
        dispatch_review_agent(&review_req(&repo_path, 42, "dependabot/npm", true), &mock).unwrap();

        let prompt = std::fs::read_to_string(worktree_dir.join(".claude-prompt")).unwrap();
        assert!(
            prompt.contains("dependency update"),
            "dependabot prompt should mention dependency update"
        );
        assert!(
            prompt.contains("update_review_status"),
            "prompt should reference MCP tool"
        );
    }

    fn fix_req(
        number: i64,
        kind: crate::models::AlertKind,
        title: &str,
        description: &str,
        package: Option<&str>,
        fixed_version: Option<&str>,
    ) -> crate::tui::FixAgentRequest {
        crate::tui::FixAgentRequest {
            repo: String::new(),
            github_repo: "acme/app".to_string(),
            number,
            kind,
            title: title.to_string(),
            description: description.to_string(),
            package: package.map(String::from),
            fixed_version: fixed_version.map(String::from),
        }
    }

    #[test]
    fn fix_prompt_includes_mcp_lifecycle_call() {
        let req = fix_req(
            42,
            crate::models::AlertKind::Dependabot,
            "CVE in lodash",
            "Prototype pollution",
            Some("lodash"),
            Some("4.17.21"),
        );
        let prompt = build_fix_prompt(&req);
        assert!(
            prompt.contains("update_review_status"),
            "fix prompt should include MCP lifecycle call"
        );
    }

    #[test]
    fn build_fix_prompt_dependabot() {
        let req = fix_req(
            42,
            crate::models::AlertKind::Dependabot,
            "CVE-2024-1234 in lodash",
            "Prototype pollution",
            Some("lodash"),
            Some("4.17.21"),
        );
        let prompt = build_fix_prompt(&req);
        assert!(prompt.contains("lodash"));
        assert!(prompt.contains("4.17.21"));
        assert!(prompt.contains("42"));
    }

    #[test]
    fn build_fix_prompt_code_scanning() {
        let req = fix_req(
            7,
            crate::models::AlertKind::CodeScanning,
            "SQL injection",
            "src/db.rs:42",
            None,
            None,
        );
        let prompt = build_fix_prompt(&req);
        assert!(prompt.contains("SQL injection"));
        assert!(prompt.contains("src/db.rs:42"));
    }
}

#[cfg(test)]
mod branch_tests {
    use super::*;

    #[test]
    fn branch_from_worktree_extracts_last_component() {
        assert_eq!(
            branch_from_worktree("/repo/.worktrees/42-fix-login"),
            Some("42-fix-login".to_string())
        );
    }

    #[test]
    fn branch_from_worktree_returns_none_for_empty() {
        assert_eq!(branch_from_worktree(""), None);
    }
}
