# Context-engineering audit (task 99245)

Source: "The new rules of context engineering for Claude 5 generation models". Six rules: judgement over rules; interfaces over examples; progressive disclosure; no repetition (simple tool descriptions); auto-memory over CLAUDE.md; rich references. Advice for CLAUDE.md: say what the repo is for, spend tokens on gotchas, skip what Claude can see in the repo, move procedures to skills.

## What already fits

- **Dispatch prompt core** (`src/dispatch/prompts.rs`). Trailing lines are short and goal-shaped. Comments record that tool names are not repeated in prose (rule 4). Already done; leave alone.
- **Review prompts** (`cve.md` 0.6 KB, `pr-review.md` 0.2 KB, `dependabot.md` 1.4 KB). Small and specific.
- **Validated knowledge** is injected only when relevant (rule 3).
- **Skills** are directories with `SKILL.md`, and `retro`/`wrap-up` were already rewritten as goal-plus-rules (learning #40961).

## Findings, by value

1. **CLAUDE.md is 17 KB and loaded into every agent.** It restates things Claude can find (build commands, toolchain pins, the hook step list, sccache setup, coverage flags) and has long bolded paragraphs of procedure. Rule: keep purpose plus gotchas, move procedures to docs/skills.
   - Keep: live data is in SpacetimeDB; verify command comes from `task_search`; `spacetime/module` is a separate crate; never `tmux kill-server`; worktree absolute paths; `main` moves; `cargo fmt` has no `--check`; conventional commits.
   - Move to `docs/testing.md` (already holds the details): first-time setup, the `verify.sh` level logic, sccache, run-suite command details.
   - Delete: the "Documentation" link list that duplicates `ls docs/`, and the per-subsystem entry-point list.
2. **Release-please prompt** (`release-please.md`, 2.4 KB) is a rigid numbered script with capitalised STOP AND REPORT. It guards a high-stakes action (auto-merge), so most of it should stay. The injection warning and gates are justified. Could shrink the repeated "do not merge" wording; low value.
3. **`preference-review` skill** (8 KB, 14 hard-rule words). Many "Never/Do not" lines, a long step-by-step with exact paths. Candidates to cut: the `languages/` extension table (the model can match extensions), the scratch-folder naming rule, the "announce at start" line. Keep: never post to GitHub, review PR code at `headRefOid`.
4. **`allium-loop` skill** (6.8 KB) is a long numbered kickoff with fallbacks for every input. Rule 1 says give a goal and the few rules that matter (state file, fresh agent per iteration, never fork). Candidate to cut to about half.
5. **`decompose-review`** (6 KB) and **`wrap-up`** (6.3 KB): check for examples that bound behaviour. `wrap-up` is the user-chosen path flow, high-stakes, so mostly keep.
6. **MCP tool descriptions** (`src/mcp/handlers/`, about 232 KB source). Not read in full. Rule 2 says encode behaviour in schema (enums, required params) not examples. Learning #713 already warns that example `tag_filter` values bound retrieval. Follow-up: grep descriptions for "Example", "e.g." and long when-to-use lists.
7. **Spec-first instruction** is repeated in three near-identical forms (`spec_first_instruction`, `brainstorm_instruction`, quick dispatch), each ending with the same "not the end of the task" paragraph plus a separate `plan_not_a_stopping_point_instruction`. The comments say this is deliberate after regressions (#4188, #4366), so do not cut without an eval.
8. **Auto-memory** (rule 5): dispatch uses its own knowledge base with rating. That is the repo's equivalent; no change.

## Risks

- Prompt wording is gated by snapshot tests, `dispatch-prompt.allium` and `prompt_trailing_lines_name_no_mcp_tool`. Any text change in `prompts.rs` needs spec, then snapshot updates.
- Past regressions justify some strong rules. Cut only with an eval: `claude plugin eval` with `focus: trace` (learning #65538).
- `claude doctor` (per the post) can flag conflicts in skills and CLAUDE.md. Run it as a cheap first pass.

## Proposed follow-up tasks

1. Slim CLAUDE.md to gotchas (docs-only, low risk). Do first.
2. Trim `preference-review` and `allium-loop` to goal plus key rules; run plugin evals before and after.
3. Audit MCP tool descriptions for examples and repeated guidance.
4. Run `claude doctor` over skills and CLAUDE.md and apply its findings.
