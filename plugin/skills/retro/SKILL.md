---
name: retro
description: Sub-step of the wrap-up skill, not a way to finish a task. The wrap-up skill invokes this automatically before its commit step — to complete, finish, wrap up, or end a task, always use the wrap-up skill, never this one. Use it directly only when the user explicitly runs /retro or asks for a session retrospective. Reflects on whether the user corrected or steered you this session, fixes small agent-context drift in place so the next agent does better, and opens follow-up tasks only for what it must not fix itself.
---

# Retro

**Announce at start:** "I'm using the retro skill to run a session
retrospective."

The goal: leave the next agent dispatched into this repo better off than you
were. You can only act on what is in front of you — this session, this repo's
agent-facing docs, and the shared knowledge base — so reflect on what the user
told you, and change the context that would have saved them the effort.

## What to look for

Moments where the user corrected or steered your approach, confirmed a design choice among
options you offered, or stated a convention that is written down nowhere: a
code review comment that changed something you had written, a design choice the
user made, a correction to your plan. These count even if it didn't cost time:
a future agent would hit the same choice and might get it wrong.

Feedback the user actually gave is the subject. A stale spec is the `weed`
skill's job, and a command that failed until you found the right invocation is
a tooling problem, not a correction.

Note what went well too. "Nothing notable" is a real and common answer;
inventing a finding is how retro turns into busywork.

## What is worth changing

For each moment, ask whether a change to agent-facing context would have
prevented it. These surfaces reach the next agent:

| Surface | Reaches the next agent via |
|---|---|
| root `CLAUDE.md` | loaded into every dispatch prompt |
| `docs/specs/*.allium` | the source of truth agents consult |
| `docs/*.md` | linked from `CLAUDE.md`, read on demand |
| `plugin/skills/*/SKILL.md` | the skill the next agent invokes |
| the knowledge base | injected into the prompt (see the `learnings` skill) |
| the repo's verify command | shown in `get_task`'s response and run by `wrap_up` itself (not in the dispatch prompt) |

The test: would the next agent do better? Every finding must trace to a
concrete moment from this session. A true but harmless inaccuracy is not one.

Also ask whether the root cause sits in the tool or environment running you —
the sandbox, dispatch itself, or Claude Code — rather than in this repo. If so,
read `references/filing.md` before you act: a local workaround alone does not
close it.

## Fix it or file it

Fix it yourself, now, when the surface is agent-facing prose (`CLAUDE.md`, a
`docs/` page other than a spec, or a skill), your work made it wrong or this
session proved it wrong, and the correction is small and self-evident. You have
the context and the next step is a commit, so a fix here is cheap. A spec is
yours to edit only to describe behaviour this session already implemented.

File a task with `create_task` instead when the fix needs a design judgement,
is not small, concerns a pre-existing inaccuracy whose history you do not know,
or would make a spec describe behaviour the code lacks — that is a
spec → tests → code loop with its own dispatch. Also file a `bug` for a
concrete defect you could not fix in scope.

On a `done` wrap-up always file instead of fixing: nothing rebases or pushes,
so an edit is stranded in the worktree and a task is not. Wrap-up settles the
action before it invokes you — it is in the invocation or in `wrap_up_mode`
from `get_task`. If you do not know it, ask.

`references/filing.md` has the tags, the duplicate check and the call details.
Zero tasks is the normal outcome.

## Report

Print a short summary:

```markdown
## Session Retrospective

**Went well:** {bullets}
**User corrected or steered you on:** {bullets}
**Context fixed in this session:** {what and where, or "none needed"}
**Follow-up tasks created:** #<id> (<tag>: <title>){, or "none needed"}
**Root-cause issues flagged (elsewhere):** {root cause + repo, or "none"}
```

This is not the end of the session. Retro runs as a sub-step of `wrap-up`, so
resume the calling skill's next instruction in the same turn. Do not stop here.

## Relationship to other skills

- **`learnings`** owns reusable pitfalls, conventions and preferences via
  `record_learning`. If you find one — a convention, a pitfall, a design or
  style choice the user corrected or confirmed — record it right away, and don't wait to
  be asked.
- **`summarize`** owns the recap of what shipped. Retro is about the session
  and the repo's docs.
