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
    item_line, log_line, newest_first, note_line, notes_in_order, open_items, Epic, EpicNote,
    EpicOrchestration, Learning, OrchestratorLogEntry, ReadyMarker, StartRule, SubStatus,
    TaskStatus, TmuxWindow, WaitingItem, ORCHESTRATOR_LOG_SHOWN,
};
use crate::process::ProcessRunner;
use crate::tmux;

use super::agents::{prompt_launch_command_with, LaunchFlags};
use super::prompts::DISPATCH_PLUGIN_DIR;

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
    /// Whether to continue the conversation Claude Code keeps for the
    /// directory: decided by the caller before the start is recorded
    /// ([`holds_a_conversation`]), so the start's log entry says which it was.
    pub resume: bool,
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
    let resume = launch.resume;
    std::fs::write(launch.dir.join(PROMPT_FILE), launch.prompt)
        .with_context(|| format!("failed to write the prompt in {dir}"))?;

    let flags = LaunchFlags {
        base: format!("{DISPATCH_PLUGIN_DIR} --name {}", launch.name),
        mcp_config: config.map(|p| p.to_string_lossy().into_owned()),
    };
    let extra = if resume { " --continue" } else { "" };
    let command =
        prompt_launch_command_with(&runner.agent_binaries().claude_quoted(), &flags, extra);
    let orchestrator_env = format!("DISPATCH_ORCHESTRATOR={}", launch.name);
    let argv = [
        "env",
        "-u",
        "DISPATCH_TASK_ID",
        orchestrator_env.as_str(),
        "CLAUDE_CODE_EXPERIMENTAL_CODE_MODE=1",
        "sh",
        "-c",
        command.as_str(),
    ];
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
    super::caller_identity::write_mcp_config_at(launch.dir, &config)
}

