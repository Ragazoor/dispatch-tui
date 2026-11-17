---
max_turns: 15
timeout_seconds: 300
allowed_tools: [Read, Glob, Grep, Skill, mcp__dispatch__learning_rate, mcp__dispatch__learning_search, mcp__dispatch__learning_record, mcp__dispatch__learning_delete]
tags: [learnings]
runs: 2
---
I am working on task #42. My prompt had a "Validated knowledge" entry: "[#7 repo] task_search returns prose with labelled lines, not JSON." I read it and parsed the task_search reply by its labelled lines, which worked, so it saved me from a mistake. Use the learnings skill to do what it says about that entry.
