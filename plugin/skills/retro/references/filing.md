# Filing and root-cause detail

Detail for retro Step 3: what to do when the root cause lies outside this repo, which tags you may file, and how to file.

### When the root cause is the tool or environment

A local doc workaround treats the symptom, not the defect — writing one is not
sufficient closure when Step 2's root-cause question came back yes. Apply the
workaround first, under the rules above, if it helps the next agent
immediately; that part doesn't change. Then also deal with the defect itself:

- **This task's own repo owns the root cause** — you're dispatched into the
  repo that actually has the bug (e.g. a dispatch/sandbox defect found while
  working in dispatch's own repo). File it with `create_task`, under the
  filing rules below, same as any other finding.
- **A different repo owns the root cause** — the common case: you're
  dispatched into an unrelated repo and the defect is in dispatch's sandbox or
  in Claude Code itself. Do not file silently onto a board outside this task's
  own repo — a dispatched agent should not decide unprompted to open a task on
  another repo's board. Flag it instead: name the root cause and which repo you
  believe owns it in Step 4's output, so the user sees it before the session
  closes and can file it themselves.

Either way, do not let Step 4's summary read as if the workaround were the
finding's full resolution — say plainly that the root cause is still open.

### What you may file

Only two tags:

- `bug` — a concrete defect with observable wrong behaviour.
- `chore` — a context improvement that passed Step 2 but that you must not fix
  yourself under the rules above.

**Never file a `feature`.** Speculative refactors and enhancement ideas are not
retro findings. "This invariant is enforced by convention, so the same omission
could recur" is a hypothetical, not a defect — and it is the shape of every
retro-filed task that later got archived unread. If it matters, it will come
back as a real bug with a real incident behind it.

### Before you file

- **Check for a duplicate.** Call `list_tasks` and look for an existing task
  covering the finding. Do not file a second one.
- **One task per finding, not per file.** The same wrong statement repeated
  across three documents is one task that lists all three, not three tasks.
- **Write it so a cold agent can act.** `title` names the specific change.
  `description` references this task's ID and says what the next agent will hit
  if it stays unfixed — e.g. "Found during task #123 — the user corrected me on
  test files using `Test`, not `Spec`; record this as the repo's convention."

`repo_path` and `epic_id` are both required, and neither is inherited — nothing
about the call guesses them for you. A retro finding almost always belongs in
the same epic as the task that found it, so pass the epic id from your own
prompt. Pass `null` only when the finding genuinely stands alone.

**Zero tasks is the normal outcome.** Most sessions should file none. Filing one
is unremarkable. Filing several means you are recording nits, not findings — go
back to Step 2 and drop the ones that cost this session nothing.
