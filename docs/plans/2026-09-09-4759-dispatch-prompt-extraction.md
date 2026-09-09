# Extract `dispatch-prompt.allium` from `rule DispatchTask`

Task #4759. Extraction B from the spec audit
(`docs/plans/2026-09-09-4738-allium-spec-audit.md`).

## What moves

`rule DispatchTask` in `docs/specs/dispatch.allium` is 1121 lines. Concern 4 of
the four the audit identified — *what text reaches the agent* — is about 676 of
them:

| block | lines in `dispatch.allium` |
|---|---|
| `@guarantee DesignStepMatchesTheReposSpecs` | 171–237 |
| `@guarantee NoLineRestatesTheDesignStep` | 238–281 |
| `@guarantee ThePromptNamesNoToolMerelyToSayItExists` | 282–311 |
| `@guarantee AReviewRunbookCarriesOnlyTheBranchThatApplies` | 419–712 |
| `@guarantee TheDepOnlyAllowlistAdmitsOnlyDeclarativeDependencyFiles` | 713–747 |
| `@guidance` "== Unified prompt skeleton ==" | 1038–1243 |

`DispatchTask` is left owning claim → provision → launch, about 470 lines.

## What does not move (settled with the user)

- **`OneNameForStartingATask`** — a board action-hint label, not prompt text.
- **`EveryTaskAgentLaunchesInAutoMode`**, **`AgentCarriesItsOwnCallerIdentity`**,
  **`PromptIsSeparatedFromTheLaunchFlags`** — all three constrain the launch
  command, not the text it carries.
- **The rebase and worktree preambles** — they are prompt lines, but *what they
  say* is decided by fresh-vs-reused, start-point selection and PR head-branch
  resolution. Splitting that one decision across two files would be worse than
  the citation. `dispatch-prompt.allium` names the two slots in the skeleton and
  cites `dispatch.allium` for what fills them.

## Rule shape

Five rules, each keyed on `AgentLaunched(task, mode)` — the trigger
`learnings.allium: AugmentDispatchPromptWithLearnings`, `agent-tree.allium` and
`repo-sync.allium` already key their own launch-time obligations on. This is
what buys the split: `allium plan` derives obligations per rule, so five rules
give five `rule_success` obligations and their failure arms instead of one.

```
rule ComposeAgentPrompt           -- skeleton, task block, trailing block,
                                  -- the deliberately absent verify command
rule NameTheDesignStep            -- spec-first | brainstorming, and the
                                  -- no-restatement rule that pairs with it
rule ClassifyDependencyBump       -- title/body -> BumpKind, seven ordered reads
rule RenderDependencyBumpRunbook  -- one branch per kind, dep-only allowlist
rule RenderPrReviewRunbook        -- find the PR, run the review command, wait
```

The classifier is its own rule because its inputs (title, description) and its
output (a `BumpKind`) are separable from the text that renders around them —
`src/dispatch/bump.rs` is already a module of its own.

## Retyping

The audit says both remaining extractions need the prose *retyped*, not moved.
The typed constructs this one adds:

- `enum BumpKind { patch | minor | major | non_major | digest | unknown }` —
  today only `src/dispatch/bump.rs` declares it; no spec does.
- `value Bump { kind, package?, from?, to? }` — what the classifier produces and
  the `Bump:` line renders, with an invariant that a multi-row digest table
  names no package.
- `enum DesignStep { spec_first | brainstorming }` — the branch
  `DesignStepMatchesTheReposSpecs` turns on.

## Steps

1. Write `docs/specs/dispatch-prompt.allium`: header, the three types, the five
   rules, guarantees redistributed onto the rule each constrains.
2. Cut the six blocks from `dispatch.allium`; leave a one-line pointer where
   each was so a reader of `DispatchTask` still finds the prompt spec.
3. Update `dispatch.allium`'s `Scope:`/`Excludes:` header to name the new spec.
4. Fix `tasks.allium: CreateQuickTask`'s stale "write a plan in docs/plans/ and
   attach it" sentence — the code follows the spec-first sequence — and point it
   at the new spec.
5. Repoint every `docs/specs/dispatch.allium` citation in `src/` that names a
   moved guarantee. There are ~10; nothing checks them, per the audit's own
   closing finding.
6. `allium plan docs/specs/dispatch-prompt.allium`, and confirm each derived
   obligation has a test. Add what is missing.
7. `allium check docs/specs/*.allium` (one invocation, zero errors), both doc
   gate scripts, `allium weed` over the changed area, then the repo's verify
   command.

## What this must not change

No behaviour. No prompt text. The extraction is a move plus a retype; every
`src/dispatch/prompts.rs` snapshot must be byte-identical afterwards.

---

## What actually landed

| | before | after |
|---|---:|---:|
| `dispatch.allium` | 2074 | **1417** |
| `rule DispatchTask` | 1121 | **460** |
| `dispatch.allium` longest construct-free run | 1121 | **297** |
| `dispatch-prompt.allium` | — | **900** (new) |
| `dispatch-prompt.allium` longest construct-free run | — | **138** |
| obligations from `allium plan` on the moved material | 3 | **18** |

