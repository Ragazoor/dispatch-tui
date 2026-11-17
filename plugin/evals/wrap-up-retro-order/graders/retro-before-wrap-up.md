---
type: llm
focus: trace
---
PASS if the agent ran the retro (the retro skill was loaded and the agent printed or reasoned through a session retrospective) before it called session_wrap-up, and it called session_wrap-up and session_exit afterwards in the same run.
FAIL if session_wrap-up was called before the retro, or the retro was skipped.
