---
name: wrap-up
description: Use to finish a dispatch task and close its session — whenever work in a dispatch worktree is complete and the user says to wrap up, finish, close out, finalise, or "we're done here", even if they don't name this skill. The user first chooses one of three paths: rebase onto the task's base_branch (dispatch-driven), author and open a draft GitHub PR yourself (you write the title and body), or done with no git operations. It then runs the retro, commits remaining changes, and calls wrap_up, which checks the commit and runs the repo's verify command itself. Always use this rather than calling wrap_up/exit_session by hand — the path must be settled before the retro, and the retro must run before the commit.
---

# Wrap Up

choose the action → `/retro` → commit → `wrap_up(action)` → `exit_session(token, action, ...)`.

`wrap_up` refuses an uncommitted worktree and a failing verify command, so there is no verify step here, except before pushing on the pr path (`references/pr.md`). `exit_session` is mandatory on every path: without it the task does not move and the session does not close.

- **rebase** — dispatch rebases and fast-forwards `{base_branch}`; `exit_session` marks the task Done and kills your tmux window.
- **pr** — you do it; read `references/pr.md` when this path is chosen. Write a real title and body from the diff and run `gh pr create --draft`.
- **done** — no git operations. For research, planning, or work already on `{base_branch}`.

**Announce at start:** "I'm using the wrap-up skill to complete this task."

If invoked with an argument (`/wrap-up rebase`, `pr` or `done`), that is the chosen action: skip Step 4. Ignore any other argument.

## Step 1: Get the task ID from the current branch

Run `git rev-parse --abbrev-ref HEAD` and take the leading integer of the `{id}-{slug}` branch name (`42-fix-login-bug` → `42`). If the branch does not match, stop and tell the user: "This branch doesn't follow the dispatch naming convention (`{id}-{slug}`). Cannot determine task ID."

## Step 2: Get task details

Call `get_task`. The response is prose with one labelled line per field; a missing line means the field is unset.

- `Base branch: <name>` is `{base_branch}`; fall back to `main` if absent.
- `Wrap-up mode: <mode>` (`rebase`, `pr` or `done`) is printed only when set. If present and no argument was given, treat it as an argument: skip Step 4 and use that action.

A preset mode is the user's earlier answer, not a pause. Run everything from here without further input and do not stop before `exit_session` returns. The failure is silent: agents have gone idle at a blank prompt with the branch unmerged and the task stuck in `running`.

## Step 3: Simplify code changes (conditional)

If the branch changed only docs, configs, snapshots or lock files, skip this step. Otherwise sum added plus removed source lines, committed and not:

```bash
git diff {base_branch}..HEAD --numstat -- '*.rs' '*.py' '*.ts' '*.js' '*.tsx' '*.jsx' '*.go' '*.java' '*.cpp' '*.c' '*.h' '*.swift' '*.kt' '*.rb' '*.cs'
git diff --numstat -- '*.rs' '*.py' '*.ts' '*.js' '*.tsx' '*.jsx' '*.go' '*.java' '*.cpp' '*.c' '*.h' '*.swift' '*.kt' '*.rb' '*.cs'
```

Under **100** changed lines, skip (task #4973's retrospective: review sub-agents rarely find anything on a diff that small). At 100 or more, run `Skill({ skill: "simplify" })`; its changes are committed in Step 6. Its summary is not a stopping point: go straight on to Step 4.

## Step 4: Ask the user to choose

Skip this step if an argument or `Wrap-up mode` already gave the action.

Otherwise use `AskUserQuestion` and wait. Rebase fast-forwards a shared branch, the PR path opens a PR under the user's identity, and done integrates nothing; even work that looks like a PR may want a rebase. Ask:

> Wrap up task #{id} (`{title}`):
> **(r)** rebase onto `{base_branch}` — dispatch fast-forwards `{base_branch}` and kills this tmux window
> **(p)** author and create a draft PR — you draft the title/body, run `gh pr create`, then record the URL via `exit_session`
> **(d)** done — no git operations
> **(Esc / n)** cancel

On cancel, exit without calling any tool.

## Step 5: Run the retro

Run `Skill({ skill: "retro" })` and tell it which action you are wrapping up with. It runs after the action is settled (on `done` a fix would be stranded, so retro files a task instead) and before the commit (so its fixes travel with your work). Leave the follow-up tasks it files alone.

## Step 6: Commit uncommitted changes

Run `git status --porcelain`. If it is not clean, `git add` the relevant files by name, review `git diff --cached`, and `git commit -m "..."` with a short message. `wrap_up` refuses a dirty worktree.

## Step 7: The closing sequence

Run these back to back in one turn; any pause risks going idle with `{base_branch}` already fast-forwarded and the task still in its old status.

1. **Rate retrieved knowledge.** `rate_learning(learning_id, task_id, verdict)` for each entry from "## Validated knowledge for this task" or `query_learnings` that you acted on and have not rated: `helped` if you applied it, `wrong` if it misled you (or `delete_learning` if clearly wrong). See the `learnings` skill.
2. **`wrap_up(task_id, action)`.** It returns an **Exit token**. If it refuses — uncommitted changes, a failing verify command, a rebase conflict — show the user the exact message. For a verify failure or a dirty tree, fix it, commit (Step 6) and call `wrap_up` again; for a conflict, suggest resolution steps. Any other refusal (a backlog or review-tagged task, no worktree, the main checkout not on `{base_branch}`): show the exact message and stop. Do not call `exit_session` without a token.
3. **`exit_session(task_id, token, action)`**, plus `pr_url` on the pr path only. Make it your final action. If it errors, or succeeds but reports that the close did not take effect, read `references/exit-session-errors.md` before reacting.
