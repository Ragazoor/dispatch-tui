---
description: "Drive the Allium spec-first convergence loop (Loop A) from a spec/design document: tend the spec, propagate tests, implement to green, weed, repeat until converged"
allowed-tools: ["Read", "Write", "Bash", "Glob", "Grep"]
---

# Allium Spec-First Loop

This skill starts a ralph loop that drives the Allium spec-first convergence
loop (Loop A) from a design/spec document toward converged spec, tests, and
code. It is the spec-first sibling of `allium-weed-loop`.

## Instructions

1. **Resolve the input design/spec document**, in priority order:
   1. **Explicit arg** — if a path was passed to the skill, use it.
   2. **Recent context** — otherwise scan the recent conversation for a
      design/spec document created or referenced this session (e.g. a file just
      written under `docs/superpowers/specs/` or `docs/plans/`, or a path the
      user just named). If exactly one clear candidate exists, use it and
      **tell the user which document was picked** so they can catch a wrong
      guess.
   3. **Ask** — if args and context yield nothing, or multiple candidates are
      ambiguous, ask for the doc path via AskUserQuestion.

2. **Resolve the target Allium spec file** under `docs/specs/`: pick the most
   relevant existing `.allium` file, or propose a new filename derived from the
   feature. Confirm with the user via AskUserQuestion when ambiguous.

3. **Read the prompt file** at `.claude/skills/allium-loop/prompt.md`.

4. **Substitute** the resolved paths into the prompt body: replace
   `{{DESIGN_DOC}}` with the design-doc path and `{{TARGET_SPEC}}` with the
   target spec path.

5. **Create the ralph loop state file** directly at `.claude/ralph-loop.local.md`
   using the Write tool. Use this exact format, substituting the prompt content
   from step 4:

```markdown
---
active: true
iteration: 1
session_id: SESSION_ID
max_iterations: 6
completion_promise: "SPEC CONVERGED"
started_at: "TIMESTAMP"
---

[SUBSTITUTED PROMPT CONTENT FROM prompt.md]
```

Get the session ID by running `echo $CLAUDE_CODE_SESSION_ID` and the timestamp
with `date -u +%Y-%m-%dT%H:%M:%SZ`.

6. **Tell the user** the ralph loop is active (naming the design doc and target
   spec), then start working on the prompt immediately.
