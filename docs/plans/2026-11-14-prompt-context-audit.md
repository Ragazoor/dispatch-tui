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

## Proposal 4 follow-up: skill line-by-line read (task #99188)

Read `allium-loop` (`SKILL.md`, `prompt.md`, `references/handling-results.md`),
`preference-review` and `decompose-review` (`SKILL.md`, plan template). All three
have an Allium spec and a test that pins each guarantee by phrase, so every cut
below avoids a pinned phrase. A phrase-pin test fails if the words go, but no test
checks that an agent still behaves the same: the guard column says which cuts have
more than that. No eval runs these skills, so **every cut is behaviour-unguarded**.

Left alone on purpose: the convergence gate, the red check, never weaken tests,
never commit the state file, the dirty-tree rule, the retry and stall limits, the
`fork` ban, "never post to GitHub", "never edit the PR's code", the `headRefOid` and
merge-base reading rules, the plan-directory reuse check, `auto_run_plan` and
decision items.

| Skill | Cut | Why it restated | Guard |
|---|---|---|---|
| allium-loop `SKILL.md` | "Unlike a ralph-loop-style skill…" contrast | Describes a skill the agent is not running | none (unguarded) |
| allium-loop `SKILL.md` | design-doc step: how a dispatched agent reaches a spec, brainstorm vs spec | Model picks the candidate from context | `allium_loop_text_holds_each_session_guarantee` pins "first-class input" and "tell the user which document was picked", kept |
| allium-loop `SKILL.md` | verify-command examples (`cargo test`, `npm test`…) | Examples of what the model knows | pins `**Project docs**`, kept; behaviour unguarded |
| allium-loop `SKILL.md` | `model` cost rationale and the "two rules reinforce" paragraph | Cost reasoning and a fact the Agent tool description states (a fork ignores `model`) | `allium_loop_dispatches_iteration_agents_on_sonnet`, `allium_loop_dispatch_still_forbids_fork`, and the pinned "passing `model` set to the state file's `model` value"; all kept |
| allium-loop `prompt.md` | "language- and stack-agnostic" sentence | Repeats `SKILL.md`, which the test pins | none |
| allium-loop `prompt.md` | model-override rationale; the step 7 repeat of it | Same rule stated three times | pinned "Do NOT pass a `model` override to `allium:tend` (or to `allium:weed`", kept once |
| allium-loop `prompt.md` | `git status` vs `git diff` explanation | Model knows the difference | pins the `git status --porcelain -- docs/specs/` command, kept |
| allium-loop `prompt.md` | three guardrail lines (escalate, honour parameters, fix code) | Each repeats a step: 2, 5 and 6 | pins "no magic numbers" (step 5), "fix the CODE, not the tests" and the AskUserQuestion rule (step 6), kept in place |
| preference-review | "`owner` and `repo` come from the URL…" | Trivial | none |
| preference-review | step 4 "groups of about eight" | Numeric rule where "split many" is enough | none |
| preference-review | comment voice adjectives (short, direct, no hedging) | The model has the operator's voice notes in `general.md` | none; the voice notes remain |
| decompose-review | "Low findings are small issues to fix in passing…" | Reason; "Low ones included" stays | `decompose_review_skill_text_holds_each_session_guarantee` pins "Take **every** finding, Low ones included" |
| decompose-review | grouping step 1 (extract paths), naming examples | Trivial | none |
| decompose-review | "This confirmation is also why Step 6 sets `auto_run_plan`" | Repeated in 6b | pins "`auto_run_plan`: `true`" and the Decisions exception in 6b, kept |
| decompose-review | `wrap_up_mode` reason and the four-line report template | Model writes the list; "Dispatch the epic from the TUI" stays | pinned "Display the created epic and all task IDs", kept |

Not cut, though it reads like judgment: `preference-review` step 3 "never review the
diff inline yourself" (spec `ReviewerCountFollowsDiffSize`, a user-chosen rule) and
the `plan-template.md` decision text (pinned).