/// Whether Claude Code keeps a conversation for `dir`, so `--continue` has
/// one to resume. Claude Code files a directory's conversations under
/// `~/.claude/projects/<the directory with every character that is not a
/// letter or a digit replaced by '-'>/`, one `.jsonl` file each; `--continue`
/// with none there fails, which would end the session at once.
pub fn holds_a_conversation(claude_json: &Path, dir: &Path) -> bool {
    let Some(home) = claude_json.parent() else {
        return false;
    };
    let key: String = dir
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let projects = crate::setup::claude_dir_in(home).join("projects").join(key);
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

/// The orchestrator's memory of its epic, as its prompt shows it
/// (`orchestrators.allium: EnoughForAFreshStart`, `GivenItsLearnings`).
#[derive(Default)]
pub struct OrchestratorMemory {
    /// Every note of the epic.
    pub notes: Vec<EpicNote>,
    /// The epic's waiting items; the prompt shows the open ones.
    pub open_items: Vec<WaitingItem>,
    /// The epic's log; the prompt shows the newest entries.
    pub log: Vec<OrchestratorLogEntry>,
    /// The learnings that fit the epic, best first.
    pub learnings: Vec<Learning>,
}

/// `dispatch-prompt.allium: OrchestratorSessionPrompt`. `tasks` come in board
/// order.
pub fn build_orchestrator_prompt(
    name: &str,
    epic: &Epic,
    orchestration: &EpicOrchestration,
    tasks: &[OrchestratorPromptTask],
    memory: &OrchestratorMemory,
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
    push_memory(&mut p, epic, memory);
    p.push_str(
        "\nFirst, before anything else, look at what happened while no session of yours ran: \
         agents that need input, ready tasks that a free slot could start, and anything the \
         operator has answered. Read the board through the dispatch tools; this list is the \
         board as your session started.\n",
    );
    p
}

/// The memory part of the prompt: notes, open items, the newest log entries,
/// how to keep them, a note from a learning, and the learnings.
fn push_memory(p: &mut String, epic: &Epic, memory: &OrchestratorMemory) {
    let section = |p: &mut String, heading: &str, lines: Vec<String>| {
        p.push_str(heading);
        if lines.is_empty() {
            p.push_str("- none\n");
        }
        for line in lines {
            p.push_str(&format!("- {line}\n"));
        }
    };
    section(
        p,
        "\nYour notes for this epic (id, [done] when ticked):\n",
        notes_in_order(&memory.notes, epic.id)
            .into_iter()
            .map(note_line)
            .collect(),
    );
    section(
        p,
        "\nItems that wait for the operator:\n",
        open_items(&memory.open_items, epic.id)
            .into_iter()
            .map(item_line)
            .collect(),
    );
    section(
        p,
        &format!(
            "\nThe newest entries of the epic's log, newest first (orchestrator-log_search with epic_id {} reads the rest):\n",
            epic.id.0
        ),
        newest_first(&memory.log, |_| true)
            .into_iter()
            .take(ORCHESTRATOR_LOG_SHOWN)
            .map(log_line)
            .collect(),
    );
    p.push_str(
        "\nThis memory outlives your session, so keep it as you go. Give a one-line reason with \
         every task_dispatch. After you answer an agent, record it with orchestrator-answer_record. \
         Raise what only a person can decide with waiting-item_create, and resolve an item with \
         waiting-item_resolve once the operator has answered it. Keep notes with epic-note_create, \
         epic-note_update and epic-note_delete. When the epic is done, record a closing summary \
         with orchestrator-summary_record: what it delivered, what is left open, and the notes \
         worth keeping as learnings.\n\
         \nA note is a to-do or a fact about this epic's work now. What will still be true and \
         useful after the epic is done is a learning, for the knowledge base.\n",
    );
    if !memory.learnings.is_empty() {
        p.push_str(
            "\n## Validated knowledge for this epic\n\nThe following knowledge has been validated by \
             previous agents. Apply it where relevant.\n\n",
        );
        for l in &memory.learnings {
            p.push_str(&format!(
                "- [#{} {}, \u{2191}{}] {}\n",
                l.id.0,
                l.scope.as_str(),
                l.upvote_count,
                l.summary
            ));
        }
    }
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
            &OrchestratorMemory::default(),
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

    /// dispatch-prompt.allium: ThePromptsParts, the memory part
    /// (orchestrators.allium: EnoughForAFreshStart, GivenItsLearnings).
    #[test]
    fn the_prompt_carries_the_memory_and_the_learnings() {
        use crate::models::{EpicId, LearningScope, LogEntryKind};
        let epic = EpicBuilder::new(7).title("E").build();
        let at = chrono::Utc::now();
        let note = |id, text: &str, done| EpicNote {
            id,
            epic_id: EpicId(7),
            text: text.into(),
            done,
            written_at: at,
        };
        let memory = OrchestratorMemory {
            notes: vec![note(1, "ticked", true), note(2, "use v3", false)],
            open_items: vec![WaitingItem {
                id: 5,
                epic_id: EpicId(7),
                host: "h".into(),
                task_id: Some(TaskId(9)),
                about: "may I drop it?".into(),
                tried: "read it".into(),
                recommendation: "keep it".into(),
                raised_at: at,
                resolved_at: None,
            }],
            log: vec![OrchestratorLogEntry {
                id: 3,
                epic_id: EpicId(7),
                host: "h".into(),
                at,
                kind: LogEntryKind::StartedTask,
                task_id: Some(TaskId(9)),
                text: "first step".into(),
                question: None,
                trouble: None,
            }],
            learnings: vec![super::super::prompts::tests::seed(
                42,
                LearningScope::Epic,
                3,
            )],
        };

        let p = build_orchestrator_prompt(
            "zlata",
            &epic,
            &switch(StartRule::MarkedOnly, None),
            &[],
            &memory,
        );

        let order = [
            "Your notes for this epic",
            "- #2 use v3",
            "- #1 [done] ticked",
            "Items that wait for the operator:",
            "- #5 task #9: may I drop it? — tried: read it — recommends: keep it",
            "orchestrator-log_search with epic_id 7 reads the rest",
            "started_task task #9 — first step",
            "with every task_dispatch",
            "orchestrator-summary_record",
            "A note is a to-do or a fact",
            "## Validated knowledge for this epic",
            "[#42 epic, \u{2191}3] learning 42",
            "First, before anything else",
        ];
        let mut from = 0;
        for part in order {
            let at = p[from..]
                .find(part)
                .unwrap_or_else(|| panic!("{part:?} missing or out of order in:\n{p}"));
            from += at + part.len();
        }
        assert!(!p.contains("learning_rate"), "{p}");
    }

    #[test]
    fn an_empty_memory_says_none_and_has_no_knowledge_block() {
        let epic = EpicBuilder::new(7).title("E").build();
        let p = build_orchestrator_prompt(
            "zlata",
            &epic,
            &switch(StartRule::MarkedOnly, None),
            &[],
            &OrchestratorMemory::default(),
        );
        assert_eq!(p.matches("- none").count(), 4, "{p}");
        assert!(!p.contains("Validated knowledge"), "{p}");
    }

    #[test]
    fn the_start_rule_everything_and_the_default_limit_are_said() {
        let epic = EpicBuilder::new(7).title("E").build();
        let p = build_orchestrator_prompt(
            "zlata",
            &epic,
            &switch(StartRule::Everything, None),
            &[],
            &OrchestratorMemory::default(),
        );
        assert!(
            p.contains("You start every backlog task. At most 3 task agents"),
            "{p}"
        );
        assert!(p.contains("- none"), "{p}");
        assert!(!p.contains("Its plan"), "{p}");
    }
}
