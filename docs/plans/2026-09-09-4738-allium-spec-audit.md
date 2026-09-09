# Allium spec audit — is the split still right?

Task #4738. Audited all 15 specs in `docs/specs/` at commit `4e4b4beb`
(rebased onto `main`, 2026-09-09).

**Verdict: keep the file split as it is. Do not add specs by domain.**
The corpus is not badly divided — three individual specs have each grown a
second domain *inside* them. Fix those three by extraction; leave the other
twelve alone.

---

## 1. What the corpus looks like

| spec | lines | rules | invariants¹ | surfaces | types² | prose³ |
|---|---:|---:|---:|---:|---:|---:|
| core | 2774 | 0 | 12 | 0 | 38 | 86% |
| feeds | 2659 | 15 | 2 | 0 | 4 | 90% |
| dispatch | 2434 | 7 | 1 | 8 | 0 | 88% |
| agent-tree | 2121 | 20 | 14 | 2 | 9 | 82% |
| epics | 1515 | 22 | 0 | 2 | 0 | 74% |
| tasks | 1476 | 21 | 0 | 0 | 0 | 77% |
| mcp-task-tools | 1212 | 14 | 0 | 0 | 0 | 79% |
| agent-health | 994 | 16 | 0 | 0 | 0 | 78% |
| repo-sync | 874 | 11 | 4 | 4 | 8 | 71% |
| pr-workflow | 818 | 11 | 0 | 0 | 0 | 77% |
| learnings | 720 | 8 | 1 | 0 | 0 | 88% |
| split-pane | 643 | 11 | 0 | 0 | 0 | 77% |
| observability | 588 | 11 | 2 | 0 | 6 | 75% |
| task-watchers | 322 | 8 | 1 | 0 | 1 | 70% |
| todo | 113 | 0 | 2 | 0 | 1 | 83% |

¹ top-level plus nested. ² entities + values + enums + contracts.
³ comment and blank lines as a share of the file.

Two structural facts say the split itself is sound:

- **The dependency graph is acyclic and shallow.** Every spec imports
  `core`; eight also cite `dispatch`, six cite `tasks`. Nothing else.
  The one edge out of `core` is a single prose cross-reference to
  `dispatch`.
- **Every spec carries a `Scope:`/`Excludes:` header that names its
  neighbours.** They agree with each other. `todo.allium` even records
  that it was extracted from `core.allium` — the precedent for the
  extractions below.

`allium check docs/specs/*.allium` reports **zero errors**. The 133
`unreachableTrigger` and 98 `field.unused` infos are all artefacts of
one-file-per-module: a trigger emitted in `tasks` and consumed in
`agent-health` looks unresolved to each in turn. **More files would make
that noise worse, not better** — a second argument against splitting by
domain for its own sake.

## 2. Where the specs actually hurt

The useful measure is not file length. It is **the longest run of lines
that declares no Allium construct** — prose the tooling cannot see, so
neither `allium weed` nor `allium propagate` has anything to check it
against.

| spec | longest construct-free run | starts at |
|---|---:|---|
| **core** | **1342** | `823` (Config) |
| **dispatch** | **1121** | `121` (`rule DispatchTask`) |
| feeds | 413 | `1299` (`rule RoleRoutedFeedSync`) |
| agent-tree | 407 | `1619` (`surface AgentTreeCompanionPane`) |
| every other spec | ≤ 261 | — |

Two outliers, both roughly 3× the next worst. They are findings 1 and 2.

### Finding 1 — `core.allium` holds a board rendering spec, untyped

`core.allium` declares its scope as "Shared domain model" and
"Excludes: Rules, invariants". It has 0 rules. But lines **1013–2167 —
1155 consecutive lines, 42% of the file — declare no Allium construct at
all.** They are prose, and they are prose about *behaviour*:

- **Board Columns** (1013–1167): which column a card lands in, the
  1-based navigation index space, and an abandoned 8-sub-column design
  recorded so it is not re-derived.
- **Column Identity and Focus** (1168–2306): the board's whole visual
  language. Per-column identity hues; the "hue answers *which column*,
  intensity answers *is it focused*" split and its two named exceptions;
  header-bar fill and label colour; the closed set of card border
  colours and their precedence; the neutral ground ramp; the cursor
  frame and bold title; the select-all checkbox.

Only the tail of that second section is typed: `PaletteToken`,
`ColumnChrome`, `EpicCardChrome` (2168–2241) and `BoardNeutralRamp`
(2242–2306), which carries 4 nested invariants. Everything above it —
about 1000 lines — is invisible to the toolchain.

