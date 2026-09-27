# Allium Weed Loop — Per-Iteration Task

You are one iteration of a fresh-agent-per-run loop that reconciles spec/code drift by running
`allium:weed` and fixing what it finds. You have NO memory of any previous iteration — everything
you need to know about prior progress is in the git history and the current state of the repo.

**Base branch:** `{{BASE_BRANCH}}`
**Verify command:** `{{VERIFY_COMMAND}}`
**Iteration number:** `{{ITERATION_NUMBER}}`

## Your Task This Run

### 1. Rebase

```bash
git fetch origin {{BASE_BRANCH}}
git rebase origin/{{BASE_BRANCH}}
```

If the rebase produces conflicts inside `docs/specs/` files, resolve conservatively: preserve both
sides' content wherever the correct resolution isn't unambiguous from the diff alone, never
silently drop a clause, and call out exactly what you did in your final report.

### 2. Weed

Use the `Agent` tool with `subagent_type: "allium:weed"`, in **check** mode, comparing all of
`docs/specs/` — every `.allium` file in that directory — against the implementation in `src/`.
Point it at the directory, not a hardcoded file list: the spec set grows, and a fixed list
silently puts the rest of the tree outside this loop's reach.

Prompt the weed agent with:
> Weed the dispatch specs in docs/specs/ — every `.allium` file in that directory — against the
> implementation in src/. Run in check mode. Focus on finding undocumented behavior — code paths,
> state transitions, validation rules, or edge cases that exist in the implementation but are
> missing from the spec. Classify each finding as: spec bug (spec wrong, code correct), code bug
> (code wrong, spec correct), or undocumented behavior (code does something useful not in spec).
> Report all findings with file locations.

Do NOT pass a `model` override here — `allium:weed` pins its own strong model, which wins over
inheriting yours. An override "for consistency" would downgrade the loop's one judgement-heavy
step, which is exactly what dispatching you on a cheaper model was meant to leave alone.

Record whether this pass found **zero** findings — you need that for step 5 regardless of what
you fix below.

### 3. Process findings

For each finding from the weed agent:

- **Undocumented behavior** (code does something useful not in spec): use the `Agent` tool with
  `subagent_type: "allium:tend"` to add the behavior to the spec. Prompt it with the specific
  behavior to add and where it was found. No `model` override, same reasoning as step 2.
- **Spec bug** (spec is wrong, code is correct): same as undocumented behavior — dispatch
  `allium:tend` to update the spec to match the code.
- **Code bug** (code contradicts a correct spec): do NOT fix automatically. Use
  `AskUserQuestion` to describe the bug and ask whether it should be fixed. Only fix if the user
  confirms, then run `{{VERIFY_COMMAND}}` to confirm the fix doesn't regress anything else. If
  declined, leave it and say so in your final report — do not silently drop it.

If step 2 found zero findings, there is nothing to process here.

### 4. Commit

Stage and commit the changes you made this run (spec files, and any user-approved code fixes —
never `docs/plans/`, never this loop's own state file
`.claude/allium-weed-loop-state.local.md`). Required whenever you changed anything: the next
iteration is a fresh agent with no memory of this one, so it can only see your progress via git
history, and its own rebase step needs a clean tree to start from.

```
docs: align allium spec with implementation

- Added [specific behaviors] to [the spec files you changed]
- [Any code fixes if user-approved]
```

If nothing changed (step 2 found zero findings), there is nothing to commit — do not create an
empty commit.

### 5. Report

End your final message with exactly two labelled lines:

```
WEED_CLEAN: yes|no
SUMMARY: <one line: what changed this run, or "no changes" if step 2 found nothing>
```

`WEED_CLEAN: yes` means step 2's weed pass found **zero findings**, full stop — it does not mean
"I fixed everything I found." A run that found and fixed three undocumented behaviors is still
`WEED_CLEAN: no`, because the point of this label is confirming that a *subsequent* pass finds
nothing, not that this pass tidied up after itself (learning #750: a single clean pass can still
miss a real divergence — the driver requires two clean passes in a row before declaring the loop
converged).

## Important Rules

- Never skip the rebase step — you need the latest code each iteration.
- Never auto-fix code bugs — always ask the user first, in this same run.
- Keep spec changes minimal and precise — only add what the code actually does.
- Never stage or commit `.claude/allium-weed-loop-state.local.md`.
- Never end with a dirty tree when you changed anything — always commit before ending.
- Always end with the exact two-line `WEED_CLEAN:` / `SUMMARY:` block from step 5 — the driver
  parses it literally, and a report missing either label is treated as an error.
