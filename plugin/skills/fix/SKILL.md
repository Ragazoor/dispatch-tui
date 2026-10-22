---
name: fix
description: >-
  File an issue as a fix task and dispatch it to a new agent in the repo that
  owns it. Use when the user says "/fix", "dispatch a fix for this", "someone
  should fix this", or you hit a bug you should not fix in this session.
---

# Fix

**Announce at start:** "I'm using the fix skill to dispatch a task for this."

Turn the issue in the conversation into a task on the board and start an agent
on it. Do not edit any code yourself: the new agent works in its own worktree.

## Step 1: Write the task

The new agent has none of this conversation. From the context, write:

- `title`: one line naming the defect, not the fix.
- `description`: what is wrong, how to see it (command, error text, file and
  symbol, repo and branch you saw it on), what you already ruled out, and the
  fix if you know it. Leave out secrets and tokens: the description is stored
  in the shared board. If the user gave extra words after `/fix`, treat them as
  the issue.

If the context does not say what is wrong, ask the user one question.

## Step 2: Find the repo

Work out which repo owns the defect, which may not be the one you are in. If
two repos are plausible, ask the user. Then, in that repo, in one Bash call
(`cd <path>; git ...`, never `git -C`):

```bash
cd <repo>; git rev-parse --path-format=absolute --git-common-dir
```

Strip the trailing `/.git`. That main repo root is `repo_path`, never a
worktree path. If the command fails, or the output does not end in `/.git`,
ask the user for the path instead of guessing. Do not edit the repo or switch
its branch; the task is how work reaches it.

## Step 3: Check for a duplicate

Call `list_tasks` with `query` set to the key words of the defect and
`repo_paths` set to `[repo_path]`, so the search is not limited to your own
epic. If an open task already covers it, tell the user its id and stop.

## Step 4: Confirm

Nothing is created until the user confirms. This is also the guard against
fix tasks spawning more fix tasks on their own. Ask with `AskUserQuestion`,
showing the title, the repo and the epic. Offer: dispatch, edit the title or
description, or cancel. Apply what they ask for.

## Step 5: Create and dispatch

Call the `dispatch` MCP tool `create_task` with `tag`: `"fix"`, the board
label that says a person asked for this fix. For `epic_id`, pass your own epic
only if the fix belongs to it; otherwise pass `null` for a standalone task.

If `create_task` errors, for example on a closed epic, report the error and
stop. Otherwise call `dispatch_task` with the returned id. If that errors,
report the task id and the error, and leave the task in backlog.

## Step 6: Report

Say the task id, the repo and that it is dispatched. Say the new agent starts
at a design step and waits for the user in its tmux window. Then carry on with
your own work.
