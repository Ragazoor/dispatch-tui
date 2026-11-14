# Learnings iteration 10: delete copies of rules agents already see

Task #99042, run 10, 2026-11-14. Follows `2026-11-08-98686-learnings-iteration-9.md`.

Two changes, both approved by the operator from the inventory below:

- **Deleted ten entries that restate always-loaded context**: #495 (the dispatch prompt's
  design step, word for word), #703 (the operator's memory line "fix pre-existing
  issues"), #12300 (CLAUDE.md "live data is in SpacetimeDB"; its SQLite part is also out
  of date), #442 (CLAUDE.md "skim new commits' content"), #736 and #735 (CLAUDE.md tmux
  rules), #81925 and #90382 (CLAUDE.md "cargo fmt inside the module crate"), #817 and
  #90333 (the operator's memory index lines).
- **Recorder bar** (`plugin/skills/learnings/references/recording.md`): do not record what
  an agent already sees every session (the dispatch prompt, a loaded skill, the memory
  index). A copy takes an injection slot, and its `helped` ratings prove nothing.
- `scripts/learnings-metrics.py` gains `repo_helped_metrics`: the rated-only verdict share
  per repo, before the mute, after the mute and after this run's deletes.

No spec or code change: the store's behaviour is the same, only its contents and the
recording guidance changed.

## Why this, why now

**The mute is live but did not move the whole-store share.** Replaying every injection
against timestamped `rate_learning` events: until 2026-11-08 the board injected 2 to 13
pairs a day that the mute would drop; from 2026-11-09 it injected none. The whole-store
`not_applicable` share stayed at 51% (W45 to W46), because new Scala repos joined after
the mute and the repo mix shifted. Per repo, dispatch moved the predicted way: rated-only
`helped` 30% to 34% (with the copies counted; 27% to 33% without them, below). Small, a
few points, as run 9 expected.

**Changing the query text does not help (run 9's item 2, now ruled out).** Within-task
AUC of cosine for helped over `not_applicable`, 597 pairs over 227 tasks: full text 0.563,
title only 0.566, title plus first paragraph 0.533, title plus 400 characters 0.570, the
BGE query prefix 0.576. Cosine barely separates verdicts whatever the query.

**The freed slots went to copies of standing rules.** After the mute, five entries that
restate always-loaded context (#495, #703, #12300, #442, #653) took 66 of 280 dispatch
injection slots (24%, up from 15% before the mute). Four of them are pure copies; #653
adds a reason the prompt lacks, so it stays. Their `helped` ratings (16 on #495, 13 on
#703) are a false signal: the agent had the rule from the prompt or memory anyway. An
embedding scan of every learning against CLAUDE.md, the memory index, every skill and the
prompt strings found six more low-traffic copies.

Considered and not built:

- **A larger upvote weight in ranking.** Inside the injected set, cosine plus a capped
  upvote boost looked better (AUC 0.633 at today's cap of 10, 0.663 at 30). But raw upvotes
  leak the task's own `helped` (0.687 raw, 0.57 with later verdicts removed), the gap is
  within noise for 227 tasks, it scores only inside the five already chosen, and a cap
  above 10 moves only the 11+ upvote entries, which are the standing rules. Not built.
- **A per-entry `helped`-rate prior** (time-ordered, AUC 0.636). It would change ranking,
  which run 8 left to the operator. Proposed below, not built.

## Metrics table (run-to-run)

Deleting an entry deletes its retrieval and verdict rows, so the after column and the
per-repo rows below are read without the ten copies.

| Measure | Run 9 (after) | Run 10 (before) | Run 10 (after) |
|---|---:|---:|---:|
| Learnings | 795 | 844 | 834 |
| Never upvoted | 390 | 415 | 413 |
| Negative score | 5 | 5 | 5 |
| Retrieval rows (tasks deleted) | 10,555 | 11,294 | 11,088 |
| Total upvotes | 1,459 | 1,579 | 1,470 |
| Retrieved 20+, never upvoted | 12 | 12 | 12 |
| Unrated and older than 30 days | 184 | 199 | 198 |
| `record_learning` refused, latest week | 21 / 155 (W45) | 8 / 82 (W46) | 8 / 82 |
| **Injected pairs not_applicable** (all repos) | 274 / 532 (51%, W45) | 226 / 438 (51%, W46) | 205 / 384 (53%, W46) |
| Injected pairs helped (all repos) | 107 / 532 (20%, W45) | 98 / 438 (22%, W46) | 84 / 384 (21%, W46) |
| Injected pairs unrated | 151 / 532 (28%) | 113 / 438 (25%) | 94 / 384 (24%) |
| **Dispatch, rated-only helped**, before mute | n/a | 98 / 331 (30%) | 80 / 290 (27%) |
| **Dispatch, rated-only helped**, after mute | n/a | 77 / 225 (34%) | 63 / 190 (33%) |
| Dispatch slots taken by copies, after mute | n/a | 66 / 280 (24%) | 0 |
| Muted (learning, repo) pairs | 15 | 34 | 34 |

## Next run should try

1. Read `repo_helped_metrics`' "dups deleted" row for W47 and W48. Success: dispatch's
   rated-only `helped` stays at or above 33% now that the false `helped` from copies is
   gone, and `not_applicable` falls below 66%. A flat row means the freed slots refill
   with off-topic entries at the base rate.
2. Ask the operator about a per-entry `helped`-rate prior in ranking (time-ordered
   within-task AUC 0.636 against cosine's 0.563). Run 8 kept ranking as the operator's
   call. Bootstrap by task before claiming a gain, and re-rank all candidates (scope
   filter, multiplier, mute), not just the five injected.
3. Re-run the copy scan when CLAUDE.md, a skill or the prompt gains a rule: an embedding
   match above 0.85 against those paragraphs is a good first filter, then read both.
   The scan code is easy to rebuild: decode the stored embeddings, embed each paragraph
   with the ONNX model in `.fastembed_cache` (CLS pooling), take the best match.
4. The top `not_applicable` entries now are #8196, #90216, #596, #552, #734 and #653.
   Most are muted in dispatch already. Check where #653 still lands.
