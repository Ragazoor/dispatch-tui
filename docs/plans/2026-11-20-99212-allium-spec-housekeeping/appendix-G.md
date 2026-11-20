# Appendix G — workspaces, admin-surfaces, board-layout, board-visuals, keybindings

Condensed report of one read-only review agent for task #99212, at commit b03f5ba0 (main, 2026-11-18). Part of docs/plans/2026-11-20-99212-allium-spec-housekeeping.md. Line numbers drift as packages land: search for the cited symbol, not the line. Acceptance checks run from the worktree root.

# Appendix G — workspaces, admin-surfaces, board-layout, board-visuals, keybindings (review #99212, HEAD b03f5ba0)

Checker: whole-set `allium check` (3.6.1) gives 0 diagnostics for all five; no baseline entries. Diff 10b68974..HEAD read for all five: every removed code line; removed comment lines in full for board-layout, board-visuals, keybindings and admin-surfaces; for workspaces the removed rules (in full) and the tail from WritesRequireSight on, with the remaining removed comment-only lines covered by reading the current text in full.

Read in full: workspaces 1-1330; admin-surfaces 1-677; board-layout 1-1712; board-visuals 1-1570; keybindings 1-986. References: assessing-specs, actioning-findings in full; language-reference 3-108, 633-655, 818-836, 1033-1080, 1127-1166, 1233-1290, 1694-1791, 1904-2120 read; the rest of language-reference not read; patterns.md headings only.

Spec-vs-code checks run (not a weed pass): KeyNamespace/KeyContext vs `src/keybindings.rs`; every cited module/board symbol and test name (all exist); revoke idempotency (store + `src/service/people.rs::set_super_admin`); the admin text mode (`InputMode::InputAdminText`); `set_work_package_visibility`; `check_private_has_creator`; 169 cross-spec `x.allium: Name` citations (declaration-checked).

Checker-fact correction (for package 0): the #98956 fact "qualified collections as a method receiver raise reference.unknownName" does not reproduce here. workspaces 583/602/612 (`core/Workspaces.any`, `wp/WorkPackages.any`) are clean today. On a scratch copy of admin-surfaces with `use "./work-packages.allium" as wp` added, `workspaces/UserProfiles.any(...)`, `core/Workspaces`, `wp/WorkPackages` and even `core/NoSuchThings` (edits confirmed by grep) kept admin-surfaces at 0 diagnostics and the set total at 102. 3.6.1 validates no qualified collection name in these positions.

## Carried from the 98983 Appendix G

| Item | Status at HEAD |
|---|---|
| W orphan `*Allowed` / W-Q1 | resolved: option (b), `contract StoreWriteChecks` 97-378 |
| W1, W2 | resolved (246 "D6", 254 "D6") |
| W3 | resolved (no task/learning numbers) |
| W4 | resolved (black boxes, 416-423) |
| W5 | resolved (`core/` throughout) |
| W6, W7, W8 | resolved (moved into contract invariants; 704-705, 730-731 cross-reference them) |
| W9 | by-design, still (MigrateStoreToWorkspaces 1042-1091) |
| W10 | resolved (SetOwnNameOnCommit 480) |
| A1 | resolved (113) |
| A2 | partly: Dependencies rewritten but now names WorkPackage as core's (G-A-1) |
| A3 | resolved (AdminTextCommitted/AdminConfirmed, 9 commit rules 277-377) |
| A4 | resolved (N1 comments 101, 120, 131-133, 384, 397) |
| L1 | mostly resolved; residue at 175-182, 758-764 (G-L-4, G-L-5) |
| L2, L6 | resolved |
| L3 | closed by-design: `filter(coll, x => p)` is the language reference's own black-box example; justified at 1393-1397 |
| L4 / L-Q1 | resolved: option (b), 1655-1712 |
| L5 | resolved (1658-1665); wording nit G-L-7 |
| L optional (gg chord 137-159, column nav 99-120, selection preservation 1084-1104 prose only) | still prose only (G-L-10) |
| V1, V2 | resolved (195-201, 1261-1265; no "No test covers") |
| K1, K2, K4, K5, K6, K7 | resolved |
| K3 (code) | resolved: `src/keybindings/table.rs` jump_to_agent text names resume |

