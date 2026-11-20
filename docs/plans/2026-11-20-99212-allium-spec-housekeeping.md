# Allium spec housekeeping — third review and work plan

Task #99212 (recurring). Investigation only: no spec was edited. The fixes are the
subtasks of the epic "Allium spec housekeeping 3" (24 tasks) (see "Work packages").

Line numbers are as of commit `b03f5ba0` (main, 2026-11-18). They drift as packages
land: search for the cited symbol, not the line. The findings per cluster are in the
appendix files in `docs/plans/2026-11-20-99212-allium-spec-housekeeping/`; each task names its appendix.

## Baseline

CLI `allium` 3.6.1 (current: `cargo search allium-cli` gives 3.6.1). 43 specs, all
`-- allium: 3`, 52,529 lines. Six specs are new since the last review and had never
been reviewed: `allium-loop-skill`, `decompose-review-skill`, `wrap-up-skill`,
`learnings-skill`, `retro-skill`, `voice-skill` (epic #69698).

```bash
allium check docs/specs/*.allium     # always the whole set
allium analyse docs/specs/*.allium
./scripts/check-allium.sh            # the gate; baseline in scripts/allium-check.baseline
```

| Severity | Code | Now | At #98983 |
|---|---|---|---|
| error | any | 0 | 0 |
| warning | `definition.unused` | 12 (all baselined) | 12 |
| warning | `externalEntity.missingSourceHint` | 4 (baselined) | 4 |
| warning | `entity.unused` | 0 | 1 (`DataDirectory`) |
| info | `rule.unreachableTrigger` | 0 | 0 |
| info | `field.unused` | 86 (core 81, storage 1, allium-loop-skill 2, decompose-review-skill 2) | 70 |
| analyse | findings | 6 (learnings-skill 4, sync 2) | 3 |

The gate passes: 43 specs, 0 errors, 16 baselined warnings, 86 info.

What moved since #98983:
- `definition.unused`: `SessionLaunchFailure` and `StartupAbortReason` went (Q-S1);
  agent-tree's `FileDiff` and `AgentTreeNode` came in when #98983's package 6 dropped the
  render events. Both are real checker gaps with their comments (Appendix B1, Q-B1-1).
- `field.unused`: 15 new in core. 12 are the new `ArchivedTask` record (stored history;
  core's header note covers it; 10 read only in code, `labels` and `archived_by` read
  nowhere, Q-A1). `Task.url`, `left_feed`, `default_dispatch_mode` are read in other
  specs (cross-file reads are never credited). `DataDirectory.path` (storage) is new now
  that the entity is referenced. The four skill-spec fields are read nowhere (Appendix S).
- analyse: the four learnings-skill conflicts are three false positives and one real but
  unreachable overlap; merging rules clears them (Appendix S). The two sync conflicts are
  false positives with an accurate guidance comment (Appendix D).

The checker is quiet, so this review is again about best practice and the spec/code
drift it cannot see.

## Carried forward from the last epic

Epic #69699 is done: 24 of 24 tasks, plus follow-ups #99287, #99296, #99307. The
reviewers re-checked every item of its appendices against `b03f5ba0`. Still open, and
folded into this epic's packages:

- Appendix G A2 (partly): admin-surfaces' Dependencies say `WorkPackage` comes from core;
  it lives in work-packages.allium (package 21).
- Appendix G L1: history text at board-layout 175-182 and 758-764 (package 22).
- Appendix C: one feeds history remnant at 126-129, "preserving wire compatibility"
  (package 7).

Overlapping work outside this epic (do not duplicate):
- #99317 (running, "Iterate on the learnings system"): learnings.allium,
  learnings-skill.allium and wrap-up's rating step. Package 20 runs after it or
  coordinates with its agent first.
- #99234 (backlog): preference-review SKILL.md vs spec drift and the retro frontmatter.
  Its keep-awake part landed in `9b2f6088`.

## Decisions (apply them in every package)

Standing decisions 1-6 from docs/plans/2026-11-05-98956-allium-spec-housekeeping.md
("Decisions taken with the user") and N1-N2 from
docs/plans/2026-11-14-98983-allium-spec-housekeeping.md ("Decisions") still hold: per-spec
key surfaces, actors once in core, one start and one exit event in startup, present
tense only, checker gaps restructured or commented, the gate with a warning baseline,
the N1 split for events no rule listens to, one present-tense sentence for a one-time
sweep the code still runs. Read them there.

New in this review, decided with the user on 2026-11-18 and 2026-11-20:

- **N3. Prose `@invariant` inside an entity.** The language reference allows a prose
  `@invariant` only in a contract; inside an entity it wants `invariant Name { expr }`.
  Convert each one: an expression invariant where the property is expressible,
  otherwise move the prose to a top-level comment under Invariants (or into a contract
  if one fits). Keep the name, because other specs and code cite it. Each owning package
  converts its own spec: keybindings 7 (package 23), store-split 5 (9), host 3 (3),
  onboarding-guide 3 (13), storage 2 and core 1 (3), sync 1 (10), spacetime-memory-store
  1 (11). `@invariant` inside a contract stays. **N3 overrides the appendices**: Appendix D
  proposes top-level prose for every one and Appendix A calls them by-design; follow N3.
- **N4. Rules that share their trigger's name: rename them all, in one package that runs
  last.** The language reference does not forbid a shared name (it only requires that
  rules sharing a trigger agree on its parameters); the rename is this repo's convention.
  Rename the trigger, keep the rule (code and tests cite rule names), and update every
  emitter and `provides` line in every spec. 67 pairs in 11 specs at `b03f5ba0`: tasks 15,
  workspaces 15, epics 8, work-packages 6, dispatch 5, sync 5, board-layout 4,
  spacetime-seed 4, settings 3, feeds 1, host 1. **No other package renames a trigger**,
  even where its appendix proposes it; package 24 owns them all.

Per-spec behaviour questions are NOT decided here. Each package lists its questions; the
agent asks the user before changing that behaviour.

## Checker facts (additions to the #98956 and #98983 lists)

The earlier "Checker facts" still hold except where corrected here.

- **Re-confirmed: a qualified collection as a method receiver raises
  `reference.unknownName`**, for a real name and a made-up one alike
  (`core/Hosts.any(...)` and `core/NoSuchThings.any(...)`, scratch-tested 2026-11-20).
  Appendix G's "correction" (no diagnostic at all) is wrong; ignore it. The #98956 fact
  and its workarounds stand.
- **`actor … identified_by: X where …` is not validated.** An undeclared `X` passes
  silently (core's `McpCaller`), and a field read inside it is not credited (Appendix A).
- **Prose `@invariant` inside an entity is accepted silently** (see N3).
- **Temporal triggers fire once, when the condition becomes true** (language reference,
  "Temporal triggers"; not scratch-tested). A `requires` that is false at that moment blocks the rule for
  good; nothing re-arms it. Put such a guard into the trigger's condition (a derived
  "due" value) instead. Seen in agent-tree's seven refresh timers (Appendix B1, B1-T-1).
- **`this` is undefined inside a collection `where`** in a rule; the checker does not
  flag it (store-split, feeds, keybindings).
- **`not exists x.field`** is not valid: `exists` applies to entities. Use `= null`.
- **String concatenation (`"…" + reason`)** is outside the language's arithmetic table but
  used across the set. Accepted as a repo convention; do not report it.
- **Find-or-create in ensures** (`if not exists X: X.created(…)`) stays as the #98983
  appendix prescribed it; do not reverse it.

## Conventions every package follows

Everything under "Conventions every package follows" in the #98956 and #98983 plans
still applies. In particular, and newly frequent in this review:

- **Trigger renames belong to package 24 only (N4).** Other packages leave a rule that
  shares its trigger's name alone, even when their appendix lists the rename.
- **N1 comments**: every kept event no rule listens to gets "observable output; no rule
  listens". Most clusters found 15-25 without it.
- **Every task's footer**: run `./scripts/check-allium.sh` (the whole set) and expect no
  new warning and no new `field.unused`; ask the user before any behaviour change; run
  `allium:weed` on the package's specs at the end and fix what it finds (ask when it is a
  behaviour choice); before wrap-up, merge `main` into the branch (`git log --oneline
  HEAD..main`). Wrap up with rebase.
