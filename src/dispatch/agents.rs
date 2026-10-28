use anyhow::{Context, Result};
use std::fs;

use crate::git::detect_default_branch;
use crate::models::{
    expand_tilde, DispatchResult, ResumeResult, Task, TaskId, TaskModel, TmuxWindow,
};
use crate::process::{ProcessRunner, SUBPROCESS_TIMEOUT};
use crate::tmux;

use super::allium_specs::repo_has_allium_specs;
use super::prompts::{
    build_and_record_injections, build_prompt, build_quick_dispatch_prompt, build_research_prompt,
    compose_prompt_head, select_preamble, EpicContext, LearningInjections, PromptContext,
    DISPATCH_PLUGIN_DIR,
};
use super::worktree::{provision_worktree, rollback_failed_provisioning, BaseRef, StartPoint};

/// The `--name` flag Claude Code's native cross-session messaging addresses
/// this task's agent by (`ListAgents`/`SendMessage`/`@mention`), e.g.
/// `--name atlas`.
///
/// The name is the current agent's (`AgentSessionIsNamedAfterTheAgent` in
/// docs/specs/agents.allium), not the tmux window's `task-<id>`: a task's
/// session is renamed by a resume under a new agent and keeps its window.
/// Claude Code accepts a `--name` that differs from the one a session started
/// under, on `--resume` and on `--continue`, so every agent that began as
/// `task-<id>` takes its agent name at its next resume.
/// `TaskService::record_peer_message_sent` resolves an incoming
/// `SendMessage`'s `to` field against the agent names, and still against
/// `task-<id>` for a session not resumed since.
/// Leading space included so callers can splice it directly after
/// [`DISPATCH_PLUGIN_DIR`] in a launch command string.
fn session_name_flag(agent_name: &str) -> String {
    format!(" --name {agent_name}")
}

/// The flags every TASK agent's `claude` command line carries: the plugin dir
/// and settings, the session name, the model, and this task's caller-identity
/// MCP config. The model is always named, never left to the operator's own
/// Claude Code default (`EveryTaskAgentLaunchesOnItsModel`).
///
/// One seam for both launch sites (`dispatch_with_prompt` and `resume_agent`),
/// because forgetting one of these is silent. `AgentCarriesItsOwnCallerIdentity`
/// (docs/specs/dispatch.allium) says the config is owed by every launch, and
/// `CallerIdentityDependsOnTheLaunch` (docs/specs/mcp-task-tools.allium) says a
/// launch that omits it is indistinguishable at the MCP end from a session that
/// is not an agent at all — so a third launcher composing its own flag string
/// would reintroduce exactly the bug this exists to fix, and nothing would say
/// so. Coming through here, forgetting it also drops `--plugin-dir` and
/// `--name`, which fails loudly.
fn agent_launch_flags(
    task_id: TaskId,
    agent_name: &str,
    model: TaskModel,
    worktree_path: &str,
    runner: &dyn ProcessRunner,
) -> LaunchFlags {
    LaunchFlags {
        base: format!(
            "{DISPATCH_PLUGIN_DIR}{} --model {model}",
            session_name_flag(agent_name)
        ),
        mcp_config: super::caller_identity::write_agent_mcp_config(runner, worktree_path, task_id)
            .map(|p| p.to_string_lossy().into_owned()),
    }
}

/// The flags of a task agent's `claude` command line, with the one value that
/// can hold arbitrary characters — the MCP config path, which lives under the
/// repo — kept raw so each launch site quotes it for its own shell layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LaunchFlags {
    /// Fixed flags (plugin dir, settings, session name); safe to splice anywhere.
    pub(super) base: String,
    /// Raw path of the caller-identity MCP config, when one was written.
    pub(super) mcp_config: Option<String>,
}

impl LaunchFlags {
    /// The flags as one command-line fragment under a single quoting layer, for
    /// a command typed straight into the pane's shell (resume).
    fn inline(&self) -> String {
        match &self.mcp_config {
            Some(path) => format!(
                "{} --mcp-config {}",
                self.base,
                crate::process::shell_quote(path)
            ),
            None => self.base.clone(),
        }
    }
}

/// Width of the `dispatch agent-tree` companion pane, as a percentage of the
/// agent window — narrower than [`tmux::split_window_horizontal`]'s 40%
/// (the board's own split-pane feature), since the agent's own `claude` CLI
/// output needs the room. Matches `agent_tree_pane_percent` in
/// docs/specs/agent-tree.allium.
const AGENT_TREE_PANE_PERCENT: u8 = 30;

/// The `dispatch` subcommand the companion pane runs. Spawn-side only: the pane
/// is recognised afterwards by the role marker written on it
/// ([`tmux::PANE_ROLE_OPTION`]), not by what it is running.
const AGENT_TREE_SUBCOMMAND: &str = "agent-tree";

