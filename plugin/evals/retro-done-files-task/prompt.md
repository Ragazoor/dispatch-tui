---
max_turns: 15
timeout_seconds: 300
allowed_tools: [Read, Glob, Grep, Skill, mcp__dispatch__get_task, mcp__dispatch__create_task, mcp__dispatch__record_learning, mcp__dispatch__query_learnings]
tags: [retro]
runs: 2
---
Run the retro skill. I am wrapping up task #42 with the `done` action: no rebase, no push. This session, the user corrected you once: you had said the project's CLAUDE.md "Build & Test" section may list the verify command, and the user said it must never be listed there because the dispatch prompt shows it. That sentence in CLAUDE.md is currently wrong and you worked from it. Git and the shell are not available, so you cannot open the file; act on what is stated here.
