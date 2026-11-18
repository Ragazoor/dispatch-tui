# Learnings iteration 11: the rating step lost its `not_applicable` line

Task #99185, run 11, 2026-11-18. Follows `2026-11-14-99042-learnings-iteration-10.md`.

One change: the wrap-up skill's rating step names the `not_applicable` case again.

- **Skill** (`plugin/skills/wrap-up/SKILL.md`, Step 7.1): rate, in one `learning_rate`
  call, each entry you acted on (`helped` or `wrong`) and every injected entry that did
  not fit the task `not_applicable`.
- **Spec** (`docs/specs/wrap-up-skill.allium`, `RateBeforeClosing` and
  `RateKnowledgeAndWrapUp`): the guarantee now holds the `not_applicable` case, and says
  why the skill must name it itself rather than only point at the `learnings` skill.
- **Test**: `wrap_up_skill_text_holds_each_session_guarantee` pins the new sentence. It
  failed before the skill edit.
- `scripts/learnings-metrics.py` gains `wrap_up_rating_metrics`: of the done tasks that ran
  the wrap-up closing sequence, the share that rated every injected entry, per week.

## Why this, why now

**The verdict signal was collapsing.** Unrated injected pairs rose to 67% in W47 (W46: 18%).
W47 is not over, but most of those pairs belong to tasks that are already done. Splitting
done tasks by whether they ran the wrap-up closing sequence (trajectory has
`wrap_up`/`session_wrap-up`/`exit_session`/`session_exit`):

| First injection | Wrapped-up tasks | Rated every injected entry | Injected pairs unrated |
|---|---:|---:|---:|
| 2026-11-06 to 11-15 | 124 | 112 (90%) | 40 / 547 (7%) |
| 2026-11-16 to 11-18 | 53 | 27 (51%) | 123 / 265 (46%) |

Wrapped-up tasks that rated nothing went from about one a day to 8 to 10 a day from
2026-11-16. Commit 5491dc74 (2026-11-14, "trim ... wrap-up duplication") cut Step 7.1 from
"... Rate each injected entry that did not fit this task `not_applicable`" to "Rate
retrieved knowledge you acted on and have not rated yet, as the `learnings` skill
describes". The board reinstalled the plugin on its next restart; the drop starts the day
after (the installed copy is reinstalled on board restart).

The cause is likely, not proven. For it: four wrapped-up, unrated tasks read in full
(99227, 99254, 99263, 99290) all loaded the trimmed Step 7.1 and the prompt's
`not_applicable` line, made no rating call and never mentioned rating. Task 99296 called
`learning_rate` with an empty list, as if nothing qualified. Against it: the skippers
cluster by epic. Since 2026-11-16, epics 69703 and 69702 have 11 of 12 wrapped-up tasks
unrated, while epic 69699 has 16 of 17 rated under the same skill text. And the rated-only
`helped` share did not jump after the trim (below), which a pure "acted on nothing, so
rated nothing" effect would cause. So the trim is the change that removed the explicit
instruction, and some agents still rate without it. This run restores the text that gave
90%.

This matters beyond the metric: the per-repo mute (run 9) runs on `not_applicable`
verdicts. Unrated off-topic entries never reach the mute's threshold, so they keep their
slots.

Other drops are not caused by the skill: done tasks that never ran wrap-up (quick tasks,
dependabot cards, "Wrap-up check" test tasks) rose too (13 on 2026-11-18), but the skill
never runs there.

**Correction to run 10's reading.** The "dups deleted" row (dispatch rated-only `helped`
42%, 99/231 over 50 tasks) is real but uncertain. Bootstrap by task: 95% interval 36% to 50%,
against 24% to 40% for 2026-11-08 to 11-14. The difference's interval is 0.6 to 22 points:
above zero, but barely. Split at the trim: 50% (63/125) from 11-14 17:30 to 11-16, and 34%
(36/106) after. The rating drop does not inflate the `helped` share, but the window is
small. Read it again once rating recovers.

## Declined this run

- **A wider mute** (mute when an entry's `helped` share in the repo is under 20%, not
  only at zero `helped`). Chronological replay over 1,813 injected pairs since
  2026-10-29: 41 more `not_applicable` avoided, 5 `helped` lost, almost all from four
  entries (#8196, #90216, #569, #596). Freed slots refill at about the base rate, so the
  net gain is about one point. Thresholds were picked on the same data. A share rule
  also means a muted entry gets no more verdicts, so it never lifts: a ranking change,
  which runs 8 and 10 left to the operator.
- **The per-entry `helped` prior** (run 10's item 2): still the operator's call, and
  its input is the verdict stream this run repairs. Wait for W48 data.
- **Re-running the copy scan** (run 10's item 3): no rule was added to CLAUDE.md, a
  skill or the prompt that would create new copies.

## Metrics table (run-to-run)

The after column equals the before column: the skill change reaches agents only after
the board's startup check reinstalls the plugin.

| Measure | Run 10 (after) | Run 11 (before) | Run 11 (after) |
|---|---:|---:|---:|
| Learnings | 834 | 884 | 884 |
| Never upvoted | 413 | 445 | 445 |
| Negative score | 5 | 5 | 5 |
| Retrieval rows (tasks deleted) | 11,088 | 12,062 | 12,062 |
| Total upvotes | 1,470 | 1,604 | 1,604 |
| Retrieved 20+, never upvoted | 12 | 14 | 14 |
| Unrated and older than 30 days | 198 | 212 | 212 |
| `record_learning` refused, latest week | 8 / 82 (W46) | 11 / 115 (W46) | read W48 |
| Injected pairs not_applicable (all repos) | 205 / 384 (53%, W46) | 285 / 539 (52%, W46); 92 / 445 (20%, W47 partial) | read W48 |
| Injected pairs helped (all repos) | 84 / 384 (21%, W46) | 152 / 539 (28%, W46); 54 / 445 (12%, W47) | read W48 |
| **Injected pairs unrated** | 94 / 384 (24%, W46) | 101 / 539 (18%, W46); **299 / 445 (67%, W47)** | read W48 |
| **Wrapped-up done tasks rating every injected entry** | n/a | 93 / 105 (88%, W46); **27 / 53 (50%, W47)** | read W48 |
| Dispatch, rated-only helped, dups deleted | 65 / 203 (32%, after mute) | 99 / 231 (42%; 95% CI 36–50%) | read W48 |
| Muted (learning, repo) pairs | 34 | 44 | 44 |

W46 grew between runs because tasks rated after run 10 read it, so run 10's W46 numbers
were partial.

## Web check

Not run this time: the finding came from the store's own data, and no outside work bears
on a skill line that was cut.

## Next run should try

1. After the board restarts on this commit, read `wrap_up_rating_metrics` for W48.
   Success: 85% or more of wrapped-up tasks rate every injected entry, and unrated
   injected pairs fall back under 20%. If not, the closing sequence is not the only
   cause; look at whether `session_wrap-up`'s response should nudge on rebase and done
   too. Today it nudges only on the pr path (`src/mcp/handlers/tasks/wrap_up.rs`).
2. Out-of-scope retrievals came back: 12 of 436 repo or epic-scoped retrievals in W47
   fell outside the task's scope, after two weeks at 0. List those 12 rows and find the
   path that leaks.
3. Any future trim of `plugin/skills/wrap-up/SKILL.md` must keep the
   `not_applicable` line; the guarantee test now fails if it goes.
4. With W48 verdicts in, re-read the dispatch rated-only `helped` share and bootstrap it
   by task. Then put the per-entry `helped` prior to the operator with that number.