`DispatchTask`'s remaining 297-line run is its provisioning `@guidance` — the
fetch-classification, start-point and rollback chain, which is one coherent
obligation and reads as one, the same call the audit made for
`feeds.allium`'s 413-line `RoleRoutedFeedSync`.

### Rules

Five, all keyed on `AgentLaunched(task, mode)` as planned:
`ComposeAgentPrompt`, `NameTheDesignStep`, `ClassifyDependencyBump`,
`RenderDependencyBumpRunbook`, `RenderPrReviewRunbook`. `allium check` reports
zero errors across the whole corpus in one invocation.

Two `allium.definition.unused` warnings are new: `Bump` and `DesignStep` are
referenced only from `contract PromptComposer`'s signatures, and the checker
does not count a signature as a reference. The corpus already carries nine of
that class. Referencing them from an invented field would have bought silence
with a construct nothing implements, which is worse.

### Retyping

Three constructs that were prose before:

- `enum BumpKind` — the six kinds, matching `src/dispatch/bump.rs`.
- `value Bump` — its four fields, plus two invariants the code stated only in a
  doc comment: `AnUnreadableBumpNamesNothing` (the unknown kind has nothing to
  append) and `ASourceVersionNeedsATarget` (`prompt_line` gates both versions on
  `to`, so a source without a target is a version the line drops).
- `contract PromptComposer` — `holds_allium_specs`, `design_step` and
  `classify_bump`, with two `@invariant`s: `NoReadFailureIsItsOwnAnswer` and
  `ClassificationNeverReachesTheNetwork`. `design_step`'s signature is
  `(has_plan, holds_specs) -> DesignStep?`, which is `Preceding::resolve`
  exactly — the null arm is the plan path, which names no design step.

### Tests

Four added, filling the four obligations nothing covered:

- `a_source_version_never_arrives_without_a_target` and
  `an_unreadable_bump_names_nothing` (proptest) — the two new `Bump` invariants.
- `classification_is_a_pure_function_of_the_title_and_the_body` (proptest) —
  `ClassificationNeverReachesTheNetwork` from the observable side, and the one
  place a whole `Bump` is compared rather than field by field.
- `build_prompt_without_pr_review_tag_omits_the_review_runbook` —
  `rule-failure.RenderPrReviewRunbook.1`. Every pre-existing pr-review test sets
  the tag, so a change that rendered the runbook unconditionally passed all of
  them.

The three properties share a generator, `a_bot_input()`, that produces titles
and bodies shaped like the two bots' real output. Free-form text classifies as
`Unknown` almost every time, which would have left the two version properties
vacuously true; `the_bot_input_generator_reaches_every_kind` pins that the
generator still reaches all six kinds. The source-version property was
mutation-checked — inverting the invariant in `classify` makes it fail.

Every other obligation was already covered, mostly by `src/dispatch/bump.rs`'s
45 classification tests and `src/dispatch/allium_specs.rs`'s nine.
`18 obligations, 18 covered, 0 uncovered`.

### Prose that changed rather than moved

A line-level diff confirms the only lines that left `dispatch.allium` without
reappearing are deliberate edits:

- 13 bare `--` paragraph separators at seams where a paragraph moved to another
  rule and the separator had nothing left to separate.
- `OneNameForStartingATask`'s "see the unified prompt skeleton below" now names
  `dispatch-prompt.allium: ComposeAgentPrompt`.
- The `== Unified prompt skeleton ==` heading became the guarantee
  `EveryPromptHasTheSameSkeleton`.
- "DispatchTask emits one of **two** variants of build_prompt:" said two and
  then listed three. The replacement says three.
- `(see "only when the task has no plan" above)` was already a phantom before
  this extraction — no such phrase exists anywhere in the corpus. It now names
  `dispatch.allium: DispatchResearchTask`, which is what it meant.
- The three-line "Neither runbook forbids /wrap-up any more" restatement was
  dropped. Its longer twin, which states the same rule with the same citation,
  moved to the review-runbook section note where it covers both rules; the two
  were adjacent after the split. The drop is recorded in the spec itself.

### Two pre-existing errors fixed in passing

- `learnings.allium` said the `wrap_up` tool is "defined in dispatch.allium". It
  is defined in `mcp-task-tools.allium: WrapUpViaMcp`.
- `tasks.allium: CreateQuickTask` said the quick-dispatch agent is instructed to
  "write a plan in docs/plans/ and attach it". It has not been for some time —
  the code names the same spec-first sequence every other no-plan prompt names,
  and writing a plan doc is the agent's judgement call on both paths.

### Citations repointed

15 in `src/` and 5 in other specs. Nothing checks a cross-file spec citation by
name — the finding #4738 recorded and opened as #4760 — so these were found by
grepping each moved guarantee's name and reading the surrounding comment.

### Verification

