---
name: learnings
description: Manage the knowledge base lifecycle — query, upvote, and record entries. Use at wrap-up or whenever you want to contribute to the shared knowledge base.
---

# Knowledge Base

Use this skill to interact with the shared knowledge base — recording new entries and upvoting entries that proved useful.

For action-specific knowledge base queries at the right moment, use the domain skills:
`/codebase-knowledge`, `/code-conventions`, `/test-conventions`, `/pr-workflow`,
`/dispatch-workflow`, `/troubleshoot`, `/improvement`.

**Announce at start:** "I'm using the learnings skill to interact with the knowledge base."

## Upvoting useful entries

When a knowledge base entry proves useful during your work — you acted on it and it helped — call:
```
upvote_learning(learning_id=<id>, task_id=<your task id>)
```

Do this at the moment it helps, not deferred to wrap-up.

**Upvote when:** An entry saved you from a pitfall, matched a convention you applied, or guided a decision you made.

**Don't upvote:** Entries you read but didn't act on, or entries that turned out to be wrong.

## Recording new entries

Before finishing a task, ask: *Did I discover anything non-obvious that a future agent would benefit from knowing?*

### Record if:

- The user expressed a **preference** explicitly that isn't already in CLAUDE.md
- You built a **landscape understanding** of a codebase area worth sharing
- You found a **convention** that applies broadly but isn't visible from reading the code
- A specific **workflow pattern** solved a cross-repo or cross-task problem elegantly
- This epic or project has a **procedural step** every agent working here should follow

### Do NOT record:

- Code patterns readable from source code — the code is self-documenting
- Things already in CLAUDE.md, README, or existing docs
- Git history — visible via `git log` / `git blame`
- Debugging solutions where the fix is in the commit
- Things too specific to generalise — if it won't apply to other tasks, skip it
- How you fixed a specific problem — that's in the code and commit message

### Picking a kind

| Kind | Use for |
|------|---------|
| `pitfall` | Silent failures, API traps, behaviour surprises — warn future agents |
| `convention` | Preferred patterns or style for this codebase |
| `preference` | Explicit user preference expressed during the task |
| `tool_recommendation` | Specific tool or library for a problem type |
| `procedural` | Step-by-step instructions to prefix dispatch prompts (epic-level) |
| `landscape` | Codebase/system overviews — service maps, module responsibilities |

### Picking a scope

| Scope | Use when | `scope_ref` |
|-------|----------|-------------|
| `user` | Personal workflow preference, applies to all work | omit |
| `repo` | Codebase-wide convention or landscape entry | omit (auto-derived) |
| `project` | Applies to all tasks in this project | omit (auto-derived) |
| `epic` | Shared design decision for this epic only | omit (auto-derived) |
| `task` | One-off note; not auto-injected into future prompts | omit (auto-derived) |

**Default to `repo` for code conventions and `user` for workflow preferences.**

### Writing a good summary

- **One sentence only.** If you need two, the entry is too broad — split or drop it.
- **Name the specific thing.** Not "be careful with DB queries" but "TaskPatch double-Option means `Some(None)` clears a field, `None` leaves it unchanged."
- **Lead with the actionable insight.** What should a future agent do differently?