- **History passages found in passing** in a spec you are already editing: rewrite them
  in the present tense in the same task, as a separate commit.
- **Module edits** (packages 6, 10, 17, 21 when the answer needs one): rebuild
  `module.wasm` with `./scripts/build-managed-module.sh`, mirror the rule in
  `src/sync/memory_caller/`, cover it in `tests/memory_caller_conformance.rs`, bump
  `RULES_VERSION` and `BOARD_RULES_VERSION` together.

## Work packages

Epic: (created after this plan). Each package edits mostly its own files; the "Run
after" column lists the orderings that matter. Every acceptance check was run at
`b03f5ba0` and fails today; it passes once the package is done. The checks are in the
appendix, under "Work packages".

| # | Package | Files | Appendix (codes) | Run after | Questions for the user |
|---|---|---|---|---|---|
| 1 | wrap-up + retro skill specs (entry gaps) | wrap-up-skill, retro-skill | S: WP-S1 without S-L* and the wrap-up rating item (S-W1 RateBeforeClosing) | — | Q1 retro: flag vs record for a foreign-root-cause convention |
| 2 | allium-loop, decompose-review, voice, fix skill specs | allium-loop-skill, decompose-review-skill, voice-skill, fix-skill | S: WP-S2 + WP-S3 (S-A*, S-D*, S-V*, S-F1) | — | Q2: create no epic when every package is dropped? |
| 3 | core, storage, settings, host: ArchivedTask, citations, history, N3 | core, storage, settings, host, tasks (ArchivedTask writer lines only), store-split:2193 | A: P-A1 + P-A2 without renames (A-C-*, A-T-2–3, A-H-2) | — | Q-A1: ArchivedTask `labels`/`archived_by` keep, show or drop |
| 4 | agent-tree + agents: refresh timers that never re-fire, tidy | agent-tree, agents, baseline (if Q-B1-1 = b) | B1: B1-P1, P2, P3 | — | Q-B1-1: keep FileDiff/AgentTreeNode baselined, or type them through a contract |
| 5 | split-pane: in-flight swap task, tidy | split-pane, src/tui/types/state.rs (comments) | B2: package 1 (B2-S-*) | — | none |
| 6 | agent-health: any viewer can flip a task through hook writes | agent-health, tests/hooks.rs, src/keybindings/table.rs; module if Q-H1 = a | B2: package 2 (B2-H-*) | — | Q-H1: host check on every hook write, or only Stop and expiry |
| 7 | feeds + feed-scripts: one story for the kept feed task, forms, N1, history | feeds, feed-scripts | C: C-WP1 + C-WP2 (C-F-*, C-FS-*) | — | none |
| 8 | pr-workflow + schedules | pr-workflow, schedules | C: C-WP3 (C-P-*, C-S-*) | — | none |
| 9 | store-split: row kinds vs tool names, forms, N3 | store-split | D: D1 (D-s-*; D-s-4 is package 3's) | 3 | none (also settles B1-A-9) |
| 10 | sync: reconnect retries forever after an accepted connection; seen-mark gap; N3 | sync; src/sync/connection.rs if D-y-Q1 = a | D: D2 (D-y-*) | — | D-y-Q1: reset reconnect attempts only after the initial subscription |
| 11 | seed + snapshot + memory-store: normalisation trigger, import refusals | spacetime-seed, spacetime-snapshot, spacetime-memory-store | D: D3 (D-d-*, D-n-*); A: P-A3 (A-M-*) | — | none |
| 12 | startup + cli: managed-store contract, import seam, exit wiring | startup, cli | E1: WP-E1-1 + WP-E1-2 (E1-S-*, E1-C-*) | — | none |
| 13 | observability + onboarding-guide, N3 | observability, onboarding-guide, src/test_log.rs (comment) | E1: WP-E1-3 (E1-O-*, E1-G-*) | — | none |
| 14 | dispatch: text pass, entry for DispatchTask, RetryFresh mode | dispatch, core (config for literals), split-pane and mcp-task-tools emitters | E2: E2-A + E2-B without renames | 5 | Q-E2-D1 unbounded remove; Q-E2-D2 dev-profile guarantee; Q-E2-D3 `== heading ==` lines |
| 15 | dispatch-prompt | dispatch-prompt | E2: E2-C (E2-P-*) | — | none |
| 16 | tasks: literal trigger parameters, invalid forms, N1 | tasks | F1: WP-F1a (F1-T-*) | 3 | none |
| 17 | epics: drift, literal trigger parameters, open questions | epics (+ patch_epic if F1-E-18 = b) | F1: WP-F1b (F1-E-*) | — | F1-E-17 [R] on append-only epic; F1-E-18 reparenting a managed epic |
| 18 | mcp-task-tools | mcp-task-tools, src/store/queries/learnings.rs (comment) | F2: P1 (F2-M-*) | — | none |
| 19 | work-packages | work-packages | F2: P2 without renames (F2-W-*) | — | none |
| 20 | learnings spec, learnings-skill spec, wrap-up rating step | learnings, learnings-skill, wrap-up-skill (rating step only) | F2: P3 (F2-L-*); S: S-L*, S-W1 | #99317 (or coordinate with its agent), 1 | none |
| 21 | workspaces + admin-surfaces | workspaces, admin-surfaces | G: G1 without renames (G-W-*, G-A-*) | — | G-W-Q1 epic editor workspace move; G-W-Q2 creatorless private package; G-W-Q3 "PR poll state" |
| 22 | board-layout + board-visuals | board-layout, board-visuals | G: G2 without renames (G-L-*, G-V-*) | — | none |
| 23 | keybindings, N3 | keybindings | G: G3 (G-K-*) | — | none |
| 24 | Trigger renames across the set (N4) | tasks, workspaces, epics, work-packages, dispatch, sync, board-layout, spacetime-seed, settings, feeds, host, and every spec that emits or provides those triggers | N4; F1 WP-F1c lists tasks/epics emitter sites; G-W-1; D-y-1; D-d-1; F2-W; A-T-1, A-H-1 | every other package (run last) | none |

Acceptance check for package 24 (fails today, prints 67 lines; passes when it prints none):

```bash
for f in docs/specs/*.allium; do awk -v F=$(basename $f) '/^rule /{r=$2; sub(/\{.*/,"",r); next} /^[[:space:]]*when:/{ if (r!="") { t=$2; sub(/\(.*/,"",t); sub(/.*\//,"",t); if (t==r) print F": "r; r="" } }' $f; done | grep . && exit 1 || ./scripts/check-allium.sh
```

Acceptance checks of merged packages: run both of the appendix's checks. A check that
asserts a rename (for example `when: OverrideFeedOwner(` absent, or the rule/trigger awk
in D2, D3, F1, G1, G2) moves to package 24; drop that clause in the earlier package.

Not a task this round: the checker gaps above (`identified_by` not validated, prose
`@invariant` inside an entity accepted). Six allium-tools tasks in epic #69689 are still
in the backlog; add these there when that work resumes.

Shared remedies that cross packages, so do not duplicate them:

- tasks.allium's ArchivedTask writer lines (1950-1971: `display_agent_name`,
  `workspace_id: epic.workspace`) belong to package 3, not 16.
