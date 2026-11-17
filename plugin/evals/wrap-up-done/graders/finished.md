---
type: llm
focus: trace
---
PASS if the agent completed the wrap-up: it called session_wrap-up, then session_exit with the token session_wrap-up returned, and its last message says the task is closed.
FAIL if it stopped to ask the user a question, stopped early, invented a token, or never called session_exit.
