# Prompt audit against the Claude 5 context-engineering rules (task #99190)

Source: "The new rules of context engineering for Claude 5-generation models"
(claude.dev). Six shifts: give judgement not rules; design interfaces not
examples; progressive disclosure; do not repeat yourself; automatic memory;
rich references. Caveat from the article: keep hard guardrails for
destructive or high-stakes actions.

This is a dated snapshot. No prompt was changed. Each change needs a spec
edit, snapshot updates and the pre-push gates, so the operator should pick.

## What is already good

- **Do not repeat yourself.** `src/dispatch/prompts.rs` already removed tool
  names from the trailing block and gates lines the addendum already said
  (`Preceding`, `trailing_block`). This is rule 4, done.
- **Judgement, not rules.** `spec_first_instruction`, `brainstorm_instruction`
  and `learning_tools_instruction` name a skill and stop. Plan-writing is
  "your judgement call".
- **Progressive disclosure.** `CLAUDE.md` links to `docs/*` on demand; the
  wrap-up skill moves PR and error detail into `references/`.
- **Validated knowledge block** is data the agent applies "where relevant".

## Findings, most valuable first

1. **`CLAUDE.md` is a catalogue (16.9 KB, 21 never/always/must lines).**
   The article says spend tokens on gotchas, not on every practice. Candidates
   to move out of the always-loaded file into `docs/` (leave a one-line link):
   First-time setup, the full test-run recipe, sccache, "Running & Debugging
   Locally". Keep: stack, the live-data pointer, the verify-command rule,
   write-scope warning (worktree absolute paths), tmux socket rule, spec-first
   rule. Several paragraphs state their own history ("A three-variant form
   collapsed…") which belongs in docs/invariants.md, not in every agent's
   context.
2. **Task-agent prompt: wrap-up wording appears in three places.**
   `wrap_up_instruction`, `plan_not_a_stopping_point_instruction` and the
   closing clause of both design instructions all say "implement before you
   stop". The spec (`NoLineRestatesTheDesignStep`) already limits the overlap;
   the design instructions could drop their last paragraph and let the
   trailing line carry it. Low risk, small win.
3. **Wrap-up skill is a numbered script with repeated warnings.** Seven steps,
   and three separate "do not stop / run back to back" warnings (Steps 2, 5,
   7). This matches learning #40961: give a goal plus the few rules that
   matter. Suggest: one paragraph stating the one real hazard (an idle agent
   with the base branch already fast-forwarded), and keep the ordered
   `wrap_up` → `exit_session` contract. Keep the user-chooses-path gate — it is
   a high-stakes guardrail the article exempts.
4. **`allium-loop/prompt.md` (7.4 KB) and `preference-review` (8 KB) are the
   most rule-dense skills** (15 and 20 negations). Each step spells out
   conservative-merge and STOP rules. Rewrite as outcome plus the genuine
   stop conditions (open questions, red check passing, test contradicts spec).
5. **MCP tool descriptions carry behaviour prose and examples.**
   `src/mcp/handlers/dispatch.rs` (60 KB, ~10 `e.g.`/Example uses).
   `update_tasks` enumerates every field in prose that the schema already
   lists. Rule 2: let the schema (enums, required) express it and keep one
   sentence for non-obvious behaviour (non-atomic multi-id). Learning #713
   already warns that example `tag_filter` values bound retrieval — check the
   other example values the same way. `docs/mcp-tool-design.md` is the place
   to add the rule.
6. **Conflicting-instruction check (rule 1).** One true conflict candidate:
   the dispatch prompt says plans are a judgement call, while `CLAUDE.md`
   "Working With the User" says "Ambiguity is a stop condition, not a
   judgement call" and "Behaviour changes start in the spec". They apply to
   different things, but an agent reading both can stall. Add one clause
   saying which wins when a task is a pure research or prompt-copy change.
7. **Runbook templates (`prompts/dependabot/*`, `cve/*`, `release-please/*`,
   10 KB total).** Already split by branch so only the relevant fragment
   renders — good progressive disclosure. These are high-stakes (merging
   dependency PRs), so keep their guardrails. Only trim the commentary-style
   sentences.
8. **Rich references (rule 6).** Allium specs and snapshot tests are already
   code-as-spec here. Nothing to add. Optional: eval suites in
   `plugin/evals` can act as the rubric for the rewrites above; run them
   before and after (set `focus: trace` per learning #65538).

## Suggested order

1. Trim `CLAUDE.md` (1). No code change, no snapshot impact.
2. Rewrite wrap-up skill (3) and run its evals.
3. Slim tool descriptions (5) and design-instruction tails (2); both need spec
   and snapshot updates.
4. Allium-loop and preference-review rewrites (4) last; they are least used.

Expect the same safety rule throughout: keep every guardrail around merging,
pushing, deleting and `exit_session`.
