# Keep a task in Running while its agent waits on background work

Task #99124. Status: **implemented; this document is the original
proposal, kept as the record of how the design was reached.** The behaviour
is specified in `docs/specs/agent-health.allium` (HookStop,
ExpireBackgroundWait, HookSubagentStop, HookNotification,
ClassifyAgentActivity) and `docs/specs/core.allium` (BackgroundWork,
Agent.background_work); where this text and the specs differ, the specs win.

Decisions taken after the proposal:

- Cap: 30 minutes (option A), counted from the later of the deferred Stop and
  the agent's last tool call, applied by the store in one conditional write
  (`expire_background_wait`).
- No per-item detail view; the card names the kinds ("running · 2 agents ·
  1 shell"). Kinds kept: shell (the Monitor tool reports as a shell),
  workflow, monitor, MCP task.
- The wait reuses the deferred-Stop bit (`stop_pending`) rather than a new
  timestamp. The only new field is `Agent.background_work`, the counts the
  latest deferring Stop listed, overwritten by every defer.
- Rules version 49.

## Problem

An agent that starts a background shell (`Bash` with `run_in_background`,
or `Monitor`) and ends its turn moves to Review. It is not waiting for the
operator: Claude Code wakes it when the shell exits. The board shows it as
"needs me", so the operator opens agents that need nothing.

Today's sequence for one such shell:

1. `Stop` → `HookStop` flips the task to Review.
2. The shell exits → Claude Code injects a `<task-notification>` prompt →
   `UserPromptSubmit` → `HookUserPromptSubmit` flips it back to Running.
3. The agent finishes → `Stop` → Review again.

`agent-health.allium` says no fix exists, because Claude Code gave no
signal when a shell ends on its own. The earlier attempt
(`docs/superpowers/specs/2026-08-15-shell-visibility-design.md`) counted
shell starts and stops from hooks; the count inflated forever and the
feature was removed (`fd726c00`, task #4965).

## What changed in Claude Code (verified on 2.1.322)

The `Stop` and `SubagentStop` hook payloads now carry `background_tasks`:

> In-flight background work (running/pending + backgrounded) registered in
> this session. Lets hooks distinguish "session is done" from "session is
> paused waiting for background work to wake it". Empty array when nothing
> is in flight.

Each entry has `id`, `type`, `status`, `description`, plus `command` for
shells. The payload also carries `session_crons`.

Observed in a scratch project with logging hooks, both `claude -p` and an
interactive session in tmux:

| Event | `background_tasks` |
|---|---|
| `UserPromptSubmit` (operator prompt) | — |
| `Stop` | `[{type: shell, status: running}]` |
| `UserPromptSubmit` (`<task-notification>…`) | — |
| `Stop` | `[]` |

- A `Monitor` is listed as `type: shell`; a background subagent as
  `type: subagent`. Each one finishing wakes the agent and produces a fresh
  `Stop` with the shorter list.
- Interactive only: `Notification(idle_prompt)` fires about 60 s after a
  `Stop` even while a shell runs. Claude Code holds that notification back
  only for subagents, teammates and workflows, not for shells.

## Proposal

The key idea: **a snapshot, not a counter.** Every `Stop` replaces the
snapshot. No hook has to see a shell end, because the shell's end always
produces a new `Stop`. That is what the removed design lacked.

1. **Hook script.** On `Stop`, count `background_tasks` entries with
   `type` in {`shell`, `workflow`} and `status` in {`running`, `pending`}.
   Pass it as `dispatch hook <id> stop --background <n>`. When the field is
   absent (older Claude Code), pass nothing: today's behaviour.
   Subagents stay on the existing `live_subagents` path. An allowlist, not
   "everything", because the builder also lists internal kinds (`dream`,
   `auto_mode_scan`, …) that never wake the agent for the operator.
2. **`HookStop` / `try_record_stop`.** With `background > 0`, keep the task
   Running and record `agent.background_at_stop = n` with its time. Do not
   set `stop_pending`: no drain is needed, the wake-up's own `Stop` decides.
   With `background = 0`, behave as today. Every `Stop` overwrites the
   snapshot.
3. **`HookUserPromptSubmit`.** Clear the snapshot (a new turn started).
4. **`HookNotification`.** Drop `idle_prompt` while the snapshot is
   positive, the same way it is dropped for live subagents. Without this,
   every waiting agent reads as Blocked after a minute.
5. **`HookSubagentStop` drain.** Do not apply a deferred Stop while the
   snapshot shows background work.
6. **Card label.** "running · 1 shell" / "running · N shells", next to the
   existing "running · N agents".

### Edge cases

- **Shell that never exits** (dev server, `tail -f`). No wake-up comes, so
  the task stays Running. This also hides an agent that asked a plain-text
  question while a dev server runs. See the decision below.
- **Crash or window gone.** `DetectCrashedAgent` handles it as today.
- **Operator kills the shell from Claude Code's UI.** Expected to wake the
  agent like any exit. Verify during implementation.
- **`/loop` and cron wake-ups** (`session_crons`). Out of scope; the same
  field could cover them later.

### Decision for the operator

What should happen to a task whose shell never ends?

- **A (recommended): time cap.** After a cap (for example 30 min) with no
  wake-up, apply the deferred Stop and move the task to Review. The card shows
  "running · 1 shell" until then.
- **B: no cap.** Stay Running. The existing 10-minute stale label is the
  only hint.
- **C: show it as a Review sub-status** ("waiting on shell") instead of
  keeping it in Running.

### Touch points (spec first)

1. `docs/specs/agent-health.allium`: rewrite the "no fix available"
   guidance in `ClassifyAgentActivity` and `HookNotification`; change
   `HookStop`, `HookUserPromptSubmit`, `HookNotification`,
   `HookSubagentStop`; add the agent field.
2. `plugin/hooks/scripts/task-status-hook`, the `dispatch hook` CLI flag,
   `src/hooks/`.
3. `spacetime/module/src/agent_state.rs` (`try_record_stop`, notification
   and prompt reducers), the `src/sync/memory_caller/` mirror,
   `tests/memory_caller_conformance.rs`, `RULES_VERSION` /
   `BOARD_RULES_VERSION`, bindings and `module.wasm`.
4. Card label rendering and its snapshot tests; `tests/hook_scripts.rs`.

### Possible follow-up

`background_tasks` also lists subagents. It could later replace the
`SubagentStart`/`SubagentStop` counting. Not part of this change.
