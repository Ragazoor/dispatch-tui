//! Launching an epic orchestrator's session (`orchestrators.allium:
//! StartOrchestratorSession`, `OrchestratorPrompt`; `dispatch-prompt.allium:
//! OrchestratorSessionPrompt`).
//!
//! An orchestrator is no task agent: it has no worktree, no task and no
//! verify command. It runs in a directory of its own under the data
//! directory, in a window whose only process is `claude`, so the window
//! closes when the session ends.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::models::{
    Epic, EpicOrchestration, ReadyMarker, StartRule, SubStatus, TaskStatus, TmuxWindow,
};
use crate::process::ProcessRunner;
use crate::tmux;

use super::prompts::DISPATCH_PLUGIN_DIR;

/// The MCP configuration the session's `--mcp-config` names, in its own
/// directory.
const CONFIG_FILE: &str = "dispatch-mcp.json";
/// The prompt file the launch command reads and deletes.
const PROMPT_FILE: &str = ".claude-prompt";

/// `orchestrators.allium`'s directory of an epic's orchestrator, under the
/// data directory.
pub fn orchestrator_dir(data_dir: &Path, epic_id: crate::models::EpicId) -> PathBuf {
    data_dir
        .join("orchestrators")
        .join(format!("epic-{}", epic_id.0))
}

/// What one launch needs.
pub struct OrchestratorLaunch<'a> {
    pub name: &'a str,
    pub window: &'a TmuxWindow,
    pub dir: &'a Path,
    pub prompt: &'a str,
    /// Claude Code's user-global configuration (`~/.claude.json`): where the
    /// dispatch MCP entry is read from and where the directory is trusted.
    pub claude_json: &'a Path,
    /// Whether the conversation Claude Code keeps for the directory may be
    /// continued. A session after one that died starts fresh.
    pub may_resume: bool,
}

/// Open the orchestrator's window and start its session in it.
///
/// A window of the same name left from an earlier session is closed first:
/// the store says no session is live, so nothing in it is wanted.
pub fn launch_orchestrator(
    launch: &OrchestratorLaunch<'_>,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    std::fs::create_dir_all(launch.dir)
        .with_context(|| format!("failed to create {}", launch.dir.display()))?;
    let dir = launch.dir.to_string_lossy().into_owned();
    // The board made the directory, so it trusts it; an untrusted directory
    // would park the unattended session on Claude Code's trust dialog.
    if !super::is_trusted_at(launch.claude_json, &dir).unwrap_or(false) {
        super::trust_at(launch.claude_json, &dir)?;
    }
    let config = write_mcp_config(launch);
    let resume = launch.may_resume && holds_a_conversation(launch.claude_json, launch.dir);
    std::fs::write(launch.dir.join(PROMPT_FILE), launch.prompt)
        .with_context(|| format!("failed to write the prompt in {dir}"))?;

    let claude = runner.agent_binaries().claude;
    let continue_flag = if resume { " --continue" } else { "" };
    let (config_flag, config_arg) = match &config {
        Some(path) => (
            " --mcp-config \"$1\"",
            Some(path.to_string_lossy().into_owned()),
        ),
        None => ("", None),
    };
    let script = format!(
        "prompt=$(cat {PROMPT_FILE}) && rm -f {PROMPT_FILE} && exec \"$0\" \
         {DISPATCH_PLUGIN_DIR} --name {name}{continue_flag}{config_flag} -- \"$prompt\"",
        name = launch.name,
    );
    let orchestrator_env = format!("DISPATCH_ORCHESTRATOR={}", launch.name);
    let mut argv: Vec<&str> = vec![
        "env",
        "-u",
        "DISPATCH_TASK_ID",
        &orchestrator_env,
        "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE=1",
        "bash",
        "-c",
        &script,
        &claude,
    ];
    if let Some(path) = config_arg.as_deref() {
        argv.push(path);
    }
    tmux::kill_window_if_present(launch.window, runner)?;
    tmux::new_window_running(launch.window, &dir, &argv, runner)
        .context("failed to open the orchestrator's window")?;
    tracing::info!(orchestrator = launch.name, window = %launch.window.as_str(), resume, "orchestrator started");
    Ok(())
}

