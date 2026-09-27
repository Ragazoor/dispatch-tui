# Allium-Weed-Loop: Fresh-Agent-Per-Run Design

**Date:** 2026-09-27
**Task:** #8197 — allium-weed loop

## Background

`.claude/skills/allium-weed-loop` drives the reconciliation loop (spec drift → spec fix) the same
way `plugin/skills/allium-loop` used to: it writes a ralph-loop state file
(`.claude/ralph-loop.local.md`) that an external, unmodifiable Stop hook feeds the same prompt
back into on every raw turn-end, incrementing an `iteration` counter each time.

Task #3715 diagnosed and fixed this exact architecture in `allium-loop`
(`docs/superpowers/specs/2026-07-26-allium-loop-fresh-agent-design.md`) and explicitly flagged
this skill as out of scope for that fix, deferring it as a follow-up task. This task is that
follow-up, triggered by the user asking why the loop is slow and token-expensive.

### Why it was slow and expensive

1. **One continuous, ever-growing session.** Every iteration's output (rebase, weed report, tend
   agent output) stays in context and is resent as input tokens on every subsequent turn. Cost
   grows with iteration count instead of staying flat.
2. **The iteration counter counts raw Stop events, not completed passes** — the same bug
   documented for `allium-loop`. A full pass (rebase → weed → process findings → commit) can span
   more than one raw turn, so the loop can exhaust `max_iterations: 10` before even one logical
   pass finishes.
3. **No incremental narrowing.** Every iteration re-weeds the entire `docs/specs/` tree against
   the entire `src/` tree, and per learning #750 this must happen at least twice in a row cleanly
   before the loop may stop — so the full-tree comparison is the cheapest part done the most
   times, in the most expensive way (accumulated context).
4. **One extra nested agent per finding.** Every undocumented-behavior or spec-bug finding spawns
   its own `allium:tend` agent, all nested inside the one growing session.

## Scope

Redesign `.claude/skills/allium-weed-loop/{SKILL.md,prompt.md}` only, following the same
fresh-agent-per-run pattern as `allium-loop`. No changes to `plugin/skills/allium-loop`, the
external `ralph-loop` plugin, or `docs/specs/*.allium` (this skill's own workflow is not modeled
by a domain spec, same as its sibling).

## Core Change: Drop the Stop-Hook Loop Entirely

Same mechanism as `allium-loop`: the invoking session becomes the loop **driver** and never does
rebase/weed/tend work itself. Each iteration is a fresh `Agent` tool dispatch (never `fork`, so the
`model` override takes effect), with no shared conversation memory — only the repo's committed
state carries over. The driver reads each iteration's two-line report and a durable state file
(`.claude/allium-weed-loop-state.local.md`, gitignored, never staged) to decide whether to
continue.

### What's different from `allium-loop`, and why

`allium-loop` drives forward from a design document (spec-first: tend → propagate → implement →
weed). `allium-weed-loop` drives in the opposite direction — there is no input document, only the
existing spec/code drift to reconcile. Three concrete differences follow from that:

- **No `design_doc` field.** Kickoff resolves `base_branch`, `verify_command`, `max_iterations`,
  and `model` the same way `allium-loop` does, but has nothing analogous to resolve for a design
  document — there isn't one.
- **Convergence is "weed clean twice in a row," not a single run's self-assessed `CONVERGED`.**
  Learning #750: a single clean `allium:weed` pass can still miss a real divergence, so the state
  file tracks `consecutive_clean_runs` and the driver only declares success once that reaches `2`.
  A stateless fresh agent cannot count across its own boundary, so each iteration reports
  `WEED_CLEAN: yes|no` (this run's own weed pass, before any fixes) instead of deciding
  `CONVERGED` itself — the driver derives convergence from the state file.
- **No red-check/propagate/implement steps.** This loop reconciles existing drift; it does not
  build new behavior from a spec. Its per-iteration shape is rebase → weed → (tend | ask-then-fix)
  → commit → report, not the seven-step spec-first sequence.

### State file

```markdown
---
active: true
runs_completed: 0
max_iterations: 10
retry_count: 0
consecutive_clean_runs: 0
base_branch: "main"
verify_command: "cargo test"
model: "sonnet"
started_at: "2026-09-27T12:00:00Z"
---
```

Gitignored (added to `.gitignore` alongside the other loop state files) and never staged by an
iteration's own commit step, for the same reason as `allium-loop`'s state file: it's mutated after
every iteration, and a committed copy would leave the tree dirty for the next iteration's rebase.

- `retry_count`: bounds retries of an errored/unparseable iteration (0 or 1), same semantics as
  `allium-loop`.
- `consecutive_clean_runs`: incremented when a run reports `WEED_CLEAN: yes`, reset to `0`
  otherwise. Convergence fires at `>= 2`.
- `model`: default `sonnet` for the iteration driver's own mechanical work (rebase, commit,
  processing findings). Nested `allium:weed`/`allium:tend` dispatches pin their own model in their
  own definitions and must never receive a `model` override from the iteration agent — same
  reasoning as `allium-loop`: an override "for consistency" would downgrade exactly the two
  judgement-heavy steps the cheaper iteration model was chosen to leave alone.
