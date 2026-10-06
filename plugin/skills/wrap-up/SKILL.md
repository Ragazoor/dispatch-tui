---
name: wrap-up
description: Use to finish a dispatch task and close its session — whenever work in a dispatch worktree is complete and the user says to wrap up, finish, close out, finalise, or "we're done here", even if they don't name this skill. The user first chooses one of three paths: rebase onto the task's base_branch (dispatch-driven), author and open a draft GitHub PR yourself (you write the title and body), or done with no git operations. It then runs the retro, commits remaining changes, and runs the repo's verify command before any git operation touches a shared branch. Always use this rather than calling wrap_up/exit_session by hand — the path must be settled before the retro, the retro must run before the commit, and the two closing calls have an ordering that is easy to get wrong.
---

# Wrap Up

Wrap up a dispatch worktree. All three paths follow the same shape:

choose the action → `/retro` → commit → verify → `wrap_up(action)` → a single `exit_session(token, action, ...)` call that applies the terminal state change and closes the session.

**`exit_session` is mandatory on every path.** `wrap_up` alone changes nothing terminal — it issues a token and, for `rebase`, does the git work. The task's status is not moved and the session is not closed until `exit_session` runs.

- **rebase** — dispatch handles the git work. `wrap_up(action="rebase")` fast-forwards `{base_branch}`; the closing `exit_session` call then marks the task Done and kills your tmux window. On a successful rebase, dispatch also re-indexes the repo in the background if it has a RAG index.
- **pr** — you handle it; read `references/pr.md` when this path is chosen. Inspect the diff you produced, write a real title and body that describe what was actually built, and run `gh pr create --draft` yourself. Dispatch deliberately does not author PR bodies: an auto-generated body is always worse than what you can write after seeing the work.
- **done** — no git operations. Use for research, planning, or work already on `{base_branch}`.

**Announce at start:** "I'm using the wrap-up skill to complete this task."

## Argument check

If the skill was invoked with an argument (e.g. `/wrap-up rebase`, `/wrap-up pr`, or `/wrap-up done`):
- Treat the argument as the chosen action (`rebase`, `pr`, or `done`)
- Skip Step 4 (AskUserQuestion) entirely
- After completing Steps 1–3, go straight to Step 5 with that action (Step 4 is the only step skipped)

If the argument is anything other than `rebase`, `pr`, or `done`, ignore it and proceed normally (Step 4 will ask).

**Precondition:** The task must have a worktree and be in "running" or "review" status. This applies to all three paths — `wrap_up` validates it server-side (`is_wrappable`) and rejects anything else, so there is no path where you can wrap up a backlog task or one without a worktree.

## Step 1: Get the task ID from the current branch

Run:
```bash
git rev-parse --abbrev-ref HEAD
```

Extract the leading integer from the `{id}-{slug}` pattern (e.g. `42-fix-login-bug` → `42`).

If the branch does not match the `{id}-{slug}` pattern, stop and tell the user:
> "This branch doesn't follow the dispatch naming convention (`{id}-{slug}`). Cannot determine task ID."

## Step 2: Get task details

Call the `dispatch` MCP tool `get_task` with the task ID from Step 1. The response is prose, one labelled line per field — read the values off the labels named below, and treat a missing line as the field being unset. Read `Base branch: <name>` and use that name wherever the instructions below refer to `{base_branch}`. If the line is absent or empty, fall back to `main`. (The rebase path resolves the real base branch server-side from the task record, so `{base_branch}` only matters for the diff/PR commands you run locally.)

Also read `Wrap-up mode: <mode>`. The line is printed only when a mode is set, so its presence is the signal. If it is there (`rebase`, `pr`, or `done`) **and** no argument was provided at invocation, treat it exactly like an argument: skip Step 4 (AskUserQuestion) and proceed to Step 5 with that action.

A preset mode is the user's answer, given earlier — not permission to skip the wrap-up itself. Everything from here is yours to run without further input from anyone: retro, commit, verify, `wrap_up`, `exit_session`. Do not stop to confirm the mode you just read, do not report that you are about to wrap up and wait, and do not end your turn anywhere before `exit_session` returns. This is worth stating plainly because the failure is silent and has happened twice: the agent suppressed the question, went idle at a blank prompt, and left the branch unmerged and the task stuck in `running` until a human noticed. A preset mode that only removes the question is strictly worse than no preset mode at all.

