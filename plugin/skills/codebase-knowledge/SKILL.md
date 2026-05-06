---
name: codebase-knowledge
description: Invoke when exploring an unfamiliar codebase to surface existing architecture and landscape entries.
---

# Codebase Knowledge

Invoke this skill before exploring an unfamiliar area of the codebase.

**Announce:** "Using /codebase-knowledge to check existing knowledge before exploring."

## Steps

1. Call `query_learnings(task_id=<your task id>, tag_filter="codebase,architecture,conventions")` to surface existing entries
2. Use any `landscape` entries as your starting map — avoid re-exploring what is already documented
3. Base guidance:
   - Read CLAUDE.md and relevant READMEs before broad exploration
   - Use the Explore subagent for codebase-wide searches
4. For each retrieved entry that proved useful: call `upvote_learning(learning_id=<id>, task_id=<your task id>)`

## What to record

After exploring, call `record_learning` with `kind="landscape"` if you built an understanding worth sharing:
- Service ownership maps ("Service X owns auth, service Y owns billing — they communicate via gRPC")
- Module responsibility summaries
- Non-obvious architectural dependencies
- Do NOT record things already in CLAUDE.md, READMEs, or git history
