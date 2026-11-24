# Learning system review, with epics that have orchestrators

Task #99395, epic #69708. Research only; no code or spec changed. Data read
from the live store (127.0.0.1:3000) on 2026-11-24. Retrieval counts undercount
the past: deleting a task deletes its retrieval rows.

## Answer in brief

The knowledge base helps, but it is noisy, and its epic scope leaks.

- Every task gets five entries, and agents rate two of every three of them
  "not applicable".
- Half of the epic-scoped entries can never reach a task again, because their
  epic is done or deleted.
- Nothing retires an entry that was once upvoted, even when the feature it
  describes is gone.

The orchestrator should own its epic's learnings at the epic's end. That
needs the learning tools to accept an orchestrator as a caller. No real
orchestrator has run yet (trial #99398 not started; the live store has no
orchestrator tables), so the orchestrator part is a design call, not a
measured one.

## What the store holds

| Measure | Value |
|---|---|
| Learnings | 916 (897 approved, 19 archived) |
| Approved by scope | repo 592, user 171, epic 134 |
| Approved in the dispatch repo | 380 |
| Retrieval rows | 12,588 (injection 3,971; search 8,224; old procedural 393) |
| Verdicts | 1,505 (helped 565, not_applicable 934, wrong 6) |
| Approved, score <= 0 | 454 (sweepable after 90 days untouched) |
| Approved, score > 0 | 443 (never swept, however old) |
| Approved, never retrieved | 174 |

The task's sample (62 of 63 repo-scoped) understated epic scope: it is 15% of
the store. The real epic-scope problem is below.

### Injection quality (since 2026-10-15)

| Measure | Value |
|---|---|
| Tasks with an injection | 467 |
| Tasks given exactly 5 (the cap) | 309; 4: 96; 3: 34 |
| Injected (task, entry) pairs | 2,174 |
| Pairs rated | 63% |
| Verdicts on those pairs | helped 468, not_applicable 909, wrong 1 |

The cap, not the 0.25 similarity floor, decides what is shown. About one
entry in three that an agent is shown and rates actually helps. Entries the
agent found through learning_search do much better (97 helped, 25 n/a, 5
wrong), because the agent chose the query.

The dispatch repo takes 1,269 of the 2,174 injected pairs. Its 380 entries
all share the same vocabulary, so generic dispatch entries match nearly any
dispatch task. Examples among the most injected:

| Entry | Injected | helped / n/a | Why it misses |
|---|---|---|---|
| #90345 allium trigger qualification | 38 | 10 / 26 | narrow Allium tip, matches any spec work |
| #8196 CLI read after store open | 31 | 1 / 17 | narrow; one helped keeps it unmuted |
| #569 README drifts | 34 | 5 / 16 | general advice, rarely acted on |
| #90216 UnseenTask signal | 32 | 3 / 16 | feed-specific |
| #565 bootstrapper workflow perms | 28 | 0 / 8 | spread over 8 Scala repos, one verdict each |

Verdicts by scope: epic 22 helped / 22 n/a (50%), repo 436 / 762 (36%),
user 107 / 150 (42%).

### The per-repo mute has two holes

- **Repo families.** The mute needs 3 n/a from tasks in *one* repo. #565 has
  8 n/a and 0 helped, but from 8 different Scala repos, so it never mutes.
- **One helped un-mutes for good.** #8196 has 17 n/a and 1 helped in the
  dispatch repo, so it stays in the pool.

### Epic scope strands entries

134 approved entries are epic-scoped. 46 point at epics that no longer exist
(21 at #325 alone) and 22 at epics that are done. So 68 of 134 can reach no
new task. Epic scope matches the exact epic id only: a task in a sub-epic does
not see its parent epic's entries. The one rule that moves epic entries
(ReScopeLearningsOnRepoGroupDelete) covers only repo-group sub-epics.

Some entries that belong to one epic were recorded at user scope and so reach
every repo: #734 (AnnotellDB runbooks, 1 helped / 11 n/a) and #12308 (pg_dump
runbook, 3 / 9).

### Nothing expires knowledge that went stale

The workspaces feature was removed (plan 2026-11-23-remove-workspaces). At
least seven approved entries still describe it: #90256 (score 3, injected 11
times), #90175 (1 wrong), #90304, #90257, #90411, #90473, #90435. #90256 can
never be swept because its score is above zero. A wrong verdict is the only
signal, and agents give it 6 times in 1,505.

### The score is not the verdicts

