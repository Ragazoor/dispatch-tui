---
name: test-conventions
description: Invoke before writing tests to surface testing conventions and placement rules from the knowledge base.
---

# Test Conventions

Invoke this skill before writing tests.

**Announce:** "Using /test-conventions to check knowledge base for testing conventions."

## Steps

1. Call `query_learnings(task_id=<your task id>, tag_filter="testing,tests")` to surface relevant entries
2. Apply any test conventions from the results
3. Base guidance (see CLAUDE.md "Where New Tests Go" table for the full rules):
   - TUI key handling → `src/tui/tests/`
   - DB schema/CRUD → `src/db/tests/`
   - Business rules → inline in `src/service/`
   - MCP handler behaviour → `src/mcp/handlers/tests/`
   - Full lifecycle → `tests/` (integration tests)
   - Use `Database::open_in_memory()` — never mock the DB layer
4. For each retrieved entry that proved useful: call `upvote_learning(learning_id=<id>, task_id=<your task id>)`

## What to record

Call `record_learning` with `kind="convention"` if you discover:
- Test placement patterns not covered by the CLAUDE.md table
- Preferred assertion patterns specific to this codebase
- Do NOT record general testing best practices — those belong in documentation
