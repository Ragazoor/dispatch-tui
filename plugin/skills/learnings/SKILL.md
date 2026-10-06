---
name: learnings
description: Manage the knowledge base lifecycle — query entries, rate the ones you acted on, record new ones, and delete those that turned out wrong. Use at wrap-up or whenever you want to contribute to the shared knowledge base.
---

# Knowledge Base

Use this skill to interact with the shared knowledge base — recording new entries and rating entries that were surfaced to you.

To *query* the knowledge base mid-task, call `query_learnings` directly, describing what you are working on in `query`. Do that when anything is unclear — before guessing or asking.

Reach for `tag_filter` only when you already know the tag you want. It is a soft score boost rather than a hard filter, so a tag you guessed at does not narrow the search — it demotes everything else until `limit` drops it, and you never learn what you missed.

**Announce at start:** "I'm using the learnings skill to interact with the knowledge base."

## Rating entries you acted on

When you act on a knowledge base entry that was surfaced to you (injected into your prompt or returned by `query_learnings`), give feedback right away:
```
rate_learning(learning_id=<id>, task_id=<your task id>, verdict="helped")
```

- `verdict="helped"` — the entry applied and was useful (upvotes it).
- `verdict="wrong"` — the entry misled you or is inaccurate (downvotes it; may go negative). Neither verdict changes the entry's status — there is no human review step. If it's clearly wrong rather than just unhelpful, delete it instead (see below).

Do this at the moment you act on it, not deferred to wrap-up. You can only rate entries that were surfaced to you this task.

**Rate `helped` when:** an entry saved you from a pitfall, matched a convention you applied, or guided a decision you made.

**Rate `wrong` when:** an entry was misleading or no longer accurate.

**Don't rate:** entries you read but didn't act on.

## Recording new entries

Before finishing a task, ask: *Did I discover anything non-obvious that a future agent would benefit from knowing?*

### Ask this first

Before writing prose about code, answer one question:

> **Could you write a failing check for a violation, from the source alone, without knowing what the author meant?**

- **Yes** → it is a lint rule, and prose is the wrong home *anywhere*. Write the lint, the test, or the gate script. Record nothing.
- **No** → it is judgment. Prose is right. Carry on below.
- **Neither** — the finding is that the code is *shaped* wrong rather than that a rule exists → it is a smell. The fix is a refactor, not a sentence.

A knowledge base entry earns its place by carrying something a machine cannot check. If a machine can check it, make the machine check it.

### Record if:

- The user expressed a **preference** explicitly that isn't already in CLAUDE.md
- You built a **landscape understanding** of a codebase area worth sharing
- You found a **convention** that applies broadly but isn't visible from reading the code
- A specific **workflow pattern** solved a cross-repo or cross-task problem elegantly
- This epic or project has a **procedural step** every agent working here should follow

### Before calling `record_learning`

Read `references/recording.md`: what not to record (above all, never name the function, type, macro, fixture, test or file that implements it), the kinds and scopes, and how to write the one-sentence summary. A `procedural` entry's `detail` must name where the agent stops and asks a human.

## Deleting stale entries

If a knowledge base entry is incorrect, outdated, or should be removed entirely, delete it:

```
delete_learning(learning_id=<id>)
```

This permanently removes the entry. Use `query_learnings` first to find the entry's ID if you only know its content.
