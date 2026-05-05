---
name: learnings
description: Use at task start to query existing learnings and understand when and how to record new ones. Guides agents through the full learning lifecycle — surfacing, confirming, and recording knowledge effectively.
---

# Learnings

Use this skill at the start of every task. It covers three moments: surfacing existing knowledge, confirming what proves useful, and deciding what to record.

**Announce at start:** "I'm using the learnings skill to surface relevant past experience for this task."

## Step 1: Query at task start

Call the `query_learnings` MCP tool with your `task_id`.

Read the results as context — they describe past conventions, pitfalls, and decisions that may be relevant to your work. Treat them as useful background, not hard rules.

If the results include similar-entry warnings (the system echoes existing entries when you later record), consider calling `confirm_learning` on an existing entry rather than adding a near-duplicate.

If no learnings are returned, proceed normally.

## Step 2: Confirm when useful — during the task

When a retrieved learning proves **useful** during your work — you acted on it and it helped — call `confirm_learning` with its `learning_id` and your `task_id`. Do this at the moment of confirmation, not deferred to wrap-up.

**Call `confirm_learning` when:** A learning saved you from a pitfall, matched a convention you applied, or guided a decision you made.

**Don't call it for:** Learnings you read but didn't act on, or learnings that turned out to be wrong or outdated (flag those to the user instead).

## Step 3: Record at wrap-up — decide what's worth keeping

Before finishing, ask: *Did I discover anything non-obvious that a future agent would benefit from knowing?*

### Record if:

- You hit a **non-obvious pitfall** that isn't documented and would trap future agents
- You found a **convention** that applies broadly but isn't visible from reading the code
- The user expressed a **preference** explicitly that isn't already in CLAUDE.md
- A specific **tool or approach** solved a problem in a way worth re-applying
- This epic or project has a **procedural step** every agent working here should follow
- The task had a complex arc worth capturing as an **episodic note** (what was tried, what worked)

### Do NOT record:

- Code patterns **readable from source code** — the code is self-documenting
- Things already in **CLAUDE.md**, README, or existing docs
- **Git history** — visible via `git log` / `git blame`
- **Debugging solutions** where the fix is already in the commit — the commit message captures the context
- **General language or framework best practices** — these belong in documentation, not learnings
- Things **too specific to generalize** — if it won't apply to other tasks, skip it or use `scope=task`
- **Duplicate entries** — if the system echoes a similar existing learning when you record, call `confirm_learning` on that entry instead

### Picking a scope

| Scope | Use when | `scope_ref` |
|-------|----------|-------------|
| `user` | Personal workflow preference, applies to all your work | omit |
| `repo` | Codebase-wide convention or pitfall | omit (auto-derived) |
| `project` | Applies to all tasks in this project | omit (auto-derived) |
| `epic` | Shared design decision for this epic only | omit (auto-derived; task must belong to an epic) |
| `task` | One-off note for this task; not auto-injected | omit (auto-derived) |

**Default to `repo` for code conventions and `user` for workflow preferences.** Use `epic` for architectural decisions that every subtask in this epic should know. Use `task` for ephemeral notes you may query later — they won't appear in future agents' prompts automatically.

### Picking a kind

| Kind | Use for |
|------|---------|
| `pitfall` | Silent failures, API traps, behavior surprises — warn future agents away |
| `convention` | Preferred patterns or style for this codebase |
| `preference` | Explicit user preference expressed during the task |
| `tool_recommendation` | Specific tool or library for a problem type |
| `procedural` | Step-by-step instructions to prefix dispatch prompts (epic-level only) |
| `episodic` | Outcome summary of this task — what was attempted, what worked (usually `scope=task`) |

### Writing a good summary

- **One sentence only.** If you need two, the learning is too broad — split it or drop it.
- **Name the specific thing.** Not "be careful with DB queries" but "TaskPatch double-Option means `Some(None)` clears a field, `None` leaves it unchanged."
- **Lead with the surprise.** "Despite what the API suggests, …" or "Unlike TaskService, EpicService does not …"
- **Include a scope signal.** Mention the filename, function name, or context so the reader knows when it applies.