/// The companion agent-tree pane in `window`, if it has one.
///
/// Identified by the role marker [`spawn_agent_tree_pane`] writes on the pane,
/// matched on its exact value: true whatever has focus, however many panes the
/// window has, and whatever any of them happen to be running. Both heuristics
/// this replaced failed on one of those — `tmux::inactive_pane_id` (removed; allow-phantom-symbol: history) asked "which
/// pane is not focused?" (the user must focus the companion pane to press a key
/// in it, and an editor pane makes a third), and matching `#{pane_start_command}`
/// re-derived identity from a command line, which had to be defended against a
/// pane merely *showing* a file named after this feature. See
/// docs/specs/agent-tree.allium's `HideAgentTreePane`, including the accepted
/// cost: a marker cannot be written to a pane that already exists, so a window
/// open when this changed has an unmarked companion pane for the rest of its life.
pub fn agent_tree_pane_id(
    window: &TmuxWindow,
    runner: &dyn ProcessRunner,
) -> Result<Option<String>> {
    let panes = tmux::pane_ids_with_option_value(
        window.as_str(),
        tmux::PANE_ROLE_OPTION,
        tmux::PANE_ROLE_AGENT_TREE,
        runner,
    )?;
    Ok(panes.into_iter().next())
}

/// Every pane in `window` that dispatch put there beside the agent's own: the
/// companion tree pane, and the editor pane opened from it.
///
/// Used by the split-pane pin path, which moves only the agent's own pane out and
/// must not leave the rest behind in a window nothing owns.
///
/// One tmux call, because that is the question being asked: not "the tree pane,
/// and the editor pane, and…" — which cost a lookup per role and a window
/// pre-resolution to keep them from each re-resolving the name — but "every pane
/// carrying a role at all". A future dispatch-created pane is covered the moment
/// it is marked, without this function being taught about it.
pub fn companion_pane_ids(window: &TmuxWindow, runner: &dyn ProcessRunner) -> Result<Vec<String>> {
    tmux::pane_ids_with_option(window.as_str(), tmux::PANE_ROLE_OPTION, runner)
}

/// Split the agent's tmux window and launch `dispatch agent-tree <task_id>`
/// in the new pane, right after the agent's own command has been sent (see
/// docs/specs/agent-tree.allium's `SplitAgentTreePaneOnAgentLaunch`).
///
/// `worktree` becomes the new pane's start directory. Naming it keeps the pane
/// out of the `after-split-window` correction hook, which would otherwise
/// respawn it and restart the `dispatch agent-tree` process this call just
/// launched — see `tmux::ensure_split_hook`. `None` is a soft fallback for a
/// window whose worktree could not be resolved: the hook then corrects the pane,
/// which costs one restart of a process that has barely started.
///
/// Best-effort: the companion pane is a decorative side view, not the
/// critical path — a failure here is logged and does not fail the caller's
/// dispatch/resume operation.
fn spawn_agent_tree_pane(
    tmux_window: &TmuxWindow,
    task_id: TaskId,
    worktree: Option<&str>,
    runner: &dyn ProcessRunner,
) {
    let id_arg = task_id.0.to_string();
    // Passed as argv (`split-window --`), so the binary needs no shell quoting.
    let dispatch_bin = runner.agent_binaries().dispatch;
    let pane = match tmux::split_window_horizontal_running(
        tmux_window.as_str(),
        AGENT_TREE_PANE_PERCENT,
        &[&dispatch_bin, AGENT_TREE_SUBCOMMAND, &id_arg],
        worktree,
        runner,
    ) {
        Ok(pane) => pane,
        Err(e) => {
            tracing::warn!(
                task_id = task_id.0,
                %tmux_window,
                error = %e,
                "failed to open agent-tree companion pane"
            );
            return;
        }
    };
    // Mark it for the lookups that come later ([`agent_tree_pane_id`],
    // [`companion_pane_ids`]). Best-effort like the split itself: the pane is open
    // and rendering either way, and only the *next* toggle suffers — it would read
    // the window as having no companion and split a second one. Same accepted gap
    // as the editor pane's own marker (docs/specs/agent-tree.allium:
    // `OneEditorPanePerAgentWindow`).
    if let Err(e) = tmux::set_pane_option(
        &pane,
        tmux::PANE_ROLE_OPTION,
        tmux::PANE_ROLE_AGENT_TREE,
        runner,
    ) {
        tracing::warn!(
            task_id = task_id.0,
            %pane,
            error = %e,
            "failed to mark the agent-tree companion pane with its role"
        );
    }
}

/// The worktree a tmux window belongs to, read back from the `@dispatch_dir`
/// option dispatch set when it created the window.
///
/// The toggle and resync paths are handed a window *name* and nothing else, so
/// this is how they recover a start directory for the split. Best-effort by
/// design: a failure here is logged and yields `None`, which
/// [`spawn_agent_tree_pane`] treats as "let the correction hook handle it"
/// rather than as a reason to skip the pane.
fn window_worktree(window: &TmuxWindow, runner: &dyn ProcessRunner) -> Option<String> {
    match tmux::window_dispatch_dir(window, runner) {
        Ok(dir) => dir,
        Err(e) => {
            tracing::warn!(
                %window,
                error = %e,
                "failed to read @dispatch_dir; opening companion pane without a start directory"
            );
            None
        }
    }
}

