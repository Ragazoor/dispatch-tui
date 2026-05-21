---
name: summarize
description: >-
  Summarize what the AI agent accomplished in this session. Produces a structured
  account of the work done — task goal, commits, files changed, and outcome. Use
  when asked to "summarize", "recap the session", "what did you do", or "session
  summary". Also useful before wrap-up to draft PR bodies.
---

# Summarize Session

Generate a structured summary of what was accomplished in this session.

**Announce at start:** "I'm using the summarize skill to generate a session summary."

## Step 1: Get context

### Determine branch and task ID

Run:
```bash
git rev-parse --abbrev-ref HEAD
```

If the branch matches `{integer}-{slug}` (e.g. `42-fix-login-bug`), extract the integer as the task ID. Otherwise, proceed without a task ID.

### Get task details (if task ID found)

Call the `dispatch` MCP tool `get_task` with the task ID. Read:
- `title` — what the task was called
- `description` — the original goal
- `base_branch` — diff target (fall back to `main` if absent)

If dispatch MCP is unavailable or no task ID was found, skip this and derive context from git history and the conversation.

## Step 2: Gather git changes

Diff target is `{base_branch}` from Step 1, or `main` if not in a dispatch worktree.

Run:
```bash
git log {base_branch}..HEAD --oneline
```

```bash
git diff {base_branch}...HEAD --stat
```

If the branch has no commits ahead (e.g. the work is only staged or unstaged), fall back to:
```bash
git diff --stat
```

## Step 3: Synthesize the summary

Using:
- The task title and description from Step 1 (if available)
- The commit list and diff stat from Step 2
- Your conversation context — what you actually did and why during this session

Write a structured summary in this format:

```
## Session Summary: {task title or branch name}

**Goal**: {task description, or a brief statement inferred from the work done}

**Commits** ({N} commits):
- `{short hash}` {commit message}
- ...

**Changes** ({N} files):
- `{file path}` — {one phrase describing what changed}
- ...

**Outcome**: {one sentence — what was accomplished, and any remaining issues or caveats}
```

**Rules**:
- Every bullet stays on one line
- File descriptions come from your knowledge of the session (not just git stat)
- If there are no commits (only uncommitted changes), omit the Commits section and describe the uncommitted work under Changes
- If the diff is large (>20 files), group files by directory and summarise each group rather than listing every file
- Skip any section that doesn't apply (e.g. no Changes if nothing was modified)
- "Outcome" must name what was actually achieved, not just restate the goal

## Step 4: Display the summary

Output the summary to the user. This is always the final step — do not offer to post the summary anywhere unless the user explicitly asks.

The summary can be reused directly as the body of a PR (in wrap-up) or as notes for a follow-up task.
