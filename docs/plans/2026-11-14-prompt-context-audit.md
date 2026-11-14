# Prompt and context audit against the Claude 5 context-engineering rules

Task #98534. Source: "The new rules of context engineering for Claude 5-generation
models" (Thariq Shihipar). Read through a summarising fetch, so the rule wording
below is paraphrased, not quoted.

## Rules used

1. Give judgment, not rules.
2. Design interfaces, not examples.
3. Progressive disclosure: load context when needed.
4. Do not repeat yourself; tool guidance lives in the tool description.
5. CLAUDE.md: what the repo is for, then gotchas; omit what Claude can discover.
6. Skills: light guides; constrain only high-stakes areas.
7. Audit and simplify regularly.

## What is already in good shape

The injected prompt (`src/dispatch/prompts.rs`) is already lean and has been
through this exercise. Trailing block is five short lines, three conditional.
It names skills and stops (`brainstorm_instruction`, `spec_first_instruction`),
names no MCP tool (`learning_tools_instruction`), and the spec
`docs/specs/dispatch-prompt.allium` forbids restating. Learning #713 (no example
`tag_filter` in agent copy) is already applied in `plugin/skills/learnings/SKILL.md`.
The review addenda (dependabot, release-please, CVE) hold merge and
untrusted-data gates. The article exempts high-stakes areas, so leave them.

## Ranked proposals

Always-loaded text first. "Guard" is the test or eval that would catch a bad cut.

| # | Surface | Size, loads | Rule | Proposal | Guard |
|---|---|---|---|---|---|
| 1 | `CLAUDE.md` "Build & Test" and "First-time setup" | ~7 KB of 17 KB, every session | 5, 3 | Move test-running mechanics (foreground timing, `--no-fail-fast`, no-pipe, tmux/spacetime skips, sccache, pre-push internals) into `docs/testing.md` (most is already there) and keep one-line gotchas with links. Keep `main` moves, verify command, worktree path, tmux-socket rules in full. | docs checkers (phantom symbol/heading), `scripts/gates.sh`. No behaviour eval: **unguarded for "does the agent still run `--no-fail-fast`"**. |
| 2 | `wrap-up` Step 3 | two 400-char `git diff --numstat` globs | 1 | Replace the language glob lists with one line: if the diff changed about 100+ lines of source, run `simplify`. | `wrap-up-*` evals do not run Bash, so **unguarded**. |
| 3 | `rate_learning` instructions | stated in the injected knowledge block, `wrap-up` Step 7.1, `learnings` skill and the tool schema | 4 | Keep the injected block (it renders only when there is something to rate) and the tool schema; cut `wrap-up` 7.1 to one sentence pointing at the `learnings` skill. | `learnings-rate-on-use`, `wrap-up-done`, `wrap-up-rebase` evals. |
| 4 | `allium-loop` (`SKILL.md` 7.7 KB + `prompt.md` 8.5 KB), `preference-review` 8 KB, `decompose-review` 6 KB | on demand | 1, 6 | Read each for rules that restate judgment the model has. Not yet read line by line. | Only `preference-review` has tests. Others unguarded. |
| 5 | Skill descriptions (`wrap-up`, `retro`) | every session (descriptions always load) | 3 | Both descriptions run to several sentences. Shorten to trigger plus the one sharp rule ("use this, never `retro`, to finish a task"). | `wrap-up-*` evals. Triggering is only checked by eval; description changes risk a missed trigger. |
| 6 | `trailing_block` TDD line | one line per dispatch | 1 | Leave. It is conditional and guarded by the spec; the user chose spec-first and TDD. | n/a |

## Not covered (needs a decision on scope)

Not yet read: `plugin/hooks` scripts, `get_task`/`wrap_up`/`exit_session` response
prose, the quick-dispatch and research builders (`prompts.rs` 760 onward), and the
real MCP tool schemas an agent receives (my size count of them was wrong, so I make
no claim). `docs/mcp-tool-design.md` already audits the tools.

## Recommendation

Do 1, 2 and 3 now: small, mostly deletion, and 3 is eval-guarded. Treat 4 and 5
as follow-up tasks, one per skill, since each needs its own read and eval run.
Per the approval-before-removal rule, nothing is removed until this list is agreed.
