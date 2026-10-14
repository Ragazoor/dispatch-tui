# Learnings iteration 1: measure the outcome, not the exposure

Task #73732, run 1, 2026-10-14. Follows `2026-10-14-73731-how-useful-is-learnings.md`
(the baseline). Change shipped: `scripts/learnings-metrics.py`, a repeatable
measurement. No spec or store change, and no deletes.

## Why no behaviour change this run

Candidate 1 (promote standing rules out of the store) is already done. The top
rules live in the wrap-up skill and in `CLAUDE.md`:

| Entry | Rule | Now lives in | Retrievals | Injections |
|---|---|---|---:|---:|
| #146, #154 | no task id / `Implements #N` in a PR body | `plugin/skills/wrap-up/references/pr.md` | 439 + 426 | 159 + 148 |
| #188 | omit the Test plan section | `pr.md` | 205 | (low) |
| #488 | reviewer-agent notes go in an HTML comment | `pr.md` | 100 | (low) |
| #82 | check `Wrap-up mode` in `get_task` | wrap-up `SKILL.md` Step 2 | 697 | 345 |
| #428 | run the test suite in the foreground | `CLAUDE.md` Build & Test | 145 | 85 |
| #133 | worktree-prefixed paths | `CLAUDE.md` Agent Working Directory | 128 | (low) |
| #322 | run `cargo fmt` before committing | `CLAUDE.md` pre-push hook note | 156 | 63 |

These nine entries took about 2,500 retrieval rows, 16% of all rows. #428 also
states a stale figure (15-20s; `CLAUDE.md` now says about 4s).

## The outcome signal

Question: do agents still break these rules? Method: every `exit_session` on the
pr path since the rules reached the skill (2026-08-21) names a PR. I fetched all
108 bodies with `gh pr view` and tested them.

| Breach | PRs (of 108) |
|---|---:|
| `Implements #N` | 0 |
| the dispatch task id | 0 |
| a `Test plan` section | 0 |

The same check on the 29 PRs before 2026-08-21 also shows 0. So this signal does
not move: the mistake is not seen in either period, and these data cannot say
whether the skill, the injected entries or the agents' own habits prevent it.
The check is a regression guard, not proof the store works.

## Metrics table (run-to-run)

Produced by `uv run python -I scripts/learnings-metrics.py --prs`.

| Measure | 73731 baseline | Run 1 |
|---|---:|---:|
| Learnings | 529 | 530 (528 approved, 2 archived) |
| Never upvoted | 256 | 256 |
| Negative score | 6 | 6 |
| Created Jul / Aug / Sep / Oct | n/a / 112 / 309 / 83 | 14 / 112 / 309 / 84 |
| Retrieval rows | 16,066 | 16,076 |
| Tasks with retrievals | 1,217 | 1,219 |
| `query_learnings` / injection / procedural | 11,394 / 4,279 / 393 | 11,394 / 4,289 / 393 |
| Total upvotes | 1,455 | 1,452 |
| Top-10 / top-30 upvote share | 44% / 63% | 44% / 62% |
| Top-50 retrieval share | 48% | 44% (the baseline counted differently) |
| Dangling retrieval ids | 76 | 76 |
| Retrieved 20+, never upvoted | 37 | 35 |
| Unrated and older than 30 days | 120 | 120 |
| PR-body rule breaches (PRs since 2026-08-21) | n/a | 0 of 108 |

The store barely moved between the two dumps, as expected within one day.

## Caveats on the data

- `upvote_count` is weaker than the baseline said. Wrap-up Step 7.1 tells agents
  to `rate_learning` each injected entry they acted on, so injected standing rules
  collect "helped" almost by default. A high score means "injected, and the agent
  reached wrap-up".
- Trajectories hold MCP calls only, not the shell. A PR body is only visible by
  fetching the PR.

## Related work found (new since the baseline)

- **VibeMemBench** ([arXiv 2609.23570](https://arxiv.org/pdf/2609.23570)): on real
  repository tasks, an injected memory record sometimes made a run fail that
  passed without it. Compact, verified records beat long raw ones. Supports
  pruning and a recorder bar.
- **PROJECTMEM** ([arXiv 2606.12329](https://arxiv.org/pdf/2606.12329)): a
  deterministic gate that warns before the agent repeats a failed fix, rather
  than passive recall. Small self-study (10 projects). Matches "mechanical check"
  over "retrieve a note".

## Proposals for the operator (nothing deleted)

Inventory above. Proposed: archive #146 and #154 (duplicates, both now in
`pr.md`), then #188, #488, #82, #428, #133, #322 (now in the skill or `CLAUDE.md`).
Effect to expect: about 2,500 fewer retrieval rows, and injection slots freed for
entries that have no other home. Needs the operator's approval first.

## Next run should try

1. After the operator rules on the archive list, re-run the metrics. The
   injection share of entries with a home elsewhere should fall to 0.
2. Find a repeated mistake that is *seen*. The PR rules show none. Sample
   trajectories for repeats: the same `wrap_up`/`exit_session` refusal text on
   one task, or `update_task` retried with the same error. A recurring one is a
   candidate for a mechanical check in `wrap_up` (PROJECTMEM's gate idea).
3. Dedupe on write, in `record_learning`, with the stored embedding.
4. An A/B for injection: skip injection on a random 10% of tasks and compare
   refusal and rework counts. This is the only way to attribute outcomes.
