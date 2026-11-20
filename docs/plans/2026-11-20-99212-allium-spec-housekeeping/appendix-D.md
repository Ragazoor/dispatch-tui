# Appendix D — store-split, sync, spacetime-seed, spacetime-snapshot

Condensed report of one read-only review agent for task #99212, at commit b03f5ba0 (main, 2026-11-18). Part of docs/plans/2026-11-20-99212-allium-spec-housekeeping.md. Line numbers drift as packages land: search for the cited symbol, not the line. Acceptance checks run from the worktree root.

# Appendix D — store-split, sync, spacetime-seed, spacetime-snapshot (review #99212, HEAD b03f5ba0)

## Coverage
- **Read in full:** store-split 1-2370, sync 1-1884, spacetime-seed 1-1218, spacetime-snapshot 1-517.
- **Diffs since 10b68974:** sync and snapshot diffs read in full. Store-split diff read except a ~24k-char middle elision; that region (920-2100) was read in full in the current file. Seed diff scanned for declaration changes only: the one new declaration is SeedsNormaliseLikeMigrations; everything else is reorder or rewording.
- **Checker:** whole-set check on a scratch copy gives 0 diagnostics for these four files, which matches the slices. The only analyse findings are sync's two conflicts. Analyse is not gated (scripts/check-allium.sh:13).
- **Rules version:** RULES_VERSION = BOARD_RULES_VERSION = 59 (spacetime/module/src/rules.rs:317, src/sync/rules_gate.rs:21). No spec in the cluster writes a version number; sync 349-362 and store-split 84-95 name the two constants only. OK.

## Named item: sync analyse conflicts (package 10, #99198)
- **Present:** sync 1455-1464, in StopOnAUserIdentityConflict's guidance, names both rules. **Accurate:** in src/sync/session.rs::SyncSession::attempt the subscribe path is the non-Conflict arm, and adopt/ensure_host_identity also sit in that arm. Accepted and IdentityConflict apply with no await between them.
- **Nit (D-y-5):** the claim that a failed connection ignores later events is proved by src/sync/connection.rs::BoardConnection::apply (its catch-all `_ => false`), not by the symbol the comment cites.

## Old Appendix D items, re-checked one by one
store-split:
- drift caller_host: resolved, black box at 1028-1032.
- drift "task closed" ensures: resolved, 1040-1043.
- drift two close rows: resolved, holds_window_after_patch at 2120 and 2126.
- bp final url: resolved, `after = patched(task, patch)` at 2111.
- bp created_or_updated: resolved, 2196-2202.
- mech dangling open-question pointer: resolved, 2297-2301.
- mech "of either kind": resolved, 2263.
- bp RequestExitSession repeat: resolved, 1485-1487.
- bp live_tokens restatement: resolved, is_live_token at 596 and 1664.
- bp Surfaces order: resolved, 2321.
- mech blank line: resolved.

sync:
- bp converse guard: resolved, 1012-1013, and both `and not` lines are gone.
- bp trivial invariant: resolved, prose at 1606-1610.
- mech "case (a)": resolved, 1557-1558.
- mech dated counts and history: resolved at 973, 994 and 1824.
- drift adopt clause: resolved, 1351-1357.

spacetime-seed:
- bp .filter.sum: resolved, 290-302.
- bp flat_map: resolved, none left.
- mech task number: resolved, 1044.
- mech history: resolved.
- mech dated counts: resolved.
- bp literal "archived": resolved, sync/config.removed_statuses at 450, 668 and 806.
- bp status set: resolved, names_a_task_status at 717.
- bp undeclared names: resolved, black boxes at 895-899. A new one was introduced; see D-d-2.
- bp section order: resolved.
- QUESTION StorePublisher: resolved, option (b) at 1140-1141.

spacetime-snapshot:
- bp order: resolved, Given and Contracts now come before Enumerations. The section banner names remain wrong (D-n-1).

**Still open from the last appendix: none.**

## Findings

