# Appendix A — core, storage, settings, host, spacetime-memory-store

Condensed report of one read-only review agent for task #99212, at commit b03f5ba0 (main, 2026-11-18). Part of docs/plans/2026-11-20-99212-allium-spec-housekeeping.md. Line numbers drift as packages land: search for the cited symbol, not the line. Acceptance checks run from the worktree root.

# Appendix A (review #99212) — core, storage, settings, host, spacetime-memory-store

HEAD b03f5ba0. Read in full: core.allium 1-1902, storage.allium 1-84, settings.allium 1-170,
host.allium 1-481, spacetime-memory-store.allium 1-180, and the whole `git diff 10b68974 HEAD` of each
(memory-store: no diff). Checker (whole set, allium 3.6.1, scratch copy): core 85 diagnostics
(81 field.unused + 4 baselined missingSourceHint), storage 1 (field.unused), settings/host/memory-store 0.
Every `.rs::symbol` citation in the five specs resolves (13 checked). References read: language-reference (targeted sections), assessing-specs, the two plan sections and Appendix A/D; actioning-findings and patterns not opened. Every `<spec>.allium: Name`
citation scripted against the cited spec; the stale ones are below.

## Carried from the previous Appendix A (epic #69699)

All closed. core C1-C8, B1, B2 (epics:1989 reads `epic.children`), D1, D2, the derived-section move and the
baseline-header wording; storage S1 (HostFile.directory, baseline line gone); settings T1 (no
ReviewCommandIsASetting in docs/specs, src/, plugin/ or spacetime/module/src); host H1, H2. C3 kept as the N2 sentence
(core:1049-1053), and the sweep still runs (spacetime/module/src/lib.rs calls
spacetime/module/src/config.rs::unify_repo_verify_commands). **Open: none.**

## New field.unused (15, not 16) — triage

oldcheck.json -> check.json: +15 (12 ArchivedTask + 3 Task), -3 (Task.has_plan, Epic.has_plan,
Task.pr_number). The slice lists 12 ArchivedTask fields, not 13; the other ArchivedTask fields
(title, repo_path, tag, plan_path, external_id, created_by, owner, private_holder, id) are credited by
name collision with same-named fields read elsewhere in core.

| Field | Bucket | Evidence |
|---|---|---|
| Task.url | read in another spec | pr-workflow:504 `task.url.url_type`, store-split:1971, mcp-task-tools:648, board-layout:1355 |
| Task.left_feed | read in another spec | feeds:2179 `requires: not task.left_feed`, feeds:2011 |
| Task.default_dispatch_mode | read in another spec | agents:169, store-split:814 (matches spacetime/module/src/dispatch_request.rs::choose_dispatch_mode) |
| ArchivedTask.description | code only | src/mcp/handlers/tasks/archived.rs::archived_detail_text, ::archived_line |
| ArchivedTask.base_branch | code only | archived.rs::archived_detail_text, ::archived_line |
| ArchivedTask.url | code only | archived.rs::archived_detail_text, ::archived_line |
| ArchivedTask.created_at | code only | archived.rs::archived_detail_text |
| ArchivedTask.completed_at | code only | archived.rs::archived_detail_text |
| ArchivedTask.epic_id | code only | spacetime/module/src/archive.rs::archived_task_visible_to, ::archived_tasks_for; archived.rs::epic_label |
| ArchivedTask.epic_title | code only | archived.rs::epic_label |
| ArchivedTask.agent_name | code only | archived.rs::archived_detail_text, ::archived_line |
| ArchivedTask.workspace_id | code only | spacetime/module/src/archive.rs::archived_task_visible_to |
| ArchivedTask.archived_at | code only | archived.rs::archived_detail_text; archive.rs::archived_tasks_for (sort) |
| ArchivedTask.labels | nowhere | copied by spacetime/module/src/archive.rs::archive_record; src/sync/decode.rs::archived_task drops it (src/models/archived.rs::ArchivedTask has no labels) |
| ArchivedTask.archived_by | nowhere | written by archive_record; no reader in src/ or the module |
| storage DataDirectory.path | code only | src/lib.rs::default_data_dir; no spec expression (host:254-256 names it in black-box prose). A cross-file read would not be credited anyway |

