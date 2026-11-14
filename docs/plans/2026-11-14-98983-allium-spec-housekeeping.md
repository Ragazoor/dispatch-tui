# Allium spec housekeeping — second review and work plan

Task #98983 (recurring). Investigation only: no spec was edited. The fixes are the
subtasks of the epic "Allium spec housekeeping 2" (see "Work packages").

Line numbers are as of commit `10b68974` (main, 2026-11-14). They drift as packages
land: search for the cited symbol, not the line.

## Baseline

CLI `allium` 3.6.1 (current: `cargo search allium-cli` gives 3.6.1). 37 specs, all
`-- allium: 3`, 47,129 lines. Three specs are new since the last review and had never
been reviewed: `grill-skill`, `fix-skill`, `preference-review`.

```bash
allium check docs/specs/*.allium     # always the whole set
allium analyse docs/specs/*.allium
./scripts/check-allium.sh            # the gate; baseline in scripts/allium-check.baseline
```

| Severity | Code | Now | At #98956 |
|---|---|---|---|
| error | any | 0 | 3 |
| warning | `definition.unused` | 12 (all baselined) | 20 |
| warning | `externalEntity.missingSourceHint` | 4 (baselined) | 6 |
| warning | `entity.unused` | 1 (baselined: `DataDirectory`) | 7 |
| warning | `reference.unknownName`, `surface.unusedBinding` | 0 | 19 |
| info | `rule.unreachableTrigger` | 0 | 300 |
| info | `field.unused` | 70 (69 core, 1 storage) | 99 |
| analyse | findings | 3 (grill-skill 1, sync 2) | 30 |

The gate (`scripts/check-allium.sh`) passes: 0 errors, 17 baselined warnings.

