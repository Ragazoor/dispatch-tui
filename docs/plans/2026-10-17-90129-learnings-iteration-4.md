# Learnings iteration 4: a repo scope that can cover a family of repos

Task #90129, run 4, 2026-10-17. Follows `2026-10-16-90120-learnings-iteration-3.md`.
Change shipped: a repo-scoped entry's `scope_ref` now matches a task whose `repo_path`
equals it **or lies under it at a path separator** (`src/embeddings.rs::repo_ref_covers`,
spec in `docs/specs/learnings.allium`, Scope Model and `QueryLearningsViaMcp`).
`scripts/learnings-metrics.py` gets a new signal row and loses a duplicated block.
No entry deleted or changed. No module, enum or rules-version change.

## Run 3's signal, read cleanly

The dumps use UTC for both the store and the trajectories. The board binary was built
2026-10-17 18:53 UTC, after commit 8ed41883.

| Cut | Repo/epic-scoped rows outside the task's scope |
|---|---:|
| W42 (mostly before the fix) | 186 / 587 (31%) |
| Since the fix, 2026-10-16 20:40 UTC | **0 / 96** |
| Since the board restart, 2026-10-17 18:53 UTC | 0 / 81 |

Run 3's change worked. Run 2's `task_id` reminder: only 3 `query_learnings` calls
since the restart, 0 refused. Too few to claim. Re-check in W43.

## The finding

User scope is the only scope that spans more than one repo, so repo-specific facts get
recorded as `user` and then leak. Among user-scoped *injection* rows (agents rate those
at wrap-up almost by default), 439 of 544 (81%) got no `helped` verdict. The top 30
entries cause 85% of those. Most are Kognic Terraform and Cloud SQL facts. The 38
entries listed below were retrieved 1,842 times; 839 (46%) landed in repos outside
`/home/ragge/Code/work/terraform/` (`kognic-mcp`, `kognic-cd`, `erp-scala`,
`scala-common`, ...), where agents rated them helped 0 times in the repos that could
be identified. Rows on deleted tasks cannot be placed.

All Terraform repos share one parent, `/home/ragge/Code/work/terraform/gcp` (187 of
188 tasks). So one repo-scoped entry with that `scope_ref` can cover them all, if the
match is by path prefix. No existing repo entry's `scope_ref` is a proper prefix of any
repo in the store, so today's retrieval is unchanged.

Per-(entry, repo) suppression from verdicts was ruled out: verdicts are not stored
(`learning_verdicts` was dropped in migration v74); they exist only in trajectories.

## Metrics table (run-to-run)

| Measure | Run 2 | Run 3 | Run 4 |
|---|---:|---:|---:|
| Learnings | 558 | 562 | 566 (564 approved, 2 archived) |
| Never upvoted | 266 | 268 | 266 |
| Negative score | 8 | 8 | 8 |
| Retrieval rows | 14,261 | 14,311 | 14,561 |
| `query_learnings` / injection / procedural | 10,610 / 3,258 / 393 | 10,610 / 3,308 / 393 | 10,760 / 3,408 / 393 |
| Total upvotes | 1,483 | 1,493 | 1,509 |
| Retrieved 20+, never upvoted | 29 | 29 | 29 |
| Unrated and older than 30 days | 131 | 131 | 129 |
| `query_learnings` refused, W42 | 23 / 73 | 23 / 73 | 23 / 76 |
| Off-scope repo/epic rows, since the fix | n/a | W42 31% | **0 / 96** |
| **User-scoped injection rows with no `helped`, W42** | n/a | n/a | **217 / 258 (84%)** |
| Top-30 entries' share of those rows | n/a | n/a | 85% |

Outcome signal for this run: the bold row. It is the before number. It only moves
after entries are rescoped (below); the code change alone cannot move it.

## What the change does and does not do

Entries have no edit action, so rescoping needs delete plus a new `record_learning`,
which loses ids and upvotes. I did not do that without the operator's say-so.

Proposal for the operator (inventory above, nothing done): re-record the clearly
Terraform-domain entries below with `scope=repo` and
`scope_ref=/home/ragge/Code/work/terraform/gcp`, then delete the user-scoped originals:
#574 #578 #579 #580 #598 #599 #601 #606 #614 #621 #630 #632 #633 #637 #670 #671 #683
#699 #700 #701 #721 #724 #737 #767 #770 #772 #775 #776 #777 #780 #791 #831 #12292
#12304 #69635 #69637. Leave #600 (kognic-cd) and #634 (a general comment rule) as user.
Collapse the three cap-length entries (#699 wrong at 15, #701 and #721 say 18) to one.
Expected effect: about 800 fewer off-target rows over the same period.

## Declined this run

- Tuning the user multiplier or the 0.25 threshold: neither filters, and run 3 showed a
  weight is not a filter.
- A new `LearningScope` variant: module change, bindings, wasm rebuild and a rules
  version bump with other agents live on the board.

## Next run should try

1. Re-run the metrics in W43. Off-scope should still read 0%; `query_learnings`
   refusals should be well under 32% if the run 2 fix is live.
2. If the operator approved the rescope, re-check the bold row. It should fall from 84%.
3. `wrap_up` "Verify is still running" hit 30 tasks and `exit_session` "no exit token"
   13 tasks in 14 days, the largest repeats now. Measure the gap between repeated
   `wrap_up` calls; if it is seconds, a server-side wait is the fix, not a note.
4. `record_learning` `unknown field content` still hits 11 tasks.