This matters more here than it would in another repo. `CLAUDE.md` says
UI and interaction behaviour "is a first-class Allium surface, not a
prose note". A thousand lines of rendering rules sitting in the model
file, as comments, is exactly the thing that instruction forbids. It is
also where the corpus's only real *self-contradiction* lives: a file
whose header excludes rules and invariants contains twelve invariants
and a rendering specification.

There is a third domain in the same file: **FlattenedView** (2307–2774,
468 lines) — the two board rendering modes, which columns are exempt
from flattening, `ColumnSectionLayout`, `SectionRef`,
`CollapsedSections`.

**Coupling, if extracted:** almost none. Of the visual and layout types
only four are cited outside `core` at all, across five lines total —
`ColumnChrome` and `ColumnSectionLayout` in prose in `tasks.allium`,
`FlattenedView` in prose in `epics.allium`, and one real typed field,
`tasks.allium:23`'s `collapsed_sections: core/CollapsedSections`.
`PaletteToken`, `EpicCardChrome`, `BoardNeutralRamp` and `SectionRef`
are cited nowhere else.

### Finding 2 — `rule DispatchTask` is 1121 lines and four concerns

`dispatch.allium` is 2434 lines. **One rule is 1121 of them (46%).**
Its body:

| block | lines |
|---|---:|
| `when` / `requires` / `ensures` | ~30 |
| `@guarantee OneNameForStartingATask` | 16 |
| `@guarantee DesignStepMatchesTheReposSpecs` | 67 |
| `@guarantee NoLineRestatesTheDesignStep` | 44 |
| `@guarantee ThePromptNamesNoToolMerelyToSayItExists` | 30 |
| `@guarantee EveryTaskAgentLaunchesInAutoMode` | 23 |
| `@guarantee AgentCarriesItsOwnCallerIdentity` | 60 |
| `@guarantee PromptIsSeparatedFromTheLaunchFlags` | 24 |
| **`@guarantee AReviewRunbookCarriesOnlyTheBranchThatApplies`** | **294** |
| `@guarantee TheDepOnlyAllowlistAdmitsOnlyDeclarativeDependencyFiles` | 35 |
| **`@guidance`** | **501** |

Four separable concerns are interleaved through it:

1. **The atomic claim and its rollback** — `backlog -> running` as a
   conditional claim, who releases it, the worktree/branch/tmux
   teardown on failure, subagent and shell drain. ~90 lines.
2. **Worktree provisioning** — `git fetch origin <base>`, the fresh
   versus reused paths, start-point choice when local is
   ahead/behind/diverged, PR head-branch resolution via `gh pr view`,
   the fetch-then-rebase preamble. ~160 lines.
3. **Launch flags** — auto mode, `--plugin-dir`, caller identity
   headers, the deliberate absence of `--permission-mode`, the trust
   gate. ~130 lines.
4. **Prompt composition** — the two `build_prompt` variants, the design
   step, the unified trailing instruction, why the verify command is
   *not* in the prompt, the `pr_review` and `dependabot` runbooks, the
   dep-only allowlist. **~600 lines**, across six guarantees and most
   of the guidance block.

Concern 4 is the one that keeps growing — `docs/plans/4706-prompt-audit.md`
and `4727-harness-vs-prompt-verdicts.md` are both about it — and it is
the least coupled to the other three. It is a domain: *what text reaches
the agent*.

`allium propagate` derives obligations per rule, so a 1121-line rule
with nine guarantees is also the worst possible unit for test
generation.

**Two surfaces in this file are not about dispatch:**

- `surface TextInputField` (1397) is the shared single-line input buffer
  for task titles, epic titles, base branch, todo titles, repo-path
  query, quick-dispatch query and filter-preset names. Nothing about it
  is dispatch-specific; it is here because `RepoPathPicker` and
  `BaseBranchPicker` are.
- `surface StatusLineDecorator` (2006, 266 lines) specs the
  `dispatch statusline` CLI process — a separate binary invocation that
  runs several times a second in every session. Its neighbour
  `TokenBudgetIndicator` reads what it writes, which is the only reason
  it is here.

### Finding 3 — `feeds.allium` contradicts its own `Excludes:` line

The header says:

> Excludes: Specific feed scripts. dispatch ships reference shell-script
> templates under `scripts/`, but the GENERIC feed runtime […] never
> embeds upstream-specific knowledge — those feed scripts are user-owned
> executables.

