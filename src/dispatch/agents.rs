use anyhow::{Context, Result};
use std::fs;

use crate::git::detect_default_branch;
use crate::models::{expand_tilde, DispatchResult, EpicId, ResumeResult, Task, TaskId, TaskStatus};
use crate::process::ProcessRunner;
use crate::tmux;

use super::prompts::{
    build_epic_planning_prompt, build_prompt, build_quick_dispatch_prompt, build_research_prompt,
    build_tmux_window_name, rebase_preamble, EpicContext, LearningInjections, ProjectContext,
    PromptContext, DISPATCH_PLUGIN_DIR,
};
use super::worktree::provision_worktree;

/// Provision worktree, build the prompt, write the prompt file, launch Claude
/// via tmux.
///
/// The `make_prompt` closure builds the full prompt string for the agent.
/// Splitting the build step into a closure lets each agent variant compose its
/// own context (learnings, plan, etc.) while keeping the post-provision launch
/// logic in one place.
///
/// `permission_mode` controls Claude's `--permission-mode` flag:
/// `None` launches in Claude's default (auto) mode, used by every task
/// agent except research. `Some("plan")` is used by the research agent so
/// investigation stays read-only.
fn dispatch_with_prompt(
    task: &Task,
    make_prompt: impl FnOnce() -> String,
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

    let prompt = make_prompt();
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
    verify_command: Option<&str>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        || {
            let mut ctx = ctx_with_learnings(injections).with_verify(verify_command);
            ctx.tag = task.tag;
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

pub fn research_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
    verify_command: Option<&str>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        || {
            let ctx = PromptContext::default().with_verify(verify_command);
            build_research_prompt(task.id, &task.title, &task.description, epic, project, &ctx)
        },
        runner,
        Some(&task.base_branch),
        Some("plan"),
    )
}

pub fn quick_dispatch_agent(
    task: &Task,
    runner: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
    project: Option<&ProjectContext>,
    injections: &LearningInjections<'_>,
    verify_command: Option<&str>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        || {
            let ctx = ctx_with_learnings(injections).with_verify(verify_command);
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
    verify_command: Option<&str>,
) -> Result<DispatchResult> {
    let epic = EpicContext {
        epic_id,
        epic_title: epic_title.to_string(),
    };
    dispatch_with_prompt(
        task,
        || {
            let ctx = PromptContext::default().with_verify(verify_command);
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

/// Fetch the verify command for a repository path from the settings store.
///
/// Logs a warning and returns `None` if the DB lookup fails so callers can
/// proceed without a verify command rather than aborting dispatch.
pub async fn fetch_verify_command(
    db: &dyn crate::db::TaskStore,
    repo_path: &str,
) -> Option<String> {
    db.get_verify_command(repo_path).await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to load verify_command; proceeding without it");
        None
    })
}

/// Re-borrow `LearningInjections` into a `PromptContext`. Inner
/// `Vec<&Learning>` clones are cheap (pointer + length copies) and let the
/// agent functions keep their callers' lifetime.
fn ctx_with_learnings<'a>(injections: &'a LearningInjections<'a>) -> PromptContext<'a> {
    PromptContext::with_learnings(LearningInjections {
        procedural: injections.procedural.clone(),
        tiered: injections.tiered.clone(),
    })
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
