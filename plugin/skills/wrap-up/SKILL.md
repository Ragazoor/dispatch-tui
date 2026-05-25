---
name: wrap-up
description: Use when implementation is complete to wrap up a dispatch worktree. Commits remaining changes, asks the user to choose between rebasing onto the task's base_branch or authoring + creating a draft GitHub PR yourself. Rebase is dispatch-driven; PR is agent-driven (you write the title and body, then call dispatch to record the URL).
---

# Wrap Up

Wrap up a dispatch worktree. The two paths:

- **rebase** — dispatch handles it. The MCP `wrap_up` tool with `action: "rebase"` does the work; the task moves to Done and your tmux window is killed.
- **pr** — you handle it. Inspect the diff you produced, write a real title and body that describe what was actually built, run `gh pr create --draft` yourself, then call MCP `wrap_up(action="pr", pr_url=<url>)` to record the URL and move the task to Review. Do not call `exit_session` — PR polling drives the task to Done on merge. Dispatch deliberately no longer authors PR bodies because the auto-generated bodies were always worse than what you can write after seeing the work.

**Announce at start:** "I'm using the wrap-up skill to complete this task."

## Argument check

If the skill was invoked with an argument (e.g. `/wrap-up rebase` or `/wrap-up pr`):
- Treat the argument as the chosen action (`rebase` or `pr`)
- Skip Step 4 (AskUserQuestion) entirely
- After completing Steps 1–3, go straight to Step 5 with that action

If the argument is anything other than `rebase` or `pr`, ignore it and proceed normally (Step 4 will ask).

**Precondition:** The task must be in "running" or "review" status. Both rebase and PR paths require the task to be wrappable.

## Step 1: Get the task ID from the current branch

Run:
```bash
git rev-parse --abbrev-ref HEAD
```

Extract the leading integer from the `{id}-{slug}` pattern (e.g. `42-fix-login-bug` → `42`).

If the branch does not match the `{id}-{slug}` pattern, stop and tell the user:
> "This branch doesn't follow the dispatch naming convention (`{id}-{slug}`). Cannot determine task ID."

## Step 2: Get task details and dispatch next epic subtask

Call the `dispatch` MCP tool `get_task` with the task ID from Step 1. Read the `base_branch` field from the response — use it wherever the instructions below refer to `{base_branch}`. If `base_branch` is absent or empty, fall back to `main`.

If the task has an `epic_id`, call the `dispatch` MCP tool `dispatch_next` with that `epic_id`. This fires the next agent immediately — before any user interaction.

If the task does not have an `epic_id`, skip the dispatch_next call.

## Step 2.5: Simplify code changes (conditional)

Check whether code was written in this branch — both committed and uncommitted:

```bash
git diff {base_branch}..HEAD --name-only
git diff --name-only
```

If the combined output includes any source code files (`.rs`, `.py`, `.ts`, `.js`, `.tsx`, `.jsx`, `.go`, `.java`, `.cpp`, `.c`, `.h`, `.swift`, `.kt`, `.rb`, `.cs`) — i.e., not only docs, configs, snapshots, or lock files — invoke the `code-simplifier` skill (not `kognic-code-quality` or any other code review skill):

```
Skill({ skill: "code-simplifier:code-simplifier" })
```

Wait for the skill to complete before proceeding. If it makes additional changes, those will be picked up in Step 3.

If there are no code file changes, skip this step entirely.

## Step 3: Commit uncommitted changes

Run:
```bash
git status --porcelain
```

If there are no changes, skip to Step 4.

If there are changes, commit them inline — do NOT invoke a commit skill or delegate to another tool. Run these commands directly:

1. `git add` the relevant files (prefer named files over `git add -A`)
2. `git diff --cached` to review what's staged
3. `git commit -m "..."` with a short message summarizing the changes

Do NOT spend time perfecting the commit message. The goal is to capture the changes, not write a polished commit. Once committed, proceed immediately to Step 4.

## Step 4: Ask the user to choose — MANDATORY

**You MUST use the `AskUserQuestion` tool here.** Do NOT skip this step. Do NOT assume a default. Do NOT proceed to Step 5 without an explicit answer from the user.

Use the `AskUserQuestion` tool with a question like:

> Wrap up task #{id} (`{title}`):
> **(r)** rebase onto `{base_branch}` — dispatch fast-forwards `{base_branch}` with this branch and kills this tmux window
> **(p)** author and create a draft PR — you draft the title/body, run `gh pr create`, then record the URL via update_task
> **(Esc / n)** cancel

If the user cancels or says no, exit without calling any tool.

## Step 5: Execute the chosen action

The task is automatically moved to "done" (rebase) or "review" (PR) on success. Do not update the task status manually except as described below for the PR path.

### If rebase:

This is a **two-call** sequence. Both calls are mandatory.

Before calling `wrap_up`, decide on **learning verdicts** for any knowledge surfaced during this task — see *Validate retrieved knowledge* below.

**Call 1 — rebase.** Call the `dispatch` MCP tool `wrap_up` with:
- `task_id`: the integer from Step 1
- `action`: `"rebase"`
- `learning_verdicts` (optional): the list you assembled in *Validate retrieved knowledge*

The tool blocks until the rebase completes and fast-forwards `{base_branch}`. It does **not** close the session — the tmux window stays alive and the task stays in its current status until you make the second call.