### store-split
- D-s-1 mech, 219, 265, 272, 291-292, 363, 517, 555, 575, 733, 737, 1397, 1474, 1492, 1570, 1596, 1622, 2051-2053, 2264, 2360, 2364. **Problem:** the MCP rename put tool names where row kinds are meant ("a session_exit row", "a session_wrap-up rebase row", "An session_exit"). The enum is `wrap_up | exit_session | close`. **Remedy:** use `exit_session` and `wrap_up` for rows and keep `session_exit` and `session_wrap-up` for the tools.
- D-s-2 bp, 504, 613, 683, 691, 700. **Problem:** prose `@invariant` inside entities, which the language reference allows in contracts only (lines 1394 and 1869). **Remedy:** move each to a top-level `-- Name:` prose invariant under Invariants.
- D-s-3 bp, 1766. **Problem:** `caller().sees(epic, epic.id)` is a method on a black box. **Remedy:** `sees(caller(), epic, epic.id)`, as feeds.allium:1405 writes it.
- D-s-4 bp, 2193. **Problem:** the literal identity-key set restates settings' Excludes. **Remedy:** a `settings/config` set; the module's spacetime/module/src/config.rs::IDENTITY_SETTING_KEYS holds the same four keys.
- D-s-5 mech, 2175-2177. **Problem:** a "Host-scoped setting writes" section banner in the middle of Rules. **Remedy:** `-- == Host-scoped setting writes ==` subheading.
- D-s-6 mech, 278, 755, 1257, 1331, 1335. **Problem:** over-long unwrapped comment lines left by edits. **Remedy:** reflow.
- D-s-7 bp, 1674-1675. **Problem:** `this in live_tokens` inside a collection `where`; `this` is undefined there. **Remedy:** `old_sessions where kind = wrap_up and not is_live_token`.
- D-s-8 bp, 654. **Problem:** implementation detail "(ctx.sender())". **Remedy:** "from the call (caller())".
- D-s-9 bp, 1831. **Problem (N1):** FeedRequestClaimed is emitted and no rule listens. **Remedy:** the "observable output; no rule listens" comment, or drop it.