**Landed on `main` after the review read `10b68974`** (merged before this plan
shipped; the gate still passes: 39 specs, 0 errors, 17 baselined warnings, 75 info):
`allium-loop-skill.allium` and `decompose-review-skill.allium` are new (epic #69698) and
were NOT reviewed here. `feeds.allium` (+188 lines, commit 37f6b697: worked-on feed tasks
stay in done), `pr-workflow.allium` (+17), `store-split.allium` (12 lines), `core.allium` and
`mcp-task-tools.allium` (1 line each) changed: packages 7, 8, 9, 1 and 16 search by symbol.
`RULES_VERSION` is now 52 (Appendix D says 51). Every acceptance check was re-run on the
merged tree and still fails, as it should before its package runs.

field.unused triage (core): 61 are read in another spec (the checker never credits a
cross-file field read) or are stored data with no spec reader; 8 are read only in code;
`Agent.is_assigned` is read by agents.allium unqualified. Only `Task.pr_number` and
`Epic.children` need action (Appendix A, D2 and B2).

The three analyse findings are all false positives (complementary guards the analyser
cannot see). Remedies: grill-skill merges the two rules (package 3); sync gets a guidance
comment (package 10).

The checker is now quiet, so this review is mostly about best practice and spec/code drift
it cannot see: about 25 drift findings, most of them small.

## Carried forward from the last epic

Epic #69688 is done: 28 of 28 tasks. #98959–#98961 no longer show on the board, but
their work landed in `dcbe5f97` (core) and `339828c0` (storage, settings, host), and the
upstream issues #106–#111 exist. The reviewers re-checked every item in the old
appendices against the current specs. What is still open is listed under "Still open" in
each appendix below and folded into this epic's packages:

- core item 9 (SQLite-era prose) and item 4 (derived-value rule applied unevenly);
- storage item 1 (`DataDirectory` unused; it is not a checker gap after all);
- feeds B7 and pr-workflow B16 (partly), startup S1, dispatch D1/D2,
  dispatch-prompt history, observability citations;
- tasks item 2 (`BookmarkSet` binding);
- workspaces' orphan `*Allowed` guard events;
- the old cross-cluster C4: the "flip to review" block is still written four times.

Epic #69698 ("allium for skills") is adding specs for the skills that have none. This
review does not duplicate it: its tasks cover decompose-review, allium-loop, retro,
learnings and wrap-up, not the three skill specs reviewed here. Package 3 adds no new
skill spec.

## Decisions (apply them in every package)

Standing decisions 1–6 from docs/plans/2026-11-05-98956-allium-spec-housekeeping.md
("Decisions taken with the user") still hold: per-spec key surfaces, actors once in core,
one start and one exit event in startup, present tense only, checker gaps restructured or
commented, the gate with a warning baseline. Read them there.

New in this review, decided with the user on 2026-11-14:

- **N1. Events no rule listens to.** Split by kind:
  - Drop an event when a derived value already carries it (agent-tree's
    `AgentTreeNodeRendered`, `AgentTreeFileDiffRendered`).
  - Keep an observable outcome that a test can check (`AgentPromptComposed`,
    `*RunbookRendered`, `KeyIgnored`, `KeybindingsShown`, `AdminScreenOpened`,
    feeds' `ScriptFileWritten`). Give each a one-line comment: "observable output; no
    rule listens".
  - Guard events (`workspaces`' eight `*Allowed`) become contract invariants or
    `requires`. Each spec decides which (the workspaces package asks).
- **N2. One-time sweeps the code still runs** (feeds' poll-claim release below rules
  version 37, the reset of old rows with an unfit sub_status, core's "rules version 43"
  note). Keep one present-tense sentence while the code runs the sweep, for example "a
  store stamped below rules version 37 releases non-setter claims once on connect". Drop
  the narrative. Removing the sweep code is a separate decision, not part of this epic.

Per-spec behaviour questions are NOT decided here. Each package lists its questions; the
agent asks the user before changing that behaviour.

## Checker facts (additions to the #98956 list)

The #98956 "Checker facts" all still hold. New ones below were verified on scratch
copies unless marked "(language reference, not scratch-tested)".

- **Primitive types have no methods** (language reference line 354; the checker does not flag them). `name.trim()`, `.length`, `.prefix(n)`,
  `.contains("\n")`, `state.upper` and `.has_control_or_format_characters` are invalid
  and not flagged. Use black-box functions: `trimmed(name)`, `upper(state)`,
  `is_single_line(s)`. The same goes for `flat_map`, `.filter(...)`, `.sum(...)`,
  `.min_by(...)`, `.count(pred)`, `.first(pred)`, `X.created_or_updated(...)`, and
  undeclared entity "methods" such as `PollOwner.claimed_if_absent(...)`.
- **Only the last `transitions` block of an entity is honoured.** The language allows
  several (`transitions status {…}`, `transitions review {…}`). The CLI reads only the last
  one. Repro: in agent-tree.allium, move `transitions status` above `transitions
  task_resolution` and two "not in the declared transition graph" warnings appear. Not
  filed upstream yet (package 0).
- **No language form types a chained trigger's parameter.** `when: T(x: SomeEnum)` is an
  error ("'name: value' form … only valid in trigger emissions"). `provides:
  T(x: SomeEnum)` clears `definition.unused`, but the checker reads it as a named argument
  and does not validate it (`T(x: NoSuchThing)` passes). Do not use it. So an enum used
  only through literals of a trigger parameter (`StartupConfigOutcome`, `BoardExitCause`)
  is a real gap. It is not filed upstream (package 0). A contract operation that takes the
  enum as a parameter does clear it, but add one only when a rule really calls it.
- **`missingSourceHint` can be silenced** by any `use` in core.allium (e.g.
  `use "./sync.allium" as sync`). The silencing is meaningless, so keep the four baseline
  entries, but the baseline header's "no annotation silences it" is wrong (package 0).
- **`entity.unused DataDirectory` is not a gap.** A field `directory: storage/DataDirectory`
  on host's `HostFile`, passed at creation, clears it (package 2).
- `when: split/WrapUpRequested(task, host, pr)` *binds* `pr` as a name. It does not match
  a literal, so a rule written this way fires for every action. Bind the parameter and
  `requires: action = pr`. (Language reference, not scratch-tested: `when:` parameters
  are binding names in every example; none shows a literal.)
- `and` does not join ensures statements (`Emit(...) and x = y`); put one per line. `and
  not SomeTrigger(...)` in an ensures negates an emission and has no meaning. (Language
  reference, not scratch-tested.)

## Conventions every package follows

Everything under "Conventions every package follows" in the #98956 plan still applies: no
`old(x)` or `= unchanged`, render and cascade events become derived values or
`transitions_to` triggers, the board-clock contract, `allium:weed` at the end, fix latent
code bugs found in passing, and `record_learning` for new conventions. Also:

- **A rule never shares its trigger's name.** Rename the trigger (`…Pressed`,
  `…Requested`). Seen: agent-health `ToggleNotifications`, split-pane `ExitSplitMode`, six
  in agent-tree.
- **Key triggers are named, never literal.** `when: UserAction("s", task)` carries a key
  literal. Use a named trigger the spec's TUI surface provides (Decision 1). Seen in
  split-pane and keep-awake.
- **Qualify core helpers**: `core/local_host()`, never a bare `local_host()` or
  `this_host`.
- **Canonical section order**: Use, Given, External Entities, Value Types, Contracts,
  Enumerations, Entities and Variants, Config, Defaults, Rules, Invariants, Actor Declarations,
  Surfaces, Deferred, Open Questions. Surfaces come last; Invariants come before them.
  Several specs put Surfaces before Invariants or Rules.
- **Comments citing code** use `path::symbol`, and so do code comments citing a spec rule:
  when a spec rule is renamed, grep `src/` for the old name.
- **Every task's footer**: run `./scripts/check-allium.sh` (the whole set) and expect no new
  warning and no new `field.unused`; ask the user before any behaviour change; run
  `allium:weed` on the package's specs at the end and fix what it finds (ask when it is a
  behaviour choice); before wrap-up, merge `main` into the branch (`git log
  --oneline HEAD..main`). Wrap up with rebase.
- **History passages found in passing** in a spec you are already editing: rewrite them
  in the present tense in the same task, as a separate commit. Do not open a follow-up.

## Work packages

Epic: #69699. No package has to run first. Each one edits mostly its own files; the
"Run after" column lists the few orderings that matter. Where a package edits a file
owned by another package, the package says so. Merge `main` before wrap-up.

| # | Task | Package | Files | Findings (appendix) | Run after | Questions for the user |
|---|---|---|---|---|---|---|
| 0 | #99191 | Checker gaps upstream + baseline header | scripts/allium-check.baseline | Checker facts (transitions block; literal-only enums; typed `provides`; missingSourceHint wording) | — | none |
| 1 | #99192 | core: present tense, built-ins, drift | core | A core C1–C8 (C8's pr-workflow read is package 8), B1, D1, D2, the `waiting_on_background` move | — | none |
| 2 | #99193 | storage, settings, host | storage, settings, host, baseline (+ citers dispatch-prompt, preference-review) | A storage S1, settings T1, host H1–H2 | — | none |
| 3 | #99205 | Skill specs: grill, fix, preference-review | the three specs, plugin/skills/preference-review/SKILL.md | A2 all | package 2 | G3: add a follow-up-decisions rule or drop the sentence? |
| 4 | #99194 | agents + agent-health | agents, agent-health (+ core or store-split for A-n7's shared `chosen_mode`) | B agents A-n1–A-n8, agent-health H-n1–H-n9 | — | none |
| 5 | #99206 | split-pane + one owner for the deferred-Stop flip | split-pane, agent-health (X-1 only) | B split-pane S-n1–S-n12, X-1 | package 4 | none (S-n4 follows Decision 1) |
| 6 | #99195 | agent-tree | agent-tree | B agent-tree T-n1–T-n14 | — | T-n2: keep DiscardAgentTreePaneOnWindowGone or delete it? |
| 7 | #99196 | feeds, feed-scripts, schedules | the three specs | C feeds (all but the PollOwner seam), feed-scripts, schedules | — | none (the v37 sweep follows N2) |
| 8 | #99207 | pr-workflow + the poll-owner claim seam | pr-workflow, feeds (FeedTick, ManualFeedTrigger) | C pr-workflow all, feeds PollOwner seam drift, A core C8 backoff read | package 7 | ClosePrOnDone: route emission or guard? |
| 9 | #99197 | store-split: close-row rules match the module | store-split | D store-split all | — | none |
| 10 | #99198 | sync, spacetime-seed, spacetime-snapshot | the three specs | D sync, seed, snapshot | — | StorePublisher: keep in seed with a comment, or move to core? |
| 11 | #99199 | startup, cli, keep-awake, onboarding, observability + code comments | the five specs, src comment fixes | E startup S1–S3, cli, keep-awake, onboarding, observability, code-comment drift | — | Q-S1: StartupAbortReport contract? C3: how CliInvocation.exit is written |
| 12 | #99208 | dispatch + dispatch-prompt mechanical | dispatch, dispatch-prompt | E dispatch D1, D3–D6; dispatch-prompt P1–P3; B split-pane S-n5's dispatch side (AgentRelaunch provides `ResumeTask` without `go_to_task`) | package 2 | none |
| 13 | #99211 | Split ProvisionDispatch into a contract | dispatch (+ dispatch-prompt citations) | E dispatch D2 / Q-D1 | package 12 | Q-D1: split, or leave it? |
| 14 | #99200 | tasks | tasks | F tasks old 2, T1–T9 | — | none |
| 15 | #99201 | epics: RecalculateEpicStatus rule | epics, core (`Epic.children` only) | F epics E1–E8, A core B2 | — | none |
| 16 | #99209 | mcp-task-tools, learnings, work-packages | the three specs, cli (M6) | F mcp M1–M7, learnings L1–L5, work-packages W1–W4, B A-n9 | package 11 | M3: sub-epic vs repo routing; M2 one-line confirm |
| 17 | #99202 | workspaces: guards, union triggers, tidy | workspaces | G workspaces W1–W9, the `*Allowed` guards | — | W-Q1: guard form ((b) contract recommended) |
| 18 | #99210 | admin-surfaces + own-name commit entry points | admin-surfaces, workspaces (SetOwnNameFromBoard) | G admin A1–A4, workspaces W10 | package 17 | none |
| 19 | #99203 | board-layout + board-visuals | the two specs | G board-layout L1–L6, board-visuals V1–V2 | — | L-Q1: TextInputField as entity, or guarantees only? |
| 20 | #99204 | keybindings | keybindings, src/keybindings/table.rs (one description) | G keybindings K1–K7 | — | none |

Shared remedies that cross packages, so do not duplicate them:

- `ReviewCommandIsASetting` is deleted by package 2, which retargets its citers.
  Package 12 rewords the dispatch-prompt guarantee after it.
- `Epic.children`: package 15 decides (read it in the new rule, or delete it in core).
  Package 1 leaves it alone.
- `pr_poll_backoff_max`: package 8 makes pr-workflow read it. Package 1 only rewords the
  other unread config values.
- Literal key triggers (`UserAction("…")`): split-pane in package 5, keep-awake in
  package 11. Both follow Decision 1; agent-health's note calling the form invalid stays.
- mcp-task-tools 2228 `(agents.allium, A5)` -> A6: package 16, not package 4.
- `PaneResumeAnswer` stays baselined (package 6 only adds its comment).
- N1 applies in every package: each events-no-rule-listens-to finding (feeds
  ScriptFileWritten, dispatch-prompt composer outputs, admin render events, keybindings
  KeyIgnored/KeybindingsListed/KeybindingsShown) gets the one-line comment or is dropped.
- The old-sweep sentences (N2): core C3 in package 1, tasks T4 in package 14, feeds 3820
  in package 7.

## Appendices: findings per cluster

Each appendix is the condensed report of one read-only review agent. Codes:
`mech` = mechanical, behaviour-neutral; `entry` = missing entry point;
`drift` = spec and code disagree (code symbol checked); `bp` = best practice;
`by-design` = expected, leave it; `QUESTION` = ask the user before changing it.
Line numbers are as of `10b68974`; search for the cited symbol, not the line.

### Appendix A — core, storage, settings, host

core.allium. Still open from #98956: item 9 (SQLite-era prose, see D1); item 4
applied unevenly (see B1, D2). 13 of 16 resolved. The custom "Interval literals" and
"Storage Boundary Validation" sections are kept on purpose (by-design).

- C1 mech 579: "(task #98992, B1)" -> drop the task number.
- C2 mech 703-705: "so after the backfill every task has one" -> "every task has one".
- C3 mech 1041-1046: "the first connection after the module that added the rule (rules
  version 43) … are not migrated" is a migration log. DECIDED (N2): if the code still
  runs the sweep, keep one present-tense sentence; otherwise keep only the present rule.