`allium check docs/specs/*.allium` (one invocation) zero errors; `cargo fmt
--check`, `cargo clippy --all-targets -- -D warnings`, `check-doc-paths.sh`,
`check-doc-symbols.sh`, `check-no-test-sleep.sh` all clean; the repo's verify
command green — 4914 passed, 0 failed, with no `tmux not available` skips.

### What `allium weed` found, and what changed because of it

Weed was run over `dispatch-prompt.allium` against `src/dispatch/`. It reported
12 divergences. One was wrong, one went to a new task, ten were fixed here.

**Dismissed.** "The contract is not attached to any surface's `contracts:`
clause." True, and true of every contract in this corpus — `repo-sync.allium`'s
`RepoSyncEngine` is free-standing too. No spec here uses `demands`/`fulfils`.
Not a divergence in this file.

**Opened as #4763** (behaviour change, out of this task's scope; recorded as the
`open question` at the foot of the new spec). `Preceding::resolve` answers the
spec question before the plan question, so a task with a plan attached in a
repo with no `docs/specs/` resolves to `NoSpecs` — and `trailing_block` gates
`{plan_not_a_stopping_point_instruction}` on `PlanWithSpecs` alone. That agent
is handed a plan, told no design step, and never told a plan is not a stopping
point: the exact regression the line was written for, in every spec-less repo.
Pre-existing; the extraction only made it visible by bringing the two
statements into one file.

**Fixed, and each one was a real defect the recut introduced or exposed:**

1. `contract PromptComposer.design_step` mis-modelled `Preceding::resolve`. It
   claimed a null arm for "a task that already carries a plan", but
   `resolve(true, false)` is `NoSpecs`, not the plan arm. `enum DesignStep`
   (two values) is now `enum Preceding` (three), matching the Rust enum value
   for value, and the operation is `preceding: (has_plan, holds_specs) ->
   Preceding` with its order-sensitivity stated. `NameTheDesignStep` now calls
   the resolver instead of branching on the boolean itself, so it cannot
   disagree with the trailing block about which branch was taken.
2. `ComposeAgentPrompt` had no mode guard, so it claimed a resume launch
   composes a prompt. `resume_agent` sends `--continue` and no positional text.
   Inside `rule DispatchTask` that scope came from the rule's own trigger;
   standing alone it has to be stated. Now `requires: mode in {standard,
   research, quick}`, the same shape and reason as
   `RefreshRepoSyncStateAfterDispatch`.
3. The module header listed `ResumeTask` as an emitter these rules answer to
   (it composes nothing) and omitted `RetryResume` entirely. Both corrected.
4. `NoLineRestatesTheDesignStep` said "the other two paths keep both lines".
   False for `no_specs`, which keeps `{tdd_instruction}` and drops
   `{allium_instruction}` — as `DesignStepMatchesTheReposSpecs` says fifty lines
   above. Pre-existing; the two guarantees only started contradicting each other
   visibly once they shared a rule. Corrected, with a note saying so.
5. `ClassifyDependencyBump` emitted `BumpClassified` that nothing received, and
   `RenderDependencyBumpRunbook` re-derived the classification from the same
   inputs — modelling one `classify` call as two, with nothing ordering them.
   The render rule now chains: `when: BumpClassified(task, bump)`.
6. `ReviewRunbookRendered(branch:)` carried a `BumpKind` in one rule and the
   `TaskTag` literal `pr_review` in the other — one parameter, two types. Split
   into `DependencyBumpRunbookRendered(task, kind)` and
   `PrReviewRunbookRendered(task)`.
7. The paragraph explaining `{plan_not_a_stopping_point_instruction}`'s gating
   landed on `NameTheDesignStep`, whose `requires: task.has_plan = false` means
   it never fires on the path that paragraph describes. Moved to
   `ComposeAgentPrompt`, beside the two plan variants.
8. The skeleton showed a blank line after `{preamble}` on every variant.
   `build_prompt` uses `IntroSpacing::SingleNewline`; only quick and research
   use `BlankLine`. Pre-existing, moved verbatim, now qualified.

Chaining (5) removed one obligation and the mode guard (2) added one, so the
count is still 18. `rule-failure.ComposeAgentPrompt.1` is covered by
`resume_agent_names_the_session_after_the_task`, which asserts the resume
command ends with `--continue` — a prompt appearing there would fail it. That
test now cites the rule.

**What weed confirmed clean**, so it is not re-litigated: the sentence-level
diff of the pre-split text against the two files shows no prose lost beyond the
one restatement the spec records; `value Bump`'s four fields and both invariants
hold on every path through `classify`; `enum BumpKind` matches the Rust enum
value for value; the seven-step ordered pass matches `classify`'s control flow
step for step, including which steps are Renovate-title-gated;
`holds_allium_specs` matches `repo_has_allium_specs` including its no-error-arm
rule; and the six decision bodies, the merge terminal's absence on routes that
forbid it, and the dep-only allowlist all match `dependabot_decision` and
`src/dispatch/prompts/dependabot.md`.

Re-verified after the fixes: `allium check docs/specs/*.allium` zero errors;
all three gate scripts clean; verify command green, 4914 passed, 0 failed, no
tmux skips.