- Iteration number is derived as `runs_completed + 1`, never stored, so it survives a resume or a
  context compaction.

### Kickoff (SKILL.md)

1. Check for an existing active state file first — same resume/abandon/cancel flow as
   `allium-loop` (never silently overwrite an active loop).
2. Resolve `base_branch` (task context, else ask), `verify_command` (task/session context, then
   project docs, else ask), `max_iterations` (explicit override, else default `10`), `model`
   (explicit override, else default `sonnet`).
3. Read `prompt.md`, write the state file, tell the user the loop is active, dispatch iteration 1.

### Per-iteration flow (prompt.md)

1. **Rebase**: `git fetch origin {{BASE_BRANCH}} && git rebase origin/{{BASE_BRANCH}}`.
2. **Weed**: `Agent` tool, `subagent_type: "allium:weed"`, check mode, comparing all of
   `docs/specs/` against `src/`. Classify each finding: spec bug, code bug, or undocumented
   behavior. No findings at all → this run's `WEED_CLEAN` is `yes` and steps 3 is skipped.
3. **Process findings** (unchanged from the current skill):
   - Undocumented behavior or spec bug → `Agent` tool, `subagent_type: "allium:tend"`, told the
     specific behavior/correction and where it was found.
   - Code bug → `AskUserQuestion` in-run (no relay through the driver, same as `allium-loop`'s
     open-question handling) — describe the bug, ask whether to fix. Fix only if confirmed.
4. **Commit**: stage and commit changed spec files and any user-approved code fixes (never
   `docs/plans/`, never the state file). Required even when nothing changed only in the trivial
   sense that a clean tree needs no commit — never leave the tree dirty otherwise.
5. **Report**: exactly two labelled lines:
   ```
   WEED_CLEAN: yes|no
   SUMMARY: <what changed this run, or "no changes" if the tree was already clean>
   ```
   `WEED_CLEAN` reflects step 2's finding count before any fixes, not whether this run's fixes
   were applied — a run that found and fixed three undocumented behaviors is still `no`, because
   the point is confirming a *subsequent* pass finds nothing, not that this pass tidied up after
   itself.

### Driver logic on receiving a report

Same shape as `allium-loop`:

- Errored/unparseable (missing either label) → `retry_count` 0→1 retry same iteration, else give
  up and report the failure.
- Real report → `runs_completed += 1`, `retry_count = 0`, then:
  - `WEED_CLEAN: yes` → `consecutive_clean_runs += 1`; else reset to `0`.
  - `consecutive_clean_runs >= 2` → converged: delete state file, report success.
  - Else if `runs_completed >= max_iterations` → give up: delete state file, summarize what's
    unresolved.
  - Else → dispatch next iteration.

## Accepted residual risk: a declined code-bug fix repeats

If the user declines to fix a real code bug, nothing is written down to say so. The next
iteration's fresh `allium:weed` pass has no memory of the decline and will surface the same finding
again, asking again. This is the current skill's existing behavior carried forward unchanged, not
a new gap introduced here — and, same as `allium-loop`'s rejected `BLOCKED` fallback, adding
machinery to remember a decline is speculative complexity for a case with no observed instance yet.
If it turns out to be a real annoyance in practice, the fix is for the declined agent turn to ask
`allium:tend` to record the deviation explicitly (as a documented known-divergence, not silence),
which surfaces as a natural follow-up if and when it's actually hit.

## Out of Scope

- `plugin/skills/allium-loop` and the external `ralph-loop` plugin.
- `docs/specs/*.allium` — no domain spec models this skill's own workflow, same as its sibling.
