---
name: wrap-up
description: Use to finish a dispatch task and close its session, when the work is done and the user says to wrap up, finish or close out. The user chooses the path first (rebase, draft PR, or done); it then runs the retro, verifies, commits and calls session_wrap-up. Always use this, never retro or a hand-called session_wrap-up/session_exit, to finish a task.
---

# Wrap Up

choose the action → `/retro` → verify (rebase and pr only) → commit → `session_wrap-up(action)` → `session_exit(token, action, ...)`.

`session_wrap-up` refuses an uncommitted worktree but does not run the verify command: you do, in Step 6. `session_exit` is mandatory on every path: without it the task does not move and the session does not close.

- **rebase** — dispatch rebases and fast-forwards `{base_branch}`; `session_exit` marks the task Done and kills your tmux window.
- **pr** — you do it; read `references/pr.md` when this path is chosen. Write a real title and body from the diff, with the `pull-request` skill that file names, and open a draft PR.
- **done** — no git operations. For research, planning, or work already on `{base_branch}`.

**Announce at start:** "I'm using the wrap-up skill to complete this task."

If invoked with an argument (`/wrap-up rebase`, `pr` or `done`), that is the chosen action: skip Step 4. Ignore any other argument.

## Step 1: Get the task ID from the current branch

Run `git rev-parse --abbrev-ref HEAD` and take the leading integer of the `{id}-{slug}` branch name (`42-fix-login-bug` → `42`). If the branch does not match, stop and tell the user: "This branch doesn't follow the dispatch naming convention (`{id}-{slug}`). Cannot determine task ID."

## Step 2: Get task details

Call `task_search`. The response is prose with one labelled line per field; a missing line means the field is unset.

- `Base branch: <name>` is `{base_branch}`; fall back to `main` if absent.
- `Wrap-up mode: <mode>` (`rebase`, `pr` or `done`) is printed only when set. If present and no argument was given, treat it as an argument: skip Step 4 and use that action.

A `Wrap-up: refused — <reason>` line means the task cannot be wrapped up: show the user that line and stop, before the retro or any commit.

A preset mode is the user's earlier answer, not a pause. Run everything from here without further input and do not stop before `session_exit` returns. The failure is silent: agents have gone idle at a blank prompt with the branch unmerged and the task stuck in `running`.

## Step 3: Simplify code changes (conditional)

Count the source lines added plus removed, committed and not (`git diff {base_branch}..HEAD --numstat` and `git diff --numstat`). Docs, configs, snapshots and lock files do not count.

Under **100** changed lines, skip (task #4973's retrospective: review sub-agents rarely find anything on a diff that small). At 100 or more, run `Skill({ skill: "simplify" })`; its changes are committed in Step 6. Its summary is not a stopping point: go straight on to Step 4.

## Step 4: Ask which path

Skip this step if an argument or `Wrap-up mode` already gave the action.

Otherwise ask and wait. When your task's prompt names an orchestrator, send it this question as a message and wait for its answer; it answers only when the task or the epic states a preferred path, and passes the question on to the operator otherwise. Without one, or when it is not running, use `AskUserQuestion`. Rebase fast-forwards a shared branch, the PR path opens a PR under the user's identity, and done integrates nothing; even work that looks like a PR may want a rebase. Ask:

> Wrap up task #{id} (`{title}`):
> **(r)** rebase onto `{base_branch}` — dispatch fast-forwards `{base_branch}` and kills this tmux window
> **(p)** author and create a draft PR — you draft the title/body, run `gh pr create`, then record the URL via `session_exit`
> **(d)** done — no git operations
> **(Esc / n)** cancel

On cancel, exit without calling any tool.

## Step 5: Run the retro

Run `Skill({ skill: "retro" })` and tell it which action you are wrapping up with. It runs after the action is settled (on `done` a fix would be stranded, so retro files a task instead) and before the commit (so its fixes travel with your work). Leave the follow-up tasks it files alone.

## Step 6: Verify, then commit

If `task_search` printed a `Verify command:` line, then on the rebase and pr paths, run it in your worktree now and fix any failure before you go on (on the pr path this is also what keeps a red build out of the push). On the done path, run no verify command: nothing from it lands on `{base_branch}`. Run the command exactly as printed, with no arguments added; it decides for itself how much your change needs checked. If it prints what it ran or skipped, report that and no more. Run it once: do not run it again after `session_wrap-up`, even when the rebase moved your branch. Dispatch does not run it for you and does not check that you did. If it fails for a reason your change did not cause, do not wrap up past it alone: coordinate with other agents and the board, as `CLAUDE.md` says.

Run `git status --porcelain`. If it is not clean, `git add` the relevant files by name, review `git diff --cached`, and `git commit -m "..."` with a short message. `session_wrap-up` refuses a dirty worktree.

If your task's prompt names an orchestrator, send it a short outcome report now, as a message: what you delivered, on which path, and what is left open.

## Step 7: The closing sequence

Run these back to back in one turn; any pause risks going idle with `{base_branch}` already fast-forwarded and the task still in its old status.

1. **Rate retrieved knowledge** in one `learning_rate` call: each entry you acted on and have not rated yet (`helped` or `wrong`), and every entry from "## Validated knowledge for this task" that did not fit this task `not_applicable`. Those verdicts are how the store learns to stop injecting an entry here. The `learnings` skill has the details.
2. **`session_wrap-up(task_id, action)`.** It returns an **Exit token**. If it refuses — uncommitted changes, a rebase conflict — show the user the exact message. For a dirty tree, commit (Step 6) and call `session_wrap-up` again; for a conflict, suggest resolution steps. Any other refusal (a backlog or review-tagged task, no worktree, the main checkout not on `{base_branch}`): show the exact message and stop. Do not call `session_exit` without a token.
3. **`session_exit(task_id, token, action)`**, plus `pr_url` on the pr path only. Make it your final action. If it errors, or succeeds but reports that the close did not take effect, read `references/exit-session-errors.md` before reacting.