- C4 mech 324: "(see learnings.allium: Priority Ordering)" -> the section is "Ranking".
- C5 bp 1011, 1014, 1022, 1023: `flat_map`, `.count(pred)`, `.first(pred)` are not
  built in. `subtree_tasks`/`subtree_epics` -> black box `descends_from(e, this)`;
  `deepest_epic_with` -> `let holders = sub_epics where subtree_tasks.any(t => t.status = status)`,
  then `holders.count = 1` and `holders.first`.
- C6 bp 1070: `verify_command.contains("\n")` -> black box `is_single_line(verify_command)`.
- C7 bp 1301 / 195: `config.inactivity_timeout` is read by nothing and duplicates
  agent-health `config.active_threshold` (`src/models/agent_events.rs::ACTIVE_THRESHOLD`)
  -> delete it; point SubStatus.stale's comment at agent-health's config.
- C8 bp 1302, 1317, 1380-1382: `status_message_expiry`, `pr_poll_backoff_max`,
  `fresh_threshold`, `aging_threshold`, `weeks_threshold` are read by no expression.
  -> pr-workflow's next_poll_at backoff reads `core/config.pr_poll_backoff_max`; the
  other four stay as documented code constants with "no rule reads it".
- B1 bp 25-28 vs 692, 864-865, 973-974: the header says a one-line single-field test is
  inlined, yet `has_plan`, `Epic.has_plan`, `is_feed`, `is_current`, `is_assigned` are
  such tests and each has a reader. -> reword the header: "inlined unless some spec
  reads it by name". Zero behaviour change.
- B2 bp 1006: `Epic.children` has no expression reader (epics:1877 is prose) -> read it
  in epics' recalculation rule (package 9) or delete it.
- D1 drift 1501-1514, 1535: spec says an unrecognised enum "must fail the read operation
  with an error"; code drops the undecodable row with a warning
  (`src/sync/decode_support.rs::drop_undecodable`). DB rationale is SQLite-era. -> "an
  undecodable row is dropped from the board with a warning, never defaulted"; keep the
  FeedRole/EpicOrigin/tmux_window soft-fail exceptions.
- D2 drift 736-739: `Task.pr_number` is null unless pr-typed in the spec;
  `src/models/url.rs::TaskUrl::pr_number` parses any url (Issue labels use it). No spec
  reads it. -> delete it and take it out of the header example.
- mech 855: derived `waiting_on_background` sits among stored fields -> under `-- Derived`.
- field.unused triage: 61 "read in another spec" (some only written or prose:
  Task/Learning/Workspace/Membership.created_at, Learning.kind/detail/last_upvoted_at/author,
  Agent.start_mode/started_at/ended_at/end_reason, SubagentEntry.started_at/agent,
  SavedRepoBranch.last_used, Setting.value, Epic.children, PollOwner.claimed_at — stored
  data with no spec reader, covered by the header note, no action); 8 read only in code;
  `Agent.is_assigned` is read by agents:516 (unqualified). No action beyond B2/D2.
- Baseline: the four `missingSourceHint` entries have their "Governed by" comments.
  The baseline header's "no annotation silences it" is inaccurate: any `use` in core
  (e.g. `use "./sync.allium" as sync`) clears them, meaninglessly. Keep the entries; fix
  the header wording.

storage.allium. 3 of 4 resolved.
- S1 bp 55-57 + baseline: `entity.unused DataDirectory` is NOT a checker gap. Verified
  on a scratch copy: `use "./storage.allium" as storage` in host plus
  `directory: storage/DataDirectory` on `HostFile` (host:79), passed in
  MintHostIdentity's `HostFile.created` (147-155), clears it (17 -> 16 warnings).
  -> do that; remove the baseline line and its "Gaps behind the entries" paragraph.

settings.allium. All 5 resolved.
- T1 bp 164-179: `rule ReviewCommandIsASetting` sits after Surfaces and listens on the
  same `SaveSetting(key, value)` as `SaveSetting` (71), so a review_command save emits
  `SettingSaved` twice. -> delete the rule; move its guidance into the
  `review_command()` comment (62-65); retarget citers settings:65, dispatch-prompt:511,
  preference-review:15.

host.allium. All 5 resolved.
- H1 mech 47: "Hosts is a registry now" -> drop "now".
- H2 bp 277-278: "a store that will not accept the label" -> "a host file that cannot be
  written" (the label lives in the host file, IdentityLivesInHostFile).

### Appendix A2 — skill specs: grill-skill, fix-skill, preference-review (first review)

grill-skill.allium.
- G1 by-design / analyse false positive 167-199: AskTheNextQuestion
  (`not decision.answered_by_code`) and ReadTheCodebaseInstead (`decision.answered_by_code`)
  have complementary guards; the analyser does not reason about Boolean complements.
  -> merge into one rule with an `if decision.answered_by_code:` branch (tested: the
  conflict disappears, 0 errors). Keep both guarantee names in guidance.
- G2 bp 141, 156-159: `GrillRequested(plan)` drops `plan`; GrillSession has no plan
  field, yet `plan_decisions(session)` needs it -> add `plan: String`.
- G3 bp 153: "New ones can turn up as answers arrive" has no rule; InterviewComplete
  fires when `open_decisions.count = 0`. -> add `FindFollowUpDecisions` on
  `Decision.status transitions_to resolved` (SKILL.md "Walk the design tree"), or drop it.
- G4 mech 46 / F6 mech fix-skill 35: header "Entities" -> "Entities and Variants".
- G5 mech 114: stored `already_in_a_spec` sits among derived values -> move up.
- G6 bp 129: `config.spec_glob` read by nothing -> read it or delete it.
- No drift against plugin/skills/grill/SKILL.md.

fix-skill.allium.
- F1 entry 199-210, 343-345: `choices: {dispatch, edit, cancel}` but only
  UserChoseDispatch/UserChoseCancel exist; SKILL.md step 4 "Apply an edit, then ask
  again" has no trigger. -> add `UserChoseEdit(request, title?, description?)` to the
  surface's provides, plus a rule that applies it and re-asks while staying `confirming`.
