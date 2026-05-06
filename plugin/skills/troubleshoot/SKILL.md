---
name: troubleshoot
description: Invoke when hitting a build failure, test failure, or CI error to check the knowledge base before debugging blindly.
---

# Troubleshoot

Invoke this skill when you encounter a build failure, failing test, or CI error.

**Announce:** "Using /troubleshoot to check knowledge base before debugging."

## Steps

1. Call `query_learnings(task_id=<your task id>, tag_filter="debugging,error,pitfall")` to check whether a previous agent has seen this
2. If a relevant entry exists, try the described approach before your own investigation
3. Base guidance:
   - Read the full error output carefully before searching — the message usually tells you the cause
   - Run the specific failing test in isolation before running the full suite
   - `cargo test <test_name> -- --nocapture` to see println output
   - Check recent git log for changes that might have introduced the failure: `git log --oneline -10`
4. For each retrieved entry that proved useful: call `upvote_learning(learning_id=<id>, task_id=<your task id>)`

## What to record

Call `record_learning` with `kind="pitfall"` if you discover a recurring failure pattern:
- Known failure classes that recur across tasks ("always run snapshot tests after UI changes")
- Non-obvious root causes for common errors
- Do NOT record one-off fixes specific to a single task
