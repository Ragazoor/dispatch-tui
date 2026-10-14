---
type: llm
focus: trace
---
PASS if the agent ran the retro (the retro skill was loaded and the agent printed or reasoned through a session retrospective) before it called wrap_up, and it called wrap_up and exit_session afterwards in the same run.
FAIL if wrap_up was called before the retro, or the retro was skipped.