- F2 bp 180-181, 247-248, 250-251, 268-269: `and` joins an emission to an assignment in
  ensures -> one statement per line.
- F3 bp 48, 50, 52, 54: ids typed `Integer?` -> `core/Epic?`, `core/Task?`.
- F4 bp 86 / 241: `config.fix_tag = "fix"` duplicates `core/TaskTag.fix` -> use the enum.
- F5 bp 57-59, 309-312: `repo_touched: Boolean -- Never true` plus an invariant is
  vacuous; the guarantee NeverEditsTheOwningRepo already states it -> drop both.
- F7 drift (minor) 113: SKILL.md announces "I'm using the fix skill …"; FixAnnounced
  carries no sentence -> add it to StartFix's guidance.

preference-review.allium.
- P1 bp 740-750, 127: custom "Settled questions (operator, 2026-11-14)" section and a
  dated cross-reference -> move each point into its home (drops_by_taste comment,
  ReviewerCountFollowsDiffSize, LoadPreferences guidance); delete the section and date.
- P2 bp 275-278, 679-680: "a hand test listed it with `find` …" anecdotes -> keep the
  present-tense reason only.
- P3 bp 24-25 vs 194: "a fixed default under the dispatch data directory" but the
  default is the literal `~/.local/share/dispatch/review-context` (ignores `--data-dir`)
  -> say so plainly.
- P4 drift (SKILL side) 124-127, 746-748: the tie rule (same specificity goes to Do not
  raise) is missing from plugin/skills/preference-review/SKILL.md step 5 -> add a sentence.

### Appendix B — agent-tree, split-pane, agents, agent-health

Coverage: agents, agent-health, split-pane read in full. agent-tree 1420-1590, 2052-2160
and the surfaces (2930-3880) were grep-scanned only; read them in package 6.

All config defaults match their code constants; analyse is clean here. The AgentTreePane
transition graph is complete (shown<->hidden). A second reviewer's "hidden transition
warnings" come from a checker bug: only the LAST `transitions` block of an entity is
honoured (repro: in agent-tree.allium move `transitions status` above
`transitions task_resolution`). Not filed upstream yet: package 0.

Old C4 (agent-end cleanup x6): agent end and non-draining clear now have one owner each.
Still duplicated: the "flip to review" block (status = review, sub_status, both stamps
null, stop_pending = false) at agent-health 412-417, 523-527, 611-615 and split-pane
216-220; DetachTmux (split-pane 208-211) writes the subagent clear inline. That is X-1.

agents.allium. G1-G8 resolved.
- A-n1 mech 77-78: "the ledger keeps that name either way" contradicts A4 (46) -> "it is
  ended, not deleted, so its name stays held until the task goes".
- A-n2 mech 42-49, 93: "Changed 2026-11-09 (task #99056): names used to be…" -> keep A4
  and A17 as present-tense rules; drop narrative and number.
- A-n3 mech 438: "(A18, task #99056)" -> "(A18)".
- A-n4 by-design 457-459: "#99056" and "epic #314" are examples of the rule. Leave them.
- A-n5 mech 64-65: A10 describes process, not behaviour -> one-line id entry or drop
  (check who cites A10).
- A-n6 mech 162-163: "Reported unreachable until store-split … provides DispatchRequested"
  is stale (store-split 785 provides it) -> delete.
- A-n7 bp 171-176: start_mode re-derives store-split's `chosen_mode` (842-844) -> define
  once and reference.
- A-n8 bp: Rules, Surfaces, Invariants -> Invariants before Surfaces.
- A-n9 mcp-task-tools 2228: "(agents.allium, A5)" should cite A6.

agent-health.allium. H1-H7 resolved (C4 part now X-1).
- H-n1 drift 39: ClassifyAgentActivity's `for` lacks `is_locally_owned`; code filters on
  it (`src/tui/update/agent.rs::tick_sub_status`) and the spec's prose (171-175) agrees -> add it.
- H-n2 mech 1249: "SubagentCountIsDerivedNotIncremented" names nothing -> "core.allium:
  LiveSubagentCountMatchesEntries".
- H-n3 drift 989: "implemented by cmd_hook" (no such fn) -> `src/main.rs` `Commands::Hook`.
- H-n4 drift 1039: "(src/main.rs, `hooks::run_peer_message`)" -> `src/hooks/mod.rs::run_peer_message`.
- H-n5 bp 1241-1249, 1281-1283: HooksWriteThroughTheBoard argues from the SQLite era
  ("two processes contending for one database file") -> restate in store terms.
- H-n6 mech 1064-1067: comment lines after `@guidance` at rule indentation -> indent.
- H-n7 mech 1184, 1192: "Two further commands … A second" (it is a third) -> reword.
- H-n8 mech 124-126: "An earlier design did that, and the count inflated forever" ->
  present-tense reason.
- H-n9 bp 308-309: rule ToggleNotifications listens on a trigger of the same name ->
  rename the trigger (e.g. NotificationsKeyPressed) here and in NotificationKeys (1396).
- H-n10 by-design 459-462, 562, 819-821, 1000-1001: "predates the field" describes
  behaviour toward older clients. Leave it.

split-pane.allium. S1-S9 resolved.
- S-n1 drift 1661: `core/Task{tmux_window: window}`; Task has no tmux_window (Agent
  does) -> `let agent = core/Agent{tmux_window: window}`, `requires: exists agent`, `agent.task`.
- S-n2 drift 1261: "@dispatch_home_dir" pane option does not exist; the directory is
  `#{pane_start_path}` (`src/tmux.rs::panes_with_home_in_window_of`) -> fix wording.
- S-n3 bp 954, 961, 1102, 1133: `tmux_focus = …` assigns an undeclared variable -> emit
  an outcome such as `TmuxFocusMoved(target)`.
- S-n4 DECIDED (Decision 1; made a convention in this review) 277, 327, 944, 1131 (and keep-awake 80, 111, 120, 176):
  `when: UserAction("s", task)` puts a literal key in a trigger; agent-health 1005-1008
  calls this form invalid. -> named key triggers provided by SplitPaneBoard
  (SplitToggleKeyPressed(task), ActivateKeyPressed(task), JumpBackKeyPressed()).
- S-n5 bp 1093: listens on dispatch's `ResumeTask` unqualified (reachable by name
  collision only) -> `use dispatch`, `dispatch/ResumeTask(...)`. dispatch's AgentRelaunch
  (dispatch 1538) provides `ResumeTask(task, message?)` without `go_to_task`.
- S-n6 mech 868: ensures repeats requires -> delete.
- S-n7 bp 395-396: rule ExitSplitMode listens on ExitSplitMode() -> rename the trigger.
- S-n8 mech 849: "the agent is ended" -> under A7 a detach pauses the agent.
- S-n9 mech 1034-1039: keybinding paragraph inside a numbered list -> move after it.
- S-n10 mech 1192-1196: PinTaskInSplitPane guidance describes SwapSplitPane's case -> point there.
- S-n11 mech 1210: "(docs/specs/agent-tree.allium: …" -> "(agent-tree.allium: …".
- S-n12 mech 1277-1278: "as before" -> drop.

agent-tree.allium. A1-A10 resolved.
- T-n1 mech 300: PaneResumeAnswer has no comment of its own saying why its warning is
  expected (283-285 covers PaneView) -> add one. The reviewer proposed typing the surface
  trigger `answer: PaneResumeAnswer` (3879) to clear the warning; do NOT: Appendix E shows
  a typed `provides` entry is read as an unvalidated named value. Keep it baselined.