Then the **Producing FeedItems** section (297–681) spends about 290
lines specifying those templates in fine detail: the four `gh search prs`
query passes and the fifth bot pass; `repos.conf`, `org.conf` and
`bots.conf` and what an empty `BOT_AUTHORS` falls back to; the
`group_by(.url)` jq idiom for merging signal arrays; which PRs carry a
`ci:` label and why an uncertain check status collapses to no label at
all; why there are two shipped producers of a `dependabot`-tagged task.

That is upstream-specific knowledge, in the spec, under a header that
says it is not there. The spec is not *wrong* about behaviour — the
templates really do work this way — the boundary statement is wrong.

`feeds.allium` also has the third-longest construct-free run, 413 lines
inside `rule RoleRoutedFeedSync`, but that one is a single coherent
obligation and reads fine. It is not a finding.

## 3. What is fine and should be left alone

- **`agent-tree.allium` (2121 lines) is not a problem.** It is the
  second-longest spec and the best-structured one: 20 rules, 14
  top-level invariants, 2 surfaces, 9 types, and only 13 outbound
  references. One cohesive feature, properly typed. Length here is
  subject matter, not sprawl. Its 407-line
  `surface AgentTreeCompanionPane` is a rendering surface doing its job.
- **Do not extend the split-by-interface axis.** `mcp-task-tools.allium`
  separates the task MCP tools from `tasks.allium`, while `epics.allium`
  and `learnings.allium` keep their `*ViaMcp` rules inline. That is the
  one inconsistency in how the corpus is divided. It is fine as it
  stands — the 12 task tools earn a file, the 6 epic and 4 learning
  tools do not — but it should not be regularised into
  `mcp-epic-tools.allium`. Splitting by interface *and* by domain gives
  every change two homes to choose between.
- **The 70–90% prose ratio is not a defect.** It is this repo's style:
  rationale, rejected alternatives, and the reason a guarantee exists.
  It is why the specs are useful to an agent. The problem in finding 1
  is not that prose exists, it is that 1000 lines of it carry behaviour
  no construct claims.

## 4. Proposal

Five extractions, in priority order. Each moves existing text; none
changes behaviour.

**A. `board-visuals.allium`** — `core.allium` 1168–2306. The board's
visual language: identity hue, the focus channel and its exceptions,
header bars, card borders, the neutral ramp, the cursor frame. Takes
`PaletteToken`, `ColumnChrome`, `EpicCardChrome`, `BoardNeutralRamp`.
~1140 lines. Highest value: it is the largest untyped behaviour block in
the corpus, and the one `CLAUDE.md` explicitly says should be a
first-class surface. Extraction is the moment to type it — the prose
should become surfaces and invariants, not move as-is.

**B. `dispatch-prompt.allium`** — the prompt-composition guarantees and
guidance out of `rule DispatchTask`. Leaves `DispatchTask` owning
claim → provision → launch, about 500 lines. Gives the prompt its own
rules so `allium propagate` can derive obligations per guarantee instead
of nine per rule. This is where the repo's recent churn is.

**C. `board-layout.allium`** — `core.allium` 1013–1167 (Board Columns)
plus 2307–2774 (FlattenedView, `ColumnSectionLayout`, `SectionRef`,
`CollapsedSections`). ~620 lines. Which column and section a card lands
in, and how the board flattens.

**D. `feeds.allium` boundary fix** — either move Producing FeedItems'
template detail (~290 lines) into `feed-scripts.allium`, or move it to
`docs/` and cite it. Then correct the `Excludes:` line. Extraction is
the better of the two: the material is normative for anyone editing
`scripts/`, and it is checked by nothing today.

**E. Two misfiled surfaces** — `TextInputField` to the board specs (it
is a shared TUI widget); `StatusLineDecorator` to
`observability.allium`, whose scope already covers the out-of-process
recording surfaces.

After A and C, `core.allium` is about 1000 lines of pure domain model —
external entities, enums, storage-boundary validation, entities, config
— which is what its header has claimed all along.

**Sequencing note.** A, C and D are text moves and can go in any order.
B changes rule boundaries, so it should land after the others, and its
own tests should be regenerated with `allium propagate` rather than
hand-split. Per learning #653, run `allium weed` over each changed area
afterwards: compressing prose into typed constructs silently drops the
cases you leave out, and no gate script catches it.

**What we are not doing.** No split of `agent-tree.allium`. No
`mcp-epic-tools.allium`. No per-domain fragmentation of the twelve
healthy specs. The answer to "should we split more into certain domains"
is no — the answer is that three specs each contain one domain that is
not theirs.