### sync
- D-y-1 bp, 468, 1268, 1299, 1470, 1508. **Problem:** a rule shares its trigger's name (ConnectionAccepted, ConnectionAttemptFailed, ConnectionDropped, SubscribeToEpic, UnsubscribeFromEpic). **Remedy:** rename the triggers (e.g. StoreAcceptedConnection, AttemptFailed, StoreDroppedConnection, SubscribeToEpicRequested, UnsubscribeFromEpicRequested) and keep the rule names, because src/sync/connection.rs, session.rs and connector.rs cite the rule names. Update the StoreBoundary and EpicSubscriptionToggle provides, plus the emitter at 1224.
- D-y-2 bp, 193. **Problem:** `@invariant AConnectedBoardDrawsOnlyDeliveredRows` sits inside the entity, between fields; contract-only, and annotations must follow structural clauses. **Remedy:** a top-level prose invariant.
- D-y-3 bp, 906-907. **Problem:** `SubscribedRowAccepted(...) and BoardRedrawn()`: `and` joins ensures, and BoardRedrawn is a render event nobody listens to. **Remedy:** one statement per line; drop BoardRedrawn (N1: the redraw is derived).
- D-y-4 bp, 778. **Problem:** `connection.held(id)` is an undeclared entity method. **Remedy:** black box `held_row(connection, id)`.
- D-y-5 mech, 1464. **Problem:** the named-item comment cites only attempt for "a failed connection ignores every later event". **Remedy:** add src/sync/connection.rs::BoardConnection::apply.
- D-y-6 drift, 1143-1153. **Problem:** AWriteWithNoConnectionIsRefused also matches a seen mark on an unidentified, disconnected board, but code drops it before any connection check (src/store/queries/tasks.rs::mark_task_seen returns Ok with no identity). **Remedy:** `and not (mutation.is_a_seen_mark and not connection.is_identified)`, and say so in the BoardWrites guarantee (1791-1793).
- D-y-7 bp, 807-808 vs 859-863. **Problem:** RegisterHostOnConnect triggers on UserIdentitySettled, but its guidance says it is sent only after the initial subscription is applied. **Remedy:** trigger on InitialSubscriptionApplied, or drop the guidance clause (check runtime::push_host_registration's call site).
- D-y-8 mech, 1635-1639. **Problem:** "the three events it delivers … only two of them" is stale; StoreBoundary provides nine. **Remedy:** reword without a count.
- D-y-9 bp. **Problem:** history at 36-37 ("it did not have one before"), 791-794 ("built for those two first; each later table adds…"), 1880 ("for the moment") and 1884 ("today"). **Remedy:** present tense.
- D-y-10 bp. **Problem (N1):** about 12 emitted events have no listener and no comment: the eight *SubscriptionAsserted events, MutationSentToStore, MutationRefused, RowDropped, SubscribedRowAccepted, SubscribedRowsDiscarded and HostRegistrationSentToStore. **Remedy:** an "observable output; no rule listens" comment on each kept one.
- D-y-11 by-design. **Problem:** the two analyse conflicts on BoardConnection.status. **Why left:** both are false positives with an accurate comment at 1455-1464, and analyse is not gated.
- D-y-Q1 QUESTION, 1878 (an existing open question that now covers more cases). **Problem:** ConnectionAccepted resets attempts (code: src/sync/connection.rs::BoardConnection::apply, Accepted arm). A refused initial subscription, a failed adopt and a failed ensure_host_identity all go through Accepted and then Dropped, so they retry every config.reconnect_backoff_base for ever. **Options:** (a) reset attempts only on InitialSubscriptionApplied (spec plus code; **recommended**, because it bounds the hot loop the ceiling exists to prevent); (b) keep and accept the loop; (c) a separate counter for post-accept failures. Whichever is picked, delete the open question.

### spacetime-seed
- D-d-1 bp, 390, 445, 524, 872. **Problem:** a rule shares its trigger's name (TakeSnapshot, RestoreSnapshot, BurnIdSequences, ImportOldStore). **Remedy:** rename the triggers to …Requested and keep the rule names, which src cites (restore.rs, import.rs, tests). Update SnapshotCommandLine's provides and the emitters at 454 and 894. snapshot 205, 217-218 and 230 cite the rule names and are unaffected.
- D-d-2 drift, 768-777. **Problem:** SeedsNormaliseLikeMigrations listens only to RestoreSnapshot, with none of its refusal guards, yet every seed write normalises, import included (spacetime/module/src/seed.rs::seed_tasks and seed.rs::seed_learnings). It also uses an undeclared `snapshot.extract(t)`. **Remedy:** `when: TableRowsWriteRequested(table, rows)`, with `for row in rows where …` per table.
- D-d-3 drift, 679-680 and 919. **Problem:** "A restore makes the store equal the snapshot" and "recovery wants the store to equal the snapshot" contradict RestoreSnapshot 485-490 and open question 1216 (a restore deletes nothing; src/spacetime/restore.rs::restore). **Remedy:** "a restore reinstates every snapshot row" or similar.
- D-d-4 bp, 278, 405, 657 (and snapshot 513). **Problem:** the illustrations use "a watch whose task is absent", but watches are dead (P1, 221). **Remedy:** use a live dependent, e.g. a subagent entry or a work-package link.
- D-d-5 drift, 221-223. **Problem:** "nothing reads or writes" task_watchers, but the module still serves them through spacetime/module/src/views.rs::visible_task_watchers. No board subscribes. **Remedy:** reword ("no board reads them"), or remove the dead view in code.
- D-d-6 entry, 266, 871-881. **Problem:** the import's checks are bare `requires`; no rule emits a refusal for them, so RefusalReason.id_conflict is never produced and RefusalIsSilentAboutNothing has no rule behind it for import. **Remedy:** an import refusal rule (or rules) with the converse guards, emitting RestoreRefused(… id_conflict / schema_mismatch / incomplete …).
- D-d-7 bp, 710-739. **Problem:** StoreRefusesUnknownStatus listens on tasks/TaskWriteRequested only, while its guidance claims the epic writes too, which have no trigger. **Remedy:** add an epic-write trigger and rule, or state the epic check as guidance-only.
- D-d-8 bp, 71, 1106-1110 with 1128-1129, and 1124. **Problem:** history and implementation. "the duplication the whole migration exists to remove"; "A store published before that row existed" twice; "by this version of the module"; `ctx.identity()`. **Remedy:** one present-tense N2 sentence and plain wording.
- D-d-9 bp, 388-398 and 1202 vs snapshot 181-191. **Problem:** a duplicate definition: seed TakeSnapshot ("is `dump-server`") and snapshot DumpAStore (also dump-server) each create a Snapshot from their own trigger. **Remedy:** keep TakeSnapshot as the obligation, make DumpAStore consume seed's trigger for the mechanics, and drop the second Snapshot.created. The behaviour is unchanged.
- D-d-10 bp. **Problem (N1):** RestoreRefused, ImportedWithoutEpic, RowStoredWithDefaultSubStatus, RowStoredRetired, StoreWriteRefused and RowDropped have no listener and no comment. **Remedy:** the N1 comment.

### spacetime-snapshot
- D-n-1 mech, 103, 178, 268, 326. **Problem:** banners "Entities" and "Rules: …" ×3 are not canonical. **Remedy:** "Entities and Variants", and one "Rules" banner with `-- ==` subheadings.
- D-n-2 bp, 378-383. **Problem:** the stop mechanics restate startup and omit that the spawned store's whole process group is signalled (startup.allium:524; test stopping_a_spawned_store_signals_its_whole_process_group in src/spacetime/tests/managed_store_real.rs). **Remedy:** point to startup's stop_managed_store instead of restating.
- D-n-3 bp, 41-42 ("now"), 457 ("any more") and 513 ("watch"). **Problem:** history and a stale example. **Remedy:** present tense and a live example.
- D-n-4 bp. **Problem (N1):** StoreSnapshotTaken, RowWritten, StoreWriteFailed and StopSignalSent are unlistened and uncommented. **Remedy:** the N1 comment.

### Cross-cluster note (no package here)
Entity-level prose `@invariant` also appears in host (3), storage (2) and core (1). It is a set-wide pattern; decide it once (convert to top-level prose invariants) and fold it into each owning package.

## Counts
- mech 6: D-s-1, 5, 6; D-y-5, 8; D-n-1.
- bp 22: D-s-2, 3, 4, 7, 8, 9; D-y-1, 2, 3, 4, 7, 9, 10; D-d-1, 4, 7, 8, 9, 10; D-n-2, 3, 4.
- drift 4: D-y-6; D-d-2, 3, 5.
- entry 1: D-d-6.
- by-design 1: D-y-11.
- QUESTION 1: D-y-Q1.

## Work packages

### D1. store-split: rename fallout and form fixes
- **Files:** docs/specs/store-split.allium.
- **Findings:** D-s-1 to D-s-9.
- **Questions:** none.
- **Order:** none.
- **Acceptance check** (fails today, exit 1):
```bash
! grep -nE 'session_exit (or close )?(row|teardown)|session_wrap-up (rebase )?(row|request)|session_wrap-up with action|An session_exit|caller\(\)\.sees|this in live_tokens|key not in \{"host_id"|^-- Host-scoped' docs/specs/store-split.allium \
 && test -z "$(awk '/^(entity|value) /{e=1} /^}/{e=0} e && /^    @invariant/' docs/specs/store-split.allium)" \
 && ./scripts/check-allium.sh
```

### D2. sync: trigger names, form and the seen-mark gap
- **Files:** docs/specs/sync.allium. If (a) is chosen for D-y-Q1, also src/sync/connection.rs and its tests.
- **Findings:** D-y-1 to D-y-10; D-y-Q1.
- **Question:** D-y-Q1; ask before touching code.
- **Order:** none. The trigger renames are local to sync, since no other spec names sync/Connection*.
- **Acceptance check** (fails today, exit 1):
```bash
test -z "$(awk '/^rule /{r=$2} /^    when: /{ if (r!="") { t=$2; sub(/\(.*/,"",t); if (t==r) print NR": "r; r="" } }' docs/specs/sync.allium)" \
 && test -z "$(awk '/^(entity|value) /{e=1} /^}/{e=0} e && /^    @invariant/' docs/specs/sync.allium)" \
 && ! grep -nE 'connection\.held\(|and BoardRedrawn|did not have one before|three events it delivers' docs/specs/sync.allium \
 && grep -q 'BoardConnection::apply' docs/specs/sync.allium && ./scripts/check-allium.sh
```

### D3. seed and snapshot: normalisation trigger, import refusals, contradictions, dedupe
- **Files:** docs/specs/spacetime-seed.allium, docs/specs/spacetime-snapshot.allium. Optionally spacetime/module/src/views.rs (D-d-5).
- **Findings:** D-d-1 to D-d-10; D-n-1 to D-n-4.
- **Questions:** none (D-d-9 changes no behaviour).
- **Order:** within the package, do D-d-1's trigger renames before D-d-9, since DumpAStore then consumes the renamed trigger.
- **Acceptance check** (fails today, exit 1):
```bash
test -z "$(awk '/^rule /{r=$2} /^    when: /{ if (r!="") { t=$2; sub(/\(.*/,"",t); if (t==r) print NR": "r; r="" } }' docs/specs/spacetime-seed.allium)" \
 && ! grep -nE 'snapshot\.extract\(|store equal the$|store to equal the snapshot|a watch whose|watches are simply gone|^-- Rules:|^-- Entities$' docs/specs/spacetime-seed.allium docs/specs/spacetime-snapshot.allium \
 && grep -q 'reason: id_conflict' docs/specs/spacetime-seed.allium && ./scripts/check-allium.sh
```
All three checks were run from the worktree at b03f5ba0; each exits 1 today.