ArchivedTask came in with ee78ad2e (2026-11-14, "specify soft delete as archived task records"). It is a
stored history record. Its only writer is tasks:1950 ArchiveDeletedTask (a write, not a read).
mcp-task-tools:1052/1147 read only `id` through `exists core/ArchivedTask{id: …}`; the other fields
appear only in @guidance output formats. So it is covered by core's header note: no reader needed
beyond A-C-1/A-C-2 and Q-A1.

## Findings

### core.allium
- A-C-1 drift core:1354, 1359-1363 — ArchivedTask has no `private` flag. A private chain with no creator has a null holder: the spec reads that as "seen by workspace members", but the code reads it as nobody (spacetime/module/src/archive.rs::archived_task_visible_to, spacetime/module/src/tables.rs::ArchivedTask `private`). -> add `private: Boolean` ("the epic chain was private at archive time"); guidance: "when private, only private_holder, nobody when null"; tasks:1950 passes `private: epic.is_effectively_private` (cross-cluster line). Better still, state it as a derived `visible_to(user)` on ArchivedTask, mirroring Task.visible_to and the code. That makes the frozen rule checkable and credits epic_id, workspace_id and owner in-file.
- A-C-2 bp core:1341-1343 — `tag: String?`, `labels: String`, `url: String?` disagree with the writer (tasks:1956-1958 passes TaskTag?, List<String>, TaskUrl?) and with src/models/archived.rs::ArchivedTask (Option<TaskTag>, Option<TaskUrl>, which keeps url_type). -> `tag: TaskTag?`, `labels: List<String>`, `url: TaskUrl?`.
- A-C-3 bp core:1318-1319 — history: "The board once had an `archived` status and removed it". -> "There is no archived status (sync.allium: RowsWithARemovedStatusAreDropped): every reader of the task table would have to skip it."
- A-C-4 bp core:1367 — dated note "Decided 2026-11-18: frozen stays." -> drop the date ("Operator decision: frozen stays.") or fold it into the preceding sentence.
- A-C-5 bp core:1240-1243 — history: "a TTL/lease design was proposed and rejected". -> present tense: "A claim never expires: the people sharing a board are collaborators, …".
- A-C-6 mech core:737 — "cost nothing today". -> "cost nothing on one machine".
- A-C-7 mech core:363 — "the stored name predates the rename; existing rows keep it". -> "the stored value for a learning_search retrieval".
- A-C-8 mech core:1187 — stale citation `sync.allium: visible_learning_verdicts` (that name is a module view, absent from sync). -> `sync.allium: SubscribeOnceIdentityIsSettled` ("rows about a held task"), or spacetime/module/src/views.rs::visible_learning_verdicts.
- A-C-9 mech core:960 — "EpicStatusRecalculation rewrites it" names prose, not a rule. -> `epics.allium: RecalculateEpicStatus`.
- A-C-10 mech core:1576 — `epics.allium: … UpdateEpic …` is not a rule. -> `EditEpic, CreateEpic, UpdateEpicViaMcp, ManagedFeedConfigViaMcp`: the rules that state the floor (epics:127, 201, 779, 996).
- A-C-11 bp core:1828 — `actor McpClient { identified_by: McpCaller … }`: no spec declares McpCaller. The checker does not validate identified_by entity names (scratch: renaming it to NoSuchEntityXyz changes nothing). It is a leftover of 299ef68a, which moved keybindings' `actor McpAgent { identified_by: McpCaller … }` into core verbatim. -> `identified_by: Host where this = local_host()`, like the other system actors. Scratch-verified: no new diagnostic, and no `facing …: core/McpClient` binding reads a field (all 16 only pass it as a trigger argument). Declaring `external entity McpCaller` adds a missingSourceHint warning and a field.unused, which would fail the gate (scratch-verified). Record the checker fact.
- A-C-12 bp core:1080-1083 — "a legacy row" plus an implementation clause ("it runs off the command drain"). -> "a row with no github_path learns its repository from the clone's origin"; drop the drain clause.
- A-C-13 bp core:130, 605, 1477, 1511, 1517 — bare code names `TaskTag::is_review`, `Task::sort_key`, `Staleness::from_age`, `MAX_EPIC_DEPTH`, `MAX_ANCESTOR_WALK`. -> path::symbol: src/models/tasks.rs::is_review, src/models/tasks.rs::sort_key, src/models/tasks.rs::from_age, spacetime/module/src/tasks_epics.rs::MAX_EPIC_DEPTH, spacetime/module/src/rules.rs::MAX_ANCESTOR_WALK.
- A-C-14 mech core:697-698 — sentence fragment "The store's spacetime/…::choose_dispatch_mode." (the code matches the derivation). -> "The store holds it (…::choose_dispatch_mode)."
- A-C-15 mech core:384, 711, 1603, 1642 — over-long lines left by edits. -> rewrap.
- A-C-16 bp core:1047-1083 — SavedRepoPath's field comments carry write and authority rules (save/adopt/delete refusals, the reach of set-verify), but core's scope excludes rules. -> move them to an @guidance on SavedRepoPath, or to the spec that owns those reducers, leaving one line per field. Low priority.
- A-C-19 bp core:1015, 1020 — `subtree_tasks: Task with epic = this or …` and `subtree_epics: Epic with parent = this or …` (added as the C5 fix) put a compound predicate in `with`. The language reference defines `with X = this` only as the back-pointing field of a relationship. -> write them as black boxes (`tasks_below(this)`, `epics_below(this)`, next to descends_from), or as a `where` projection over a declared relationship. Check the result on scratch.
- A-C-17 by-design — the "Interval literals" and "Storage Boundary Validation" sections (kept on purpose last review); RetrievalSource.procedural and LearningStatus.rejected, "stored rows may hold it" (data compatibility, N2-like); the "ready" alias at core:182 (src/models/tasks.rs `#[serde(alias = "ready")]`); the min_feed_interval floor "in the epic service", which matches src/service/mod.rs.
- A-C-18 by-design — prose `@invariant` inside entities (Host, HostFile, DataDirectory). The language reference lists it for contracts, but the checker accepts it and it is a repo-wide pattern, so not raised here.