- store-split's literal identity-key set (2193) moves to `settings/config` in package 3;
  package 9 leaves it.
- store-split vs agents writing the same agent facts at the claim (B1-A-9) is settled in
  package 9.
- The DispatchTask/DispatchResearchTask entry fix touches split-pane (990, 1008, 1118) and
  mcp-task-tools (1490): package 14 only.
- learnings-skill and wrap-up's rating step overlap #99317: only package 20 waits on it.

## Appendices

One file per cluster, each the condensed report of one read-only review agent:

| Appendix | Specs |
|---|---|
| [S](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-S.md) | allium-loop-skill, decompose-review-skill, wrap-up-skill, learnings-skill, retro-skill, voice-skill (first review); grill-skill, fix-skill, preference-review |
| [A](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-A.md) | core, storage, settings, host, spacetime-memory-store |
| [B1](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-B1.md) | agent-tree, agents |
| [B2](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-B2.md) | split-pane, agent-health, keep-awake |
| [C](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-C.md) | feeds, feed-scripts, schedules, pr-workflow |
| [D](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-D.md) | store-split, sync, spacetime-seed, spacetime-snapshot |
| [E1](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-E1.md) | startup, cli, onboarding-guide, observability |
| [E2](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-E2.md) | dispatch, dispatch-prompt |
| [F1](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-F1.md) | tasks, epics |
| [F2](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-F2.md) | mcp-task-tools, work-packages, learnings |
| [G](docs/plans/2026-11-20-99212-allium-spec-housekeeping/appendix-G.md) | workspaces, admin-surfaces, board-layout, board-visuals, keybindings |

Codes: `mech` mechanical, behaviour-neutral; `entry` missing entry point; `drift` spec
and code disagree (code symbol checked); `bp` best practice; `by-design` expected,
leave it; `QUESTION` ask the user before changing it.
