# Allium Spec-First Loop

You are in a ralph loop that drives the Allium spec-first convergence loop
(Loop A) from a design document toward converged spec, tests, and code.

**Input design/spec document:** `{{DESIGN_DOC}}`
**Target Allium spec file:** `{{TARGET_SPEC}}`

## Each Iteration

### 1. Rebase from main

```bash
git fetch origin
```
Then:
```bash
git rebase origin/main
```

### 2. Advance the spec

- **Iteration 1:** Use the Agent tool with `subagent_type: "allium:tend"` to
  translate the design document (`{{DESIGN_DOC}}`) into the target spec
  (`{{TARGET_SPEC}}`) under `docs/specs/`. Prompt it with the behavior to
  capture and the target file.
- **Later iterations:** Only run `allium:tend` if the spec needs changes (e.g.
  a test/code conflict revealed a spec error).

After tending, read the target spec's `open questions` section. If it is
non-empty, STOP and resolve each item with the user via AskUserQuestion before
proceeding — do not guess.

### 3. Propagate tests

Invoke the `/propagate` skill to generate tests for behavior that changed this
iteration. Never hand-edit generated tests.

### 4. Red check

Run the newly generated tests and confirm they FAIL. A new test that already
passes signals redundancy or vacuity — flag it and investigate rather than
proceeding silently.

### 5. Implement

Write the minimum code needed to satisfy the spec and the failing tests. Follow
the spec's rules and parameters exactly — no magic numbers. Do NOT edit the
generated tests.

### 6. Verify

```bash
cargo test && ./scripts/check-doc-paths.sh
```

- Tests fail → fix the CODE, not the tests.
- If a test genuinely contradicts correct implementation, the spec is likely
  wrong: STOP and ask the user via AskUserQuestion. Only then `allium:tend` the
  spec and re-run `/propagate`.

### 7. Weed

Use the Agent tool with `subagent_type: "allium:weed"` in check mode to compare
`{{TARGET_SPEC}}` (and related specs in `docs/specs/`) against the
implementation. Reconcile divergence: update the spec for undocumented/spec
bugs; for code bugs that contradict a correct spec, ask the user before fixing.

### 8. Convergence check

Emit `<promise>SPEC CONVERGED</promise>` ONLY when ALL hold:
- `cargo test && ./scripts/check-doc-paths.sh` passes.
- `/weed` reports no spec-code divergence.
- The target spec's `open questions` section is empty.

Otherwise, exit so the loop re-enters for another iteration.

## Guardrails (non-negotiable)

- Confirm new tests fail before implementing (spec-first red check).
- Never weaken or hand-edit generated tests.
- Escalate ambiguity, open questions, and any code-vs-test conflict by PAUSING
  and asking the user via AskUserQuestion — never guess silently.
- Honor spec parameters; no magic numbers.
- Fix code, not the contract, when the spec is correct.
- Never commit files under `docs/plans/`.
- Never skip the rebase step.

## Exit conditions

- Convergence invariant satisfied (emit the promise), OR
- Iteration budget exhausted: 6 iterations, or 2 iterations with no measurable
  progress.