---

## 5. What this task actually did

C, D and E landed in this session. A and B were opened as tasks under
epic #314 — **#4758** (`board-visuals.allium`) and **#4759**
(`dispatch-prompt.allium`) — because both need the prose *retyped* as
surfaces and rules rather than moved, which is a session's work each.

| spec | before | after |
|---|---:|---:|
| core | 2774 | **2154** |
| feeds | 2659 | **2385** |
| dispatch | 2434 | **2074** |
| observability | 588 | **867** |
| `board-layout.allium` | — | **759** (new) |
| `feed-scripts.allium` | — | **304** (new) |

**C — `board-layout.allium`.** Took `core.allium`'s Board Columns
section (the navigation index space, review-section order, the Archive
edge column, row navigation, the `gg` chord) and its whole FlattenedView
section — which held far more than `FlattenedView`: `ColumnSectionLayout`,
collapsed sections (`SectionRef`, `CollapsedSections`), board vertical
layout, the only-active filter, the repo-filter overlay layout, the board
search filter, and cursor selection preservation. `TaskStatus` and
`ColumnSection` needed `core/` qualification; `tasks.allium` gained a
second `use` for its `collapsed_sections` field, the first spec in the
corpus to import anything but `core`.

**D — `feed-scripts.allium`.** The `Producing FeedItems` section turned
out to contain two runtime rules as well (`VerifyFeed`,
`SeedExampleFeedEpic`), which stayed. The 282 lines of template detail
moved; the generic "scripts are user-owned executables writing JSON to
stdout" contract stayed, with a pointer. `feeds.allium`'s `Excludes:`
line now says where the templates are specified instead of claiming they
are not specified.

Kept as `.allium` rather than becoming `docs/feed-scripts.md`: a
verbatim move has no reflow risk across 282 lines of hand-wrapped prose
with ASCII tables, and both doc gate scripts already scan
`docs/specs/*.allium` exactly as they scan `docs/*.md`. A prose-only
spec file does validate — `allium check` accepts one with zero
constructs.

**E — two misfiled surfaces.** `TextInputField` went to
`board-layout.allium`, which already specifies the input panel's
geometry and which modes draw a form in it; the buffer that panel edits
belongs beside it. `StatusLineDecorator` went to `observability.allium`
as a recorder. Its reader, `TokenBudgetIndicator`, stayed in
`dispatch.allium` — `observability.allium`'s `Excludes:` line rules out
reader UIs, and that line is right.

Verification: `allium check docs/specs/*.allium` zero errors; both doc
gate scripts pass; `cargo fmt --check`, `cargo clippy --all-targets --
-D warnings`, `check-no-test-sleep.sh` clean; `cargo test` 4748 passed,
0 failed, with no `tmux not available` skips. A line-level diff confirms
nothing was dropped: the only lines that left `docs/specs/` without
reappearing are the 19 that were deliberately edited — type
qualifications and repointed cross-references.

`allium weed` was not run. Nothing changed except which file a section
lives in and where its cross-references point, so there is no new
spec-code divergence for it to find. Tasks #4758 and #4759 both call for
it, because both *rewrite* prose into constructs.

### One finding the audit missed, worth recording

**A cross-file prose citation is checked by nothing.** Moving a section
leaves every `core.allium: "Board Columns"` doc comment in `src/`
pointing at the wrong file, and both gate scripts stay green:
`check-doc-paths.sh` only confirms the path exists, and
`check-doc-symbols.sh` resolves backticked identifiers and
`path.rs::symbol` forms, not quoted section names. This session
repointed 32 such citations by hand across 17 files in `src/` — 18 to
`board-layout.allium` and 14 to `observability.allium`. Nothing would
have caught a miss.

Two pre-existing broken references surfaced the same way and were fixed:
`feeds.allium` cited a `"Log-warning triage"` heading that no longer
exists anywhere (the section had been renamed to "events, not state"),
and `feeds.allium`'s `RoleRoutedFeedSync` cited "scripts/fetch-reviews.sh
above" — a section that had already drifted far enough up the file that
"above" was no help.

This is a real, unbudgeted cost of every extraction, and it argues once
more against splitting for its own sake. It is also lintable, so it was
opened as **#4760**: a gate script rejecting a `<spec>.allium: "Quoted
Heading"` citation, or a same-file `see "Heading" above`, whose heading
does not exist in the target file. The same-file half is the more
valuable one — a cross-file citation at least names its target, while a
bare "above" rots in silence.