Cross-cluster (tasks.allium, cluster F, not filed here): tasks:1967 `task.display_agent_name` is not a field (core has `display_name`); tasks:1969 `workspace_id: epic.workspace` assigns a Workspace to an Integer -> `epic.workspace.id`. Fix both with A-C-1/A-C-2.

### storage.allium
- A-S-1 by-design storage:58 — DataDirectory.path field.unused: code only, and a cross-file read is never credited. Leave.
- A-S-2 by-design storage:48-50 — "An older install may also hold a tasks.db": a present-tense fact the CodeNeverTouchesLegacyDatabase invariant needs.

### settings.allium
- A-T-1 bp settings:78, 92, 107 — rules SaveSetting, ClearSetting and SetManagedFeedSettings share their triggers' names. -> rename the triggers (SaveSettingRequested, ClearSettingRequested, ManagedFeedSettingsRequested) in the rules and in the SettingWrites `provides` (138-140), the example at :129, and the 9 qualified emitters: tasks ×3, board-layout ×2, agent-health, epics, mcp-task-tools (`rg 'settings/(SaveSetting|ClearSetting|SetManagedFeedSettings)[(]' docs/specs`). Then grep src/ for the old names.
- A-T-3 bp store-split:2193 (cross-cluster D) — the literal set {"host_id", "host_label", "user_identity", "user_identity_token"} restates settings' Excludes prose. -> declare `identity_setting_keys` in settings' config and read `settings/config.identity_setting_keys` in store-split.
- A-T-2 bp settings:24 — bare reducer name "(save_setting)" on an unwrapped 130-character line. -> "(store-split.allium: SaveSettingForHost)", then rewrap. Also rewrap :102.

### host.allium
- A-H-1 bp host:262-263 — `rule RenameHost` listens on `startup/RenameHost(label)`, so the rule shares its trigger's name. -> rename the trigger in startup (e.g. `HostLabelSubmitted(label)`; emitter startup:1339) and the consumer here.
- A-H-2 mech host:236, 283-284 — a long line and a broken wrap left by edits. -> rewrap.
- A-H-3 by-design — the empty Surfaces section has an explanatory comment. HostFileWritersAreSerialised matches src/host_file/mod.rs::with_write_lock; the mint race matches d4ad2085.

### spacetime-memory-store.allium
- A-M-1 mech ms:22 — `use "./store-split.allium" as split` is never referenced, but it is load-bearing: dropping it raises missingSourceHint for SpacetimeModuleReducers and TestSuite (scratch-verified). -> add a one-line comment saying it names the spec that governs SpacetimeModuleReducers. Do not drop it.
- A-M-2 mech ms:126 — no rule listens to `BoardReadsServed`. -> N1 comment: "observable output; no rule listens".
- A-M-3 mech ms:87 — the section header "Entities". -> "Entities and Variants".
- A-M-4 bp ms:19, 132 — module-path citations `spacetime::snapshot_target::MemoryStore` and `sync::rows::SharedRows`. -> src/spacetime/snapshot_target.rs::MemoryStore, src/sync/rows.rs::SharedRows.
- A-M-5 bp ms:137-138 — "There is no SQLite-backed BoardReads." is SQLite-era negation (decision 4). -> drop it; storage.allium: NoSqlite covers it.
- A-M-6 by-design — TestDatabaseHandle.shared_store_attached is always true, and the spec names implementation parts (Store::open_in_memory, crate-type) because it is a spec of test infrastructure. Every name was checked and exists.

Counts: drift 1, bp 15, mech 11, entry 0, by-design 6, QUESTION 1.

## Questions

- Q-A1 (core:1342, 1356) — ArchivedTask.labels and .archived_by are written and never read, by the code included: the decoded model drops labels and nothing shows archived_by. Options: (a) keep both as stored-only history, with a field comment "kept, not shown"; (b) show them in task_search's archived detail (a code change); (c) drop the columns (a module migration). **Recommend (a)**: the record's purpose is keeping history whole, and (b) can follow on demand.

## Work packages

**P-A1 — core.allium: ArchivedTask fidelity, stale citations, history** (core.allium, plus tasks.allium:1950-1971 for the writer)
Findings: A-C-1 … A-C-16, A-C-19, Q-A1, and the two cross-cluster tasks lines. Ask Q-A1 first.
Ordering: the tasks.allium lines overlap cluster F's tasks package, so land either one first and merge the other; both are small.
Acceptance (fails today with 12 matches and no `private: Boolean`; passes when done):
```
! rg -n 'once had|Decided 20[0-9]{2}|was proposed and|sync\.allium: visible_learning_verdicts|EpicStatusRecalculation|UpdateEpic,|identified_by: McpCaller|TaskTag::is_review|Task::sort_key|cost nothing today|predates the rename|with epic = this or' docs/specs/core.allium && rg -q '^\s+private: Boolean' docs/specs/core.allium
```

**P-A2 — settings + host: rule/trigger name split, citation tidy** (settings.allium, host.allium, startup.allium:1339, and the emitters in tasks, board-layout, agent-health, epics and mcp-task-tools)
Findings: A-T-1, A-T-2, A-T-3 (with store-split:2193), A-H-1, A-H-2. No question.
Ordering: it touches emitters in specs other clusters own (startup: E; tasks, epics, mcp-task-tools: F; board-layout: G; agent-health: B). Run it after those packages land, or as one commit at the end, rebased onto them. The renames are mechanical: rename, then grep for old names.
Acceptance (fails today on host:262, settings:78, 92, 107 and settings:24):
```
! rg -n -U -P 'rule (\w+) \{\n(\s*--.*\n)*\s+when: (\w+/)?\1\(' docs/specs/settings.allium docs/specs/host.allium && ! rg -n '\(save_setting\)' docs/specs/settings.allium
```

**P-A3 — spacetime-memory-store tidy** (spacetime-memory-store.allium; storage needs no edit)
Findings: A-M-1 … A-M-5. No question, no ordering constraint.
Acceptance (fails today on ms:19, 87, 132, 137 and the missing comment):
```
! rg -n 'SQLite-backed|spacetime::snapshot_target|sync::rows::|^-- Entities$' docs/specs/spacetime-memory-store.allium && rg -q 'no rule listens' docs/specs/spacetime-memory-store.allium
```

Every package's footer: `./scripts/check-allium.sh` (no new warning, no new field.unused), then
`allium:weed` on the package's specs.

New checker fact (record it): `actor … identified_by: X where …` is not validated, so an undeclared X
passes silently; and a `where` field read inside identified_by is not credited (field.unused stays).