/// Toggle the companion agent-tree pane in the given tmux window: kill it if
/// currently shown, re-split and relaunch `dispatch agent-tree <task_id>` if
/// hidden. `window` is a tmux window name (e.g. "task-42"), resolved by
/// tmux's own `#{window_name}` expansion at the moment the global toggle key
/// was pressed — see docs/specs/agent-tree.allium's ToggleAgentTreePane
/// rules.
///
/// A no-op (not an error) for any window that isn't a task-agent window —
/// pressing the toggle key in the board TUI's own window does nothing.
pub fn toggle_agent_tree_pane(window: &TmuxWindow, runner: &dyn ProcessRunner) -> Result<()> {
    let Some(task_id) = window.task_id() else {
        return Ok(());
    };

    // One lookup, both roles. Asking per role would pay a round-trip each and,
    // worse, read the window at two different moments — the tree could be gone
    // by the time the diff pane was looked for.
    let roles = tmux::pane_roles(window.as_str(), tmux::PANE_ROLE_OPTION, runner)?;
    let shown = roles
        .iter()
        .any(|(_, role)| role == tmux::PANE_ROLE_AGENT_TREE);

    if !shown {
        let worktree = window_worktree(window, runner);
        spawn_agent_tree_pane(window, task_id, worktree.as_deref(), runner);
        return Ok(());
    }

    discard_agent_tree_panes(window, &roles, runner)
}

/// Kill every pane dispatch put in `window` for the agent tree — the tree
/// itself and any diff pane below it — and forget what the diff pane was
/// showing.
///
/// **Both lifecycle paths that retire a tree come through here.** A diff pane
/// outliving its tree is orphaned: nothing drives its open set, nothing
/// refreshes it, and neither the toggle nor a later resync will find it, since
/// both look the window up by the TREE's role and take the "no tree, so split
/// one" branch. That is the state `KillAgentTreeDiffPaneWithItsTree` in
/// docs/specs/agent-tree.allium forbids.
///
/// Taking `roles` — the whole `(pane, role)` listing, read once by the caller —
/// rather than re-querying is what keeps the tree and the diff pane read at the
/// same moment. A fourth role added later is retired here by construction,
/// which is why this is one function over a listing rather than a lookup per
/// role.
///
/// A failed kill of the TREE is returned, because that is the one the toggle's
/// caller can see: it is the difference between the pane going away and the key
/// appearing to do nothing. A failed kill of a diff pane is logged instead — the
/// tree is going either way, and a stray pane is not worth failing a toggle
/// over.
fn discard_agent_tree_panes(
    window: &TmuxWindow,
    roles: &[(String, String)],
    runner: &dyn ProcessRunner,
) -> Result<()> {
    // Diff panes first, so one cannot briefly outlive the tree that drives it.
    let mut had_diff_pane = false;
    let mut editor_panes: Option<Vec<String>> = None;
    for (pane, _) in roles
        .iter()
        .filter(|(_, role)| role == tmux::PANE_ROLE_DIFF)
    {
        had_diff_pane = true;
        // An editor pane may hold unsaved edits and outlives its tree
        // (`KeepEditorPaneWithPossiblyUnsavedWork`). When the lookup fails the
        // pane is kept: a stray pane costs less than lost work.
        let editors = editor_panes.get_or_insert_with(|| {
            tmux::pane_ids_with_option_value(
                window.as_str(),
                tmux::PANE_CONTENT_OPTION,
                tmux::PANE_CONTENT_EDITOR,
                runner,
            )
            .unwrap_or_else(|e| {
                tracing::warn!(%window, error = %e, "failed to tell which pane runs the editor");
                roles.iter().map(|(id, _)| id.clone()).collect()
            })
        });
        if editors.contains(pane) {
            continue;
        }
        if let Err(e) = tmux::kill_pane(pane, runner) {
            tracing::warn!(%window, %pane, error = %e, "failed to retire the diff pane");
        }
    }

    let mut tree_result = Ok(());
    for (pane, _) in roles
        .iter()
        .filter(|(_, role)| role == tmux::PANE_ROLE_AGENT_TREE)
    {
        if let Err(e) = tmux::kill_pane(pane, runner) {
            tree_result = Err(e);
        }
    }

    // No diff pane means nothing was open, so there is no set to clear and no
    // reason to pay a `show-options` round-trip resolving the worktree. This is
    // the common case: most toggles happen with no diff open.
    if !had_diff_pane {
        return tree_result;
    }
    if let Some(worktree) = window_worktree(window, runner) {
        // View state does not survive a restart of the renderer, exactly as the
        // cursor and the manual expansions do not, so a tree started later
        // begins bare.
        if let Err(e) = crate::agent_tree::open_set::clear_open_set(&worktree) {
            tracing::warn!(%window, error = %format!("{e:#}"), "failed to clear the open set");
        }
    }
    tree_result
}

