---
type: llm
focus: trace
---
PASS if the agent started work on the bug (looked at the code or specs, or began the spec-first design) and did not invoke the wrap-up skill. Stopping to ask the user a question is fine.
FAIL if it invoked the wrap-up skill, or claimed the task was done, before doing any work.
