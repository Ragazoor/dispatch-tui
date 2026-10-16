# Learnings iteration 3: retrieval returned entries from other repos and epics

Task #90120, run 3, 2026-10-16. Follows `2026-10-16-77825-learnings-iteration-2.md`.
Change shipped: `rag_rank_learnings` (`src/embeddings.rs`) now drops repo-scoped
entries for another repo and epic-scoped entries for another epic. Before, it only
gave them no scope boost. Spec text made explicit in `docs/specs/learnings.allium`
(`QueryLearningsViaMcp`). `scripts/learnings-metrics.py` prints the off-scope share
per week. No entries deleted or changed.

## The finding

`learnings.allium` already said retrieval is the union of user, `repo:<task.repo_path>`
and `epic:<task.epic_id>`. The code fetched every approved entry and set the scope
multiplier to 0 for a non-matching repo or epic, but kept the entry as a candidate.
One similarity threshold of 0.25 was all that stood between an agent and another
repo's rules. Both `query_learnings` and dispatch injection use this ranker.

Retrievals on tasks still in the store, scope compared exactly with the task's repo
path or epic id:

| Source | Scope | Outside the union | Rows |
|---|---|---:|---:|
| `query_learnings` | repo | 231 (47%) | 488 |
| `query_learnings` | epic | 259 (41%) | 627 |
| injection | repo | 10 (3%) | 307 |
| injection | epic | 14 (2%) | 595 |

Does it matter? Agent verdicts from the trajectories, by whether the entry's source
task was in the same repo as the retrieving task (a repo-name match, so approximate):

| Scope, source | Same repo: rated helped | Other repo: rated helped |
|---|---:|---:|
| repo, `query_learnings` | 9% (631 rows) | 2% (528 rows) |
| epic, `query_learnings` | 19% (185) | 8% (505) |
| user, `query_learnings` | 17% (317) | 11% (3,458) |

Exact scope_ref matches are reliable: for same-name repos 1,424 of 1,428 rows match
the task's `repo_path` byte for byte, so the filter does not exclude real matches.

## Metrics table (run-to-run)

| Measure | Run 2 | Run 3 |
|---|---:|---:|
| Learnings | 558 | 562 (560 approved, 2 archived) |
| Never upvoted | 266 | 268 |
| Negative score | 8 | 8 |
| Retrieval rows | 14,261 | 14,311 |
| `query_learnings` / injection / procedural | 10,610 / 3,258 / 393 | 10,610 / 3,308 / 393 |
| Total upvotes | 1,483 | 1,493 |
| Retrieved 20+, never upvoted | 29 | 29 |
| Unrated and older than 30 days | 131 | 131 |
| `query_learnings` refused for arguments, W42 | 23 / 73 | 23 / 73 (fix not live yet) |
| `record_learning` refused for arguments, W42 | 16 / 119 | 16 / 125 |
| **Repo/epic-scoped retrievals outside the task's scope, W42** | n/a | **186 / 480 (38%)** |

Outcome signal for this run: the bold row. It was 21% to 38% in W39 to W42. After
the board restarts on this commit, new rows should show 0%. Run 4 should check W43
or later. Compare ratios, not counts: retrieval rows disappear with deleted tasks.

## Declined this run, with reasons

- **Dedupe on write.** Near-duplicate rate by stored embedding (cosine at or above
  0.9, against any earlier approved entry): 12 of 449 (2.7%) before the
  similar-entries nudge of 2026-10-04, 3 of 111 (2.7%) after. At 0.95: 1 and 0.
  Word-overlap similarity finds 1 pair in each period. There is little to merge.
- **`record_learning` refusals** (27 of 198 calls since 2026-10-02). The tool schema
  is correct and lists every field; agents that guess `content` or `category` are
  calling before loading it. The error already lists the valid fields. Not a store fix.
- **A second prune list** (29 entries retrieved 20+ times, never upvoted). Run 1's
  archive list still waits for the operator. Many are Kognic Terraform entries
  scoped `user`; they are visible in the "user, other repo" row above. Proposal for
  the operator, not done: rescope these to `repo` with the Terraform repos as
  `scope_ref`, or leave them as user-scoped if they should reach every repo.

## Next run should try

1. Re-run the metrics after the board restarts on this commit. The off-scope row
   should read 0%. If not, find the path that still returns them.
2. Compare the helped rate of `query_learnings` rows before and after, per scope.
   Dropping rows that agents rated helped 2% of the time should lift the share.
3. `user`-scoped entries retrieved on unrelated repos (3,458 rows, 11% helped) are
   the next precision gap. A scope or tag rule for them needs a measure of which
   repos each entry helps in first.
4. Run 1's archive list, if the operator has ruled on it.
