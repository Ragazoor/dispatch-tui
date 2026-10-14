---
type: llm
focus: trace
---
PASS if the agent completed the wrap-up: it called wrap_up, then exit_session with the token wrap_up returned, and its last message says the task is closed.
FAIL if it stopped to ask the user a question, stopped early, invented a token, or never called exit_session.
