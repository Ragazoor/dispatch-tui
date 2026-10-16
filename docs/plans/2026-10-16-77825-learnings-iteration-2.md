# Learnings iteration 2: the knowledge-base reminder broke its own call

Task #77825, run 2, 2026-10-16. Follows `2026-10-14-73732-learnings-iteration-1.md`.
Change shipped: the PR gate's reminder names `task_id` with the task's id; the
`/learnings` skill names `task_id` too; `scripts/learnings-metrics.py` now prints a
repeated-mistakes section from the MCP trajectories. No deletes.

## The repeated mistake

Run 1 found no *seen* repeated mistake in PR bodies. This run looked for one in the
trajectories: every MCP error, grouped by message, counted by distinct tasks.

The learnings tools are the hardest tools to call. Refused for their arguments:

| Tool | Calls | Refused | Share |
|---|---:|---:|---:|
| `query_learnings` | 608 | 91 | 15% |
| `record_learning` | 1,191 | 142 | 12% |
| `rate_learning` | 2,361 | 1 | 0% |
| `get_task`, `create_task`, `update_task` | 4,424 | 61 | 0–4% |

It is getting worse. `query_learnings` refusals per ISO week: W39 2/32, W40 12/50,
W41 9/31, W42 23/73 (32%). In the last 14 days, "missing field `task_id`" on
`query_learnings` is the error the most distinct tasks hit (33), ahead of every
`wrap_up` and `exit_session` error.

Cause: 79 of the 88 refused calls passed only `query`. The PR gate's reminder said
"call `query_learnings`, describing this change in `query`" and nothing else. Of
the 44 refused calls on tasks still in the store, 35 came within five minutes after
that task's gate fired. 85 tasks made a `query_learnings` call in that window, and
35 of those first attempts were refused (41%). Agents nearly always retried
(95% recover), so the cost is a wasted turn on the one forced knowledge-base
lookup per PR, not lost recall.

## The change

- Spec: `PrLearningsGate` in `docs/specs/pr-workflow.allium` now says the reminder
  names `task_id`, the one argument `query_learnings` requires, with the task's
  own id.
- Test: `tests/hooks.rs::pr_gate_reminder_names_task_id_with_this_tasks_id`
  (failed first, then passed).
- Code: the reminder is composed per task with its id.
- Skill: `plugin/skills/learnings/SKILL.md` names `task_id` in its query line.

The parameter stays required. The fix surfaces the value instead of inferring it
(the operator prefers required parameters plus context in the prompt).

## Metrics table (run-to-run)

Store measures from `uv run python -I scripts/learnings-metrics.py`.

| Measure | 73731 baseline | Run 1 | Run 2 |
|---|---:|---:|---:|
| Learnings | 529 | 530 | 558 (556 approved, 2 archived) |
| Never upvoted | 256 | 256 | 266 |
| Negative score | 6 | 6 | 8 |
| Created Oct | 83 | 84 | 113 |
| Retrieval rows | 16,066 | 16,076 | 14,261 (see below) |
| Tasks with retrievals | 1,217 | 1,219 | 1,028 |
| `query_learnings` / injection / procedural | 11,394 / 4,279 / 393 | 11,394 / 4,289 / 393 | 10,610 / 3,258 / 393 |
| Total upvotes | 1,455 | 1,452 | 1,483 |
| Top-10 / top-30 upvote share | 44% / 63% | 44% / 62% | 44% / 61% |
| Top-50 retrieval share | 48% | 44% | 49% |
| Dangling retrieval ids | 76 | 76 | 76 |
| Retrieved 20+, never upvoted | 37 | 35 | 29 |
| Unrated and older than 30 days | 120 | 120 | 131 |
| PR-body rule breaches | n/a | 0 of 108 | not re-run |
| **`query_learnings` refused for arguments, latest week** | n/a | n/a | **23 / 73 (W42)** |
| **`record_learning` refused for arguments, latest week** | n/a | n/a | **16 / 119 (W42)** |

The two bold rows are the outcome signal this run adds. The "after" number needs a
week of agents on a board built from this commit: the board composes the reminder,
so the fix takes effect when the operator's board restarts on it. The skill line
takes effect when the plugin is reinstalled. The weekly rows count every call,
retries included, so compare them week to week, not with the 41% above.

Retrieval rows fell because deleting a task deletes its retrieval rows (the task
delete reducer detaches learnings and drops their retrievals). 8,590 rows still
point at tasks that are gone, presumably rows carried over when retrievals moved
into the shared store on 2026-09-26, before the cascade reached them. Retrieval counts therefore
shrink as tasks are deleted and are not comparable run to run. Trajectory files
are never deleted, so prefer them for anything measured over time.

## Not done this run, and why

- **Run 1's archive list** (#146, #154, #188, #488, #82, #133, #322) still waits
  for the operator. #428 is gone. Nothing deleted.
- **Dedupe on write.** `record_learning` already lists up to five similar entries
  and warns against a copy. Its effect is not measured yet; measuring the
  near-duplicate rate before and after that nudge comes first.

## Other repeated mistakes seen (outside the learnings tools)

- `wrap_up` "Verify is still running" hit 30 tasks in 14 days, up to 6 calls on one
  task. Agents poll instead of waiting. Belongs to the wrap-up flow, not here.
- `record_learning` "unknown field `content`" hit 9 tasks: agents guess the field
  name, likely calling the tool before its schema is loaded.

## Next run should try

1. Re-run the metrics. Expect `query_learnings` refusals to fall well below 32% in
   the first full week after the board restarts. If they do not, the gate was not
   the only source; look at the remaining `{query}`-only calls.
2. The `record_learning` refusals (`content`, `category`, missing `kind`/`scope`):
   find what prompt or skill steers agents to those names, the way this run found
   the gate.
3. Measure the duplicate rate of entries recorded after the similar-entries nudge,
   by embedding similarity, before building dedupe on write.
4. Link errors to entries: for each repeated error class, is there an entry that
   covers it, and was it retrieved before the error? None of the top classes has
   one today, which is right for tool-call shape mistakes; the fix for those is
   the prompt, not the store.
