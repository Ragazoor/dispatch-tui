---
name: pr-workflow
description: Invoke before creating or updating a PR to surface PR conventions and preferences from the knowledge base.
---

# PR Workflow

Invoke this skill before creating or updating a pull request.

**Announce:** "Using /pr-workflow to check knowledge base for PR conventions."

## Steps

1. Call `query_learnings(task_id=<your task id>, tag_filter="pr,pull-request")` to surface relevant entries
2. Apply any PR format preferences or conventions from the results
3. Base guidance (apply unless knowledge base overrides):
   - Ensure all tests pass before opening
   - Link the relevant issue in the PR description
   - Keep PRs focused — one logical change per PR
   - `gh pr create --title "..." --body "$(cat <<'EOF' ... EOF)"`
4. For each retrieved entry that proved useful during your PR work: call `upvote_learning(learning_id=<id>, task_id=<your task id>)`

## What to record

Call `record_learning` if you observe a PR convention not already in the knowledge base:
- Team PR format preferences ("always include a test plan section")
- Repo-specific review process requirements
- Do NOT record one-off task context or how you fixed a specific bug
