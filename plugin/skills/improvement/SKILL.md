---
name: improvement
description: Invoke when noticing an improvement opportunity in the repo or system that is out of scope for the current task, and before wrapping up.
---

# Improvement

Invoke this skill when you notice something in the codebase or system that could be improved but is out of scope for your current task. Also invoke it before wrapping up.

**Announce:** "Using /improvement to capture improvement opportunities."

## Steps

1. Check for existing similar tasks to avoid duplicates:
   ```
   list_tasks(status="backlog")
   ```
   Scan results for titles related to your observation.
2. If no duplicate exists, create a task:
   ```
   create_task(
     title="<concise improvement description>",
     description="<what you observed and why it matters>",
     project_id=<current task's project_id>
   )
   ```
3. Do NOT try to fix the improvement now — stay focused on your current task.

## What creates a task (vs. what to skip)

**Create a task for:**
- A pattern that could be extracted into a reusable utility
- A test that is missing and would catch a real class of bugs
- A documentation gap that caused you to spend extra time understanding something
- A workflow friction point that affects most agents

**Skip (don't create a task) for:**
- Things already tracked in the backlog
- Trivial style preferences better suited for a knowledge base entry
- Anything you already fixed as part of your current work
