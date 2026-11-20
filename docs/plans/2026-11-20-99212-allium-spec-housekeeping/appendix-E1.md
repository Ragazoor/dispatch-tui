# Appendix E1 — startup, cli, onboarding-guide, observability

Condensed report of one read-only review agent for task #99212, at commit b03f5ba0 (main, 2026-11-18). Part of docs/plans/2026-11-20-99212-allium-spec-housekeeping.md. Line numbers drift as packages land: search for the cited symbol, not the line. Acceptance checks run from the worktree root.

# E1 — startup, cli, onboarding-guide, observability (review at b03f5ba0)

Reviewer: read-only. allium 3.6.1, run at b03f5ba0 on a scratch copy of the whole set. `check`: 5 diagnostics, all startup's baselined `allium.definition.unused` (lines 712, 731, 741, 750, 783); cli, onboarding-guide and observability have none. `analyse`: 0 findings for all four. Citation sweep (`x.allium: Name`, `x.allium's Name`, comma lists, quoted headings, wrapped lines joined): every cited name exists in its target spec.

**Read in full:** startup.allium 1–2841, cli.allium 1–880, onboarding-guide.allium 1–404, observability.allium 1–691; the whole `git diff 10b68974 HEAD` for the four specs, including b03f5ba0's startup hunk; standing rules (98956 l.33–132, 98983 l.76–167); Appendix E; the four slices; assessing-specs.md and actioning-findings.md. **Scanned only:** language-reference.md (the enum section, about lines 490–545, read; otherwise grepped). patterns.md was not read. Code checked for every drift item and every citation used below.

## Appendix E carry-over (last review, epic #69699)

| Item | Status |
|---|---|
| S1 "throwaway database"/"database path" | resolved |
| S2 StartupAbortReason comment | resolved (comment gone; enum now credited through StartupAbortReport) |
| S3 past-tense narrative 168–174 | resolved there; more history elsewhere, see E1-S-2 |
| Q-S1 StartupAbortReport contract | resolved, option (a): contract 665–683, rules 2052–2078; SessionLaunchFailure and StartupAbortReason are no longer warned |
| drift: code cites ConnectToTheStoreOnceTheHostIsNamed | resolved (no hit in src/ or tests/) |
| C1 `this_host` / C2 / C3 CliInvocation never created | resolved (CliInvocationStarts and four exit rules); C3's exit wiring is only partly done, see E1-C-1 and E1-C-2 |
| G1 mangled header | resolved |
| O1 implementation citations | resolved (one path::symbol remains at 289, kept on purpose) |
| O2, O3 | resolved |
| O4 decorator facing | resolved (`core/HookRunner`) |
| drift: src/main.rs "dispatch.allium: StatusLineDecorator", usage.rs "swallowed" | resolved |

No Appendix E item is still open.

**Baseline audit (startup, five `definition.unused`):** every entry still has its why-comment, and each is still a real checker gap. StartupConfigOutcome (712) and BoardExitCause (783) are used only as literals of a trigger parameter (not filed upstream). RetireOutcome (731), ManagedAddressState (741) and ModulePublishOutcome (750) are used only as contract return types (#106). The slices give line numbers from e0e4b62a; the numbers here are current.

## Findings

### startup.allium
| Code | Class | Line | Problem | Remedy |
|---|---|---|---|---|
| E1-S-1 | mech | 1174, 2314 | "four artefacts", but ConfigArtefact has five (693–707) | "five", or drop the count |
| E1-S-2 | bp | 359–363, 696–698, 1041, 1303, 1633–1635, 1795, 2304–2308, 2491–2492 | History: "A launch that raced the gap…", "introduced for, but it now carries", "the one this rule exists to change", "the job it had when minting seeded…", "was the operator's decision", "as before", "removing the standalone setup command would…have quietly cost", "the hand-run `spacetime publish` step left…" | Present-tense rationale (Decision 4) |
| E1-S-3 | bp | 1312–1318 | "Task.host is about to become a value other people read", citing a dated design doc (docs/superpowers/specs/2026-09-13-…). Task.host is already shared | Present tense; drop the doc citation |
| E1-S-4 | bp | 1378–1381 | "(an ambient … predicate, tried and reverted)" is history | Drop the parenthesis |
| E1-S-5 | bp | 1412 | "the behaviour decision 1 removed" cites a decision list this spec does not have | State the behaviour ("a hostname taken silently as the label") |
| E1-S-6 | mech | 842, 1460, 2403 | The spec cites itself as "startup.allium: X" | Bare rule name |
| E1-S-7 | mech | 1497, 1817, 2267 | Overlong comment lines | Reflow |
| E1-S-8 | bp | 1866–1868 | The ensures asserts `recorded_module_hash() = embedded_module_hash()` unconditionally, but the guidance (1873) says recording can fail. `record_module_hash` is called by no rule | `ensures: ModuleHashRecorded(operator, kept: record_module_hash(embedded_module_hash()))` (the StoreIdentityPinned shape); add the event to the N1 list at 894 |
| E1-S-9 | bp | 1114–1156, 295, 306 | Rules use undeclared black boxes `can_enter_session`, `session_failure_reason` and `can_start_fresh_board`, while no rule calls the contract operations `enter_session` and `restart_in_session` | `let failure = enter_session(session, argv)` and branch on it; either model the restart through `restart_in_session` or drop it from the contract and say the rules decompose it |
| E1-S-10 | mech | 1096–1100 | StartFreshBoardInSurvivingSession has no `can_start_fresh_board(session)` guard, so on failure it fires beside ReportFreshBoardFailure (SessionAttached and StartupAborted together) | Add the guard (or fold it into E1-S-9's `let`) |
| E1-S-11 | drift | 983–987 | RefuseWhenTheSessionCannotBeIdentified overlaps DrawInTheWindowThisBoardWasStartedIn (899) and RefuseToLaunchFromInsideTheBoardWindow (959). `src/startup/launch.rs::plan_launch` decides the window role before the empty session name | Add `and not launching_as_the_started_board and not launching_from_the_board_window` to the guard |
| E1-S-12 | mech | 2135–2139 | ForgetAStaleStoreRecordOnAManagedLaunch requires only `explicit_store_server() = null`. Its guidance (2146–2149) says it runs after the CLI check lets the launch through, and in the process that draws the board | Align the guard with the guidance (`spacetime_cli_on_path()` and the drawing process), after checking the order in src/runtime/ |
| E1-S-13 | mech | 1734, 2139, 2214 | Inconsistent first argument: StoreAddressForgotten takes `operator` at 2139 and `core/local_host()` at 2214; BoardWarned also takes the host | One shape, with named arguments |
| E1-S-14 | drift | 1750–1751 | "names the store's log when it did not" is untrue for the new stop-timeout failure. `src/spacetime/managed_store.rs::ManagedStore::check_health` says the old store is still running and to restart it by hand, with no log path | "names the store's log when the start failed, and says the old store is still running when the stop timed out" |
| E1-S-15 | bp | 516–524 | Contract-wide properties sit inside `restart_managed_store`'s comment: every command bounded by a timeout, and stops signal a process group (in fact only the spawned store's group; `src/spacetime/managed_store.rs::SpacetimeManagedStore::stop` signals an adopted store per pid) | Move them to `@invariant`s on ManagedStoreHost (e.g. EveryStoreCommandIsBounded, StoppingReachesTheSpawnedGroup) |
| E1-S-16 | drift | 484–491 | The spec says nothing of the publish lock. Launches sharing the managed store serialise publish and record under a file lock; one that cannot take it within the publish timeout warns and goes on; each process publishes from its own module file (`src/spacetime/managed_store.rs::ManagedStorePorts::lock_publish`, b03f5ba0 M3) | Add an `@invariant PublishAndRecordAreSerialised` to ManagedStoreHost |
| E1-S-17 | drift | 1640–1667 | Import seam. ImportBringsUpTheManagedStore emits ManagedStoreRequested, so in the spec an import continues into StoreChosenAtStartup. That runs the pin check (contradicting cli 354–355, which exempts import), PinTheStoreOnceItAnswers, PublishTheBoardsStoreOnItsSession and DrawBoardAfterTheHostIsNamed (BoardDrawn). On abort, StopTheManagedStoreWhenStartupAborts stops an adopted store and BoardExited fires. "Stops only a store it started" is guidance only. Code: `src/spacetime/managed_store.rs::ManagedStore::bring_up_for_import` stops at bring-up and `stop_spawned_on_exit` stops only a spawned store; src/cli/store_import.rs connects with `StorePin::Unchecked` | Give the import its own path after ManagedStoreReady (a holder/purpose parameter on the store triggers, or an ImportStoreReady trigger the board rules do not hear), plus a rule that stops only a spawned store through `stop_spawned_store`. The remedy stays in startup: cut the import off before StoreChosenAtStartup, so sync.allium's OpenBoardConnection (sync 372) is never reached. spacetime-snapshot.allium 362 (`startup/ManagedStoreStopRequested`) needs a look only if the spawned-only stop gets its own trigger |
| E1-S-18 | by-design | 709–783 | The five baselined `definition.unused` | Keep (see audit above) |
| E1-S-19 | by-design | 1026–1030 | "Implementation: tmux new-session -A" in guidance explains why `-A` upholds NeverCreatesASecondSession | Keep |
| E1-S-20 | by-design | 2672–2676 | "a rename surface, which does not exist yet" | Matches host.allium 461–470 (no surface, a decision) |
| E1-S-21 | by-design | 2799–2805 | The path::symbol citations all exist (claude_paths.rs, DISPATCH_PLUGIN_DIR, claude_dir, settings_path, plugin_dir_under, home_dir, StartupPaths::resolve) | Keep |

### cli.allium
| Code | Class | Line | Problem | Remedy |
|---|---|---|---|---|
| E1-C-1 | entry | 248, 319, 365, 538 | Only `plan` (538) emits CliStoreRequested. repo set-verify/clear-verify/list and prune-repo-paths never request the store, so the host-file and pin refusals the guidance names for them (335–338) cannot apply in the model. `store import`'s exemption arm (256, 325) is unreachable too | Have each store-opening rule emit `CliStoreRequested(invocation)`, or provide it on DispatchCommandLine. Let the triggers carry `invocation` instead of `current_invocation()` (443, 538) |
| E1-C-2 | entry | 194–205 | CliFailed's only emitters are three refusal rules. The failures the guidance names have no emitter: plan file not found (542–545), task not found (554), unreachable store (267–269), set-verify refusal (415–419), import with no CLI (625–627) | DispatchCommandLine provides `CliFailed(invocation, reason)`, like CliCompleted |
| E1-C-3 | bp | 751–773 | `PrLearningsGateCheck` and `PrGateBlocked` are run by Claude Code's PreToolUse hook, yet sit on an Operator-facing surface (the O4 pattern). `PrGateBlocked` is also the process-side twin of pr-workflow's `PrCreateBlocked` (pr-workflow 92–100), but nothing links the two | Move both to a surface facing `core/HookRunner`, and name `PrCreateBlocked` as the verdict that `PrGateBlocked` reports (a guidance line; the verdict crosses a process boundary over HTTP, so a direct chain would misstate it) |
| E1-C-4 | bp | 622–623 | "at most 30 seconds' worth of startup" is a literal (`src/cli/store_import.rs` START_TIMEOUT) | A config value (e.g. `import_store_start_timeout: Duration = 30.seconds`) |
| E1-C-5 | by-design | 65–70, 88–92 | Enum values separated by newlines with no `|`. The reference shows only the `|` form, but the checker accepts this one and core, dispatch-prompt, retro-skill and board-visuals use it too | Leave; a set-wide style choice, not this cluster's |
| E1-C-6 | by-design | 877, 879 | Both open questions still stand (clap still exits 2 for a pr-gate parse error; import still ignores the recorded address) | Keep |
| E1-C-7 | by-design | 788–794 | The SQLite mentions are the standing NoSqlite guarantee | Keep |

### onboarding-guide.allium
| Code | Class | Line | Problem | Remedy |
|---|---|---|---|---|
| E1-G-1 | bp | 243 | "The tour, as agreed:" is conversation history | "The tour:" |
| E1-G-2 | bp | 28–52 | The header "Design decisions" restates the guarantees (OnePageNoBuildNoNetwork, ImitationOnly…, WalkthroughReads…, ReadmeLinks…) and GuideKeysAreRealBindings | Keep only the scope and the key-subset rationale (48–52); the rest is said once below |
| E1-G-3 | by-design | 61, 305–307 | Local `Visitor`/`GuideReader`: a public page visitor, not a dispatch party | Keep |
| E1-G-4 | by-design | 48–52 | Checked against docs/guide/index.html's dispatch-keys block: z, U and the Space card actions are present; n, E, D, [ ], gg, G, L, H are absent | Consistent |

### observability.allium
| Code | Class | Line | Problem | Remedy |
|---|---|---|---|---|
| E1-O-1 | drift | 415–418 vs 137–140 | "Enforced on every insert… the bound holds continuously… Below the cap the eviction is a no-op" contradicts the slack: `spacetime/module/src/config.rs::record_usage_event` prunes only once the count passes cap + USAGE_PRUNE_SLACK | The bound holds within cap + slack; the scan runs once per slack rows |
| E1-O-2 | bp | 139–140 | The literal `1_000` in a config comment | `usage_event_prune_slack: Integer = 1_000` (module USAGE_PRUNE_SLACK) |
| E1-O-3 | drift (code comments) | — | `src/test_log.rs` header cites `DbCallSlowWarning` in observability.allium, which no longer exists (38–39 says there is none). The `spacetime/module/src/config.rs::record_usage_event` doc says "no reader ever sees the table over the cap", untrue with the slack | Fix both comments |
| E1-O-4 | entry | 97–117, 397–399 | No rule records an `mcp_tool` event; the enum values `mcp_tool` and `agent` appear only in guidance. Code (`src/mcp/handlers/dispatch.rs::handle_tools_call`) records every resolved tool call: action is the tool name ("unknown" for an unregistered one), detail null, actor `agent` for a task caller and `human` for a session caller. "Agent-driven" at 397 understates that | A rule `when: McpToolCall(...)` ensuring FeatureUsageRecorded(category: mcp_tool, …), with the actor by caller kind |
| E1-O-5 | bp | 175–188 | The ensures calls the black box `append_jsonl(...)` as a statement | Emit an observable outcome (e.g. TrajectoryAppended(path, entry), listed under N1) |
| E1-O-6 | bp | 213, 274 | Rules named "…IsSilent" both log a warning (src/service/usage.rs itself says "fire-and-forget is not silent") | Rename (e.g. …IsLoggedNotRaised); update the cites in src/test_log.rs, src/service/usage.rs and src/tui/commands/usage.rs |
| E1-O-7 | mech | 689 | The open question names "trajectory.jsonl"; the files are `<task_id>.jsonl` (50) | Fix the name |
| E1-O-8 | mech | 227–228 | Repeats the N1 note at 156–159 | Drop |
| E1-O-9 | mech | 146, 556 | Overlong lines | Reflow |
| E1-O-10 | by-design | 317 | Listens to `keybindings/KeyActionRun`: keybindings.allium 596–610 raises it for this rule, so it is not a Decision 1 breach | Keep |
| E1-O-11 | by-design | 687, 691 | Both open questions still stand (`src/mcp/trajectory.rs::append_entry` opens with append and no lock) | Keep |

**Counts:** mech 9 (S-1, S-6, S-7, S-10, S-12, S-13, O-7, O-8, O-9); bp 14 (S-2, S-3, S-4, S-5, S-8, S-9, S-15, C-3, C-4, G-1, G-2, O-2, O-5, O-6); entry 3 (C-1, C-2, O-4); drift 6 (S-11, S-14, S-16, S-17, O-1, O-3); by-design 11; QUESTION 0.

**Questions:** none. Every behaviour above is already decided in code or in the spec's own guidance. E1-S-17's remedy, an import path, describes what the code does; it changes no behaviour.

## Work packages

**WP-E1-1: startup housekeeping, managed-store contract and the import seam.** Files: docs/specs/startup.allium; docs/specs/cli.allium only where the import seam touches it (354–355 and 766–768 wording). Findings: E1-S-1 to E1-S-17. No QUESTION. Order: land before WP-E1-2 (both touch the import's store path). Follow the 98983 footer: `./scripts/check-allium.sh` with no new warning and no new `field.unused`; weed at the end. If E1-S-9 renames black boxes, grep src/ for cites. Acceptance (fails today, exit 1):
```sh
cd . && f=docs/specs/startup.allium && ! grep -nE "four artefacts|tried and reverted|Task.host is about to|decision 1 removed|A launch that raced the gap|the hand-run|AbortWhenTheModulePublishFails as before|would otherwise have quietly cost|it was introduced for|store's log when it did not" $f && ! grep -nE "^\s+recorded_module_hash\(\) = embedded_module_hash\(\)$" $f && sed -n '/^rule StartFreshBoardInSurvivingSession/,/^}/p' $f | grep -q "can_start_fresh_board" && grep -qiE "publish lock|PublishAndRecordAreSerialised" $f && [ "$(grep -c 'stop_spawned_store' $f)" -ge 3 ] && grep -q "= enter_session(" $f
```

**WP-E1-2: cli exit and store wiring.** Files: docs/specs/cli.allium. Findings: E1-C-1 to E1-C-4. No QUESTION. Order: after WP-E1-1. Acceptance (fails today, exit 1):
```sh
cd . && f=docs/specs/cli.allium && s=$(sed -n '/^surface DispatchCommandLine/,/^}/p' $f) && echo "$s" | grep -qE "^\s+CliFailed\(" && ! echo "$s" | grep -qE "^\s+PrGateBlocked\(" && [ "$(grep -c 'CliStoreRequested(' $f)" -ge 5 ] && ! grep -n "30 seconds" $f
```

**WP-E1-3: observability and onboarding-guide.** Files: docs/specs/observability.allium, docs/specs/onboarding-guide.allium, src/test_log.rs (comment), spacetime/module/src/config.rs (comment only, optional: if editing the comment stales module.wasm's source stamp, leave it and record why; the acceptance check does not require it). For E1-O-6, if renaming: src/service/usage.rs and src/tui/commands/usage.rs. Findings: E1-O-1 to E1-O-9, E1-G-1, E1-G-2. No QUESTION. Independent of WP-E1-1 and WP-E1-2. Acceptance (fails today, exit 1):
```sh
cd . && o=docs/specs/observability.allium && ! grep -nE "Enforced on every insert|trajectory\.jsonl|1_000 rows past" $o && grep -q "usage_event_prune_slack" $o && grep -q "category: mcp_tool" $o && ! grep -n "as agreed" docs/specs/onboarding-guide.allium && ! grep -n "DbCallSlowWarning" src/test_log.rs
```

All three checks were run at b03f5ba0 and exit 1.
