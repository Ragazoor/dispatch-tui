---
name: allium-weed-loop
description: >-
  Fresh-agent-per-run loop that runs allium weed to find undocumented
  behaviour, updates the specs to match, and asks before touching code bugs.
  Use when the specs in docs/specs/ have drifted behind the implementation and
  the gap is too wide to close in one pass — after a large feature landed, or
  when a weed run returns more findings than one session can absorb.
allowed-tools: ["Read", "Write", "Bash", "Agent", "AskUserQuestion"]
---

# Allium Weed Loop

This skill drives the spec-drift reconciliation loop from undocumented behaviour toward an
aligned spec. It is the weed-first sibling of `allium-loop` (spec-first: design doc → implement →
weed). It is language- and stack-agnostic.

The session that invokes this skill becomes the loop **driver**: it dispatches one fresh subagent
per iteration (via the `Agent` tool, never `fork` — each iteration has no memory of prior ones,
only the repo's own committed state carries over) and decides whether to continue based on that
subagent's final report. The driver never does the rebase/weed/tend work itself.

Unlike a ralph-loop-style skill, this does not rely on an external Stop hook or
`.claude/ralph-loop.local.md`. See
`docs/superpowers/specs/2026-09-27-allium-weed-loop-fresh-agent-design.md` for why that
architecture was replaced.

## Instructions

### Kickoff

1. **Check for an existing active loop first.** Look for
   `.claude/allium-weed-loop-state.local.md` with `active: true`. If found, read it and tell the
   user a loop is already active, including its progress (`started_at`, `runs_completed` of
   `max_iterations`). Then use AskUserQuestion to ask whether to:
   - **Resume** — dispatch the next iteration using the existing file's values (do not reset
     `runs_completed`, `retry_count`, or `consecutive_clean_runs`).
   - **Abandon** — delete the state file and continue with a fresh kickoff below.
   - **Cancel** — stop here, do nothing further.

   Never silently overwrite an active state file.

2. **Resolve the base branch** to rebase onto each iteration — in priority order:
   1. **Task context** — if this session is running as a dispatched task, use that task's
      `base_branch` (e.g. via the dispatch MCP `get_task` tool).
   2. **Ask** — if there is no task context to read a base branch from, ask the user via
      AskUserQuestion rather than assuming `main`.

3. **Resolve the verify command** for this repo — in priority order:
   1. **Task/session context** — a verify command already surfaced this session.
   2. **Project docs** — a documented test/build command in this repo's `CLAUDE.md`, `AGENTS.md`,
      `README`, or equivalent.
   3. **Ask** — if none is found, ask the user via AskUserQuestion before starting the loop.

4. **Resolve `max_iterations`**: an explicit value if the user asked for one when invoking the
   skill (e.g. "run allium-weed-loop with max_iterations 20"), else default `10`.

5. **Resolve `model`** — the model each iteration agent runs on: an explicit value if the user
   asked for one, else default `sonnet`. The iteration agent's own work (rebase, process findings,
   commit) is mechanical; nested `allium:weed`/`allium:tend` dispatches pin their own model
   regardless (see the note in step 2 of the prompt file), so sonnet on the driver does not affect
   spec-judgement quality.

6. **Read the prompt file** at `.claude/skills/allium-weed-loop/prompt.md`.

7. **Create the loop state file** directly at `.claude/allium-weed-loop-state.local.md` using the
   Write tool:

```markdown
---
active: true
runs_completed: 0
max_iterations: MAX_ITERATIONS
retry_count: 0
consecutive_clean_runs: 0
base_branch: "BASE_BRANCH"
verify_command: "VERIFY_COMMAND"
model: "MODEL"
started_at: "TIMESTAMP"
---
```

   Get the timestamp with `date -u +%Y-%m-%dT%H:%M:%SZ`.

8. **Tell the user** the loop is active (base branch, verify command, `model`, and
   `max_iterations` — noting the last two can be changed by asking), then dispatch iteration 1
   immediately (see "Each Iteration" below).

### Each Iteration

1. Substitute `{{BASE_BRANCH}}`, `{{VERIFY_COMMAND}}`, and `{{ITERATION_NUMBER}}` into the prompt
   content read in kickoff step 6. The iteration number is 1-indexed and always derived from the
   state file as `runs_completed + 1` — correct after a resume or a context compaction, not only
   on a clean run.

2. Dispatch it: call the `Agent` tool with a **fresh subagent** (do not pass `subagent_type:
   "fork"`), passing `model` set to the state file's `model` value, and this filled-in prompt as
   its task.

   A `fork` ignores the `model` override and runs on the session model, so dispatching a fresh
   subagent is what makes the pin take effect at all. Read `model` from the state file rather than
   remembering it, for the same reason the iteration number is derived there.

3. When that call's result arrives (a task-notification — this may land in a different turn than
   the one that dispatched it):

   - **The subagent errored, was skipped, or returned an unparseable report** — a report is
     unparseable if it is missing EITHER required label (`WEED_CLEAN:` or `SUMMARY:`); a partial
     or malformed report is an error, not a result. Read `retry_count` from the state file.
     - If `retry_count == 0`: set it to `1`, keep `{{ITERATION_NUMBER}}` unchanged, and
       re-dispatch the same iteration (repeat step 2).
     - If `retry_count` was already `1`: delete the state file and tell the user this iteration
       failed twice and the loop has stopped — do not retry indefinitely.
   - **A real report was returned** (both labels present): increment `runs_completed` by exactly
     1 and reset `retry_count` to `0`. Then update `consecutive_clean_runs`: if this run's
     `WEED_CLEAN` is `yes`, increment it by 1; otherwise reset it to `0`.
     - `consecutive_clean_runs >= 2` → converged: delete
       `.claude/allium-weed-loop-state.local.md` and report success to the user, including the
       final `SUMMARY`. Two consecutive clean passes, not one, because a single clean
       `allium:weed` pass can still miss a real divergence.
     - Else if `runs_completed >= max_iterations`: delete the state file, summarize to the user
       exactly what's unresolved and why, and stop. Never emit a false convergence claim to exit
       early.
     - Else: dispatch the next iteration (repeat step 1). The iteration number advances on its
       own, since `runs_completed` was just incremented.
