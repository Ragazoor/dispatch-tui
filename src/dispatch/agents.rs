use anyhow::{Context, Result};
use std::fs;

use crate::git::detect_default_branch;
use crate::models::{
    expand_tilde, AlertKind, DispatchResult, EpicId, ResumeResult, Task, TaskId, TaskStatus,
};
use crate::process::ProcessRunner;
use crate::tmux;
use crate::tui::{FixAgentRequest, ReviewAgentRequest};

use super::prompts::{
    build_dependabot_review_prompt, build_epic_planning_prompt, build_fix_task_prompt,
    build_pr_review_prompt, build_prompt, build_quick_dispatch_prompt, build_research_prompt,
    build_tmux_window_name, rebase_preamble, EpicContext, LearningInjections, ProjectContext,
    PromptContext, DISPATCH_PLUGIN_DIR,
};
use super::repo_map::{self, SystemCtagsExec};
use super::stderr_str;
use super::worktree::provision_worktree;

/// Provision worktree, generate the repo map, build the prompt, write the
/// prompt file, launch Claude via tmux.
///
/// The `make_prompt` closure receives the optional repo-map text generated
/// over the freshly-provisioned worktree. Splitting the build step into a
/// closure lets each agent variant compose its own context (learnings, plan,
/// etc.) while keeping the post-provision repo-map generation in one place.
///
/// `permission_mode` controls Claude's `--permission-mode` flag:
/// `None` launches in Claude's default (auto) mode, used by every task
/// agent except research. `Some("plan")` is used by the research agent so
/// investigation stays read-only. Review and fix agents use a separate
/// path (`provision_and_dispatch`) and pass `acceptEdits`.
fn dispatch_with_prompt(
    task: &Task,
    make_prompt: impl FnOnce(Option<String>) -> String,
    runner: &dyn ProcessRunner,
    base_branch: Option<&str>,
    permission_mode: Option<&str>,
) -> Result<DispatchResult> {
    if task.repo_path.is_empty() {
        anyhow::bail!(
            "Repository path is not set. Edit the task (press 'e') to set it before dispatching."
        );
    }
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

    let repo_map = {
        let s = repo_map::settings();
        if s.budget_tokens == 0 {
            None
        } else {
            repo_map::generate(
                &SystemCtagsExec,
                s.binary.as_ref(),
                std::path::Path::new(&provision.worktree_path),
                s.budget_tokens,
                s.timeout,
            )
        }
    };

    let prompt = make_prompt(repo_map);
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
    let permission_flag = match permission_mode {
        Some(mode) => format!(" --permission-mode {mode}"),
        None => String::new(),
    };
    let claude_cmd = format!(
        "bash -c 'prompt=$(cat .claude-prompt) && rm -f .claude-prompt \
         && claude {DISPATCH_PLUGIN_DIR}{permission_flag} \"$prompt\"'"
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
    project: Option<&ProjectContext>,
    injections: &LearningInjections<'_>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        |repo_map| {
            let ctx = ctx_with_map(injections, repo_map);
            build_prompt(
                task.id,
                &task.title,
                &task.description,
                task.plan_path.as_deref(),
                epic,
                project,
                &ctx,
            )
        },
        runner,
        Some(&task.base_branch),
        None,
    )
}

pub fn pr_review_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        |repo_map| {
            let ctx = PromptContext::with_map(LearningInjections::default(), repo_map);
            build_pr_review_prompt(task.id, &task.title, &task.description, epic, project, &ctx)
        },
        runner,
        Some(&task.base_branch),
        None,
    )
}

pub fn dependabot_review_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        |repo_map| {
            let ctx = PromptContext::with_map(LearningInjections::default(), repo_map);
            build_dependabot_review_prompt(
                task.id,
                &task.title,
                &task.description,
                epic,
                project,
                &ctx,
            )
        },
        runner,
        Some(&task.base_branch),
        None,
    )
}

pub fn research_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        |repo_map| {
            let ctx = PromptContext::with_map(LearningInjections::default(), repo_map);
            build_research_prompt(task.id, &task.title, &task.description, epic, project, &ctx)
        },
        runner,
        Some(&task.base_branch),
        Some("plan"),
    )
}

pub fn fix_task_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        |repo_map| {
            let ctx = PromptContext::with_map(LearningInjections::default(), repo_map);
            build_fix_task_prompt(task.id, &task.title, &task.description, epic, project, &ctx)
        },
        runner,
        Some(&task.base_branch),
        None,
    )
}

pub fn quick_dispatch_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
    injections: &LearningInjections<'_>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        |repo_map| {
            let ctx = ctx_with_map(injections, repo_map);
            build_quick_dispatch_prompt(
                task.id,
                &task.title,
                &task.description,
                epic,
                project,
                &ctx,
            )
        },
        runner,
        Some(&task.base_branch),
        None,
    )
}

