---
name: wrap-up
description: Use to finish a dispatch task and close its session — whenever work in a dispatch worktree is complete and the user says to wrap up, finish, close out, finalise, or "we're done here", even if they don't name this skill. The user first chooses one of three paths: rebase onto the task's base_branch (dispatch-driven), author and open a draft GitHub PR yourself (you write the title and body), or done with no git operations. It then runs the retro, commits remaining changes, and runs the repo's verify command before any git operation touches a shared branch. Always use this rather than calling wrap_up/exit_session by hand — the path must be settled before the retro, the retro must run before the commit, and the two closing calls have an ordering that is easy to get wrong.
---

# Wrap Up

Finish a dispatch worktree: commit the work, check it, integrate it, and close the session. All three paths follow one shape:

choose the action → `/retro` → commit → verify → `wrap_up(action)` → a single `exit_session(token, action, ...)` call.

**`exit_session` is mandatory on every path.** `wrap_up` alone changes nothing terminal: it issues a token and, for `rebase`, does the git work. The task's status does not move and the session does not close until `exit_session` runs.

- **rebase** — dispatch does the git work. `wrap_up(action="rebase")` fast-forwards `{base_branch}`; `exit_session` then marks the task Done and kills your tmux window. A successful rebase also re-indexes the repo in the background if it has a RAG index.
- **pr** — you do it; read `references/pr.md` when this path is chosen. Write a real title and body from the diff you produced and run `gh pr create --draft`. Dispatch does not author PR bodies.
- **done** — no git operations. For research, planning, or work already on `{base_branch}`.

**Announce at start:** "I'm using the wrap-up skill to complete this task."

If invoked with an argument (`/wrap-up rebase`, `pr` or `done`), that is the chosen action: skip Step 4 and do the rest. Ignore any other argument and let Step 4 ask.

The task must have a worktree and be in "running" or "review" status; `wrap_up` rejects anything else.

## Step 1: Get the task ID from the current branch

Run `git rev-parse --abbrev-ref HEAD` and take the leading integer of the `{id}-{slug}` branch name (`42-fix-login-bug` → `42`). If the branch does not match, stop and tell the user: "This branch doesn't follow the dispatch naming convention (`{id}-{slug}`). Cannot determine task ID."

## Step 2: Get task details

Call `get_task` with the task ID. The response is prose with one labelled line per field; a missing line means the field is unset.

- `Base branch: <name>` is `{base_branch}`; fall back to `main` if absent. (The rebase path resolves it server-side; it only matters for the local diff and PR commands.)
- `Wrap-up mode: <mode>` (`rebase`, `pr` or `done`) is printed only when set. If it is present and no argument was given, treat it as an argument: skip Step 4 and use that action.
- `Verify command: <command>`, if present, is for Step 7. If absent, Step 7 is a no-op.

A preset mode is the user's earlier answer to the question, not a pause. Run everything from here without further input: retro, commit, verify, `wrap_up`, `exit_session`. Do not stop to confirm the mode and do not end your turn before `exit_session` returns. The failure is silent: agents have gone idle at a blank prompt with the branch unmerged and the task stuck in `running`.

## Step 3: Simplify code changes (conditional)

Check whether this branch has source changes, committed or not:

```bash
git diff {base_branch}..HEAD --name-only
git diff --name-only
```

If no source files changed (only docs, configs, snapshots or lock files), skip this step.

Otherwise sum added plus removed source lines:

```bash
git diff {base_branch}..HEAD --numstat -- '*.rs' '*.py' '*.ts' '*.js' '*.tsx' '*.jsx' '*.go' '*.java' '*.cpp' '*.c' '*.h' '*.swift' '*.kt' '*.rb' '*.cs'
git diff --numstat -- '*.rs' '*.py' '*.ts' '*.js' '*.tsx' '*.jsx' '*.go' '*.java' '*.cpp' '*.c' '*.h' '*.swift' '*.kt' '*.rb' '*.cs'
```