upvote_count runs ahead of the standing verdicts: #653 has 69 against 47
helped, #311 53 against 11, #734 12 against 1. The extra votes predate the
one-verdict-per-task table. Ranking boost and sweep immunity both read this
score, so old votes keep entries alive and ranked.

### The orchestrator today (code on main)

- Its prompt lists learnings ranked against the epic title and description,
  with no repo (list_learnings_for_epic). So it gets user and own-epic entries
  only, never repo entries: 66% of the store. The spec says "chosen as for a
  task of that epic" (GivenItsLearnings); code and spec disagree.
- It writes no retrieval rows and its prompt does not name learning_rate. All
  four learning tools take a task id, and an orchestrator has none. So it
  cannot rate, search or record, as the spec's undecided note already says.
- Its closing summary names notes "worth keeping as learnings", but nothing
  turns them into one.

Evidence from the hand orchestrator (zlata2, task #99396): it had a task, so
the tools worked; it used one entry (#90120). Its log
(docs/plans/2026-11-23-epic-69708-orchestration-log.md in that worktree) holds
four facts that outlive the epic: store rotation does not block and a
non-module branch can do it; a gate the user names in a task text is not
covered by a general delegation; an agent that says "done" while still editing
is normal, not stuck; plans guess wrong about which steps touch the module.
None of them is in the knowledge base. That is the promotion gap in practice.

## Recommendation

### On the open question (orchestrators.allium)

Yes, the orchestrator should curate its epic's learnings, with a narrow job,
done at one moment: when it writes the closing summary of a done epic.

1. Let the learning tools take an orchestrator caller (epic id in place of a
   task id): search, record, rate and delete.
2. At the summary, for each note the summary names as worth keeping, the
   orchestrator records a learning at **repo or user scope**, not epic scope.
   An epic-scoped entry stops being useful when the epic ends; that is the
   data above. This is promotion by a deliberate record call, so O17's line
   between notes and learnings stays.
3. In the same step it reviews the epic's own epic-scoped entries: re-scope
   what still holds (to the parent epic or the repo), delete the rest.
4. While the epic runs, it rates the entries its task agents report as
   helpful or wrong (task agents already ask it first), and gives the same
   selection rule as a task: user, epic and the repos of its tasks.

A store rule backs this up for epics with no orchestrator: when an epic is
deleted, its epic-scoped entries move to the parent epic, or to the repo of
its tasks at the root. That generalises the repo-group rule.

### Changes the data supports now

In order of value for task agents:

1. **Inject fewer, better entries.** Replace the fixed cap of 5 with a
   higher floor or a cut relative to the best match, and record the score on
   each retrieval row so the floor can be tuned from data.
2. **Close the mute's holes.** Count n/a across the entry's whole scope (all
   repos it covers), and mute on a ratio (say n/a >= 3 and n/a >= 4 x helped),
   not on "no helped ever".
3. **Let upvoted entries expire.** Sweep on last_upvoted_at: an entry not
   helped in 90 days is archived whatever its score. Rebase upvote_count on
   the verdict table once.
4. **Fix epic scope.** Match ancestor epics, and re-scope on any epic delete.
   Archive or re-scope the 68 stranded entries once.
5. **Orchestrator learnings** (the open question above), after trial #99398
   shows what an orchestrator actually wants to keep.
6. **Scope guidance.** The /learnings skill should steer one-project facts
   (a migration runbook) to epic scope, not user scope.

## Proposed tasks (not created; the operator decides)

In a new sub-epic, "Learning system: precision and lifecycle", under #69708
or under the Dispatch root:

| # | Task | Depends on |
|---|---|---|
| A | Spec + code: record cosine and rank on each retrieval row | — |
| B | Spec + code: relative injection cut instead of a fixed 5 | A (tune from a week of data) |
| C | Spec + module: mute across the scope's repos, ratio rule | — |
| D | Spec + module: sweep on last helped date; rebase upvote_count on verdicts | — |
| E | Spec + module: epic scope covers sub-epics; re-scope on epic delete; one-off repair of the 68 stranded entries | — |
| F | Spec: answer the open question as above (orchestrator caller for the learning tools, summary-time promotion and review) | trial #99398 |
| G | Code: build F | F |
| H | Fix the GivenItsLearnings gap: the orchestrator's selection includes its tasks' repos | — (or fold into G) |
| I | One-off clean-up: delete the entries about the removed workspaces feature | — |

I can be done by an agent today with learning_delete; it needs only the
operator's yes.
