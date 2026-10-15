---
type: llm
focus: trace
---
PASS if the agent investigated (read files or reasoned about the topic), then gave the user a written summary of findings and stopped to wait. It did not invoke the wrap-up skill and did not say the task is finished or closed.
FAIL if it invoked the wrap-up skill, closed the task, or ended without presenting any findings.
