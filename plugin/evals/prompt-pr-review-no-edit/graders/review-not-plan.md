---
type: llm
focus: trace
---
PASS if the agent treated this as a review of PR 99: it tried to run the code-review step on that PR (or said it could not and reviewed another way), wrote no plan, and stopped to wait for the user.
FAIL if it wrote a plan, proposed to implement changes itself, or finished the task without reviewing.
