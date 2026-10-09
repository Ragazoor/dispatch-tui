# Filing and root-cause detail

Detail for retro's "Fix it or file it": root causes outside this repo, which tags you may file, and how to file.

### When the root cause is the tool or environment

A local doc workaround treats the symptom, not the defect, so it is not sufficient closure. Write it first if it helps the next agent now, then deal with the defect itself:

- **This task's own repo owns it** (for example a dispatch defect found while working in dispatch): file it with `create_task`.
- **A different repo owns it** (the common case: you work in an unrelated repo and the defect is in dispatch's sandbox or Claude Code): do not file silently onto another repo's board. Flag it in the report, naming the root cause and the repo you think owns it, so the user can file it.

Either way, say plainly that the root cause is still open.

### What you may file

Only two tags:

- `bug` — a concrete defect with observable wrong behaviour.
- `chore` — a context improvement that passed the "worth changing" test but that you must not fix yourself.

**Never file a `feature`.** Speculative refactors and enhancement ideas are not retro findings. "This invariant is enforced by convention, so the same omission could recur" is a hypothetical, not a defect. If it matters, it will come back as a real bug.

### Before you file

- **Check for a duplicate** with `list_tasks`.
- **One task per finding, not per file.** The same wrong statement in three documents is one task that lists all three.
- **Write it so a cold agent can act.** The `title` names the change. The `description` references this task's ID and says what the next agent will hit if it stays unfixed.

`repo_path` and `epic_id` are required and not inherited. A finding usually belongs in the epic of the task that found it, so pass your own epic id, or `null` if it stands alone.

**Zero tasks is the normal outcome.** Filing several means you are recording nits. Drop the ones that cost this session nothing.
