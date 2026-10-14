---
max_turns: 15
timeout_seconds: 300
allowed_tools: [Read, Glob, Grep, Skill]
tags: [grill]
runs: 2
---
Grill me on this plan: "Let agents retry a failed wrap_up automatically up to 5 times, with no delay between tries, and mark the task Failed if all 5 fail." Interview me. Git and the shell are not available, and the workspace is empty, so do not search for files.
