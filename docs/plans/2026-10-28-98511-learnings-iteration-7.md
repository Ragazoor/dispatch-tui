# Learnings iteration 7: fix the denominator, rescope four leaks

Task #98511, run 7, 2026-10-28. Follows `2026-10-25-98456-learnings-iteration-6.md`.

Changes, with the operator's approval in this session:

- `scripts/learnings-metrics.py` now prints the user-scoped unhelped share twice: over
  all injection rows, and over rows on tasks that rated something. It also counts PR
  bodies with a bullet or numbered list, split at the prose-rule commit (215c89e2).
- #565 (database-bootstrapper workflow permissions) rescoped to `repo`,
  `/home/ragge/Code/work/scala`; #500 (pglogical dump-then-subscribe) to `repo`,
  `/home/ragge/Code/work/terraform/gcp`. In place, by the `seed_learnings` reducer as in
  run 5; a diff against a backup showed only `scope` and `scope_ref` changed on both.
- #90138 and #797 deleted (0 upvotes, 6 and 7 injections since 2026-10-25, 0 helped).

No spec, module or code change.

## Finding: the bold row was partly a measurement error

Only `helped` and `wrong` verdicts exist. A task that rates nothing contributes all its
injected rows as "unhelped". Since 2026-10-25, 16 of 28 tasks with user-scoped injections
rated anything. Over rated tasks only:

| Week | All rows | Rated tasks only |
|---|---:|---:|
| W41 | 68/88 (77%) | 54/74 (72%) |
| W42 | 156/185 (84%) | 129/158 (81%) |
| W43 | 124/152 (81%) | 73/101 (72%) |
| W44 (partial) | 85/100 (85%) | 40/55 (72%) |

So the rescope in run 5 moved the real share from 81% to 72%, and it is flat since. The
85% in W44 comes from unrated tasks. Use the right-hand column from now on.

## Other measurements

- Bullets in PR bodies, before the prose rule (landed today): 151 of 181 PRs. After:
  0 of 0. Run 6's signal cannot be read yet; rerun in a week.
- `record_learning` refused for arguments: 11 / 42 in W44 (26%), spread over `content`,
  `title`, `scope`, `task_id` and scope words used as `kind`. Each recovers on the next call.
- "wrap_up: Verify is still running" hit 30 tasks since 10-14. It is an informational
  wait message, not a learnings mistake.
- The 35 entries retrieved 20+ times and never upvoted: nearly all were found by agents
  through `query_learnings` (the agent asked for them), not injected. Retrieved but not
  rated is weak evidence of harm, so none are pruned. Top: #446 (108), #585 (78), #272 (66).

## Metrics table (run-to-run)

| Measure | Run 6 (after) | Run 7 (before) | Run 7 (after) |
|---|---:|---:|---:|
| Learnings | 644 | 670 | 668 |
| Never upvoted | 318 | 329 | 328 |
| Negative score | 6 | 5 | 5 |
| Retrieval rows | 14,245 | 15,129 | 15,095 |
| Total upvotes | 1,121 | 1,183 | 1,183 |
| Retrieved 20+, never upvoted | 33 | 35 | 35 |
| Unrated and older than 30 days | 163 | 170 | 170 |
| `query_learnings` refused, latest week | 1 / 50 | 0 / 15 (W44) | 0 / 15 |
| Off-scope repo/epic rows, latest week | 1 / 923 | 8 / 438 (W44) | 8 / 438 |
| User-scoped unhelped, all rows | 36 / 42 since rescope | 85 / 100 (W44) | read W45 |
| **User-scoped unhelped, rated tasks only** | n/a | **40 / 55 (72%, W44)** | read W45 |
| PRs with a bullet list | n/a | 151 / 181 | read in a week |

## Declined this run

- A scope field on `LearningPatch` plus an MCP edit tool (run 5 declined it; advised
  again). Four hand rescopes in 15 days do not justify a module change, bindings and a
  rules bump, and it moves no number by itself.
- A `not_applicable` verdict. It would separate "unrated" from "did not apply", and it is
  the outcome signal this store lacks. But it changes `VerdictKind`, the module, bindings
  and the injection prompt. A cheaper step first: the rated-only column above.
- An alias for `content` in `record_learning`: run 3's reasons hold.
- Pruning the 35 never-upvoted entries: see above.

## Next run should try

1. Read W45: rated-only share should stay at or under 72%, and #565 and #500 should
   appear only in scala and terraform tasks.
2. Rerun `--prs` for the bullet count after the prose rule.
3. Consider the `not_applicable` verdict as a spec-first change if the rated-only share
   still sits near 72%: the remaining rows are then unrated or truly irrelevant, and
   only a new verdict tells them apart.
4. Still open: #12308 and #734 span kognic-cd, terraform and staff_engineer, so they stay
   `user`; check whether a `/home/ragge/Code/work` parent scope fits.