/// Close the orchestrator's window, if it is there.
pub fn end_orchestrator(window: &TmuxWindow, runner: &dyn ProcessRunner) -> Result<()> {
    tmux::kill_window_if_present(window, runner)
}

/// The session's own MCP configuration, identifying it by name. `None` when
/// it could not be written, and the session then launches without one: it
/// reaches the tools as a plain session.
fn write_mcp_config(launch: &OrchestratorLaunch<'_>) -> Option<PathBuf> {
    let Some(config) =
        crate::setup::dispatch_entry_identifying_orchestrator(launch.claude_json, launch.name)
    else {
        tracing::warn!(
            claude_json = %launch.claude_json.display(),
            "no dispatch MCP entry to derive the orchestrator's caller identity from"
        );
        return None;
    };
    let path = launch.dir.join(CONFIG_FILE);
    match crate::setup::write_json_file(&path, &config) {
        Ok(()) => Some(path),
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), path = %path.display(), "failed to write the orchestrator's MCP config");
            None
        }
    }
}

/// Whether Claude Code keeps a conversation for `dir`, so `--continue` has
/// one to resume. Claude Code files a directory's conversations under
/// `~/.claude/projects/<the directory with every character that is not a
/// letter or a digit replaced by '-'>/`, one `.jsonl` file each; `--continue`
/// with none there fails, which would end the session at once.
fn holds_a_conversation(claude_json: &Path, dir: &Path) -> bool {
    let Some(home) = claude_json.parent() else {
        return false;
    };
    let key: String = dir
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let projects = home.join(".claude").join("projects").join(key);
    std::fs::read_dir(projects).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
    })
}

/// One task of the orchestrator's subtree, as its prompt lists it.
pub struct OrchestratorPromptTask {
    pub id: crate::models::TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub sub_status: SubStatus,
    pub ready: Option<ReadyMarker>,
    pub agent: Option<String>,
}

