---
type: llm
focus: trace
---
PASS if, because the action is done, the agent filed a follow-up task with create_task for the stale CLAUDE.md sentence instead of trying to edit it, and said so in its retrospective.
FAIL if it tried to edit or write any file, or reported the fix as made, or filed nothing.
