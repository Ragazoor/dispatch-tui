---
max_turns: 12
timeout_seconds: 300
allowed_tools: [Read, Glob, Grep, Skill, mcp__dispatch__get_task, mcp__dispatch__wrap_up, mcp__dispatch__exit_session, mcp__dispatch__create_task, mcp__dispatch__update_tasks, mcp__dispatch__record_learning, mcp__dispatch__query_learnings]
tags: [dispatch-prompt]
runs: 2
---
Your task is:
Task:
  ID: 42
  Title: Fix the authentication bug
  Description: Users cannot log in after the password hash migration

If the task changes what the system does, design it spec-first: elicit the intended behaviour with `allium:elicit`, capture it in docs/specs/ with `allium:tend`, generate tests with `allium:propagate` and confirm they fail before you write any code, implement the minimum that makes them pass, then check spec and code agree with `allium:weed`.

Writing a plan to docs/plans/ and attaching it with update_tasks is your judgement call, not a requirement — do it only if the implementation is big enough that its steps are worth recording.

The spec is not the end of the task — implement it in this same session (or, for an epic-decomposition task, create work packages for its subtasks instead) and verify your work before wrapping up.

Knowledge base: the `/learnings` skill manages it — check it when something is unclear, and record what you find.

When your work is done — finishing implementation, or (for an epic-decomposition task) creating work packages for its subtasks, and not before — use the dispatch:wrap-up skill to commit any remaining changes and finalise the task.