Also read `Verify command: <command>`, if present, and hold onto it for Step 7. If the line is absent, there is nothing to verify and Step 7 is a no-op.

## Step 3: Simplify code changes (conditional)

Check whether code was written in this branch — both committed and uncommitted:

```bash
git diff {base_branch}..HEAD --name-only
git diff --name-only
```

If the combined output includes no source code files (`.rs`, `.py`, `.ts`, `.js`, `.tsx`, `.jsx`, `.go`, `.java`, `.cpp`, `.c`, `.h`, `.swift`, `.kt`, `.rb`, `.cs`) — i.e. only docs, configs, snapshots, or lock files — skip this step entirely.

Otherwise, measure how much source actually changed:

```bash
git diff {base_branch}..HEAD --numstat -- '*.rs' '*.py' '*.ts' '*.js' '*.tsx' '*.jsx' '*.go' '*.java' '*.cpp' '*.c' '*.h' '*.swift' '*.kt' '*.rb' '*.cs'
git diff --numstat -- '*.rs' '*.py' '*.ts' '*.js' '*.tsx' '*.jsx' '*.go' '*.java' '*.cpp' '*.c' '*.h' '*.swift' '*.kt' '*.rb' '*.cs'
```

Sum the first two columns (added + removed lines) across every row of both commands. If the total is under **100** changed lines, skip this step — a diff that small is one you've already read in full while writing it, and four review sub-agents consistently come back with nothing on diffs this size (task #4973's retrospective: real fixes — a test that couldn't fail, a blocking call left on the TUI's frame loop, an O(n log n) sort-key bug — showed up almost exclusively on diffs well past this size).

If the total is 100 or more, invoke the `simplify` skill to review and apply improvements:

```
Skill({ skill: "simplify" })
```

Wait for the skill to complete before proceeding. If it makes additional changes, those will be picked up in Step 6.

`simplify` ends by printing a summary of what it fixed and skipped. That summary
is not a stopping point — go straight on to Step 4 (or Step 5, if the action is
already settled) in the same turn. The warning above about not ending your turn
before `exit_session` applies here as much as anywhere, and this step is the
easiest place to forget it: a long cleanup pass followed by a written summary
reads like the end of a turn, and the calling skill has not even reached its
commit yet.

## Step 4: Ask the user to choose

Use the `AskUserQuestion` tool, and wait for an actual answer before going on.

This is the one genuinely irreversible decision in the whole skill, which is why it belongs to the user rather than to you. Rebase fast-forwards a shared branch; the PR path pushes a branch and opens a PR under the user's GitHub identity; done closes the task with the work integrated nowhere. Guessing wrong is not a slightly-suboptimal choice — it publishes something or discards an integration step, and there is no undo from inside this skill. Even when the work obviously "looks like a PR", the user may have a reason to rebase. Ask.

The exception is when the action was already chosen for you — an invocation argument or `wrap_up_mode` from Step 2. That is the user's answer, given earlier; don't ask again.

Use the `AskUserQuestion` tool with a question like:

> Wrap up task #{id} (`{title}`):
> **(r)** rebase onto `{base_branch}` — dispatch fast-forwards `{base_branch}` with this branch and kills this tmux window
> **(p)** author and create a draft PR — you draft the title/body, run `gh pr create`, then record the URL via `exit_session`
> **(d)** done — no git operations (use for research, planning, or work already on `{base_branch}`)
> **(Esc / n)** cancel

If the user cancels or says no, exit without calling any tool. Nothing has been
committed yet at this point, and the retro has not run — a cancel here leaves the
worktree exactly as you found it.

## Step 5: Run the retro

Invoke the retro skill:

```
Skill({ skill: "retro" })
```

Wait for it to complete before proceeding.

Retro reflects on where this session lost time and may fix small inaccuracies in
`CLAUDE.md`, a page under `docs/`, or a skill so the next agent dispatched here
does better. Two things about this position are deliberate:

- It runs **after the action is settled** (Step 4), so retro knows whether its
  fix can reach `{base_branch}` at all. Tell it the action you are wrapping up
  with. On `done` there is no rebase and no push, so a fix would be stranded in
  the worktree and retro should file a task instead of editing.
- It runs **before the commit** (Step 6), so anything it does fix is committed
  with the session's work and travels with the rebase or the PR.

Do not defer it to the closing sequence. After `wrap_up` the rebase path has
already fast-forwarded `{base_branch}`, so a later commit strands those fixes on
a branch nobody merges, and the PR path has already pushed.