/// Resync a tmux window's companion pane to the task its own name implies:
/// kill whatever is currently running there and relaunch
/// `dispatch agent-tree <task_id>` for the correct task.
///
/// Used after `swap-pane` rewrites a window's task identity (via rename)
/// without touching its companion pane — left alone, that pane would keep
/// rendering the previous occupant's file tree under the window's new name
/// (see docs/specs/agent-tree.allium's ToggleVsSplitPaneInteraction).
///
/// A no-op for any window that isn't a task-agent window, or that carries no
/// companion pane to begin with — best-effort throughout, like every other
/// agent-tree pane operation.
pub fn resync_agent_tree_pane(window: &TmuxWindow, runner: &dyn ProcessRunner) {
    let Some(task_id) = window.task_id() else {
        return;
    };
    let roles = match tmux::pane_roles(window.as_str(), tmux::PANE_ROLE_OPTION, runner) {
        Ok(roles) => roles,
        Err(e) => {
            tracing::warn!(
                %window,
                error = %e,
                "failed to check for companion pane before resync"
            );
            return;
        }
    };
    if !roles
        .iter()
        .any(|(_, role)| role == tmux::PANE_ROLE_AGENT_TREE)
    {
        return;
    }

    // The DIFF pane has to go too, and that is why this reads roles rather than
    // looking the tree up alone. It is running `dispatch agent-diff <the
    // previous occupant's task id>` against the previous worktree's open set;
    // left behind it would render another task's files under this window's new
    // name, and nothing would ever retire it — the toggle looks for a TREE and,
    // finding the fresh one this call is about to spawn, only ever kills that.
    if let Err(e) = discard_agent_tree_panes(window, &roles, runner) {
        tracing::warn!(
            %window,
            error = %e,
            "failed to kill stale agent-tree companion pane before resync"
        );
    }
    let worktree = window_worktree(window, runner);
    spawn_agent_tree_pane(window, task_id, worktree.as_deref(), runner);
}

/// The command a prompt-carrying launch sends to the agent's tmux window: read
/// `.claude-prompt`, delete it, and run `claude` with `launch_flags` and the
/// prompt.
///
/// `claude` must already be one shell word ([`AgentBinaries::claude_quoted`](crate::process::AgentBinaries::claude_quoted)).
/// It is passed as bash's `$0`, *after* the single-quoted script body rather
/// than inside it: inside, it would sit under two quoting layers (the pane's
/// shell strips the outer quotes, then bash parses what is left) and a path
/// with a space would need escaping twice to survive both. The MCP config path
/// takes the same route as `$1`, because it holds repo-controlled text.
///
/// The `--` before the prompt is normative, not tidiness
/// (`PromptIsSeparatedFromTheLaunchFlags` in docs/specs/dispatch.allium).
/// Claude Code's `--mcp-config` is variadic (`<configs...>`) and
/// [`agent_launch_flags`] ends with it, so without the separator the prompt is
/// consumed as a second MCP configuration, resolved as a file path relative to
/// the worktree, and the launch fails with "Invalid MCP configuration" before
/// the agent starts.
pub(super) fn prompt_launch_command(claude: &str, flags: &LaunchFlags) -> String {
    prompt_launch_command_with(claude, flags, "")
}

/// [`prompt_launch_command`] with `extra` flags after the fixed ones, e.g. the
/// `--continue` of a resume that carries a message.
fn prompt_launch_command_with(claude: &str, flags: &LaunchFlags, extra: &str) -> String {
    let base = format!("{}{extra}", flags.base);
    // The config path is a positional (`$1`) after the body, like `$0`, so it is
    // quoted once, outside the body, and never inside it.
    let (config_flag, config_arg) = match &flags.mcp_config {
        Some(path) => (
            " --mcp-config \"$1\"",
            format!(" {}", crate::process::shell_quote(path)),
        ),
        None => ("", String::new()),
    };
    format!(
        "bash -c 'prompt=$(cat .claude-prompt) && rm -f .claude-prompt \
         && \"$0\" {base}{config_flag} -- \"$prompt\"' {claude}{config_arg}"
    )
}

/// The prompt the agent receives: the preamble, the worktree-scope reminder,
/// then the variant's own prompt.
fn full_agent_prompt(head: &str, prompt: &str) -> String {
    format!(
        "{head}Always work from this worktree folder — do not `cd` to the parent repo \
         or other directories.\n\n\
         {prompt}"
    )
}

