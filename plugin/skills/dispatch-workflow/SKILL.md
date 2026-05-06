---
name: dispatch-workflow
description: Invoke when using dispatch MCP tools to surface effective workflow patterns from the knowledge base.
---

# Dispatch Workflow

Invoke this skill when you are about to use dispatch MCP tools in a non-trivial way.

**Announce:** "Using /dispatch-workflow to check knowledge base for effective dispatch patterns."

## Steps

1. Call `query_learnings(task_id=<your task id>, tag_filter="dispatch,mcp,workflow")` to surface relevant entries
2. Apply any patterns from the results
3. Core dispatch patterns (always apply):
   - **Cross-repo investigation:** dispatch a new task in the target repo and use `send_message` to receive findings — do NOT chain `cd` commands across repos
   - **Cross-task collaboration:** use `send_message(task_id=<sibling>, message="...")` to share findings or ask questions
   - **Status updates:** call `update_task(task_id=<id>, status="...")` to keep the kanban board current
   - **Sub-task creation:** always pass `project_id` and `epic_id` (if applicable) when calling `create_task`
4. For each retrieved entry that proved useful: call `upvote_learning(learning_id=<id>, task_id=<your task id>)`

## What to record

Call `record_learning` with `kind="convention"` or `kind="preference"` if you discover:
- Effective MCP tool combinations or sequencing patterns
- Cross-repo collaboration approaches that worked well
- Do NOT record task-specific solutions — only broadly reusable patterns
