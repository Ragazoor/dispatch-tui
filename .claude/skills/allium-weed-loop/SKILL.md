---
description: "Ralph loop that runs allium weed to find undocumented behavior, updates the spec, and asks about code bugs"
allowed-tools: ["Read", "Write", "Bash"]
---

# Allium Weed Loop

This skill starts a ralph loop that iteratively aligns the allium spec with the implementation.

## Instructions

1. **Read the prompt file** at `.claude/skills/allium-weed-loop/prompt.md`.

2. **Create the ralph loop state file** directly at `.claude/ralph-loop.local.md` using the Write tool. Use this exact format, substituting the prompt content from step 1:

```markdown
---
active: true
iteration: 1
session_id: SESSION_ID
max_iterations: 10
completion_promise: "SPEC ALIGNED"
started_at: "TIMESTAMP"
---

[PROMPT CONTENT FROM prompt.md]
```

Get the session ID by running `echo $CLAUDE_CODE_SESSION_ID` and the timestamp with `date -u +%Y-%m-%dT%H:%M:%SZ`.

3. **Tell the user** the ralph loop is active, then start working on the prompt immediately.