Under **100** changed lines, skip: you have read a diff that small in full, and review sub-agents rarely find anything on one (task #4973's retrospective). At 100 or more, run `Skill({ skill: "simplify" })` and wait for it; any changes it makes are picked up in Step 6.

`simplify` ends with a summary. That is not a stopping point: go straight on to Step 4 (or Step 5 if the action is settled) in the same turn.

## Step 4: Ask the user to choose

Skip this step if an argument or `Wrap-up mode` already gave the action.

Otherwise use `AskUserQuestion` and wait for the answer. This is the one irreversible decision here: rebase fast-forwards a shared branch, the PR path pushes a branch and opens a PR under the user's identity, and done closes the task with the work integrated nowhere. Even when the work looks like a PR, the user may want a rebase. Ask:

> Wrap up task #{id} (`{title}`):
> **(r)** rebase onto `{base_branch}` — dispatch fast-forwards `{base_branch}` with this branch and kills this tmux window
> **(p)** author and create a draft PR — you draft the title/body, run `gh pr create`, then record the URL via `exit_session`
> **(d)** done — no git operations (use for research, planning, or work already on `{base_branch}`)
> **(Esc / n)** cancel

On cancel, exit without calling any tool. Nothing is committed and no retro has run, so the worktree is as you found it.

## Step 5: Run the retro

Run `Skill({ skill: "retro" })` and tell it which action you are wrapping up with.

The order matters. Retro runs after the action is settled because on `done` a fix would be stranded (nothing rebases or pushes), so retro must file a task instead. It runs before the commit so its fixes travel with your work; after `wrap_up` the rebase has already fast-forwarded and the PR has already been pushed. Retro may file follow-up tasks; leave them alone.

## Step 6: Commit uncommitted changes

Run `git status --porcelain`. If clean, go to Step 7.

Otherwise commit inline, without a commit skill or another tool: you already have the context. `git add` the relevant files (named files over `git add -A`), review with `git diff --cached`, then `git commit -m "..."` with a short message. Do not polish it; the commit exists so no work is lost, and the PR body is where the real explanation goes.

## Step 7: Run verification

If Step 2 showed no `Verify command`, go to Step 8.

Otherwise run it in your worktree now, **before** calling `wrap_up`, so the code is confirmed good before any git operation touches the shared branch, on all three paths. If it fails, fix the issue, return to Step 6 to commit, and re-run. Do not continue to Step 8, or call `wrap_up(action="rebase")`, with a failing verify command.

## Step 8: The closing sequence

Run these three back to back in one turn. Each is a tool call, not a milestone to report; any pause risks going idle with `{base_branch}` already fast-forwarded and the task still in its old status. The task moves to "done" (rebase, done) or "review" (pr) automatically.

**A. Rate retrieved knowledge** — see below.

**B. Call `wrap_up`** with `task_id` and `action`. It returns an **Exit token** and does not close the session or move the status.

| action | what `wrap_up` does |
|---|---|
| `rebase` | blocks until the rebase completes and fast-forwards `{base_branch}`; can fail on conflict or if the repo isn't on `{base_branch}` |
| `pr` | nothing; `pr_url` travels with `exit_session` |
| `done` | nothing |

If `wrap_up` errors, show the user the exact message and stop. Do not call `exit_session`: you have no valid token. For a rebase conflict, suggest resolution steps.

**C. Call `exit_session`** with `task_id`, `token`, the same `action`, and `pr_url` on the pr path only. It applies the terminal state change, clears the tmux window and consumes the token atomically. Make it your final action; skipping it leaves the window alive, the task stuck, and on the PR path the PR unrecorded.

If `exit_session` errors, or succeeds but reports that the close did not take effect, read `references/exit-session-errors.md` before reacting.

### Rate retrieved knowledge

Dispatch injects relevant knowledge into your prompt under "## Validated knowledge for this task", and `query_learnings` can surface more. Rate each entry with `rate_learning` when you act on it (see the `learnings` skill). This step is the backstop: rate anything you acted on and have not yet rated.

```
rate_learning(learning_id=<id>, task_id=<id>, verdict="helped")
```

- `helped` — relevant, and you applied it.
- `wrong` — misleading, outdated, or contradicted by current code. If clearly wrong, use `delete_learning` instead of downvoting.

Only entries surfaced to you this task can be rated; don't rate ones you didn't act on.