/// `dispatch-prompt.allium: OrchestratorSessionPrompt`. `tasks` come in board
/// order.
pub fn build_orchestrator_prompt(
    name: &str,
    epic: &Epic,
    orchestration: &EpicOrchestration,
    tasks: &[OrchestratorPromptTask],
) -> String {
    let mut p = format!(
        "You are {name}, the orchestrator of epic #{id} ({title}) on this machine.\n\n\
         You start the epic's tasks, watch their agents, answer their questions, and tell the \
         operator when a person is needed or the epic is finished. You do none of the work \
         yourself: you edit no file, have no worktree, and never merge, rebase, approve, close \
         or delete anything. Each task's own agent finishes its task. You talk to agents and use \
         the dispatch tools.\n",
        id = epic.id.0,
        title = epic.title,
    );
    if !epic.description.trim().is_empty() {
        p.push_str(&format!("\nThe epic:\n{}\n", epic.description.trim()));
    }
    if let Some(plan) = epic.plan_path.as_deref().filter(|s| !s.trim().is_empty()) {
        p.push_str(&format!("\nIts plan: {plan}\n"));
    }
    let rule = match orchestration.start_rule {
        StartRule::MarkedOnly => "only backlog tasks marked ready",
        StartRule::Everything => "every backlog task",
    };
    p.push_str(&format!(
        "\nYou start {rule}. At most {limit} task agents of this epic run at once on this machine.\n",
        limit = orchestration.limit(),
    ));
    p.push_str(
        "\nWhen a task agent is in trouble (stale, crashed, failed to start, or hit a conflict in \
         its wrap-up), act once: ask it for status, resume it, start it again, or tell it to \
         resolve the conflict. If the same trouble comes back, pass it to the operator with what \
         you tried and what you recommend.\n",
    );
    p.push_str("\nThe epic's tasks (id, title, status, sub-status, ready, agent):\n");
    if tasks.is_empty() {
        p.push_str("- none\n");
    }
    for t in tasks {
        let ready = match (t.status, t.ready, orchestration.start_rule) {
            (TaskStatus::Backlog, Some(_), _)
            | (TaskStatus::Backlog, None, StartRule::Everything) => ", ready",
            (TaskStatus::Backlog, None, StartRule::MarkedOnly) => ", not ready",
            _ => "",
        };
        let agent = t
            .agent
            .as_deref()
            .map(|a| format!(", agent {a}"))
            .unwrap_or_default();
        p.push_str(&format!(
            "- #{} {} — {}/{}{ready}{agent}\n",
            t.id.0,
            t.title,
            t.status.as_str(),
            t.sub_status.as_str()
        ));
    }
    p.push_str(
        "\nFirst, before anything else, look at what happened while no session of yours ran: \
         agents that need input, ready tasks that a free slot could start, and anything the \
         operator has answered. Read the board through the dispatch tools; this list is the \
         board as your session started.\n",
    );
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EpicBuilder, EpicId, TaskId};

    fn switch(rule: StartRule, max: Option<i64>) -> EpicOrchestration {
        EpicOrchestration {
            epic_id: EpicId(7),
            start_rule: rule,
            max_parallel: max,
            switched_on_at: chrono::Utc::now(),
        }
    }

    fn listed(
        id: i64,
        status: TaskStatus,
        ready: Option<ReadyMarker>,
        agent: Option<&str>,
    ) -> OrchestratorPromptTask {
        OrchestratorPromptTask {
            id: TaskId(id),
            title: format!("T{id}"),
            status,
            sub_status: SubStatus::None,
            ready,
            agent: agent.map(str::to_string),
        }
    }

    /// dispatch-prompt.allium: OrchestratorSessionPrompt, ThePromptsParts.
    #[test]
    fn the_prompt_has_its_parts_in_order() {
        let epic = EpicBuilder::new(7)
            .title("Epic orchestrators")
            .description("Build the orchestrator.")
            .plan(Some("docs/plans/p.md"))
            .build();
        let tasks = [
            listed(
                1,
                TaskStatus::Backlog,
                Some(ReadyMarker::Operator),
                Some("ada"),
            ),
            listed(2, TaskStatus::Backlog, None, Some("linus")),
            listed(3, TaskStatus::Running, None, Some("grace")),
        ];

        let p = build_orchestrator_prompt(
            "zlata",
            &epic,
            &switch(StartRule::MarkedOnly, Some(2)),
            &tasks,
        );

        let order = [
            "You are zlata, the orchestrator of epic #7 (Epic orchestrators)",
            "you edit no file, have no worktree, and never merge",
            "Build the orchestrator.",
            "Its plan: docs/plans/p.md",
            "You start only backlog tasks marked ready. At most 2 task agents",
            "act once",
            "- #1 T1 — backlog/none, ready, agent ada",
            "- #2 T2 — backlog/none, not ready, agent linus",
            "- #3 T3 — running/none, agent grace",
            "First, before anything else",
        ];
        let mut from = 0;
        for part in order {
            let at = p[from..]
                .find(part)
                .unwrap_or_else(|| panic!("{part:?} missing or out of order in:\n{p}"));
            from += at + part.len();
        }
    }

    #[test]
    fn the_start_rule_everything_and_the_default_limit_are_said() {
        let epic = EpicBuilder::new(7).title("E").build();
        let p =
            build_orchestrator_prompt("zlata", &epic, &switch(StartRule::Everything, None), &[]);
        assert!(
            p.contains("You start every backlog task. At most 3 task agents"),
            "{p}"
        );
        assert!(p.contains("- none"), "{p}");
        assert!(!p.contains("Its plan"), "{p}");
    }
}
