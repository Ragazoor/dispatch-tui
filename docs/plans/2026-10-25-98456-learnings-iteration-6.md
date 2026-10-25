# Learnings iteration 6: the skill and the store disagreed

Task #98456, run 6, 2026-10-25. Follows `2026-10-22-94228-learnings-iteration-5.md`.

One change, with the operator's approval in this session:

- `plugin/skills/wrap-up/references/pr.md` now says PR bodies are short prose
  paragraphs, never bullet lists. Before, it told agents to flag risks "in a few short
  bullets" and showed a bulleted example, while user-scoped learning #90187 (the
  operator's ban on bullets) said the opposite. An agent got both and had to guess.
- #90187 deleted: its rule now lives in the skill. #61441 and #731 stay (#61441 was
  rated helped 9 times; neither contradicts the skill).

No spec, module or code change: the rule is agent-facing skill prose, which
`TheAuthoringRulesHaveOneHome` already keeps outside the spec. No test pins `pr.md`.

## Measured before the change (2026-10-25, W43 partial)

Post-rescope window: injection rows since 2026-10-22 11:30 (the run 5 rescope).

| Signal | Value |
|---|---:|
| Injection rows, all scopes | 250 (repo 177, user 42, epic 31) |
| **User-scoped injection rows with no `helped`** | **36 / 42 (86%)**; before the rescope W43 ran at 106 rows |
| Repo-scoped rows with no `helped` | 129 / 177 (73%) |
| Rows on tasks that rated anything | 180 / 250 |
| Repo/epic rows outside the task's scope | 1 / 923 (W43) |
| `query_learnings` refused for arguments | 1 / 50 (W43) |
| `record_learning` refused since the rescope | 21 / 51 calls, 16 tasks; 12 are argument errors |

The rescope worked on volume: user-scoped injections fell from about 106 a week to 42
in three days. The unhelped share did not fall (86%), so the leftover user entries are
mostly standing rules the agents already follow or that do not apply (see below).

The remaining unhelped user rows are led by #61441 (6), #69640 (5), #90187 (4), then
Cloud SQL runbook entries. #61441 is helped when it matters, so the unhelped rows
there are tasks without a PR.

`record_learning` argument errors: agents guess `content`, `category`, `type` or
`title`, every one a field of some other memory tool. All 12 were on 2026-10-23, and
each task recovered on its next call. Run 3 declined an alias; nothing here changes
that, because the error lists the right fields.

## Metrics table (run-to-run)

| Measure | Run 4 | Run 5 (after) | Run 6 (before) | Run 6 (after) |
|---|---:|---:|---:|---:|
| Learnings | 566 | 613 | 645 | 644 |
| Never upvoted | 266 | 297 | 318 | 318 |
| Negative score | 8 | 7 | 6 | 6 |
| Retrieval rows | 14,561 | 13,270 | 14,249 | 14,245 |
| Total upvotes | 1,509 | 1,043 | 1,119 | 1,121 |
| Retrieved 20+, never upvoted | 29 | 28 | 33 | 33 |
| Unrated and older than 30 days | 129 | 154 | 163 | 163 |
| `query_learnings` refused, latest week | 23 / 76 | 0 / 32 | 1 / 50 | 1 / 50 |
| Off-scope repo/epic rows, latest week | 0 / 96 | 0 / 303 | 1 / 923 | 1 / 923 |
| User-scoped injection rows, no `helped` | 217 / 258 (W42) | 111 / 133 (W43, forecast 44 / 60) | **36 / 42 since the rescope** | read W44 |

Honest read: this run moves no store number now. It removes a contradiction between
the store and the skill. The effect to look for is in PR bodies, below.

## Declined this run

- Deleting #61441 as a copy of `pr.md`: it was rated helped 9 times, so agents use it.
- An alias for `content`/`category` in `record_learning`: run 3's reasons still hold.
- Retiring the 33 entries retrieved 20+ times and never upvoted: the count grew from
  28 to 33, but nothing shows harm, and the inventory needs the operator. Next run.

## Web check

None this run. Nothing in runs 2 to 5 was contradicted; the change is a conflict fix,
not a new design.

## Next run should try

1. Count bullet lists in PR bodies from `--prs` before and after this commit lands (add
   a bullet check to `scripts/learnings-metrics.py`; the PR-body checker does not
   count them today). That is the outcome signal for this run.
2. Read W44 for the bold row. Expect below 70% unhelped with user rows under 60 a week.
3. Look for more skill-versus-store conflicts: search entries whose summary says
   "never" or "always" and grep `plugin/skills/` for the opposite. This run found one by
   reading the top unhelped entries; a script can do it for all of them.
4. Show the inventory of the 33 entries retrieved 20+ times and never upvoted, and ask
   which to archive.
5. The `staff_engineer` create-new-db template (outside this repo) still shows a
   bulleted PR body; the operator should update it.