- T-n2 QUESTION/drift 1134-1138: "DetectCrashedAgent deliberately leaves … tmux_window
  set"; a crash clears it (`src/tui/update/agent.rs::handle_agent_crashed`). Is
  DiscardAgentTreePaneOnWindowGone still needed? (a) keep as a safety net with corrected
  prose; (b) delete it.
- T-n3 drift 1023: "`join_pane` drains the companion panes" -> it is
  `src/dispatch/split_panes.rs::join_task_window_into_pane`.
- T-n4 mech 1649: "one field with two writers" -> "several writers" (NoticeSource has five).
- T-n5 mech 122-131: "The agents section added two more writers …" -> list them.
- T-n6 mech 1207-1209, T-n7 mech 3581, T-n8 mech 3202 ("task #99033"): history -> delete or rephrase.
- T-n9 bp 1360, 2037: AgentTreeNodeRendered / AgentTreeFileDiffRendered render events no
  one consumes; derived `rendered_nodes`/`rendered_diffs` already exist -> drop (standing convention).
- T-n10 bp 1947-1957, 1972: EditorPaneOutlivesItsTree's ensures restate requires; 1972
  is a disjunctive postcondition -> `if pane.diff_pane_content != editor: pane.diff_pane = null`;
  delete the rule, keep its prose as guidance.
- T-n11 mech 2392-2394, 2517-2519: self-assignments in `else` branches -> leave unwritten.
- T-n12 bp 1885: `ensures: pane.diff_pane != null` is an assertion -> conditional create.
- T-n13 bp 1672, 1698, 2308, 2505, 2533, 2557: rules share their trigger's name -> rename
  triggers to `…Pressed`.
- T-n14 mech: Enumerations before Value Types/Contracts -> canonical order.
- T-n15 by-design 291-296, 1113, 3239-3245: "a board that predates …" compatibility. Leave.
- T-n16 by-design 658-660: the transitions-block checker gap. Keep the comment; file upstream.

X-1 bp: one owner for the deferred-Stop flip (agent-health trigger such as
`DeferredStopApplied(task)`; split-pane raises `agent_health/SubagentsVoided`). Structure
only, not behaviour.

### Appendix C — feeds, feed-scripts, schedules, pr-workflow

