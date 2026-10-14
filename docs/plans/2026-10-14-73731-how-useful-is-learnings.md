# How useful is the learnings store?

Task #73731, run 2026-10-14. Research only: no code or spec change. Numbers come
from a read-only dump of the live store's `learnings` and `learning_retrievals`
tables. Related: `2026-08-27-4402-knowledge-base-prune.md` (a 300-entry delete).

## Short answer

The store is used a lot, but nothing shows it changes outcomes. We measure
*exposure* (16k retrievals) and a weak *usefulness* signal (agents upvote about
1 retrieval in 11). We do not measure *prevented mistakes*, which is the thing
the operator cares about. Value is concentrated in a few dozen entries, and most
of those are standing rules that belong in the prompt or skills, not in a
retrieval store.

## What the data says

| Measure | Value |
|---|---:|
| Learnings (527 approved, 2 archived) | 529 |
| Never upvoted (score 0) | 256 (48%) |
| Negative score | 6 |
| Created Aug / Sep / Oct 2026 | 112 / 309 / 83 |
| Retrieval rows | 16,066 over 1,217 tasks |
| …via `query_learnings` / prompt injection / procedural | 11,394 / 4,279 / 393 |
| Total upvotes vs retrieval rows | 1,455 (0.09 per retrieval) |
| Top 10 entries' share of upvotes | 44% (top 30: 63%) |
| Top 50 entries' share of retrievals | 48% |
| Retrieval rows pointing at deleted entries | 76 ids (incl. the two most retrieved, #4 and #5) |
| Unrated and older than 30 days | 120 |

- **The top-rated entries are standing rules**, not lessons: "never write
  `Implements #N` in a PR body" (three near-duplicates, 117 + 83 + 13 upvotes),
  "omit the Test plan section" (107), "research tasks often use
  `wrap_up_mode` done" (140). These fire on every PR. A skill or the dispatch
  prompt would deliver them reliably; a similarity search only delivers them
  when the query happens to match.
- **Duplicates exist.** The PR-number rule is stored at least twice. Nothing
  merges them at write time.
- **Heavy retrieval does not mean useful.** 37 entries were retrieved 20+ times
  and never upvoted (e.g. a 103-hit note on grepping env vars; Terraform
  naming rules that fire on unrelated tasks). Retrieval is cheap to fire and
  rating is rare, so a zero score is ambiguous: ignored, or helped but unrated.
- **Most retrieval is agent-initiated** (71%). Agents call `query_learnings`
  themselves, so recall depends on the agent remembering to ask, before the
  mistake rather than after.
- **Entries cover task-local detail.** Pitfalls are 263 of 529. Many are
  one-off tool quirks tied to a moment (wrap_up fetch flake, sandbox `.mcp.json`).
  These go stale and, after the 4402 prune, still accumulate (309 in Sep alone).
- **No outcome signal.** Nothing links a learning to "the agent did not repeat
  mistake X". The user's complaint, repeating the same mistakes, cannot be
  confirmed or refuted from this data.

## What the research says

Evidence is thin and mostly from SWE-bench-style benchmarks, not live agent
fleets.

- **SWE-ContextBench** ([arXiv 2602.08316](https://arxiv.org/abs/2602.08316)):
  precisely selected, summarised experience helps (accuracy, cost, mostly on
  hard tasks). Unfiltered or wrongly selected experience gives little or
  negative benefit. Retrieval precision matters as much as having the memory.
- **MemGovern** ([arXiv 2601.06789](https://arxiv.org/pdf/2601.06789)): curated
  "experience cards" gave about +4.7% on SWE-bench, with extra tokens spent on
  search. A modest gain from a governed, well-formed store.
- **Muscle Memory for Agents** ([arXiv 2608.08995](https://arxiv.org/pdf/2608.08995)):
  argues to *compile* repeated patterns into dedicated specialists or rules
  rather than only retrieve notes. Reported win in personalisation; small
  accuracy cost. Aimed at user-preference repeats, which matches our top entries.
- **Voyager-style skill libraries** (see the survey, [arXiv 2512.13564](https://arxiv.org/pdf/2512.13564)):
  entries are admitted only after verification, then retrieved by similarity.
  Our store admits without verification.
- **Claude Code's own design**: always-loaded index (CLAUDE.md, about 200
  lines) plus on-demand topic files and hooks. Third-party writeups note
  auto-captured notes vary in quality and can store wrong facts.
- **Attribution is an open problem** in the field too: a task can pass because
  the agent re-derived the answer, not because memory helped.

## Where our design diverges from what works

1. **Admission has no gate.** Recorded entries are approved at once, with no
   check for duplicates, staleness risk or verification.
2. **Standing rules ride on a similarity search.** The research favours
   compiling these into always-on instructions or checks.
3. **Retrieval is mostly pull.** The agent must know to ask, which fails
   exactly when it does not know it is about to err.
4. **No feedback on outcomes**, only optional upvotes.

## Recommendation

Do not replace the store with a different memory system. Nothing found beats
"curated, precise, small" and our problem is curation and routing, not the
storage engine. Change in this order, each step small and measurable:

1. **Promote standing rules out of the store.** Take entries that are upvoted
   10+ times and are rules about every task (PR-body rules, wrap_up_mode,
   fmt churn, foreground test run) and move them into the dispatch prompt,
   `plugin/skills/*/SKILL.md` or a mechanical check (`wrap_up` or the pre-push
   hook). Delete the store copies. This targets the "same mistakes" complaint
   directly, since these are the repeated ones.
2. **Dedupe on write.** `record_learning` should return the nearest existing
   entries and ask the agent to rate or edit one instead of adding a copy.
   Needs the embedding already stored.
3. **Add a cheap outcome signal.** Log whether an injected learning was cited
   in the final transcript or wrap-up, or whether a trajectory mistake matches
   a learning that was retrieved. Without this, every later change stays a
   guess. A sample of 30 trajectories (see #851 on measuring from trajectories)
   would answer the user's question in an afternoon.
4. **Prune by evidence.** Archive entries that were retrieved 20+ times with no
   upvote, and old unrated ones, after showing the inventory first (per the
   user's approval-before-removal rule).
5. **Tighten the recorder bar.** Drop the one-off tool quirk class unless it is
   likely to recur; the 4402 rule (no implementation names) already shows the
   direction.

Unanswered: whether agents actually act on injected entries. Step 3 is the
cheapest way to find out and should come before any larger rework.