Open from last appendix: A2 (partial), L1 residue, L optional, W9 (by-design).

## Findings

### workspaces.allium
- G-W-1 bp 446-947: 15 rules share their trigger's name (SetDisplayName, ClaimSuperAdmin, AdoptOwnerlessUserBoardTasks, GrantSuperAdmin, RevokeSuperAdmin, CreateWorkspace, RenameWorkspace, DeleteWorkspace, AddMember, SetMemberRole, RemoveMember, SetEpicVisibility, MoveEpicToWorkspace, SetWorkPackageVisibility, SuperAdminDeletesUnseenItem) -> rename the triggers (`…Requested`). Keep the rule names, because src/ comments cite them. Update the surfaces' provides (StoreReads, WorkspaceAdministration, SuperAdministration) and the emitters: admin-surfaces (20 sites), epics.allium 903/906 and work-packages.allium 189.
- G-W-2 mech 416-423: the black-box list leaves out is_super_admin, item_is_private, item_holder, item_workspace, item_kind, scope_item, recording_workspace, known_identities, store_publisher, ever_claimed_super_admin and link_item_visible -> list each in one line.
- G-W-3 bp 827: `learning.item_in(epic.subtree)` is an undeclared entity method -> use a black box `learning_item_in(learning, epic.subtree)` (and list it).
- G-W-4 mech 916: the `WorkspaceAndVisibilityStepShown` emission has no N1 comment -> add "observable output; no rule listens."
- G-W-5 mech 704: the requires reads `caller() != null`, but the contract (101-102) says the rule states the same check as `epic.created_by != null` -> write `epic.created_by != null`.
- G-W-6 mech 19-21: Dependencies omits epics.allium, whose DeleteEpic is emitted at 961 -> add it.
- G-W-7 bp 108-377, 754, 1069-1072, 1194-1195: about 40 bare code names ("Module: check_epic_move", `on_sight_lost`, `rules.rs: WORKSPACES_RULES_VERSION`, `models::shown_name`). All exist (verified), but the convention is `path::symbol`, and only that form is checked by check-doc-symbols -> write `spacetime/module/src/visibility.rs::check_epic_move` etc., or drop the ones no reader needs (Decision 4). The same ruling applies to G-A-6, G-L-9 and G-K-9.
- G-W-8 by-design 1281: `surface StoreWriteChecks` shares its name with the contract it fulfils. The checker accepts this and the pairing is intended.
- G-W-Q1 QUESTION 1326 (open question): should the TUI epic editor's WORKSPACE section become editable? (a) make it editable, listing the workspaces the person administers; (b) keep the move MCP-only. Recommend (b): keep the open question until a board move is wanted.
- G-W-Q2 QUESTION 1328: should a creatorless package be refused as private? `spacetime/module/src/visibility.rs::set_work_package_visibility` does not check. (a) refuse it, mirroring EpicIsPrivateOnlyWithACreator; (b) allow it (it becomes unseen and is listed for the hatch). Recommend (a): it is cheap and symmetric with epics, and only legacy rows are affected.
- G-W-Q3 QUESTION 1330: the guarantee's "PR poll state" (1172). Recommend: answer "nothing else" and reword it to "the task's PR fields and poll-owner rows". Then drop the open question.

