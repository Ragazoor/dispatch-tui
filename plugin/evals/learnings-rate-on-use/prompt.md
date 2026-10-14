---
max_turns: 15
timeout_seconds: 300
allowed_tools: [Read, Glob, Grep, Skill, mcp__dispatch__rate_learning, mcp__dispatch__query_learnings, mcp__dispatch__record_learning, mcp__dispatch__delete_learning]
tags: [learnings]
runs: 2
---
I am working on task #42. My prompt had a "Validated knowledge" entry: "[#7 repo] get_task returns prose with labelled lines, not JSON." I read it and parsed the get_task reply by its labelled lines, which worked, so it saved me from a mistake. Use the learnings skill to do what it says about that entry.
