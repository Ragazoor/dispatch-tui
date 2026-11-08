# Learnings iteration 9: mute entries that keep missing in one repo

Task #98686, run 9, 2026-11-08. Follows `2026-10-29-98645-learnings-iteration-8.md`.

Two changes, both approved by the operator in this session:

- **Prune by evidence.** Deleted #747, #90215, #90240, #768 and #753. Each had 4 to 10
  `not_applicable` verdicts, no `helped`, 0 or 1 upvotes and almost no `query_learnings`
  hits. #753 was also a near-duplicate of #81922.
- **Per-repo dispatch mute.** Dispatch injection now leaves out a learning that has 3 or
  more `not_applicable` verdicts from tasks in the task's repo and no `helped` verdict
  there. Spec: `core/Learning.off_topic_in_repo` and `core/config.off_topic_verdicts_to_mute`
  (core.allium), used in `AugmentDispatchPromptWithLearnings` (learnings.allium). Code:
  the board now subscribes to `visible_learning_verdicts` (the view was already in the
  module), and `list_learnings_for_dispatch_rag` filters before ranking, so the slot goes to
  the next candidate. `query_learnings` is unchanged. The mute changes no score, status or
  `updated_at`, and it is derived, not stored. One later `helped` from that repo lifts it.
  No module, bindings or rules-version change.
- `scripts/learnings-metrics.py` prints the mute's current reach.

## Why this, why now

Run 8's verdict now flows. Unrated injected pairs fell from 83% (W44, before) to 28% (W45).
That shows a real problem: half of all injected pairs are `not_applicable` (274/532 in W45).
Only 20% are `helped`.

Three things the data ruled out:

- **The similarity floor.** `RAG_SIMILARITY_THRESHOLD` (0.25) never filters on dispatch.
  Every injected pair scored above 0.52 with BGE-small, so every task gets the full 5.
  Cosine barely separates the verdicts: median 0.740 for helped and 0.714 for
  not_applicable. A floor of 0.62 drops 12% of not_applicable and 4% of helped. Measured
  by embedding task text locally with the store's ONNX model (0.97 agreement with the
  stored vectors), over 846 pairs and 168 tasks.
- **Pruning alone.** `not_applicable` spreads over 207 entries, and 127 of them have just
  one. Most heavy entries are worth keeping: they are user preferences (off-topic is not
  wrong), or AnnotellDB/terraform entries that agents query often and that miss only in
  sibling migration epics.
- **Cutting the cap.** Rank 1 is helped 33% of the time and ranks 2 to 5 only 12 to 19%.
  But dropping ranks 4 and 5 would lose 27% of helped pairs.

A chronological replay of 10 days of verdicts, mute at threshold 3 counted per repo:
51 off-topic injections avoided and 3 helped lost (threshold 4: 34 avoided, 0 lost).
Counting per repo answers run 8's objection that an entry off-topic in one repo can fit
another. The replay does not refill the slot. In reality the next candidate takes it, at
roughly the base rate, so expect about half that gain.

Web check: recent work also feeds usage back into retrieval, for example a per-memory
utility score updated after use (MemArchitect,
[emergentmind 2603.18330](https://www.emergentmind.com/papers/2603.18330)) and retrievers
that keep their past relevance judgments
([arXiv 2608.22767](https://arxiv.org/pdf/2608.22767), +2 to 7%). Only abstracts read.

## Other findings

- The top repeated tool error is `record_learning` without `task_id` (14 tasks in 14
  days; 27 calls overall). Each one recovers on the next call. It is the same class as the
  `content` alias that runs 3, 5 and 6 declined: one wasted call, no harm. Left alone.
- Retrieval rows fell from 15,340 to 10,597 because tasks were deleted (#81922). Do not
  read that as less use.
- PR bodies with a bullet list after the prose rule: 13 of 48 (before: 151 of 180).

## Metrics table (run-to-run)

The after column was read later the same day. Other agents recorded and deleted entries
meanwhile, so it is not this run's 5 deletes alone.

| Measure | Run 8 (after) | Run 9 (before) | Run 9 (after) |
|---|---:|---:|---:|
| Learnings | 677 | 798 | 795 |
| Never upvoted | 329 | 391 | 390 |
| Negative score | 5 | 5 | 5 |
| Retrieval rows (tasks deleted) | 15,340 | 10,597 | 10,555 |
| Total upvotes | 1,211 | 1,456 | 1,459 |
| Retrieved 20+, never upvoted | 35 | 12 | 12 |
| Unrated and older than 30 days | 171 | 184 | 184 |
| `record_learning` refused, latest week | 14 / 56 (W44) | 21 / 155 (W45) | 21 / 155 |
| **Injected pairs unrated** | 311 / 373 (83%, W44) | 151 / 532 (28%, W45) | read W46 |
| **Injected pairs not_applicable** | 0 (no verdict) | 274 / 532 (51%, W45) | read W46 |
| Injected pairs helped | 60 / 373 (16%) | 107 / 532 (20%, W45) | read W46 |
| Muted (learning, repo) pairs | n/a | n/a | 15 (9 in dispatch) |
| PRs with a bullet list, after prose rule | 1 / 2 | 13 / 48 | 13 / 48 |

## Declined this run

- Raising the similarity floor: the weak separation above. Revisit only if a better query
  text (title without the long description) separates the verdicts better.
- A score penalty for `not_applicable`: the mute does the job per repo without changing
  the score anywhere.
- Deleting user preferences or high-query entries on `not_applicable` counts alone.

## Next run should try

1. The board must restart on this commit before the mute works. Then read W46 and W47.
   Success: the `not_applicable` share of injected pairs falls below 51% and `helped`
   holds at 20% or more. The "most often not_applicable" top ten should lose the muted
   ids. Freed slots refill, so expect a few points, not 10.
2. If the share does not move, the problem is in the candidates that fill the slots. Try
   the query text: embed the title plus the first paragraph of the description. Long
   recurring descriptions (like this task's) probably blur the query vector. The replay
   tool is easy to rebuild: decode stored embeddings from the dump and run the ONNX model
   in `.fastembed_cache` with CLS pooling.
3. #703 (fix pre-existing defects, 26 upvotes, 8 helped and 12 not_applicable) is a
   standing rule riding on similarity search. It is a candidate to move into the dispatch
   prompt (the baseline's first recommendation). Ask the operator.
