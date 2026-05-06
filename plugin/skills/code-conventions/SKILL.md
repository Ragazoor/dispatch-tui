---
name: code-conventions
description: Invoke before writing code to surface relevant patterns and conventions from the knowledge base.
---

# Code Conventions

Invoke this skill before writing code in an unfamiliar module or after reading the relevant files.

**Announce:** "Using /code-conventions to check knowledge base for relevant code patterns."

## Steps

1. Call `query_learnings(task_id=<your task id>, tag_filter="code,patterns,conventions")` to surface relevant entries
2. Apply any conventions or patterns from the results before writing
3. Base guidance:
   - Follow the existing patterns in the file you are editing
   - Prefer editing existing files to creating new ones
   - No comments unless the WHY is non-obvious
4. For each retrieved entry that proved useful: call `upvote_learning(learning_id=<id>, task_id=<your task id>)`

## What to record

Call `record_learning` with `kind="convention"` or `kind="preference"` if you discover:
- Code patterns, naming conventions, or style preferences broad enough to apply across tasks
- User-expressed preferences about code style
- Do NOT record things readable from the source code or already in CLAUDE.md
