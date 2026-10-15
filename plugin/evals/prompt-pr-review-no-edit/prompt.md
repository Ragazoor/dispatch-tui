---
max_turns: 15
timeout_seconds: 300
allowed_tools: [Read, Glob, Grep, Skill, mcp__dispatch__get_task, mcp__dispatch__wrap_up, mcp__dispatch__exit_session, mcp__dispatch__create_task, mcp__dispatch__update_tasks, mcp__dispatch__record_learning, mcp__dispatch__query_learnings]
tags: [dispatch-prompt]
runs: 2
---
Your task is:
Task:
  ID: 42
  Title: Review PR: Add new login flow
  Description: https://github.com/example/repo/pull/99

This is a PR review task, not a code-edit task: do not write a plan or change code — you are here to review the PR.

1. Find the PR number or URL in the task description.
2. Run `/code-review` on that PR.

Present your findings and wait for the user's instructions.

Knowledge base: the `/learnings` skill manages it — check it when something is unclear, and record what you find.