### admin-surfaces.allium
- G-A-1 mech 21-23: Dependencies says WorkPackage comes from core.allium; it is in work-packages.allium -> fix, and add `use "./work-packages.allium" as wp`.
- G-A-2 mech 238, 404, 554, 558, 580, 612, 614: unqualified collections `UserProfiles`, `Workspaces`, `Epics`, `WorkPackages` -> `workspaces/UserProfiles`, `core/Workspaces`, `core/Epics`, `wp/WorkPackages` (with the new `use`). Scratch-tested: no new diagnostic.
- G-A-3 mech 280-536: black boxes trimmed (workspaces'), item_named, unseen_items_for and visible_workspaces are not declared here -> add one comment block listing them.
- G-A-4 by-design 102, 142-256: the purpose and pane literals (new_workspace_name, workspaces_pane, …) have no enum. No language form types a trigger parameter, so an enum would be definition.unused (#98983 checker fact).
- G-A-6 bp 68, 603: bare code names `models::resolve_person`, the `unseen_items` view -> `src/models/people.rs::resolve_person` and `spacetime/module/src/visibility.rs::unseen_items` form (G-W-7 ruling).
- G-A-5 by-design 361-364, 500-503: the store refuses revoking a non-super-admin (`spacetime/module/src/workspace.rs::revoke_super_admin`), and the board maps that refusal to success (`src/service/people.rs::set_super_admin`), so "changes nothing and succeeds" holds for both the key and the tool.

### board-layout.allium
- G-L-1 bp 1527, 1536, 1544, 1568: four rules share their trigger's name (CycleLensForward, CycleLensBackward, ToggleHiddenEpic, ToggleHiddenWorkPackage) -> rename the triggers (`…Pressed`) in the rules and in BoardLensKeys/BoardFilterOverlay.
- G-L-2 bp (stale citation) 1409-1410: "core.allium's BoardSelection"; it is declared at tasks.allium:249 (core.allium:11 says so) -> "tasks.allium's".
- G-L-3 by-design 96-97: "tasks.allium: DeleteKeyRouting" names a labelled passage on the Board surface (tasks.allium 2600), the same case as UnseenIsAsMissing. It is not a phantom.
- G-L-4 bp history 175-182: "Placing its card by the epic's own recorded status picked one of them … read as 'running'" narrates the old design -> present tense ("placing by recorded status would hide …").
- G-L-5 bp (N2) 758-764: a 7-line passage on legacy per-column folds -> one present-tense sentence while the code still reads that format.
- G-L-6 bp 922-924, 969-970: "a live board holds about 150 epics, nested up to five levels" is a dated snapshot -> state the reason without counts.
- G-L-7 mech 1660-1661 (and keybindings 148): "the admin screen's name input". One InputAdminText mode serves all four admin prompts (`src/tui/update/admin.rs::open_admin_prompt`) -> "the admin screen's text prompt".
- G-L-8 bp banners: prose banners (Board Columns … Cursor Selection Preservation, 37-1104) come before Value Types, and "Board View" / "Other Entities" are not canonical -> rename to "Entities and Variants". Demote the prose banners to `-- ==` subheads in one leading description block.
- G-L-9 bp 1708-1709: `src/tui/text_caret.rs: insert, delete_before, …` -> path::symbol form (`src/tui/text_caret.rs::insert`, …). The symbols exist.
- G-L-10 by-design (optional entry, carried): the gg chord, column navigation and selection preservation are prose only. They are UI mechanics, already keyed in keybindings rows. Leave unless the user wants rules.
- G-L-11 by-design 1392-1402: `@guidance` inside the BoardView entity, listing black boxes. It is last, as required.

### board-visuals.allium
- G-V-1 mech 625, 1454: `config.message_flash_ttl` has no local config; its owner is core.allium:1409 -> `core/config.message_flash_ttl`.
- G-V-2 bp 1173: "against today's implementation" -> present tense ("the renderer").
- G-V-3 bp 1028-1029: "spends forty lines" is a stale count -> drop the number.
- G-V-4 bp 976-991, 941-950: restate 203-220 and 157-187 almost word for word -> keep one copy and cross-reference. Optional: the header (27-30) keeps rationale beside constructs on purpose.
- G-V-5 mech 1111-1113: banner "Entities" -> "Entities and Variants". The leading prose banner "Column Identity and Focus" (35-803) is G-L-8's case.
- G-V-6 by-design: the five cited tests exist (`src/tui/tests/rendering/palette_and_frames.rs`, `src/tui/ui/kanban/cards/tests.rs`), and the checker accepts several `context` clauses per surface.

### keybindings.allium
- G-K-1 bp 586: `KeyBindings where this.namespace = namespace`: a `where` predicate must not reference `this` (language-reference "with and where"; validation rule 3) -> bind the trigger parameter as `ns` and write `where namespace = ns`.
- G-K-2 bp 588, 591: `.first` on an unordered collection (validation rule 58: a warning, an error next version) -> use a black box `the_only(applicable where …)`, justified by ContextsDoNotOverlapForAKey.
- G-K-3 bp 681-683: `requires: if namespace != null: …` has no else -> `requires: namespace = null or key_namespace_is_valid(namespace) or key_family_is_valid(namespace)`.
- G-K-4 bp 691: "about 44 KB" is a dated size -> "too large for one reply".
- G-K-5 mech 148: "the admin name input" -> "the admin screen's text prompt" (with G-L-7).
- G-K-6 mech: black boxes key_has_ctrl_or_alt, key_namespace_is_valid, key_family_is_valid and contexts_overlap are not listed -> one comment block.
- G-K-7 by-design 741-762: KeyUniquePerContext is implied by ContextsDoNotOverlapForAKey. TableIsWellFormed names both; leave it.
- G-K-8 by-design: KeyNamespace (40) and KeyContext (27) match `src/keybindings.rs` exactly.
- G-K-9 bp 122, 840-842: bare code names `JUMP_BACK_KEY`, `AGENT_TREE_TOGGLE_KEY`, `ObservedEvent::PaneKey`, `record_usage_event_logged` -> `path::symbol` (`src/runtime/mod.rs::JUMP_BACK_KEY`, …) per the G-W-7 ruling.

Counts: mech 12, bp 19, entry 0, drift 0, by-design 9 new (+2 carried: W9, L3; L-optional is G-L-10), QUESTION 3.

## Work packages

**G1 — workspaces + admin-surfaces housekeeping.** Files: docs/specs/workspaces.allium, docs/specs/admin-surfaces.allium, plus the three emitter lines in epics.allium (903, 906) and work-packages.allium (189). Findings: G-W-1..7, G-A-1..3, G-A-6. Questions: G-W-Q1..Q3 (ask before touching the open questions or adding a package creator check). Ordering: G-W-1's trigger renames edit epics.allium and work-packages.allium lines that other clusters' packages may also edit. Run G1 after those packages, or hand the three lines to them. Do the workspaces renames and the admin-surfaces emitters in one commit.
Acceptance (run from the worktree root; each clause fails today on its own, exit 1):
```
awk '/^rule /{r=$2} /^[[:space:]]*when:/{t=$2; sub(/\(.*/,"",t); sub(/.*\//,"",t); if(t==r){print FILENAME": "r; bad=1}} END{exit bad}' docs/specs/workspaces.allium \
 && ! grep -nE '^[^-]*[^/a-z_](UserProfiles|Workspaces|Epics|WorkPackages)\b' docs/specs/admin-surfaces.allium \
 && ! grep -n 'WorkPackage from core.allium' docs/specs/admin-surfaces.allium \
 && grep -B3 -n 'WorkspaceAndVisibilityStepShown(' docs/specs/workspaces.allium | grep -q 'no rule listens'
```

**G2 — board-layout + board-visuals housekeeping.** Files: docs/specs/board-layout.allium, docs/specs/board-visuals.allium. Findings: G-L-1, G-L-2, G-L-4..9, G-V-1..5. No questions. Ordering: none inside the cluster. G-L-7 shares wording with G-K-5.
Acceptance (run from the worktree root; each clause fails today on its own, exit 1):
```
awk '/^rule /{r=$2} /^[[:space:]]*when:/{t=$2; sub(/\(.*/,"",t); sub(/.*\//,"",t); if(t==r){print FILENAME": "r; bad=1}} END{exit bad}' docs/specs/board-layout.allium \
 && ! grep -nE 'saved before folds were per section|picked one of them|today.s implementation|(^|[^/])config\.message_flash_ttl' docs/specs/board-layout.allium docs/specs/board-visuals.allium \
 && ! grep -n -A1 "core.allium's\s*$" docs/specs/board-layout.allium | grep -q BoardSelection
```

**G3 — keybindings expression fixes.** Files: docs/specs/keybindings.allium. Findings: G-K-1..6, G-K-9. No questions.
Acceptance (run from the worktree root; each clause fails today on its own, exit 1):
```
! grep -nE 'where this\.|\)\.first\b|about 44 KB' docs/specs/keybindings.allium \
 && ! grep -n -A2 '^    requires:$' docs/specs/keybindings.allium | grep -qE '^[0-9]+-\s+if '
```

Every package footer: whole-set `./scripts/check-allium.sh` with no new warning; `allium:weed` on the package's specs; merge main before wrap-up.
