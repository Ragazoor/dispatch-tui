# Learnings iteration 12: no behaviour change; the W47 leak was a moved task

Task #99317, run 12, 2026-11-18. Follows `2026-11-18-99185-learnings-iteration-11.md`.

This run changes no behaviour. It fixes one metric and reads run 11's open questions.

- `scripts/learnings-metrics.py`, `off_scope_metrics`: an epic-scoped retrieval is judged
  against the epic the task was created in (from its `create_task` trajectory) as well as
  its current epic. A row whose current epic was created after the retrieval, with no
  creation epic known, is skipped and counted in a new "skipped" column.

## Why no behaviour change

**Run 11's fix is not live yet.** Its commit (edc42747) landed at 18:24 CET today. The
installed copy, `~/.claude/plugins/local/dispatch/skills/wrap-up/SKILL.md`, was last
written at 17:31 and still holds the trimmed Step 7.1. The board reinstalls it on its next
restart. Nothing measures the fix until then, and a second change to the same step now
would blur the reading.

**The rating drop is not tied to one wrap-up path.** Run 11 left two other causes open: the
server nudge (`session_wrap-up` nudges on the pr path only) and the epic clustering.
Wrapped-up done tasks that rated every injected entry, split by first injection:

| Split | 2026-11-09 to 11-15 | 2026-11-16 to 11-18 |
|---|---:|---:|
| `wrap_up_mode` rebase | 38 / 39 (97%) | 24 / 38 (63%) |
| `wrap_up_mode` pr | 5 / 6 (83%) | 3 / 5 (60%) |
| `wrap_up_mode` unset | 49 / 59 (83%) | 3 / 15 (20%) |
| Closed with the old tools (`wrap_up`, `exit_session`) | 93 / 105 (88%) | 20 / 44 (45%) |
| Closed with the new tools (`session_wrap-up`, `session_exit`) | n/a | 12 / 16 (75%) |
| Wrap-up reply carried a rating nudge | 8 / 11 (72%) | 2 / 8 (25%) |

Every mode fell, and the tasks that got the nudge fell too, so a nudge on rebase and done
would not have prevented it. Run 11's reading stands: the trimmed skill line is the likely
cause. The epic clustering also stands: epic 69703 rated 0 of 8, epic 69702 3 of 8, epic
69699 16 of 17. Samples after 11-16 are small.

**The 12 "out-of-scope" retrievals in W47 were not a leak** (run 11's item 2). All 12 are
prompt injections on 2026-11-17 06:44 into tasks 99271 to 99275. Task 90128 created them
with `epic_id` 57346 at 06:43, and the injected entries (#90220, #90303, #90305) are scoped
to epic 57346. Epic 69704 was created at 13:17, and the five tasks moved into it then. The
metric compared the row against the current epic. With the fix, W47 reads 0 of 471.

## Declined this run

- **A rating nudge in `session_wrap-up`'s reply on the rebase and done paths.** The table
  above shows the drop on every path, and nudged tasks dropped too.
- **`record_learning` argument refusals** (`missing field task_id`: 8 tasks;
  `unknown field content`: 6 tasks since 11-04). Run 3 declined this with reasons that
  still hold: the schema is right and the error lists the valid fields. All 21 refused calls
  since 11-04 were followed by a valid call from the same task.
- **Dedupe on write, the wider mute, the per-entry `helped` prior**: declined in runs 3, 10
  and 11; no new evidence.

## Metrics table (run-to-run)

| Measure | Run 11 (after) | Run 12 (before) | Run 12 (after) |
|---|---:|---:|---:|
| Learnings | 884 | 886 | 886 |
| Never upvoted | 445 | 446 | 446 |
| Negative score | 5 | 5 | 5 |
| Retrieval rows (tasks deleted) | 12,062 | 12,097 | 12,097 |
| Total upvotes | 1,604 | 1,610 | 1,610 |
| Retrieved 20+, never upvoted | 14 | 14 | 14 |
| Unrated and older than 30 days | 212 | 212 | 212 |
| Off-scope repo/epic retrievals (W47) | 12 / 436 | 12 / 471 | **0 / 471** (metric fix) |
| `record_learning` refused, latest week | 11 / 115 (W46) | 4 / 54 (W47 partial) | 4 / 54 |
| Injected pairs helped (all repos, W47 partial) | 54 / 445 (12%) | 60 / 480 (12%) | 60 / 480 |
| Injected pairs not_applicable (W47 partial) | 92 / 445 (20%) | 111 / 480 (23%) | 111 / 480 |
| **Injected pairs unrated** (W47 partial) | 299 / 445 (67%) | 309 / 480 (64%) | 309 / 480 |
| **Wrapped-up done tasks rating every injected entry** (W47) | 27 / 53 (50%) | 32 / 60 (53%) | 32 / 60 |
| Dispatch, rated-only helped, dups deleted | 99 / 231 (42%) | 105 / 256 (41%) | 105 / 256 |
| Muted (learning, repo) pairs | 44 | 50 | 50 |

## Web check

One search. Closest to this store: RL Developer Memory
([arXiv 2605.01567](https://arxiv.org/abs/2605.01567)) logs each retrieval as a decision
and ties later outcomes back to it, as our verdicts do; Memory-as-a-Tool
([arXiv 2601.05960](https://arxiv.org/pdf/2601.05960v2)) keeps a curated lessons journal
with list-then-select retrieval. Only summaries read. Neither addresses an agent that
skips the rating step, which is our current problem.

## Next run should try

1. Find when the board reinstalled the wrap-up skill: the installed `SKILL.md`'s mtime,
   once it holds the `not_applicable` line. Split `wrap_up_rating_metrics` at that time,
   not at a week boundary. Success: 85% or more of wrapped-up tasks rate every injected
   entry, and unrated injected pairs fall under 20%.
2. If it does not recover, read the epic clustering (69703 and 69702 against 69699): what
   do their prompts or flows load that 69699's do not?
3. Six tasks since 11-04 rated an entry that was never retrieved for them
   ("learning N was not retrieved during task N"). One (#90215, task 99033) matches run 9's
   delete that day. Look at where agents saw the other five ids, for instance the
   similar-entries list in `record_learning`'s reply.