Retro may also file follow-up tasks. That is expected — leave them alone.

## Step 6: Commit uncommitted changes

Run:
```bash
git status --porcelain
```

If there are no changes, skip to Step 7.

If there are changes, commit them inline — run these commands yourself rather than invoking a commit skill or delegating to another tool. A commit skill would re-derive context you already have and can pull in its own conventions; you just watched this work happen, so you can stage and describe it in three commands:

1. `git add` the relevant files (prefer named files over `git add -A`)
2. `git diff --cached` to review what's staged
3. `git commit -m "..."` with a short message summarizing the changes

Don't polish the message. This commit exists so no work is lost before the branch is integrated — on the rebase path it lands on `{base_branch}` among your earlier commits, and on the PR path the PR body is where the real explanation goes. Once committed, move straight to Step 7.

## Step 7: Run verification

If Step 2 did not show a `Verify command`, skip straight to Step 8 — there is nothing to run.

If it did, run that command in your worktree now, **before** calling `wrap_up`. This is what closes the gap where an agent only learns about verification after `wrap_up(action="rebase")` has already fast-forwarded `{base_branch}` — by running it here, you confirm the code is good before any git operation touches the shared branch, on all three paths.

If it passes, continue to Step 8.

If it fails, fix the issues, then go back to Step 6 to commit the fix, and re-run this step. Do not proceed to Step 8 — and do not call `wrap_up(action="rebase")` — with a failing verify command.

## Step 8: The closing sequence

Every path ends with the same three steps. Only Step B differs by action, plus the PR path's authoring work (`references/pr.md`) which happens *before* this sequence. The task moves to "done" (rebase, done) or "review" (pr) automatically — don't set the status by hand.

Run the three back to back in one turn. Each is a tool call, not a milestone to report on: there is nothing here for the user to read or approve, and every pause is a chance to go idle with `base_branch` already fast-forwarded and the task still stuck in its old status.

**A. Rate retrieved knowledge.** See *Rate retrieved knowledge* below.

**B. Call `wrap_up`** with `task_id` (the integer from Step 1) and `action`. This returns an **Exit token** (a UUID string). It does not close the session and does not move the task's status — that all waits for Step C. What it does beyond issuing the token depends on the action:

| action | what `wrap_up` does | notes |
|---|---|---|
| `rebase` | blocks until the rebase completes and fast-forwards `{base_branch}` | can fail on conflict, or if the repo isn't on `{base_branch}` |
| `pr` | nothing — no `pr_url` here, it travels with `exit_session` | |
| `done` | nothing | |

If `wrap_up` returns an error, show the user the exact message and stop. Do not call `exit_session` — you have no valid token, and the task stays in its current status. For a rebase conflict, suggest resolution steps.

**C. Call `exit_session`** with `task_id`, `token` (from Step B), `action` (must match the action you passed to `wrap_up`), and `pr_url` on the pr path only. This single call applies the terminal state change, clears the tmux window, and consumes the token — atomically. There is no follow-up call; this closes the loop.

Do not stop between B and C: call `exit_session` next, as the final action. Skipping it leaves the tmux window alive and the task stuck in its old status — and on the PR path, the PR unrecorded.

If `exit_session` errors, or succeeds but reports that the close did not take effect, read `references/exit-session-errors.md` before reacting.

### Rate retrieved knowledge

When dispatch starts an agent, it injects relevant knowledge into the prompt under "## Validated knowledge for this task". You may also call `query_learnings` mid-task. Each surfacing is recorded as a retrieval, and the knowledge base learns which entries are useful from your ratings.

Rate via the `rate_learning` MCP tool at the moment you act on an entry, not here — see the `learnings` skill. This step is the backstop: for anything you acted on this task and have not yet rated, rate it now.

```
rate_learning(learning_id=<id>, task_id=<id>, verdict="helped")
```

- `verdict="helped"` — the entry was relevant and you applied it (upvotes it).
- `verdict="wrong"` — the entry was misleading, outdated, or contradicts current code (downvotes it; may go negative). There is no `needs_review` state or human curation step — if it's clearly wrong, delete it with `delete_learning` instead of just downvoting.

Only entries surfaced to you this task can be rated. There is no separate "unused" verdict — simply don't rate entries you didn't act on. `wrap_up` does not accept verdicts; rate through `rate_learning`.