pub fn epic_planning_agent(
    task: &Task,
    epic_id: EpicId,
    epic_title: &str,
    project: &ProjectContext,
    runner: &dyn ProcessRunner,
) -> Result<DispatchResult> {
    let epic = EpicContext {
        epic_id,
        epic_title: epic_title.to_string(),
    };
    dispatch_with_prompt(
        task,
        |repo_map| {
            let ctx = PromptContext::with_map(LearningInjections::default(), repo_map);
            build_epic_planning_prompt(
                task.id,
                &task.title,
                &task.description,
                &epic,
                project,
                &ctx,
            )
        },
        runner,
        Some(&task.base_branch),
        None,
    )
}

/// Re-borrow `LearningInjections` into a `PromptContext` carrying the
/// generated repo map. Inner `Vec<&Learning>` clones are cheap (pointer +
/// length copies) and let the agent functions keep their callers' lifetime.
fn ctx_with_map<'a>(
    injections: &'a LearningInjections<'a>,
    repo_map: Option<String>,
) -> PromptContext<'a> {
    PromptContext::with_map(
        LearningInjections {
            procedural: injections.procedural.clone(),
            tiered: injections.tiered.clone(),
        },
        repo_map,
    )
}

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
            let default_branch = detect_default_branch(&config.repo_path, runner);

            let _ = runner.run(
                "git",
                &["-C", &config.repo_path, "fetch", "origin", &default_branch],
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

    // Write prompt and launch Claude.
    // Uses `--permission-mode acceptEdits`: review and fix agents make direct code
    // changes and don't need interactive approval for every edit. Task agents use
    // `plan` mode instead (see dispatch_with_prompt).
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
    req: &ReviewAgentRequest,
    runner: &dyn ProcessRunner,
) -> Result<DispatchResult> {
    let prompt = format!(
        "Review PR #{} in {}.\n\n\
         Run `/anthropic-review-pr:review-pr {}` to perform a comprehensive code review.\n\n\
         Wait for the user.",
        req.number, req.github_repo, req.number
    );

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
pub fn build_fix_prompt(req: &FixAgentRequest) -> String {
    let repo = &req.github_repo;
    let number = req.number;
    match req.kind {
        AlertKind::Dependabot => {
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
                 Wait for the user.",
                req.title
            )
        }
        AlertKind::CodeScanning => {
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
                 Wait for the user.",
                req.title, req.description
            )
        }
    }
}

/// Dispatch a Claude agent to fix a security vulnerability in an isolated worktree.
pub fn dispatch_fix_agent(
    req: FixAgentRequest,
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

/// The fixed tmux window name used for the main claude session.
pub const MAIN_SESSION_WINDOW: &str = "dispatch-main";

/// Launch a plain interactive `claude` session in a new tmux window.
///
/// Unlike task agents, this session has no task context, no prompt file, and
/// no `--permission-mode` flag — it opens as a plain interactive Claude Code
/// session with dispatch plugins available.
///
/// Returns the name of the created tmux window.
pub fn create_main_session(dir: &str, runner: &dyn ProcessRunner) -> Result<String> {
    let window = MAIN_SESSION_WINDOW;

    tmux::new_window(window, dir, runner).context("failed to create main session tmux window")?;

    tmux::send_keys(window, &format!("claude {DISPATCH_PLUGIN_DIR}"), runner)
        .context("failed to send keys to main session tmux window")?;

    tracing::info!(%window, %dir, "main session created");

    Ok(window.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::process::MockProcessRunner;

    #[test]
    fn create_main_session_creates_tmux_window_in_given_dir() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // new-window
            MockProcessRunner::ok(), // send-keys -l
            MockProcessRunner::ok(), // send-keys Enter
        ]);
        let result = create_main_session("/home/user", &mock);
        assert!(result.is_ok());
        let window = result.unwrap();
        assert_eq!(window, MAIN_SESSION_WINDOW);

        let calls = mock.recorded_calls();
        // First call: tmux new-window
        assert!(calls[0].1.contains(&"new-window".to_string()));
        assert!(calls[0].1.iter().any(|a| a.contains("/home/user")));
        assert!(calls[0].1.iter().any(|a| a == MAIN_SESSION_WINDOW));
    }

    #[test]
    fn create_main_session_sends_claude_with_plugin_dir() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // new-window
            MockProcessRunner::ok(), // send-keys -l
            MockProcessRunner::ok(), // send-keys Enter
        ]);
        create_main_session("/home/user", &mock).unwrap();

        let calls = mock.recorded_calls();
        // send-keys call passes "claude <plugin_dir>" as the command
        let all_args: Vec<String> = calls.iter().flat_map(|(_, args)| args.clone()).collect();
        let has_plugin_dir = all_args
            .iter()
            .any(|a| a.contains("claude") && a.contains("--plugin-dir"));
        assert!(
            has_plugin_dir,
            "expected claude with plugin dir in send-keys, got: {all_args:?}"
        );
    }
}