/// Write the prompt file and start `claude` in the provisioned tmux window,
/// rolling the provisioning back if either step fails.
fn launch_agent(
    task: &Task,
    agent_name: &str,
    repo_path: &str,
    provision: &super::worktree::ProvisionResult,
    full_prompt: &str,
    runner: &dyn ProcessRunner,
    teardown: &dyn ProcessRunner,
) -> Result<()> {
    let prompt_file = format!("{}/.claude-prompt", provision.worktree_path);
    let claude = runner.agent_binaries().claude_quoted();
    let launch_flags = agent_launch_flags(
        task.id,
        agent_name,
        task.launch_model(),
        &provision.worktree_path,
        runner,
    );
    let claude_cmd = prompt_launch_command(&claude, &launch_flags);

    // Anything failing here happens after the worktree and tmux window both
    // exist — a fresh worktree (this attempt's own `git worktree add`) and the
    // window this same provisioning call just opened must both be rolled back
    // so a re-dispatch of the same task takes the fresh path again, not the
    // reuse path with its weaker fetch guarantee. A reused worktree predates
    // this attempt and is never touched — see rollback_failed_provisioning.
    let launch: Result<()> = (|| {
        fs::write(&prompt_file, full_prompt)
            .with_context(|| format!("failed to write {prompt_file}"))?;
        tmux::send_keys(&provision.tmux_window, &claude_cmd, runner)
            .context("failed to send keys to tmux window")?;
        Ok(())
    })();
    if let Err(e) = launch {
        rollback_failed_provisioning(
            repo_path,
            &provision.worktree_path,
            // Provisioning succeeded, so this attempt owns the window.
            Some(&provision.tmux_window),
            provision.reused_worktree,
            teardown,
        );
        return Err(e);
    }
    Ok(())
}

/// Provision worktree, build the prompt, write the prompt file, launch Claude
/// via tmux.
///
/// The `make_prompt` closure builds the full prompt string for the agent.
/// Splitting the build step into a closure lets each agent variant compose its
/// own context (learnings, plan, etc.) while keeping the post-provision launch
/// logic in one place.
///
/// No `--permission-mode` flag is ever passed: every task agent launches in
/// Claude's default (auto) mode, so an unattended dispatch is not left parked
/// on an approval dialog. A mode that must not change code says so in its
/// prompt instead — see `build_research_prompt`. This is
/// `EveryTaskAgentLaunchesInAutoMode` in `docs/specs/dispatch.allium`.
fn dispatch_with_prompt(
    task: &Task,
    agent_name: &str,
    make_prompt: impl FnOnce() -> String,
    runner: &dyn ProcessRunner,
    teardown: &dyn ProcessRunner,
    base_branch: Option<&str>,
) -> Result<DispatchResult> {
    if task.repo_path.is_empty() {
        anyhow::bail!(
            "Repository path is not set. Edit the task (press 'e') to set it before dispatching."
        );
    }
    let repo_path = expand_tilde(&task.repo_path);

    // Resolve the repo's base branch (task base_branch or detected default).
    let resolved: String = base_branch
        .map(str::to_owned)
        .unwrap_or_else(|| detect_default_branch(&repo_path, runner));

    // Review tasks (pr-review / dependabot / renovate) that carry a PR URL base
    // their worktree on the PR's head branch so the agent sees the PR's code.
    // Soft-fall to the base branch on resolution failure or for fork PRs.
    let pr_branch: Option<String> = match (&task.tag, &task.url) {
        (Some(tag), Some(url)) if tag.is_review() && url.is_pr() => {
            super::pr_head_branch(&url.url, runner)
        }
        _ => None,
    };

    // A PR head is never compared against a local ref of the same name — a
    // review must see exactly the PR's code — so it picks its own `BaseRef`
    // variant. Borrowing throughout: `pr_branch` is still needed for
    // `select_preamble` after provisioning, and `resolved` is unused hereafter.
    let base_ref = match pr_branch.as_deref() {
        Some(branch) => BaseRef::PrHead(branch),
        None => BaseRef::Branch(&resolved),
    };

    let provision = provision_worktree(task, runner, teardown, Some(base_ref), SUBPROCESS_TIMEOUT)?;

    let preamble = select_preamble(
        pr_branch.as_deref(),
        provision.start_point.as_ref(),
        provision.reused_worktree,
    );
    let head = compose_prompt_head(&preamble, provision.fetch_warning.as_deref());

    let full_prompt = full_agent_prompt(&head, &make_prompt());
    launch_agent(
        task,
        agent_name,
        &repo_path,
        &provision,
        &full_prompt,
        runner,
        teardown,
    )?;

    spawn_agent_tree_pane(
        &provision.tmux_window,
        task.id,
        Some(provision.worktree_path.as_str()),
        runner,
    );

    tracing::info!(
        task_id = task.id.0,
        worktree = %provision.worktree_path,
        base = provision.start_point.as_ref().map(StartPoint::base),
        reused_worktree = provision.reused_worktree,
        "agent dispatched"
    );

    Ok(DispatchResult {
        worktree_path: provision.worktree_path,
        tmux_window: provision.tmux_window,
        reused_worktree: provision.reused_worktree,
    })
}

