---
name: retro
description: Sub-step of the wrap-up skill, not a way to finish a task. The wrap-up skill invokes this automatically before its commit step — to complete, finish, wrap up, or end a task, always use the wrap-up skill, never this one. Use it directly only when the user explicitly runs /retro or asks for a session retrospective. Reflects on whether the user corrected or steered you this session, fixes small agent-context drift in place so the next agent does better, and opens follow-up tasks only for what it must not fix itself.
---

# Retro

A dispatched-task-scoped retrospective. This is dispatch's own version of a
session retro — narrower than a human's long-lived Claude Code session retro,
because a dispatched agent works in one isolated worktree and can only act on
what's in front of it: this task's session, this repo's docs, and the shared
knowledge base.

**Announce at start:** "I'm using the retro skill to run a session
retrospective."

## Step 1: Did the user correct or steer you this session?

List concrete moments where the user corrected your approach, confirmed a
design choice among options you offered, or stated a convention or
preference that wasn't written down anywhere — a moment, not a category.
This step is about **feedback the user actually gave**, not a context gap you
found on your own: a stale spec is the `weed` skill's job (spec-code
alignment), and a command that failed until you found the right invocation
is a tooling problem, not a correction from the user.

- A code review comment that changed something you'd already written —
  naming, structure, style, or architecture.
- A design choice the user made when you offered options, rather than
  picking one yourself.
- A convention or preference the user stated in conversation that isn't
  captured in `CLAUDE.md`, a spec, or the knowledge base.
- A correction to a plan or approach you proposed, before or during
  implementation.

This counts even if it didn't cost time in the moment: if a future agent in
this repo would hit the same choice and get it wrong, or have to ask again,
that's the same category of loss.

Also note what went well and is worth repeating.

Keep both grounded in what actually happened this session. **"Nothing notable"
is a real and common answer** — a session where the user gave no corrections
has no findings here, and inventing one is how retro turns into busywork.

## Step 2: Would a context change have prevented it?

For each moment in Step 1, ask: **is there a change to the agent-facing context
that would have prevented it?** These are the surfaces that reach the next agent:

| Surface | Reaches the next agent via |
|---|---|
| root `CLAUDE.md` | loaded into every dispatch prompt |
| `docs/specs/*.allium` | the source of truth agents consult |
| `docs/*.md` | linked from `CLAUDE.md`, read on demand |
| `plugin/skills/*/SKILL.md` | the skill the next agent invokes |
| the knowledge base | injected into the prompt (see the `learnings` skill) |
| the repo's verify command | shown in `get_task`'s response and echoed in the `wrap_up` response (not in the dispatch prompt) |

The test is **"would the next agent do better?"** — not "is this statement
inaccurate?" Every finding must trace back to a concrete moment from Step 1. If
nothing in this session was made harder by it, it is not a finding, however true
it is.

Concretely: "the user corrected me mid-review — test files use `Test`, not
`Spec`, and I'd already written one the wrong way" is a finding: recording it
as a convention saves the next agent the same correction. "A timing constant
isn't listed in the constants table" is not — nobody corrected me on it, and
filing it buys the next agent nothing.

For every finding that passes this test, also ask: **is the root cause in the
tool or environment running me — the sandbox, dispatch itself, or Claude Code —
rather than in this repo's own code or docs?** A "yes" doesn't change whether a
local doc workaround is worth writing; it changes what else is required. See
`references/filing.md`, named under Step 3.

## Step 3: Fix it here, or file it

Fix it yourself, in this session, when **all** of these hold:

- the surface is agent-facing prose — `CLAUDE.md`, a page under `docs/` other
  than `docs/specs/` (Allium specs have their own, narrower carve-out below),
  or a skill under `plugin/skills/`;
- your own work this session made it wrong, or this session proved it wrong;
- the correction is small and self-evident, needing no judgement about intended
  design.

One Allium case is also yours to fix: making a spec describe behaviour **this
session already implemented** is documentation catching up, not a design change.

You are the best-placed agent to make these fixes. You have the context, and
you are already in a worktree whose very next step is a commit — the calling
skill picks your edits up. Handing a one-line correction to a future agent who
must rebuild everything you already know is the expensive way to do nothing.

File a task with `create_task` instead — and do **not** edit — when:

- the change would make a spec describe behaviour the code does not have yet.
  That is a `spec → tests → code` loop and needs its own dispatch;
- the fix needs a judgement call about intended design;
- it is a pre-existing inaccuracy whose history you do not know;
- it is not small;
- this session is wrapping up with `action="done"` (see below).

Also file a `bug` for a concrete defect you noticed but could not fix in scope —
one with an observable wrong behaviour, not a suspicion.

**On the `done` path, file instead of fixing.** A fix you make here is carried by
wrap-up's commit step, which reaches `{base_branch}` on the rebase and PR paths
but never on `done` — that path runs no rebase and no push, so the worktree and
its commits are simply left behind. File the finding as a task instead, even
where the rules above would otherwise say "fix it here": a task survives the
worktree, an edit does not.

Wrap-up settles the action before invoking you, so you can always know which
path you are on — it is in the invocation, or in `wrap_up_mode` from `get_task`.
If you somehow reach this step without knowing, ask rather than assuming.

### Root cause outside this repo, and filing

When Step 2's root-cause question came back yes, or when you are about to file a task with `create_task`, read `references/filing.md` first. In short: apply the local workaround under the rules above, then deal with the defect itself — file it if this task's own repo owns it, otherwise flag it in Step 4's output rather than filing onto another repo's board. File only `bug` or `chore`, never `feature`, check `list_tasks` for a duplicate, and expect zero tasks to be the normal outcome.


## Step 4: Output

Print a structured summary:

```markdown
## Session Retrospective

**Went well:**
- {bullet}

**User corrected or steered you on:**
- {bullet}

**Context fixed in this session:** {what you corrected in place per Step 3, and where}{, or "none needed"}
**Follow-up tasks created:** #<id> (<tag>: <title>){, or "none needed"}
**Root-cause issues flagged (elsewhere):** {root cause + repo it belongs to}{, or "none"}
```

This is the last step of the retro skill itself — it is not the end of the session. Retro is almost always invoked as a sub-step of `wrap-up`, just before that skill's commit step. After printing this summary, immediately resume the calling skill's next instruction (wrap-up's commit step) in the same turn. Do not stop here.

## Relationship to other skills

- **`learnings`** still owns reusable pitfalls/conventions/preferences via
  `record_learning` — retro doesn't replace it. If Step 1 or Step 2 surfaces
  something learnings-shaped (a convention, a pitfall, a design or style
  choice the user corrected or confirmed), record it there right away —
  don't wait for the user to point out that you missed it.
- **`summarize`** still owns the behaviour-change recap for the user. Retro is
  about the session and the repo's docs, not what shipped.
- Retro is additive to both — run it in addition to, not instead of.
