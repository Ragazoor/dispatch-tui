---
name: decompose-review
description: >-
  Create work packages from code review findings, grouped by files they touch.
  Use after running "kognic-code-quality:code-review" or any code review that
  produces findings with file:line references. Creates an epic with subtasks
  and attaches a plan to each subtask.
---

# Decompose Review

Convert code review findings into actionable dispatch work packages. Each work
package becomes a subtask in an epic, with a plan file attached.

**Announce at start:** "I'm using the decompose-review skill to create work packages from the code review."

**Precondition:** A code review report must exist in the current conversation. The report should contain findings with `file:line` references grouped by severity.

## Step 1: Extract Review Findings

Read the code review report from conversation context. Take **every** finding, Low ones included: for each, take its severity, its file references (`file:line`; some name several files or omit the line), the issue and the suggested fix. Low findings are small issues to fix in passing, not only to report, so do not drop them for being minor. If the report has no findings, tell the user there is nothing to decompose and stop.

## Step 2: Group into Work Packages

Group findings by the files they touch:

1. Extract file paths from all `file:line` references
2. Group by directory prefix (first 2 path components, e.g., `src/mcp`, `src/tui`, `src/db`)
3. If a single finding references files in multiple groups, merge those groups
4. A Low finding stays in its group when that group also has a higher-severity finding. Move the Low findings of every Low-only group into one "Cleanups" work package, so Low findings do not each become a tiny package of their own
5. Order work packages by maximum severity (blockers first, then high, then medium, then low); "Cleanups" goes last
6. Name each work package after its primary directory or module (e.g., "MCP Handler Cleanup", "TUI Rendering Improvements")

Mark a finding as a **decision item** when the agent that implements it cannot choose the fix alone: the review offers more than one fix, the fix changes behaviour or a spec, or it removes something the user may want to keep. A decision item stays in its work package. Do not leave it out, and do not pick an option for the user.

## Step 3: Confirm the Work Packages

Nothing is created until the user confirms. Show a numbered list — each package's name, finding count by severity, decision item count, files and a one-line summary — and ask with `AskUserQuestion` whether to confirm all, drop or merge some. Apply what they ask for.

Every finding goes into a package by default. Under the list, add a "Not included" section that names each finding left out of every package, with the reason (for example, the review marks it as no change needed, or it duplicates another finding). Write "Not included: none" when nothing is left out, so the user can see that nothing was dropped.

This confirmation is also why Step 6 sets `auto_run_plan`.

## Step 4: Resolve Repository Path

Resolve the main repository root (not a worktree path):

```bash
git rev-parse --path-format=absolute --git-common-dir
```

Strip the trailing `/.git` to get the main repo path. `repo_path` on tasks must point to the main repo root so dispatch can create worktrees from it.

## Step 5: Create Epic

Call the `dispatch` MCP tool `create_epic` with:
- `title`: "Code Review: <YYYY-MM-DD>" (today's date)
- `description`: the executive summary from the code review report (3-5 bullet points)

Do **not** pass `repo_path` here — epics do not carry one, and `create_epic`
rejects the field outright (`unknown field 'repo_path'`). The repo path from
Step 4 belongs on each subtask in Step 6, which is what dispatch reads when
creating worktrees.

Save the returned `epic_id` for use in Step 6.

## Step 6: Write Plan Files and Create Tasks

Create the plan directory:
```bash
mkdir -p docs/plans/review-<YYYY-MM-DD>
```

**If that directory already exists, do not write into it** — a second review on the same day would interleave its work packages with the first's and renumber nothing. Add a distinguishing suffix instead (`review-<YYYY-MM-DD>-followup`, `-security`, …) and use it consistently for every path below. Say which directory you used when you report results in Step 7.

For each confirmed work package (in severity order):

### 6a. Write the plan file

Write a plan file at `docs/plans/review-<YYYY-MM-DD>/wp-<N>-<slug>.md` following the template in `references/plan-template.md`.

The plan must include:
- All findings for this work package with their severity, file:line, description, and fix
- A Decisions section for each decision item from Step 2: the question, the options and what each one costs. It tells the executing agent to ask the user before implementing that finding
- A changes table listing every file and what to change
- A verification section with test commands

### 6b. Create the subtask

Call the `dispatch` MCP tool `create_task` with:
- `title`: the work package name
- `repo_path`: main repo path from Step 4
- `description`: one-sentence summary of what this work package addresses
- `epic_id`: the epic ID from Step 5
- `plan_path`: absolute path to the plan file just written
- `tag`: `"chore"`
- `sort_order`: N (preserves severity ordering — blockers first)
- `auto_run_plan`: `true` — the plan was already reviewed via the work-package
  confirmation in Step 3, so the dispatched agent should implement it
  immediately instead of asking to proceed. Decision items are the
  exception: `auto_run_plan` skips the question whether to start, not the
  questions in the plan's Decisions section, which the agent asks before it
  implements those findings
- `wrap_up_mode`: `"rebase"` — so the agent rebases onto `base_branch` at
  wrap-up without asking which path to take. Review work packages are small and
  land on main; they don't each need their own PR.

## Step 7: Report Results

Display the created epic and all task IDs:

> Created epic #<id>: "Code Review: <date>" with <N> subtasks:
> - Task #<id>: <name> (blocker)
> - Task #<id>: <name> (high priority)
> - Task #<id>: <name> (medium)
> - Task #<id>: Cleanups (low)
>
> Dispatch the epic from the TUI to start working through the tasks.