Config values, shipped-script lists, route_signals precedence and schedules all match code.
`FeedCommandRun` baseline entry: comment present, real gap (#106). Keep.

feeds.allium. 27 of 28 resolved; B7 (history) partly open.
- mech 1473: `if command != old(epic.feed_command)` -> `let changed = …` before ensures.
- bp 2004, 2537, 2554: `TaskTeardown(task)` emitted as a trigger, but tasks.allium
  declares TaskTeardown a contract -> delete the emissions; comment "performs tasks.allium's
  TaskTeardown" (teardown is already carried by FeedTaskRemovedAsStale + store-split).
- bp 1413, 1418, 1432, 1437: undeclared dot-methods `caller().sees(...)`,
  `core/PollOwner.claimed_if_absent(...)`, `core/PollOwner.reassigned(...)` -> black box
  `sees(caller(), scope, scope_id)`; `if not exists core/PollOwner{…}: core/PollOwner.created(...)`;
  assign `core/PollOwner{…}.host = host`.
- drift 1250-1257 (FeedTick), 3600-3606 (ManualFeedTrigger), pr-workflow 188: the board
  creates `core/PollOwner.created(...)` directly, bypassing ClaimPollOwner (1403, which
  holds the setter and sight checks). Code calls the claim_poll_owner reducer. -> emit
  `PollOwnerClaimed(...)` instead.
- bp 2726, 2780: `route(signals)` unbound (loop var is a Task; Task has no signals);
  2727 `visible_to(user)` unbound -> iterate items; `route(item.signals)`; `m.user`.
- bp 2787-2789 (commit 10b68974): "A PR already on the board when this rule shipped keeps
  its model" is history; "a PR retired and later re-inserted" contradicts 2741 and 3447
  -> "a PR removed as stale and later emitted again is a new insert".
- drift 2804: cites `feed_ingest.rs::ingest_into` for the model write; the loop is
  `spacetime/module/src/feed_ingest.rs::ingest_role_routed` -> re-cite.
- mech 1692: empty heading "-- == Upsert semantics ==" -> merge or drop.
- mech 1395: `surface PollOwnerReducers` in Rules -> move to Surfaces.
- bp 1244-1245: an `if …:` whose body is only a comment -> invert, as ManualFeedTrigger 3599 does.
- bp history: 22-24 "legacy Review Board … remain removed"; 2603-2604 "as when teardown
  was first added"; 3346-3350 "first imposed … NOW MADE"; 3384-3391 "proposal is
  retired"; 3550 "(the reviews_parent parent-flat bug)"; 2169 "AS IMPLEMENTED".
- DECIDED (N2) 3820-3832: "CLAIMS FROM BEFORE THE RULE (a one-time sweep) … rules version
  37" is a migration log, but the code still runs it
  (`spacetime/module/src/config.rs::release_claims_of_non_setter_hosts`). (a) keep one
  present-tense sentence: "a store stamped below rules version 37 releases non-setter
  claims once on connect". Drop the narrative.
- by-design 956-983: ScriptFileWritten etc. are filesystem effects governed by contract
  ShippedFeedScriptFiles. Leave them.

feed-scripts.allium. 4 of 4 resolved.
- bp 185, 493: "the body cut to 500 characters" literal -> `config.pr_description_chars = 500`.
- mech 579: header "Configuration" -> "Config".

schedules.allium. 7 of 7 resolved.
- mech 108, 118, 161: headers "Configuration", "Setting and clearing", "Firing" ->
  "Config" and one "Rules" header with `-- ==` sub-headings.

pr-workflow.allium. 10 of 11 resolved; B16 partly open: bare `pr_closed`, `conflict`,
`pr_unreachable` (486, 488, 518) beside `core/SubStatus.…` (162, 212) -> one form.
- drift 208-222, 240-243, 378-386, 419-425: PollPrStatus, PrMerged and PrClosed write
  `task.status`/`sub_status` on the board; the module's ReportPrStatus/ReportPrUnreadable
  (462-529) write them again. Contradicts the PR-state seam (CLAUDE.md) and the spec's own
  guidance (322-331, 401-406, 448-450). Code: `src/runtime/pr.rs` posts raw strings.
  -> PollPrStatus emits PrStatusReported/PrUnreadableReported and keeps counters and
  backoff only; PrMerged/PrClosed keep host effects only.
- QUESTION 531-536 vs 566-569: ClosePrOnDone triggers on every `status transitions_to done`,
  merges included; guidance says "NOT run for PrMerged" and code writes no close row on
  merge. (a) emit PrCloseRequested from the routes that write a close row (store-split
  UpdateTaskDoneWritesACloseRow, RequestCloseTask, ExitSession done/rebase) —
  recommended; (b) keep the transition trigger with a guard.
- bp 737, 806: `when: split/WrapUpRequested(task, host, pr)` binds, not matches -> both
  rules fire for every action. -> bind `action`, `requires: action = pr` / `done`.
- bp 467-491: `state.upper`, `review_decision.upper` -> black box `upper(state)`.
- bp 47-67: `@guidance` inside `entity PrPollState` -> plain comments.
- mech 454: `surface PrReportReducers` mid-Rules -> a Surfaces section at the end.
- mech 139, 152, 164, 192: unqualified `local_host()` -> `core/local_host()`.
- mech 557, 1118, 1153: cite the dead heading `store-split.allium, "ClosePrOnDone's owner"` -> store-split W5.
- bp history: 181-182 "since the override_poll_owner MCP tool was removed"; 396 "The
  Review Board UI has been removed"; 125-126 cite a `docs/plans/…-77825-…` file.

### Appendix D — store-split, sync, spacetime-seed, spacetime-snapshot, spacetime-memory-store

Coverage: sync, snapshot, memory-store read in full. store-split 811-2084 and
spacetime-seed 200-800: rule bodies read, remaining prose grep-scanned only.

Checker: 0 diagnostics here. Mirror spot-checks pass (RequestFeedRefresh, both prunes,
release_unseen_window); config literals match; RULES_VERSION = BOARD_RULES_VERSION = 51 at `10b68974` (52 after the merge).
All old Appendix D items resolved in all five specs.

sync analyse conflicts (BoardConnection.status from connected): not ruled on last time.
Neither is real. (1) InitialSubscriptionRefused only answers a subscribe sent after
host/UserIdentitySettled; host gives Settled and Conflict opposite guards; code subscribes
only in the non-Conflict arm (`src/sync/session.rs::SyncSession::attempt`). (2) In code
the conflict always wins (`attempt` applies Accepted then IdentityConflict with no await;
`BoardConnection::apply` ignores events in Failed). -> Decision 5: one paragraph in
StopOnAUserIdentityConflict's guidance naming both rules.

store-split.allium.
- drift 1060: `caller_host` unbound; TaskStatusWrittenByHand carries no host (code
  `call_host` in `spacetime/module/src/tasks_epics.rs::release_unseen_window`) -> black box
  `caller_host()` or carry the host.
- drift 1064-1076: HandMoveReleasesAWindowTheBoardDidNotSee writes a close row without
  failing the open session request; its guidance and code do
  (`session_request.rs::request_close_teardown` -> `fail_open_session_requests_for_close`).
  Breaks OneOpenSessionRequestPerTask -> add the `"task closed"` ensures.
- drift 2117-2153: update_task to done with a pr url and an unseen window fires both
  HandMove… and UpdateTaskDoneWritesACloseRow (two close rows; code writes one) -> add
  `task.current_window = null` to UpdateTaskDone's condition.
- bp 2133: `has_pr_url` reads the pre-patch url; guidance says the final url -> `patch.url ?? task.url`.
- bp 2192: `core/Setting.created_or_updated(...)` not in the language -> guarded create + update.
- mech 2286-2292: "see the open question below" points at nothing -> delete.
- mech 2253-2255: "of either kind" (there are three) -> reword.
- bp 1499-1500: RequestExitSession repeats a clause of `is_valid_token` (595) -> delete.
- bp 1681-1685: `live_tokens` restates `is_valid_token` -> derived `SessionRequest.is_live_token`.
- bp: Surfaces (770) before Rules (805) -> canonical order.
- mech 2116: missing blank line.

sync.allium.
- bp 1005-1016, 1055-1069, 1115-1127: BoardWritesThroughTheStore requires only
  `connected`, so a create on an unidentified board satisfies it and
  CreatesRequireASettledIdentity together; patched by `and not MutationSentToStore(...)`
  in ensures (negating a trigger, not in the language) -> converse guard on
  BoardWritesThroughTheStore; drop both `and not` lines.
- bp 1553-1556: SubscriptionsCoverOnlyTheOwnBoardAndItsEpics asserts `s.epic != null`
  on a non-optional field (always true) -> prose invariant or over the asks.
- mech 1506-1507: cites a "case (a)" note on `covered_epics` that does not exist.
- mech 973-977 ("roughly 1,790 archived tasks"), 995 ("as before"), 1767-1770 ("a board
  has run on an old store for two days … The operator chose") -> drop.
- drift (minor) 1299-1332: a failed adopt / ensure_host_identity between acceptance and
  subscribe is also this outage in code (`session.rs::SyncSession::attempt`) -> one clause.

spacetime-seed.allium.
- bp 268: `.filter(...).sum(e => e.row_ids.max ?? 0)` -> black box `highest_row_id(extracts, t)`.
- bp 809: `Snapshots.flat_map(...)` -> nested `for`.
- mech 1068: "(task #16385, 2026-10-04)" -> "Operator decision:".
- mech 112-114, 1114-1117, 999, 1009-1011: history -> present tense.
- mech 1048-1050, 1073: dated counts "Measured against the real old store: 14 / 26" -> drop.
- bp 435, 649, 754: literal `"archived"` duplicates sync `config.removed_statuses`; 697
  literal status set duplicates `core/TaskStatus` -> reference them.
- bp 920, 929-936, 1035, 1037, 1064: undeclared `snapshot.rows`, `snapshot.kept_tasks`,
  bare `row`, `per_table_counts_*` -> black boxes or derived values.
- bp section order: Enumerations before Value Types; Contracts after Entities; rules after
  Actor Declarations -> canonical order.
- QUESTION 872: `actor StorePublisher` declared here, though Decision 2 puts actors in
  core. (a) move to core; (b) keep, with a one-line "seed-specific" comment (recommended).

spacetime-snapshot.allium.
- bp: Contracts after Entities, `given {}` inside Entities -> canonical order.

spacetime-memory-store.allium. Nothing to report.

### Appendix E — startup, cli, dispatch, dispatch-prompt, keep-awake, onboarding-guide, observability

Analyse clean here. Cited config values match code. No `old(`, non-built-ins, misplaced
`@guarantee`/`@guidance` or unguarded temporal triggers.

Baseline audit (scratch-tested, 3.6.1): typing a rule trigger parameter is an error;
typing a `provides` entry clears the warning but is read as a named argument and not
validated (`BoardExited(cause: NoSuchThing)` passes) — do not use it. So an enum used only
through literals of a chained trigger parameter (StartupConfigOutcome, BoardExitCause) is a
real checker gap, NOT filed upstream (the baseline header wrongly says it is). A contract
operation taking the enum as a parameter clears it. Verdicts: StartupConfigOutcome and
BoardExitCause stay (literals gap; replaces the old plan's "real"); SessionLaunchFailure,
RetireOutcome, ManagedAddressState, ModulePublishOutcome, Bump, Preceding stay (#106);
StartupAbortReason: see Q-S1.

startup.allium. 9 of 10 resolved.
- S1 mech 174, 395, 979: "throwaway database", "database path" -> "data directory".
- S2 mech 727-731: comment says literals are the only use, but `restart_in_session`
  returns `StartupAbortReason?` (306) -> RetireOutcome's comment wording.
- S3 bp 168-174: past-tense narrative ("Recording the RUNNING BINARY …") -> present tense.
- Q-S1 QUESTION: give StartupAbortReason (and SessionLaunchFailure) a real parameter use:
  contract `StartupAbortReport { abort_message: (reason: StartupAbortReason) -> String }`
  (mirrors `src/startup/launch.rs::StartupAbort::message`), demanded by
  BoardLaunchCommand, called by a rule on `StartupAborted(reason)`; same for
  `session_failure_message`. 17 -> 15 warnings (verified). (a) add, only with a rule that
  calls it (recommended); (b) keep both baselined.
- drift (code comments): `src/runtime/mod.rs` (two places) and
  `src/tmux/tests/keys_and_options.rs` cite `ConnectToTheStoreOnceTheHostIsNamed` ->
  `ConnectToTheNamedStoreOnceTheHostIsNamed` / `BringUpTheManagedStoreOnceTheHostIsNamed` /
  `PublishTheBoardsStoreOnItsSession`.

cli.allium. 8 of 8 resolved.
- C1 mech 390-411: bare `this_host` -> `core/local_host()`; drop the note at 431.
- C2 mech 433-434: "A row saved before rows carried a host" -> "A row with no host".
- C3 bp 98-115, 633-640: `CliInvocation` never created, `exit` never assigned; two
  invariants constrain unwritten state -> a rule `when: CliFailed(invocation, reason)
  ensures: invocation.exit = failure` plus the success path, or say `exit` is observed only.
- by-design 691-697: SQLite hits are the standing `NoSqlite` guarantee.

dispatch.allium. 8 of 10 resolved.
- D1 mech: 11 bare `local_host()` (74, 195, 199, 895, 899, …) -> `core/local_host()`.
- D2 / Q-D1 QUESTION 187-857: ProvisionDispatch is ~36 lines of clauses plus ~630 lines of
  guidance under 18 headings. (a) move the headed blocks into a `contract
  WorktreeProvisioner` with one `@invariant` each, keeping heading names (dispatch-prompt
  and src cite them, e.g. `src/dispatch/mock_sequence.rs` cites
  `ReusedBranchWithNoOwnWorkIsFastForwarded`) — recommended, own package; (b) leave it.
- D3 bp 1231 vs 2163: Surfaces before Invariants -> canonical order.
- D4 mech 2303-2314, D5 mech 2337-2346: history -> present-tense rationale.
- D6 mech 913: "(10s)" copies store-split's config default -> drop.

dispatch-prompt.allium. 5 of 6 resolved.
- P1 mech 89, 376-379; P2 mech 1370-1377, 1382-1384: history -> present tense.
- P3 bp 1374-1376: since 7168416e the review command is configurable; the guarantee still
  claims the routed command "sizes its own review (/preference-review …)" -> "the runbook
  has no diff-size branch; sizing is the review command's". Note settings T1 (Appendix A)
  deletes `ReviewCommandIsASetting`, which this spec cites (511).
- by-design 244, 331, 416, …: AgentPromptComposed, DesignStepNamed, `*RunbookRendered` are
  the composer's observable output (see Decision N1).

keep-awake.allium. 3 of 3 resolved.
- K1 bp 191-196: "Covers every way the board process ends, a crash or kill included"
  contradicts startup 2767-2768 (no BoardExited on SIGKILL) -> BoardExited only; crash is
  `HoldDiesWithTheBoard` (255-258).
- K2 mech 60-62: `given` after Enumerations/Entities -> first.
- K3 bp 80, 111, 120, 176, 210: `UserAction("W")` literal key -> named trigger matching
  table action `toggle_keep_awake` (Decision 1; same as split-pane S-n4).

onboarding-guide.allium. 3 of 3 resolved.
- G1 mech 11: mangled header "step tabs between--           them;".
- by-design 302-304: local `actor GuideReader` is a public page visitor, not a dispatch party.

observability.allium. 6 of 7 resolved.
- O1 bp 250-262, 348-353, 371-374, 665-671: implementation citations
  (`record_usage_event_logged`, `UsageCommand::Record`, `dispatch_keyed`, `ObservedEvent::PaneKey`,
  `src/cli/statusline.rs`) -> behaviour plus one `path::symbol` where needed.
- O2 mech 547: stray "-- prose:". O3 mech 52-54: garbled parenthesis.
- O4 bp 546: StatusLineDecorator faces `core/Operator`, but Claude Code invokes it -> `core/HookRunner` or `_`.
- drift (code comments): `src/main.rs` cites "`docs/specs/dispatch.allium`:
  StatusLineDecorator" (it is in observability); `src/tui/commands/usage.rs` says
  "failures are intentionally swallowed", but `record_usage_event_logged` warns
  (`UsageWriteFailureIsSilent`).

### Appendix F — tasks, epics, work-packages, mcp-task-tools, learnings

Checker clean here; no `old(` (the tasks grep hits were `ToggleEpicFold(`). Cited
`path::symbol`s exist. MCP params match `src/mcp/handlers/*` except query_usage.

tasks.allium. 12 of 13 resolved.
- old 2, mech 1401: ToggleBookmarkFromBoard emits `BookmarkSet(viewer: …)` but SetBookmark
  (1416) binds `subscriber`.
- T1 bp: `contract TaskTeardown` (1499) mid-Rules; "Module write rules" (2221-2491) after
  surfaces; surfaces (2125-2218) before Invariants (2494) -> canonical order.
- T2 mech 4-12: Scope header omits phoenix, bookmarks, the new-task signal, build-output
  cleanup, module write rules.
- T3 mech 2414: `requires: task.exists` -> `exists task`.
- T4 bp 2529-2531: SubStatusAutoreset bullet "a row stored before these checks existed …
  reset once" is a migration log. DECIDED (N2): if the module still runs the reset
  (check `spacetime/module/src`), keep one present-tense sentence; otherwise delete.
- T5 bp history: 533 "no longer transitions"; 828 "Archive edge column … went with the
  archived status"; 1166-1168 "before this field existed"; 1689-1696 `let released =
  removed` "now COINCIDE" -> present tense; use `removed`.
- T6 drift EditTask 2018-2071: prose says only changed fields are written; ensures writes
  title/description/repo_path/plan_path/tag/base_branch/phoenix unconditionally. Code
  compares every field (`src/runtime/editor.rs::finalize_task_edit`) -> guard each with
  `edited.x != opened.x`.
- T7 drift CopyTask 1891-1903: no `base_branch` though the flow asks for it (1939-1945);
  writes `source.wrap_up_mode` though the user can replace it (1946-1950) -> confirmed values.
- T8 bp 411-418: MoveTaskForward's `last_pre_tool_use_at = now` stamp is prose only -> ensures.
- T9 mech 755-756: duplicated parenthesis.

epics.allium. 8 of 8 resolved.
- E1 entry/bp: `RecalculateEpicStatus(...)` is emitted (186, 1443, 1444, 1504, 1505;
  pr-workflow 876; feeds 2562-2565) but no rule consumes it; the derivation lives only in
  the ~110-line prose block "EpicStatusRecalculation" (1838-1955) -> a rule with that logic
  as expressions (check `recalculate_epic_chain` in spacetime/module/src). Also reads
  core `Epic.children` (Appendix A B2).
- E2 bp: narrative history in that block (1842-1844, 1870-1875, 1893-1897) -> cut.
- E3 mech 469-471: `task.worktree = task.worktree` no-ops -> delete.
- E4 mech 464-466: `filter(...)`, `min_by(...)` -> `where`; black box or comment.
- E5 bp 507-509: "delete_epic reducer is unchanged and still has the final say" -> "has the final say".
- E6 drift EditEpic 97-106: unconditional writes; code compares
  (`src/runtime/editor.rs::finalize_epic_edit`) -> guard as T6.
- E7 mech 322: `epic.subtree_epics + epic` -> `+ {epic}`.
- E8 mech 1970: EpicBoard provides `CreateEpic(title, description, parent?)`, rule
  signature (153) has `sort_order?` third -> match it.

work-packages.allium. 10 of 10 resolved.
- W1 mech 126, 132, 152, 155, 376, 390: `title.trim()` -> black box `trimmed(title)`.
- W2 mech 485-486: `not (exists t in link_task_ids where …)` invalid -> `.any(...)`.
- W3 mech 88: `enum WorkPackageItemKind` in Entities -> Enumerations section.
- W4 bp 647-649: "Computed on every render rather than cached" -> delete.

mcp-task-tools.allium. 8 of 8 resolved.
- M1 drift 1084, 2101: McpQueryUsage lacks `limit` (code
  `src/mcp/handlers/tasks/mod.rs::QueryUsageArgs`, default 50, max 500) -> add `limit?`.
- M2 drift 132-135 + open question 2376: spec says the schema states "url_type required
  whenever url is non-empty"; the code schema has no conditional, the url description
  carries "a non-empty one needs url_type" (`UpdateTaskArgs`) -> restate to match, delete
  the open question (one-line confirm with the user).
- M3 QUESTION open question 2378: explicit sub-epic plus a repo_path change. tasks
  `UpdateTaskIsOneTransaction` (2437-2440) and mcp-task-tools 365-376 already say "the
  repo decides". (a) yes, delete the open question; (b) explicit sub-epic wins (code change).
- M4 drift 1310: "Used by the /queue-plan slash command" — no such command -> drop.
- M5 mech 1303: `requires: task != null` -> `exists task`.
- M6 bp 1314-1362: a ~50-line catalogue of other CLI subcommands duplicates cli.allium;
  1359-1361 "dedicated `hook-*` subcommands" contradicts `dispatch hook <id> <kind>`
  (1349) -> move `uninstall` to cli.allium (update its Excludes), reduce the rest to
  pointers, fix the hook wording.
- M7 bp 1061-1062: "Inheriting the caller's epic was removed" -> "does not inherit".
- A-n9 (from Appendix B) 2228: "(agents.allium, A5)" -> A6.

learnings.allium. 9 of 9 resolved.
- L1 drift 228: `requires: scope in {repo, epic} implies scope_ref != null` contradicts
  the defaulting (237, 69-72) and code (`src/mcp/handlers/learnings.rs::resolve_scope_ref`)
  -> `let ref = scope_ref ?? default_scope_ref(task, scope)`; `requires: scope = epic implies ref != null`.
- L2 drift 262-263: "for a scope other than task" is stale (task scope gone) -> drop.
- L3 bp 62-67: "Rows recorded under the old scope were archived and re-scoped" -> delete.
- L4 bp 377-389: DeleteLearningViaMcp states visibility in guidance only -> `requires:
  learning.visible_to(caller())`.
- L5 bp 172: "QueryLearningsViaMcp is unchanged" -> "does not apply the mute".

### Appendix G — workspaces, admin-surfaces, board-layout, board-visuals, keybindings

Checker clean here. Old cross-cutting X1, X2, X4, X5 resolved.

workspaces.allium. ~14 resolved; one open: orphan `*Allowed` outputs (385, 402, 542, 583,
696, 717, 775, 895) — nothing consumes them. See W-Q1.
- W1 mech 735: "(…, D8)" -> D6. W2 mech 743: "(A8)" -> D6.
- W3 mech 903 "learning #90175", 995 "task #98523" -> drop.
- W4 bp 102, 147, 151-152, 260, 264-265, 279, 283-284: primitive methods `.prefix(n)`,
  `.trim()`, `.length`, `.has_control_or_format_characters` -> black boxes.
- W5 mech: `core/UnseenTasks`, `core/Bookmarks` qualified at 508-529 but unqualified at
  349-353, 646-652 -> one form (`core/X`).
- W6 bp 394-400 RepoGroupsStayPublic: `when: A(...) or B(...)` with both requires reading
  bindings only one arm binds (same shape the last review removed); duplicates requires at
  432, 458 -> split per trigger or delete.
- W7 bp 383: EpicIsPrivateOnlyWithACreator duplicates requires at 431, 457.
- W8 bp 551-583 ReParentingKeepsTheHolder: same union-trigger problem -> split.
- W9 by-design 8-12, 1011-1058, 389, 867, 99-100, 1001-1003: MigrateStoreToWorkspaces is
  live behaviour (a new store runs it). Leave.
- W10 entry 172-173: "Enter sends SetDisplayName(typed)" prose only -> with A3.
- W-Q1 QUESTION: the eight `*Allowed` guard rules. (a) inline as `requires` in the owning
  rules (epics, tasks, work-packages); (b) `@invariant`s of a `contract StoreWriteChecks`
  that the StoreWriteChecks surface demands (recommended). Keeping them as orphan events
  is ruled out by N1.

admin-surfaces.allium. 10 of 10 resolved.
- A1 drift 110: "Esc closes it"; table has `["Esc", "q"]` (`src/keybindings/table.rs`) -> both.
- A2 mech 20-22: Dependencies names surfaces this spec does not use -> drop.
- A3 entry 131, 144, 157, 169, 198, 211, 223, 249: every text-input and confirm key ends at
  an orphan `TextInputOpened`/`ConfirmationOpened`; the write is guidance only -> add
  `AdminTextCommitted(purpose, text)` and `AdminConfirmed(purpose, target)` to AdminScreen,
  one rule per purpose emitting `workspaces/…`.
- A4 bp 99, 116, 265, 285, 386: orphan render events (`AdminScreenOpened`, …) -> see Decision N1.

board-layout.allium. 9 resolved. Optional: gg chord 137-159, column navigation 99-120,
selection preservation 1070-1088 are prose only.
- L1 bp history: 255-267, 317-323, 333-337, 360-363, 568-569, 589, 658, 695, 925 -> present tense.
- L2 drift 806: "There are no `A` and `F` toggle keys" (`A` opens admin) -> delete.
- L3 bp 1117, 1222, 1256, 1271, 1314, 1353, 1371, 1372: `filter(coll, x => p)` not in the
  black-box list (1375-1380) -> `coll where p`.
- L4 bp 1633-1732 TextInputField: surface `let buffer`/`let caret` plus nine `-- ensures:`
  comments; nothing consumes TypeChar/Backspace/Cursor*. See L-Q1.
- L5 drift 1636-1641: field list omits display-name, admin and work-package inputs
  (`src/tui/input/table.rs` maps them to N::BoardText); "not on this surface yet" -> fix.
- L6 mech 812, 822: `tmux_window` -> "a live session (current_window set)".
- L-Q1 QUESTION: (a) an entity `TextBuffer {buffer, caret}` with one rule per motion;
  (b) drop `provides`/`let`/`-- ensures:` and keep the four guarantees (recommended;
  caret logic is a UI helper, `src/tui/text_caret.rs`).

board-visuals.allium. All resolved.
- V1 drift 197, 1260-1261: "the count of selectable items in that column"; code
  (`src/tui/ui/kanban/mod.rs`) counts task and epic cards incl. folded hidden counts, work
  packages as 0 -> say so.
- V2 bp 1103, 1327-1328: "No test covers it" -> docs/testing.md uncovered list.

keybindings.allium. All 4 resolved; enums match `src/keybindings.rs`.
- K1 drift 273-287: inventory lacks `b` toggle_bookmark and `V`
  toggle_work_package_visibility (`src/keybindings/table.rs`).
- K2 drift 60-64: board_text inventory omits display-name and admin inputs.
- K3 drift (code side): table.rs `jump_to_agent` description "Switch tmux to the agent's
  window", but `src/agent_tree/render/agents.rs::space_action` also resumes -> fix the code string.
- K4 bp 559-575: ModifiedPressMatchesOnlyItsOwnRow and KeyWithoutARowCannotAct both fire
  KeyIgnored for an unbound Ctrl/Alt press, contradicting OneLookupPerPress (720) ->
  invariant/guidance or drop its ensures.
- K5 bp: `external entity TestSuite` (705) under Actor Declarations; `contract
  KeybindingDriftGate` (853) after surfaces -> canonical order.
- K6 bp history: 53, 183, 185, 227-228, 397, 416-420, 779-781, 842-846 -> present tense.
- K7 bp 539, 575, 609, 642: `KeyIgnored`, `KeybindingsListed`, `KeybindingsShown`
  unconsumed -> see Decision N1.