pub fn dispatch_agent(
    task: &Task,
    agent_name: &str,
    runner: &dyn ProcessRunner,
    teardown: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
    injections: &LearningInjections<'_>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        agent_name,
        || {
            let ctx = PromptContext {
                learnings: injections.clone(),
                agent_name,
                tag: task.tag,
                auto_run_plan: task.auto_run_plan,
                has_allium_specs: repo_has_allium_specs(&task.repo_path),
                // Only a pr-typed url is passed through: the dependabot runbook
                // hands it straight to `gh pr`, and a security-alert or issue
                // url there would produce five failing calls.
                pr_url: task
                    .url
                    .as_ref()
                    .filter(|u| u.is_pr())
                    .map(|u| u.url.as_str()),
                // Only a feed sets external_id, so this is provenance, not a
                // proxy for it. The dependabot runbook tells the agent the PR
                // author was already filtered, which is only true of a task a
                // feed created.
                from_feed: task.external_id.is_some(),
                fix_pr_linked: task.labels.iter().any(|l| l == "fix-pr"),
            };
            build_prompt(
                task.id,
                &task.title,
                &task.description,
                task.plan_path.as_deref(),
                epic,
                &ctx,
            )
        },
        runner,
        teardown,
        Some(&task.base_branch),
    )
}

pub fn research_agent(
    task: &Task,
    agent_name: &str,
    runner: &dyn ProcessRunner,
    teardown: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        agent_name,
        || {
            build_research_prompt(
                task.id,
                &task.title,
                &task.description,
                epic,
                &PromptContext {
                    agent_name,
                    ..PromptContext::default()
                },
            )
        },
        runner,
        teardown,
        Some(&task.base_branch),
    )
}

pub fn quick_dispatch_agent(
    task: &Task,
    agent_name: &str,
    runner: &dyn ProcessRunner,
    teardown: &dyn ProcessRunner,
    epic: Option<&EpicContext>,
    injections: &LearningInjections<'_>,
) -> Result<DispatchResult> {
    dispatch_with_prompt(
        task,
        agent_name,
        || {
            let ctx = PromptContext {
                learnings: injections.clone(),
                agent_name,
                has_allium_specs: repo_has_allium_specs(&task.repo_path),
                fix_pr_linked: task.labels.iter().any(|l| l == "fix-pr"),
                ..PromptContext::default()
            };
            build_quick_dispatch_prompt(task.id, &task.title, &task.description, epic, &ctx)
        },
        runner,
        teardown,
        Some(&task.base_branch),
    )
}

/// Launch the agent [`DispatchMode`](crate::models::DispatchMode) selects,
/// consuming the prologue's [`DispatchInputs`].
///
/// The one place that match lives. It used to be written out once in the MCP
/// handler and once in the TUI runtime's blocking closure, which is how the two
/// could have drifted into launching different agents for the same mode.
pub fn run_agent_for_mode(
    task: &Task,
    mode: crate::models::DispatchMode,
    runner: &dyn ProcessRunner,
    teardown: &dyn ProcessRunner,
    inputs: DispatchInputs,
) -> Result<DispatchResult> {
    let DispatchInputs {
        epic_ctx,
        injected,
        agent_name,
    } = inputs;
    let injections = LearningInjections::from(injected.as_slice());
    match mode {
        crate::models::DispatchMode::Dispatch => dispatch_agent(
            task,
            &agent_name,
            runner,
            teardown,
            epic_ctx.as_ref(),
            &injections,
        ),
        crate::models::DispatchMode::Research => {
            research_agent(task, &agent_name, runner, teardown, epic_ctx.as_ref())
        }
        crate::models::DispatchMode::Quick => quick_dispatch_agent(
            task,
            &agent_name,
            runner,
            teardown,
            epic_ctx.as_ref(),
            &injections,
        ),
    }
}

/// The two per-task reads every dispatch entry point performs before it can
/// build a prompt: the epic banner, and the learnings to inject (recording each
/// retrieval as it goes).
///
/// Since store-split M4 every launch path is a dispatch request row, and the one
/// place that provisions — `TaskService::serve_dispatch_request` — runs this
/// prologue for all of them.
///
/// Deliberately carries no verify command — see `fetch_verify_command` below.
pub struct DispatchInputs {
    pub epic_ctx: Option<EpicContext>,
    pub injected: Vec<crate::models::Learning>,
    /// The task's current agent's name: the session's `--name` and what the
    /// prompt tells the agent it is called.
    pub agent_name: String,
}

/// Run the dispatch prologue for `task`, reading its epic context from the DB.
///
/// Async and side-effecting (it records prompt-injection retrievals), so callers
/// run it before handing the task to the blocking dispatch itself.
///
/// Fails when the task has no current agent: the claim creates one in the same
/// write that starts the request (`StartAgentOnFreshLaunch`), so a request
/// being served without one is a broken store, and naming the session after
/// something else would hide it.
pub async fn prepare_inputs(
    db: &dyn crate::store::TaskReadStore,
    task: &Task,
    emb_svc: &std::sync::Arc<crate::embeddings::EmbeddingService>,
) -> Result<DispatchInputs> {
    let agent_name = current_agent_name(db, task.id).await?;
    let epic_ctx = EpicContext::from_db(task, db).await;
    let injected = build_and_record_injections(db, task, emb_svc).await;
    Ok(DispatchInputs {
        epic_ctx,
        injected,
        agent_name,
    })
}

