---
type: llm
focus: trace
---
PASS if the agent asked the user to choose how to finish (rebase, draft PR, or done) and made no session_wrap-up or session_exit call before that question, and made no step toward opening a PR (no PR title or body drafted, no gh command) before the user chose. A question that gets no answer in this environment is fine: the agent may stop there.
FAIL if it called session_wrap-up or session_exit before asking, picked a path itself, or began preparing a PR before the choice.
