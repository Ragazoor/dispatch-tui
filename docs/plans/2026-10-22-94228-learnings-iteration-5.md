# Learnings iteration 5: the curation that waited four runs

Task #94228, run 5, 2026-10-22. Follows `2026-10-17-90129-learnings-iteration-4.md`.
Changes shipped, all with the operator's approval in this session:

- 38 user-scoped entries moved to `repo` scope **in place** (ids, upvotes and
  embeddings kept): 34 Kognic Terraform and Cloud SQL facts to
  `/home/ragge/Code/work/terraform/gcp`, and #617 #600 #742 #4107 to
  `/home/ragge/Code/work/kognic-cd`.
- 9 entries deleted: #699 (cap length 15, wrong) and #721 (a copy of #701), and run 1's
  archive list #146 #154 #188 #488 #82 #133 #322, whose rules live in the wrap-up skill
  or `CLAUDE.md`.
- `plugin/skills/learnings/references/recording.md`: a scope row for a family of repos
  (`repo` with the parent directory as `scope_ref`), so new domain facts stop landing
  in `user`.
- `scripts/learnings-metrics.py` reads each entry's scope as of the retrieval
  (`RESCOPED_AT`, `RESCOPED_FROM_USER`), so the rescope does not rewrite past weeks.

No spec, module or code change. The spec already allowed a parent `scope_ref` (run 4)
and keeps the scope table in the skill alone (`TheAuthoringRulesHaveOneHome`).

## How the rescope was done

`LearningPatch` has no scope field, and no MCP tool edits an entry. The
`seed_learnings` reducer upserts a full row by id, so each entry was re-read just before
its own call and sent back with only `scope` and `scope_ref` changed. A diff of the
whole table against the backup showed exactly those two columns changed on the 38 ids.
The one other changed row (#81923) was another agent's upvote. This is a one-off data
fix by hand, not a path to repeat: it skips `validate_learning_scope`.

## Measured before the change

W43 (Mon 2026-10-19 to now, partial):

| Signal | Value |
|---|---:|
| Repo/epic rows outside the task's scope | 0 / 303 (run 3's fix holds) |
| `query_learnings` refused for arguments | 0 / 32 (run 2's fix holds; was 23 / 77 in W42) |
| `record_learning` refused for arguments | 9 / 79 |
| **User-scoped injection rows with no `helped`** | **111 / 133 (83%)** |

On tasks that rated anything since 2026-10-12, 56 of 250 user-scoped injections were
rated helped. #670 (Terraform `postgresql_extension`) was injected 22 times, rated
helped 0 times, mostly in kognic-cd, kognic-mcp and opentelemetry-scala.

Run 4's third lead is closed: `wrap_up` "Verify is still running" has no hit after
2026-10-16 and "no exit token" none after 2026-10-10. Both still showed in the 14-day
error list only because of the window.

## Expected effect (forecast, not measured)

The script run as if the new scopes had held all of W43:

| W43 | As it happened | With the new scopes |
|---|---:|---:|
| User-scoped injection rows | 106 | 60 |
| …with no `helped` | 89 (83%) | 44 (73%) |
| Rows the rescoped entries now lose (outside their new repo) | n/a | 152 of 264 |

(106 and 89, not 133 and 111: deleting an entry deletes its retrieval rows.) The real
after number needs a week of retrievals: read W44.

## Metrics table (run-to-run)

| Measure | Run 2 | Run 3 | Run 4 | Run 5 |
|---|---:|---:|---:|---:|
| Learnings | 558 | 562 | 566 | 622 → 613 |
| Never upvoted | 266 | 268 | 266 | 299 → 297 |
| Negative score | 8 | 8 | 8 | 8 → 7 |
| Retrieval rows | 14,261 | 14,311 | 14,561 | 15,524 → 13,270 |
| `query_learnings` / injection / procedural | 10,610 / 3,258 / 393 | 10,610 / 3,308 / 393 | 10,760 / 3,408 / 393 | 9,856 / 3,021 / 393 (after) |
| Total upvotes | 1,483 | 1,493 | 1,509 | 1,615 → 1,043 |
| Retrieved 20+, never upvoted | 29 | 29 | 29 | 29 → 28 |
| Unrated and older than 30 days | 131 | 131 | 129 | 155 → 154 |
| `query_learnings` refused, latest week | 23 / 73 | 23 / 73 | 23 / 76 | **0 / 32 (W43)** |
| Off-scope repo/epic rows, latest week | n/a | 31% | 0 / 96 | 0 / 303 (W43) |
| **User-scoped injection rows with no `helped`** | n/a | n/a | 217 / 258 (84%, W42) | **111 / 133 (83%, W43) before; forecast 44 / 60 (73%)** |

"→" is before → after this run's deletes. Upvotes fell by 572 because the seven
standing rules held most of them; the top-10 upvote share fell from 42% to 23%.

## Declined this run

- A scope field on `LearningPatch` plus an MCP edit tool. It is the clean way to curate,
  but a module change, bindings and a rules bump; it moves no number by itself, and
  the in-place reseed did the one job needed now.
- `record_learning` "unknown field `content`" (4 calls in W43): run 3 declined it, the
  error already lists the fields.

## Web check

Recent work keeps pointing the same way: retrieval should filter by scope and limit
irrelevant memories (survey, [arXiv 2602.06052](https://arxiv.org/pdf/2602.06052));
one 2026 paper treats "inject nothing" as a valid choice when no memory fits
([arXiv 2604.27283](https://www.opentrain.ai/papers/learning-when-to-remember-risk-sensitive-contextual-bandits-for-abstention-aware--arxiv-2604.27283/),
small offline tests). Only abstracts read.

## Next run should try

1. Read W44 for the bold row. Expect about 73% or lower, and about half the user-scoped
   injection rows. If not, check that the rescoped ids still carry `repo` scope.
2. Check that new Terraform or kognic-cd entries are recorded as `repo` with a parent
   `scope_ref` once the plugin is reinstalled with this skill text. Count new
   user-scoped entries whose source task's repo is under `terraform/gcp` or `kognic-cd`.
3. The remaining unhelped user-scoped rows: #12308 and #734 (migration runbooks that
   span kognic-cd and Terraform), #69640, #702, #69639, #477. Look at where each one
   helps before rescoping.
4. If curation keeps needing hand work, build the scope patch and an edit tool
   (declined above) so agents can rescope without a delete.