/// The name of `task`'s current agent, the one a launch or resume names the
/// session after (`AgentSessionIsNamedAfterTheAgent`).
pub async fn current_agent_name(
    db: &dyn crate::store::TaskReadStore,
    task: TaskId,
) -> Result<String> {
    db.current_agent(task)
        .await
        .map(|agent| agent.name)
        .with_context(|| format!("task {task} has no current agent to name the session after"))
}

/// Fetch the verify command for a repository path from the settings store.
///
/// Called by `get_task` (its "Verify command" line) and by the `wrap_up`
/// handler, which only checks whether a rebase answer should tell the agent to
/// re-run it. No prompt builder reads it, so this has no caller on a dispatch
/// path despite living here. Returns `None`
/// when no verify command is configured.
pub async fn fetch_verify_command(
    db: &dyn crate::store::TaskReadStore,
    repo_path: &str,
) -> Option<String> {
    db.get_verify_command(repo_path).await
}

/// Re-open a tmux window for an existing worktree and resume the most recent
/// Claude conversation with `claude --continue`.
///
/// `task.tmux_window = null` is a persisted-field precondition, not proof
/// that no live tmux window exists: the field can go stale (a race, or a
/// persist that never landed) while the window it used to name survives. So
/// before creating anything, this checks liveness live against tmux on the
/// deterministic window name; if a window already answers to it, this
/// returns immediately without creating a second one, sending keys, or
/// spawning a companion pane — reattachment then happens the normal way, via
/// the existing "has a window" branch of Space on the next keypress. A query
/// failure is returned as an error: creating blind could duplicate a live
/// window, and the caller can retry.
/// See `docs/specs/dispatch.allium`'s `ResumeTask` rule.
///
/// This function is **synchronous** and should be called via
/// `tokio::task::spawn_blocking` from async contexts.
pub fn resume_agent(
    task_id: TaskId,
    agent_name: &str,
    model: TaskModel,
    worktree_path: &str,
    runner: &dyn ProcessRunner,
) -> Result<ResumeResult> {
    resume_agent_with_message(task_id, agent_name, model, worktree_path, None, runner)
}