If `wrap_up` returns an error (e.g. rebase conflict, repo not on `{base_branch}`), show the user the exact error message and suggest resolution steps. Do not call `exit_session`. The task remains in its current status.

**Call 2 — reflect.** On a successful `wrap_up` response, the response text contains an **Exit token** (a UUID string). Immediately call the `dispatch` MCP tool `exit_session` with:
- `task_id`: the integer from Step 1
- `token`: the exit token from the wrap_up response

`exit_session` returns a reflection prompt. If you found pitfalls, conventions, or tool tips during this session, call `record_learning` for each one now.

**Call 3 — close.** Call `exit_session` again with the same `task_id` and `token`. This closes the session. The token is consumed and cannot be reused.

Do NOT stop between Call 1 and Call 3. Skipping `exit_session` leaves the tmux window alive and the task stuck.

#### Validate retrieved knowledge

When dispatch starts an agent, it injects relevant knowledge into the prompt under "## Validated knowledge for this task". Agents may also call `query_learnings` mid-task. Each surfacing is recorded as a retrieval; at wrap-up the knowledge base needs to know whether each entry was useful.

For every learning that was injected into your prompt or returned by `query_learnings` during this task, decide one of:

- `helped` — the entry was relevant and you applied it. Acts as an upvote.
- `unused` — the entry appeared but did not apply to this task. Recorded for telemetry; honest "not applicable" — not a default.
- `wrong` — the entry was misleading, outdated, or contradicts current code. Routes the entry to `needs_review` for human curation.

Then pass the verdicts to `wrap_up`:

```jsonc
{
  "task_id": 42,
  "action": "rebase",
  "learning_verdicts": [
    {"learning_id": 7, "verdict": "helped"},
    {"learning_id": 12, "verdict": "unused"},
    {"learning_id": 19, "verdict": "wrong"}
  ]
}
```

Skipping verdicts is allowed (omit the field), but leaves the knowledge base unable to learn from this task. Provide verdicts whenever retrievals exist.

### If PR — author the PR yourself, then record the URL:

You are creating a real PR with a title and body that reflect the actual work. Dispatch will not do this for you. Follow this sub-flow:

#### 5a. Inspect what changed

```bash
git log {base_branch}..HEAD --oneline
git diff {base_branch}...HEAD --stat
git diff {base_branch}...HEAD
```

Read the output. Build a mental model of what shipped: which files changed and why, which behaviours were added/removed/fixed, what the user-visible effect is. If the diff is large, focus on the changes that matter for review (skip generated files, snapshot updates, formatting churn).

#### 5b. Draft the PR title and body

**Title** — imperative mood, ≤72 characters, describes the change as a single action. Examples:
- `fix(auth): handle expired refresh tokens without 500ing`
- `feat(tui): add project filter to archive view`
- `refactor(db): split TaskPatch builder into smaller methods`

Avoid `wip:`, `task #N:`, or anything that just restates the task title. The title should be useful in `git log --oneline`.

**Body** — Markdown, this structure:

```markdown
## Summary
- {what changed and why, 1–4 bullets, plain language}
- {keep one bullet per logical change so reviewers can scan}

## Test plan
- [ ] {how to verify the change manually or via tests}
- [ ] {any edge case worth re-running}
- [ ] {tests added/updated, if relevant}

Implements #{task_id}.
```

If the change has UI implications, add screenshots or a description of the visual effect under a `## Notes` section. Skip sections that don't apply (e.g. no Test plan if the change is documentation-only) — don't pad.

#### 5c. Push and create the draft PR

Find the repo slug from the remote:

```bash
git remote get-url origin
```

The slug is the `owner/repo` portion (e.g. `git@github.com:Acme/dispatch.git` → `Acme/dispatch`).

Push the branch:

```bash
git push -u origin {branch}
```

If the push is rejected (non-fast-forward), STOP. Do not force-push without the user's explicit authorisation. Show them the error and ask how to proceed.

Create the PR. Use a HEREDOC for the body so newlines and Markdown survive shell quoting:

```bash
gh pr create --draft \
  --base {base_branch} \
  --head {owner}:{branch} \
  --repo {owner}/{repo} \
  --title "{your authored title}" \
  --body "$(cat <<'EOF'
{your authored body}
EOF
)"
```

`{owner}` is the first part of the repo slug. The `{owner}:{branch}` format is required so `gh` resolves the branch in the same repo as `--repo` (rather than your authenticated user's namespace).

`gh pr create` prints the PR URL on stdout. Capture it.

If `gh` reports `a pull request for branch '...' already exists`, parse the URL it returns and use that — the PR already exists and your job is just to record it.

#### 5d. Record the PR with dispatch

Call the `dispatch` MCP tool `wrap_up` with:
- `task_id`: the integer from Step 1
- `action`: `"pr"`
- `pr_url`: the URL from Step 5c

This moves the task to Review and starts PR status polling.

**Do NOT call `exit_session` after this.** The PR path does not issue an exit token. Your session is complete once `wrap_up(action="pr")` succeeds. Dispatch moves the task to Done automatically when the PR merges.

If `wrap_up` returns an error, show the user the exact error message. Do not retry creating the PR — it already exists. Fix the reported issue then retry the `wrap_up` call, or ask the user to record the URL manually from the TUI.
