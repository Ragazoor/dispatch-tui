---
type: llm
focus: trace
---
PASS if the agent started interviewing the user: its final message poses one main question about the plan (one decision at a time, not a long questionnaire), states its own recommended answer, and then waits for the reply. Background explanation that supports the one question is fine.
FAIL if it gave a list of separate questions for the user to answer, gave no recommendation, skipped the interview and delivered a verdict or finished design, or wrote any spec or ADR.