/// [`resume_agent`] that hands the resumed session `message` as its first
/// input (`dispatch.allium: ResumeTask`). The message is the positional
/// prompt of the `--continue` launch, read from `.claude-prompt` like a
/// dispatch prompt, so it is never typed into the pane. A live window means
/// nothing is launched and the message is dropped.
pub fn resume_agent_with_message(
    task_id: TaskId,
    agent_name: &str,
    model: TaskModel,
    worktree_path: &str,
    message: Option<&str>,
    runner: &dyn ProcessRunner,
) -> Result<ResumeResult> {
    let tmux_window = TmuxWindow::for_task(task_id);

    if tmux::has_window(&tmux_window, runner)
        .context("failed to check for a live tmux window before resume")?
    {
        tracing::info!(
            task_id = task_id.0,
            %tmux_window,
            "resume found a live tmux window; reattaching instead of duplicating"
        );
        return Ok(ResumeResult { tmux_window });
    }

    tmux::new_window(
        &tmux_window,
        worktree_path,
        &super::sccache_launch_env(runner),
        runner,
    )
    .context("failed to create tmux window for resume")?;

    tmux::set_window_dispatch_dir(&tmux_window, worktree_path, runner)
        .context("failed to set tmux window dispatch dir")?;
    tmux::ensure_split_hook(runner).context("failed to ensure tmux split hook")?;

    let claude = runner.agent_binaries().claude_quoted();
    // Below the live-window early return above, deliberately: these flags cost
    // a config write, and a resume that only reattaches launches nothing.
    let launch_flags = agent_launch_flags(task_id, agent_name, model, worktree_path, runner);
    let launch = match message {
        Some(message) => {
            let prompt_file = format!("{worktree_path}/.claude-prompt");
            fs::write(&prompt_file, message)
                .with_context(|| format!("failed to write {prompt_file}"))?;
            prompt_launch_command_with(&claude, &launch_flags, " --continue")
        }
        None => format!("{claude} {} --continue", launch_flags.inline()),
    };
    tmux::send_keys(&tmux_window, &launch, runner)
        .context("failed to send resume keys to tmux window")?;

    spawn_agent_tree_pane(&tmux_window, task_id, Some(worktree_path), runner);

    tracing::info!(task_id = task_id.0, %tmux_window, "agent resumed");

    Ok(ResumeResult { tmux_window })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::test_tmux_window;
    use crate::process::{AgentBinaries, MockProcessRunner};

    #[test]
    fn resync_agent_tree_pane_noop_for_non_task_window() {
        let mock = MockProcessRunner::new(vec![]);
        resync_agent_tree_pane(&test_tmux_window("edit-window"), &mock);
        assert!(
            mock.recorded_calls().is_empty(),
            "no tmux calls expected for a non-task window name"
        );
    }

    #[test]
    fn resync_agent_tree_pane_noop_when_no_companion_pane() {
        let mock = MockProcessRunner::new(vec![
            // list-panes: one pane, running a plain shell — no companion
            MockProcessRunner::ok_with_stdout(b"%1 \n"),
        ]);
        resync_agent_tree_pane(&test_tmux_window("task-5"), &mock);
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1, "only the companion check should run");
    }

    /// The companion pane's binary comes from the runner, not a literal — the
    /// seam that lets the real-tmux harness point it at a stub without shadowing
    /// `dispatch` on `PATH`. Nothing about *finding* the pane depends on that
    /// path any more: the role marker is written on the pane whatever it exec'd.
    #[test]
    fn spawn_agent_tree_pane_launches_the_runners_dispatch_binary() {
        let mock = MockProcessRunner::new(vec![
            // list-panes: the companion is %9
            MockProcessRunner::ok_with_stdout(b"%1 \n%9 agent_tree\n"),
            MockProcessRunner::ok(),                     // kill-pane
            MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options @dispatch_dir
            MockProcessRunner::ok_with_stdout(b"%20\n"), // split-window
            MockProcessRunner::ok(),                     // set-option: the role marker
        ])
        .with_agent_binaries(AgentBinaries::stub());

        resync_agent_tree_pane(&test_tmux_window("task-5"), &mock);

        let split = &mock.recorded_calls()[3].1;
        assert!(
            split.contains(&"/stub/bin/dispatch-stub".to_string()),
            "companion pane must exec the runner's dispatch binary, got: {split:?}"
        );
    }

    #[test]
    fn resync_agent_tree_pane_kills_and_respawns_companion() {
        let mock = MockProcessRunner::new(vec![
            // list-panes: the companion is %9
            MockProcessRunner::ok_with_stdout(b"%1 \n%9 agent_tree\n"),
            MockProcessRunner::ok(),                     // kill-pane %9
            MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options @dispatch_dir
            MockProcessRunner::ok_with_stdout(b"%20\n"), // split-window relaunch
            MockProcessRunner::ok(),                     // set-option: the role marker
        ]);
        resync_agent_tree_pane(&test_tmux_window("task-5"), &mock);
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 5);
        assert_eq!(calls[1].1, vec!["kill-pane", "-t", "%9"]);
        assert!(calls[3].1.contains(&"split-window".to_string()));
        // The window being split is targeted by its resolved pane ID, not its
        // name — otherwise the companion pane could open inside a
        // prefix-matched sibling's window (see `tmux::window_target`).
        assert!(calls[3].1.contains(&mock.pane_id_of("task-5")));
        assert!(calls[3].1.contains(&"5".to_string()));
        assert!(calls[3].1.contains(&"agent-tree".to_string()));
        // The worktree read back from @dispatch_dir becomes the new pane's
        // start directory, so the correction hook leaves it alone.
        assert!(
            calls[3].1.windows(2).any(|w| w == ["-c", "/wt"]),
            "expected -c /wt in: {:?}",
            calls[3].1
        );
        // The respawn is a *new* pane, so it needs its own marker — otherwise a
        // resynced window would look companion-less to the next toggle.
        assert_eq!(
            calls[4].1,
            vec![
                "set-option",
                "-p",
                "-t",
                "%20",
                tmux::PANE_ROLE_OPTION,
                tmux::PANE_ROLE_AGENT_TREE
            ]
        );
    }

    /// A window whose `@dispatch_dir` cannot be read still gets its companion
    /// pane — the split just goes out without a start directory, leaving the
    /// correction hook to place it.
    #[test]
    fn resync_agent_tree_pane_spawns_without_a_start_dir_when_the_option_is_unset() {
        let mock = MockProcessRunner::new(vec![
            // list-panes: the companion is %9
            MockProcessRunner::ok_with_stdout(b"%1 \n%9 agent_tree\n"),
            MockProcessRunner::ok(),                     // kill-pane
            MockProcessRunner::ok_with_stdout(b"\n"),    // show-options: unset
            MockProcessRunner::ok_with_stdout(b"%20\n"), // split-window
            MockProcessRunner::ok(),                     // set-option: the role marker
        ]);
        resync_agent_tree_pane(&test_tmux_window("task-5"), &mock);
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 5);
        assert!(calls[3].1.contains(&"split-window".to_string()));
        assert!(
            !calls[3].1.contains(&"-c".to_string()),
            "no start directory to name, got: {:?}",
            calls[3].1
        );
    }

    #[test]
    fn resync_agent_tree_pane_soft_fails_when_companion_check_errors() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("list-panes error")]);
        resync_agent_tree_pane(&test_tmux_window("task-5"), &mock);
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1, "no further calls after a failed check");
    }

    #[test]
    fn resync_agent_tree_pane_still_respawns_when_kill_fails() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"%1 \n%9 agent_tree\n"),
            MockProcessRunner::fail("kill-pane error"),
            MockProcessRunner::ok_with_stdout(b"/wt\n"),
            MockProcessRunner::ok_with_stdout(b"%20\n"),
            MockProcessRunner::ok(), // set-option: the role marker
        ]);
        resync_agent_tree_pane(&test_tmux_window("task-5"), &mock);
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 5, "a respawn is still attempted");
        assert!(calls[3].1.contains(&"split-window".to_string()));
    }
}
