# Spike: `claude plugin eval` on the wrap-up skill (task #69635)

Epic #45057 (agent-behaviour evals, local only). Claude Code 2.1.300.

## Verdict

**Go.** `claude plugin eval` runs against `plugin/`, loads the skills, and scores the wrap-up skill. Both cases passed 6 of 6 runs with the final graders. With the `exit_session` step removed from the skill, all 4 runs failed, so the cases catch a real regression.

## What was built

`plugin/evals/` holds two cases, `wrap-up-done` and `wrap-up-rebase`, and mocks for the dispatch MCP server (`get_task`, `wrap_up`, `exit_session`, `rate_learning`, `query_learnings`, `create_task`, `record_learning`). Per-case `mocks/` override the shared ones for `get_task` and `wrap_up`.

## Findings

- **Skills load.** The trace shows `dispatch:wrap-up` firing with the plugin's own text.
- **The live store is not touched.** `plugin/` has no `.mcp.json`, and the eval does not load the user-level `dispatch` server. The mock files create a fake `dispatch` server. Each run has an empty temp workspace and a temp HOME. The plugin hooks load but exit 0, because the temp path has no task id.
- **`--mocks record` is the default and works.** One file per tool; the body is the reply.
- **Bash and git do not work in a run.** The sandbox blocks `/usr/bin/git`. The cases drop Bash and state the branch in the prompt.
- **`GRADLE_USER_HOME=./.gradle` blocks any Bash-granting run.** Use `env -u GRADLE_USER_HOME`.
- **LLM graders need `focus: trace`.** The default reads only the last message and gave 3 false failures in 6 runs.
- **The with-without baseline shows no plugin effect here.** The no-plugin arm scored 1.0, because the mocked tools are visible to it. Use `--ablation none`.
- **A hook blocks the literal PR-create command text** in any Bash command, even inside a heredoc.

## Measurements (3 runs per case, 2 cases)

| | Haiku judge (default) | `--judge-model sonnet` |
|---|---|---|
| Cost, 6 runs | $0.85 | $1.41 |
| Wall time | about 160 s | about 157 s |
| Per run | about 9 turns, 25–30 s, about $0.14 | about $0.23 |
| Judge cost per run | about $0.005 | about $0.10 |

Flakiness: 12 of 12 runs passed with the final setup. Three runs per case is too few to call it stable. The two judges agreed on every run. Haiku is enough.

## What later tasks need to change

1. Wrap the run in a helper: unset `GRADLE_USER_HOME`, set `--max-cost-usd`, use `--ablation none`, run in the background. A full suite takes about 3 minutes.
2. Share the allowed-tools list, the mocks and the grader template (`focus: trace`) across cases.
3. Add cases for `retro`, `learnings`, `grill`, `decompose-review` and `allium-loop`, with mocks for the tools each one calls.
4. Decide how to test git-heavy steps (commit, PR path), since the sandbox blocks git.
5. Decide whether the with-without comparison is worth keeping for tool-driven skills.
6. `plugin/evals/results/` is gitignored.
