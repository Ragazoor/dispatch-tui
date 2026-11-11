# Testing

Everything about running, writing, and placing tests in this repo. `CLAUDE.md`
keeps only the facts that change what you type at the prompt or how you wait
for it (the suite needs `tmux`; never pipe `cargo test` into `tail`; run it in
the foreground, with an explicit timeout on a fresh worktree's first run);
the rest lives here.

## Running tests

```bash
cargo test                                # full suite
cargo test store::tests                   # the Store router and its ports
cargo test service::                      # domain service layer
cargo test tui::tests                     # TUI input/message handling
cargo test mcp::handlers::tests           # MCP JSON-RPC handlers
cargo test --test lifecycle               # integration: full task lifecycle
cargo test --test epic_lifecycle          # integration: full epic lifecycle
cargo test --test cli                     # CLI subcommand smoke tests
cargo test tui::tests::scenarios          # key-sequence integration tests
cargo test tui::tests::snapshots          # ratatui buffer rendering tests
cargo test --test tmux_lifecycle          # real tmux: window/pane topology and cwd
cargo test --test tmux_split_hook         # real tmux: split-pane cwd, and that nothing is typed at a pane
cargo test --test tmux_window_targets     # real tmux: window-name resolution
cargo test --test tmux_diff_pane          # real tmux: agent-tree diff pane geometry, toggle target
```

That is a **selection**, not the full inventory — `tests/` also holds
`active_health.rs`, `caller_identity.rs`, `ci_gates.rs`,
`dispatch_status_lifecycle.rs`, `feed_sync.rs`, `githooks.rs`,
`managed_feeds.rs`, `trajectory.rs`, and `verify_command.rs`.
`ls tests/` is the authority; this list is a shortcut for the targets you reach
for most.

**A name filter proves a test goes red; it does not prove the test is worth keeping.** The mutation check below ("run any new assertion against the unfixed code first") is only sound when the mutated run is *unfiltered*. Break the code, run `cargo test <one_module>`, and a redundant test still goes red — you have learned that your test works, not that anything needed it. A sibling module asserting the same thing stays invisible, because you filtered it out. Run the mutation unfiltered, and read which tests fail: if yours is not the only one, you are writing a duplicate. #4737 added a "total coverage" test on this reasoning and had to delete it — `every_valid_sub_status_names_one_section` in a neighbouring module of the same file had made the claim all along.

**The full suite needs `tmux` on `PATH`.** The `--test tmux_*` targets drive a real tmux server (private `-L` socket, `-f /dev/null`, drop-guard teardown — see `tests/tmux_harness/mod.rs`). Without tmux they print `skipping: tmux not available on PATH` and pass, so a green local run isn't proof they ran; CI hard-fails instead of skipping (tmux is installed in both the `test` and `coverage` jobs).

**A sandboxed `cargo test` fails these targets for a reason that isn't your code.** The harness's tmux socket lives under `/tmp/tmux-$UID/`, outside the sandbox's write allowlist, so every `tmux_*` test panics with `error connecting to /tmp/tmux-1000/dispatch-test-… (Operation not permitted)` while the rest of the suite passes. That is the sandbox, not a regression — re-run just those targets with the sandbox disabled (or `/sandbox` to adjust the allowlist) before you go looking for what you broke. Note the skip above only covers a *missing* binary; this is the installed-but-unreachable case.

The sandbox has a second, less legible failure mode: a plain `grep`/`cargo` invocation dying with `apply-seccomp: unshare(CLONE_NEWUSER): Invalid argument` and **no** output. That string names neither the sandbox nor a path, but it is the same diagnosis — re-run the command with the sandbox disabled. It fires intermittently on commands that are otherwise perfectly sandbox-safe, so don't go hunting for which path you touched.

**Don't pipe `cargo test` into `tail`/`head`/`grep`.** A shell pipeline's exit code is the LAST command's, so `cargo test | tail -40` reports `tail`'s exit status (always 0) — combined with a truncation that happens to cut the summary lines, a failing suite reads as a clean pass. Redirect instead: `cargo test > /tmp/t.txt 2>&1; echo $?`, then `grep -E "^(test result|failures:)" /tmp/t.txt`.

Pick that redirect target with care under the sandbox: bare `/tmp` is **not** writable, and neither `$TMPDIR` nor the session scratchpad path is reliably expanded in a sandboxed shell (both fail as `Permission denied` / `No such file or directory` *before* the suite runs, which looks nothing like a sandbox problem). `mkdir -p /tmp/claude-1000/<something>` first and redirect there, or disable the sandbox for the run.

**Warm, `cargo test --no-fail-fast` takes ~60s — run it in the foreground.** The
lib target is ~5s (5,926 tests) and `tests/spacetime_module.rs` ~45s. In a fresh
worktree the first run, compile included, took ~6 minutes, which passes Claude
Code's default 120s Bash timeout. (Measured 2026-10-15: 22 cores, other agents
building, warm `sccache`, `spacetime` on `PATH`; without `spacetime` the
live-store tests skip.) A warm run used to take 5 minutes, which is why so much of the
advice you may have seen elsewhere is about managing a backgrounded run. Don't
background it: the Bash tool's
<!-- allow-phantom-symbol: names the harness's Bash tool parameter, not a code symbol -->
`run_in_background` option buys nothing at this length and costs you the
failure modes below.

Every database-touching test runs over the in-memory store, which builds no
schema. Incremental compilation after a one-file edit (~13 s) is now the larger
half of an edit→test cycle, so if a run feels slow, it is the compile.

The dev profile uses `debug = "line-tables-only"` (`DevBuildsKeepLineTablesNotFullDebugInfo` in `docs/specs/dispatch.allium`). That cut a one-file rebuild from ~50s to ~29s. A *fresh* worktree still pays a cold build (~2.5 min for the build alone; the ~6 minutes above includes the first run) and its first `tests/spacetime_module.rs` run builds the wasm module (~40s); later runs reuse the cache.

If you do background a `cargo` command anyway, two things, both learned the hard
way. **Don't run any other `cargo` command while one is backgrounded** —
`cargo fmt`/`cargo clippy` contend for the same `target/` lock, and a run killed
mid-suite for "no reason" is usually another process holding it (`ps aux | grep
cargo`). And **judge a background run only by its completion notification**: its
redirected output file lags behind (a finished run can still look mid-suite),
and `pgrep` races the teardown, so neither "the tail stopped moving" nor "no
process found" is evidence it died.

Suite is green; if a runtime test fails locally, suspect timing — `spawn_blocking`-based tests are timing-sensitive.

## The in-memory store

`Store::open_in_memory()` — the constructor behind essentially every
DB-touching test — is the production `Store` over a fresh, private in-process store: one
`SharedRows` and one `MemoryReducerCaller` that runs the module's own reducer
logic over it. Two handles never share rows, and a handle's host file lives in
a temporary directory removed with it. Nothing builds a schema, so a handle
costs microseconds.

There is no SQLite, and no handle without a store: every part of a `Store` is
required. A test whose subject is the host file itself uses
`Store::in_memory_with_host_file(dir)`; a fault it needs to inject (an
unreadable host file) is injected with a replaced `host.json`. Reads answer
from in-memory rows and are infallible by signature (no `Result`), so a read-error arm does not
exist; a write fault is injected with a refusing
`ReducerCaller` (see `RecordingCaller` in `src/store/tests/over_sync/writes.rs`).

## Snapshot tests

Snapshot review needs the `cargo insta` subcommand, which is **not** a workspace
dependency — install it once with `cargo install cargo-insta`, or use the
`INSTA_UPDATE=always` form below, which needs nothing extra.

Snapshots live in `src/tui/tests/snapshots/` and render to a 120×40 `TestBackend`. **Do not change the backend size** — it breaks all existing diffs.

Agent prompt snapshots live in `src/dispatch/snapshots/` and lock the rendered output of every `build_*_prompt` variant. `src/dispatch/prompts/` holds the two review addenda as markdown (`pr-review.md`, `dependabot.md`, inlined via `include_str!`), plus `dependabot/`, whose fragments the dependabot addendum is assembled from — the snapshot pins one assembly, so the per-route exclusivity is asserted separately in `a_dependabot_prompt_carries_only_the_branch_its_bump_takes`. The dispatch, quick-dispatch, and research bodies are string-built in `src/dispatch/prompts.rs`.

To accept intentional UI changes:

```bash
cargo insta review                                  # interactive
INSTA_UPDATE=always cargo test tui::tests::snapshots # auto-accept
INSTA_UPDATE=always cargo test dispatch::prompts_snapshots # auto-accept prompt snapshots
rm src/tui/tests/snapshots/*.snap.new                # always clean up
rm src/dispatch/snapshots/*.snap.new                 # always clean up
```

**Don't skip the `rm *.snap.new` cleanup.** A stray `.snap.new` left in the tree is picked up by the next `cargo insta review` and silently mixed into an unrelated review pass, making it easy to accept the wrong diff. Always remove them once you've accepted (or rejected) a change.

## Where new tests go

| What you're testing | Where |
|---|---|
| TUI key handling / message flow | `src/tui/tests/` |
| The `Store` router, its ports, CRUD over the in-memory store | `src/store/tests/` |
| A `Store` driven over the sync transport (its reducer calls, identity, subscriptions) | `src/store/tests/over_sync/` — `sync` itself may not name `store`, its tests included (`tests/module_layering.rs`) |
| The sync session, connection state machine, decoders and encoders | `src/sync/tests/` |
| A SpacetimeDB module change (table, column, reducer) | `spacetime/module/` tests, `spacetime/module-tests/tests/` and `tests/spacetime_module.rs`; mirror it in `src/sync/memory_caller/` |
| Service-layer business rules | inline in `src/service/<domain>/`, or a sibling `tests` file once past ~200 lines (`src/service/learnings/learning_tests.rs`) |
| MCP JSON-RPC handler behaviour | `src/mcp/handlers/tests/` |
| Full task/epic lifecycle | `tests/` (integration tests) |
| A `Task` fixture | `TaskBuilder` (`src/models/task_builder.rs`, gated like `test_tmux_window`) — don't hand-roll another `Task { .. }` literal |
| An `Epic` fixture | `EpicBuilder` (`src/models/epic_builder.rs`, gated the same way) — don't hand-roll another `Epic { .. }` literal |
| A `CreateTaskRequest`, `CreateTaskParams` or `CreateEpicParams` | `::fixture(..)` (`src/store/fixtures.rs`, `src/service/fixtures.rs`), with only the differing fields spelled out: `CreateTaskRequest { plan: Some("p.md"), ..CreateTaskRequest::fixture("T", "/repo") }` |
| Domain-type invariants | inline in the owning module, or a sibling `tests` file once past ~200 lines (`src/models/epics/tests.rs`) |
| Agent prompt rendering (all variants) | `src/dispatch/prompts_snapshots.rs` |
| Agent-facing skill copy (`plugin/skills/*/SKILL.md`) | `src/setup/plugins/tests.rs` (via `skill_body`) |
| tmux semantics — which pane, which cwd, how many panes, which window a name resolves to | `tests/tmux_lifecycle.rs` (topology/cwd) / `tests/tmux_split_hook.rs` (split-pane cwd and keystroke absence) / `tests/tmux_window_targets.rs` (exact window-name resolution under prefix collisions) / `tests/tmux_diff_pane.rs` (agent-tree diff pane geometry, and which panes the toggle kills), shared rig in `tests/tmux_harness/mod.rs` |
| tmux argv shape — that we sent the right command string | `MockProcessRunner` tests inline in `src/tmux.rs` |
| Anything that drives a dispatch/resume/provision/finish through a mock | wherever the behaviour lives, but script the runner with `DispatchScript` (`src/dispatch/mock_sequence.rs`) — never a hand-written `vec![ok(), ok(), …]`. `DispatchScript::finish()` covers the `finish_task` rebase path, including anything that reaches it through `wrap_up` |
| A `pub(in crate::tui::ui)`-or-narrower helper (unreachable from `src/tui/tests/`) | inline in the owning module, e.g. `staleness_color`/`feed_role_label` in `src/tui/ui/shared.rs`, `budget_spans` in `src/tui/ui/budget.rs` |

A test that builds a **real git repo** on disk must run git with a sanitised
environment — `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_SYSTEM=/dev/null`, and
the four `GIT_AUTHOR_*`/`GIT_COMMITTER_*` identity vars. Without them the
fixture is whatever the developer's global config makes it (`commit.gpgsign`,
`core.hooksPath`, `init.templateDir` all change what gets built), and on a
machine with no configured `user.email` — a fresh CI container — the seed commit
fails outright. Use `git` / `try_git` from `src/test_git.rs`, the one copy:
`crate::test_git` inside the crate, `dispatch_tui::test_git` from a `tests/`
target (through the `test-support` feature). Don't write another.

A test that stages an **unwritable directory** (to make a delete or a stat fail)
owes two things beyond the `chmod`: restore the permissions from a `Drop` guard,
not a line at the end — a panicking assertion in between leaves a directory
`TempDir` cannot clean — and assert `CI` is unset before skipping, because root
ignores directory permissions and `eprintln!` is swallowed by the default
harness, so the skip would report green while covering nothing. See
`deny_access_or_skip` in `src/dispatch/tests/agent_launch.rs`, and `tmux_available_or_skip`
in `tests/tmux_harness/mod.rs` for the same CI rule in its original form.

The two tmux rows are a real split, not two spellings of the same thing: a mock proves *which command we sent*, a real tmux server proves *what tmux did with it*. Read the "`MockProcessRunner` vs a real tmux server" section of `docs/conventions.md` before picking one — guessing wrong is how #3781 and #3782 stayed green while broken.

Property tests live alongside unit tests in a nested `mod property_tests` block.

Skill copy is asserted with targeted `contains` checks (not snapshots) so that deleting a specific instruction reads as a regression rather than an edit. Scope each assertion to the instruction's heading section — sibling sections repeat phrases, so a whole-document `contains` can still pass after the instruction is gone.

The same hazard has a rendering form: the buffer-search helpers in `src/tui/tests/helpers.rs` scan **every row of the whole terminal buffer**, not the overlay's rect. An overlay whose bottom rows are clipped off still satisfies an assertion for a bare `"close"` or `"cancel"`, because the board's own footer hint bar is drawn outside the popup and uses the same words. Assert the overlay's exact wording including its bracketed hints (`"[q/Esc] close"`), and run any new overlay assertion against the unfixed code first to confirm it really goes red.

An inline `#[cfg(test)] mod` that grows past ~200 lines moves out of the file. For `foo.rs` the body goes to `foo/tests.rs` behind `#[cfg(test)] mod tests;`, which keeps the module path, so `use super::*`, private access and insta snapshot names are unchanged (`src/dispatch/bump/tests.rs`). A module with many test files uses a `tests/` directory instead, as `src/store/tests/` and `src/mcp/handlers/tests/` do. A test file past ~2,000 lines splits into child modules by family, with helpers two or more children use kept in the parent. An integration target keeps its `tests/<name>.rs` entry and points each child at `tests/<name>/<child>.rs` with `#[path]` (`tests/spacetime_module.rs`), so its `--test <name>` stays valid.

Inline test modules (`mod tests`, `mod property_tests`) must have `#[allow(clippy::unwrap_used, clippy::expect_used)]` at the top — the workspace `-D warnings` policy otherwise rejects bare `unwrap()`/`expect()` calls. See `src/store/tests/mod.rs` for the canonical pattern.

## Board visuals coverage

Which tests enforce the colour and frame rules in `docs/specs/board-visuals.allium`, and which of its claims no test covers. Naming a test here couples this list to it: renaming or deleting one must update this list, and `./scripts/check-doc-symbols.sh` fails the pre-push hook when a name stops resolving.

Most of it renders a board to a buffer and reads colours back off it, the right instrument for a claim about what reaches the screen. The two border tests in `src/tui/ui/kanban/cards/tests.rs` instead call `state_border_color`, a helper too narrow to re-export, so they are tested in the module that owns it.

| Test | What it pins |
|---|---|
| **`src/tui/tests/rendering/palette_and_frames.rs`** | |
| `board_ground_is_uniform_across_columns` | uniform neutral ground |
| `neutral_ramp_is_strictly_ascending` | the three-step ramp |
| `board_ground_is_recessed_below_terminal_background` | the unfocused ground sits below the terminal background |
| `selection_does_not_lift_the_fill` | a selected card's fill equals a resting one's |
| `resting_card_border_is_neutral` | every column, through the identity lookup, none special-cased |
| `every_card_frame_is_lit_by_the_card_surface` | all four corner glyphs |
| `cards_are_inset_by_one_cell_of_column_ground` | margin width, its ground colour, and the lit rail inside it |
| `card_frame_carries_state_and_the_cursor_outranks_it` | the whole border rule on one board (see below) |
| `only_the_selected_card_has_the_cursor_white_frame` | at most one white, healthy frames neutral, no identity hue on any frame |
| `no_column_leaks_an_identity_hue_onto_a_card_frame` | the cross-column companion: all tasks healthy, so the board is one white and neutrals |
| `epic_cards_carry_purple_identity_and_a_bold_title_at_rest` | a resting epic's stripe is PURPLE, not its column's hue, and its title is bold |
| `selected_epic_frames_in_the_cursor_white_not_purple` | the selected epic frames white, and every frame is white or neutral |
| `an_epic_frame_never_takes_a_state_colour` | an epic beside a crashed task frames neutral, with the cursor in a third column so the cursor rule cannot mask the answer (`EpicFramesAreOnlyWhiteOrNeutral`) |
| `flat_view_epic_breadcrumb_is_purple` | the flattened epic-header row's title is epic purple |
| `a_card_spends_four_cells_of_its_column_on_chrome` | rendered content width against column width: exactly four apart |
| `unfocused_column_header_keeps_its_identity_colour` | an unfocused header label keeps its hue |
| `header_bar_stops_at_the_column_separators` | no header fill covers a separator cell, at five terminal widths |
| `select_all_checkbox_fill_is_neutral_in_every_column` | the checkbox fill in every column equals a resting card's border |
| `header_fill_is_uniform_across_columns` | same fill in every column at a focus state, no hue |
| `header_label_is_hued_in_every_column_at_both_focus_states` | no label collapses into its fill, and no two columns share a label colour |
| `focused_header_label_is_brighter_than_unfocused` | focus moves the label's brightness |
| `each_header_label_keeps_its_column_hue_signature` | each label keeps its source hue's channel ranking at both focus states |
| `column_top_rule_is_hued_only_while_focused` | focused rule in the identity hue, unfocused in the neutral grey, no third colour |
| `scroll_indicators_follow_the_column_top_rule` | the focused column's overflow arrow is hued, an unfocused one's grey |
| **`src/tui/tests/rendering.rs`** | |
| `focused_column_ground_is_distinct_from_unfocused` | the focus step on the ground is visible |
| `ci_status_badge_is_coloured_by_state` | the three CI badges read back green / red / yellow; an ordinary label and an unknown `ci:` value read back muted |
| `render_v2_task_card_shows_stripe` | the quarter block ▎ is present and the half block ▌ absent |
| **`src/tui/tests/rendering/selection_reorder_and_flash.rs`** | |
| `render_card_message_flash_shows_envelope` | the flash's glyph |
| `message_flash_envelope_outlives_the_old_three_second_window` | the glyph is still drawn well past a short window |
| `message_flash_expires_once_past_its_ttl` | past the threshold: swept, and no longer drawn |
| `message_flash_render_and_sweep_share_one_threshold` | the sweep and the renderer read one duration |
| `message_flash_never_colours_the_card_frame` | a live flash leaves the frame alone |
| **`src/tui/tests/rendering/new_task_signal.rs`** | |
| `unseen_task_card_shows_signal_glyph` | ◆ in SIGNAL on an unseen task's title line |
| `unseen_task_card_stripe_keeps_column_hue` | the task stripe is untouched |
| `epic_on_the_path_to_an_unseen_task_is_marked` | every epic whose subtree holds one, however deep |
| `marked_epic_stripe_is_signal_not_purple` | the epic stripe takes SIGNAL |
| `epic_mark_clears_when_the_last_unseen_task_below_it_is_seen` | derived, not stored |
| `signal_never_colours_the_card_frame` | the frame stays neutral or cursor white |
| `cursor_landing_on_an_unseen_task_clears_only_it` | entering an epic clears only the card the cursor lands on |
| `cursor_on_a_marked_epic_clears_nothing` | the epic mark is derived |
| `moving_the_cursor_onto_an_unseen_task_clears_it` | a key that moves the cursor |
| `a_refresh_that_slides_an_unseen_task_under_the_cursor_keeps_it` | a cursor that did not move sees nothing |
| `a_key_with_the_cursor_on_an_unseen_task_clears_it` | the next key after such a refresh sees it |
| `a_key_in_an_overlay_sees_nothing` | a key outside the plain board clears no mark |
| **`src/tui/tests/rendering/bookmark.rs`** | |
| `bookmarked_task_card_shows_bookmark_glyph` | ◆ in BOOKMARK on a bookmarked task's title line |
| `epic_on_the_path_to_a_bookmarked_task_is_marked` | every epic whose subtree holds one, however deep |
| `bookmarked_epic_stripe_stays_purple` | the epic stripe keeps PURPLE |
| `unseen_wins_the_glyph_slot_over_bookmark` | one ◆, in SIGNAL, on task and epic cards |
| `cursor_on_a_bookmarked_task_clears_nothing` | the bookmark outlives the cursor landing on it |
| **`src/tui/ui/kanban/cards/tests.rs`**, against `state_border_color` | |
| `every_indicator_claims_the_border_its_severity_earns` | every `CardIndicator` variant paired with the border it claims |
| `exactly_six_indicators_are_hard_failures` | the counts: six red, two amber, every variant in {red, amber, none} |

The two `cards/tests.rs` tests are a pair. The first walks a hand-written list of variants, for the same reason `state_border_color` matches exhaustively: a new variant forces a deliberate entry. But it would still pass if someone moved a variant into the red set and updated its expectation in the same edit, which is how a state gets promoted to a hard failure without anyone deciding to. The counts make that edit visible: it has to change a number that says six. Dispatching's entry is an explicit "no border", with its reason: it renders an amber *indicator* but claims no border, the rule's one judgement-rather-than-category exclusion, and an asserted absence is the only kind that survives someone "fixing" it.

`card_frame_carries_state_and_the_cursor_outranks_it` carries the rule, and its shape is the reason. It puts a crashed, a blocked and a healthy card on one board with the cursor *on the crashed one*, asserts amber on the blocked card, white on the cursor, neutral on the healthy one and no red anywhere, then moves the cursor off and asserts the suppressed red appears. That last step makes it a test of precedence; without it a renderer that forgot red would pass.

### Confine the measurement to the claim

Four tests here were first written wrong in the same way: **the measurement was not confined to the thing the test claims to check**, so each passed on broken code while reading as coverage.

- **By metric.** `each_header_label_keeps_its_column_hue_signature` cannot use colour distance: measured by distance, Review's correctly dimmed label lands nearer BLUE than PURPLE. Channel ranking survives linear mixing toward any neutral (BLUE is b > g > r dimmed or brightened, PURPLE is b > r > g), so it is the right fingerprint.
- **By source.** The epic tests take PURPLE as a literal, not from the Review column's identity lookup, though the two share the token. Sourcing it from the column encodes the conflation the test disproves.
- **By region.** `flat_view_epic_breadcrumb_is_purple` at first counted purple cells across the whole buffer, and the epic *card* is purple too, so it passed with the header drawn in grey. It now finds the row by the epic's title followed by its rule and reads only that row.
- **By source of truth.** `select_all_checkbox_fill_is_neutral_in_every_column` first compared the rendered fill with the accessor the renderer itself calls, so a wrong-*neutral* change moved both sides together. It now asserts what the spec claims, the same neutral as a resting card's border. A spec claim "X is the same value as Y" is asserted against Y, not against whatever X reads. A test that needs new access to production internals is often reaching for the wrong side of the claim.

Before trusting one of these, check that its measurement can tell its subject from everything that looks similar: the right metric, sourced independently of the claim, read from the right region, compared against the specified value. All four survive mutation testing of the *code*, because the fault is in the test.

Two fixture hazards:

- A card is *healthy* only if the fixture makes it so. A test that drives a tick has an inactive Running task reclassified as stale, which claims an amber border, so a neutral control silently turns amber. Seed recent activity on any card meant to rest.
- `header_bar_stops_at_the_column_separators` checks five widths on purpose: whether a misaligned header row shows depends on how the division rounds at that width, and one width can line up by chance and hide it.

The lit frame and the one-cell inset are both produced by `src/tui/ui/kanban/cards.rs::frame_card`, which paints the margin cells in the column ground. Its chrome total (`CARD_CHROME_WIDTH`) is what the title budget is reduced by, so a change to the margin moves truncation on every card.

### Claims no render test covers

- Epic view's enclosing panel tint: faintly purple as a mode signal, with the column grounds inside it neutral. One production site, no test; it is the only place purple means *mode* rather than *epic*.
- That a cursor or multi-selected *task* card's title is bold. The epic case is asserted; the task case is not.
- That the new-task signal and the bookmark are per viewer. The board only receives the viewer's own rows (the store's `own_unseen_tasks` and `own_bookmarks` views), so the render path has no other person's row to draw.

Style claims are the usual gap: text is caught by the snapshots, colour and weight only by a test that reads them back. A new colour or weight claim in the spec arrives with its test, or with a line in this list.

## No wall-clock sleeps in tests

Tests must never sleep on the wall clock — not to "wait for" `spawn_blocking` or detached `tokio::spawn` work, and not to cross a duration threshold — and must never measure it either. Instead await a deterministic completion signal (oneshot / `Notify` / an `BoardEvent`), inject a clock, or inject the threshold. `./scripts/check-no-test-sleep.sh` (in the pre-push hook, with its own self-test) enforces this: no `tokio::time::sleep`, `std::thread::sleep` or `.elapsed()` in test code — test files *and* top-level inline `#[cfg(test)] mod` blocks. Production use of all three is unaffected (the frame-wait deadline in `next_loop_event` is a production `tokio::time::sleep`). A deadline-bounded poll may carry an `// allow-test-sleep: <why>` marker on a `std::thread::sleep` or `.elapsed()`; the marker never exempts `tokio::time::sleep`. When a test's subject *is* a deadline, bound it structurally (`tokio::time::timeout`, `Receiver::recv_timeout`) rather than asserting on measured elapsed time. See the "No `tokio::time::sleep` in tests" section of `docs/conventions.md` for the exact scoping rule, its remaining blind spot, and the canonical patterns.

## Agent-behaviour evals

`plugin/evals/` holds `claude plugin eval` cases that check what an agent does when it follows a plugin skill (today: `wrap-up`). They are **local only**: not in CI and not in the pre-push hook. Each run is a real model call on your own credentials, so the result is noisy and costs money.

```bash
./scripts/run-evals.sh                                  # all cases, 3 runs each, $3 cap
./scripts/run-evals.sh --case 'wrap-up-*' --runs 5 --max-cost-usd 5
```

`scripts/run-evals.sh` always sets `--max-cost-usd` (default 3, or `EVAL_MAX_COST_USD`) and `--runs` (default 3, or `EVAL_RUNS`), and passes other flags through. It also unsets `GRADLE_USER_HOME`, turns off the with-without baseline and keeps the HTML report local. A full run takes a few minutes, so start it in the background from a Claude Code session. The dispatch MCP server is mocked from `plugin/evals/mocks/`; a case never touches the live store.

Things that trip up a new case: an `llm` grader needs `focus: trace` or it reads only the last message; Bash and git do not work inside a run, so state facts such as the branch in the prompt; the layout is in `plugin/evals/README.md`.

`plugin/` is embedded with `include_dir!`, so `plugin/evals/` is compiled into the binary too, but the installer skips it (`src/setup/plugins.rs::is_dev_only`) and removes a copy left by an older install. `plugin/evals/results/` is gitignored.

## Coverage

CI's `coverage` job runs `cargo tarpaulin --engine llvm --out xml --out stdout --exclude-files 'src/spacetime/bindings/*' --exclude-files 'spacetime/module/*' --timeout 600 --fail-under 86` (`--out Html` locally). It is **gated**: coverage below the floor fails the job. Each tarpaulin invocation re-runs the whole suite, so both output formats come from one run — don't add a second invocation to render another format.

**The engine is part of the measurement.** On the same tree, `--engine llvm` scored 90.28% (14846/16445 lines) and the default `Auto` engine 88.54% — a ~1.8-point instrumentation difference with no code change behind it. The floor is calibrated against llvm, which is why CI pins it; quote the engine whenever you quote a number, and don't compare a local default-engine run against the CI floor.

Two paths are excluded. Generated SpacetimeDB bindings (`src/spacetime/bindings/`) are ~7% covered and would drag the figure down for code nobody writes. The module crate (`spacetime/module/`) is a `test-support` path dependency, so tarpaulin instruments it, but its reducers run as wasm inside the live store where tarpaulin cannot see them: it scored 25.93%, with seven files at 0%, and pulled the total under the floor (85.74%, 2026-10-14). `check-spacetime-module.sh` and the live-store tests cover it instead. `--timeout 600` is per test: tarpaulin's 60s default killed the live-store tests (run 37892931546). The floor is 86; the last measurement is 92.17% (21117/22910 lines, llvm engine, both exclusions, `spacetime` on `PATH`, 2026-10-15). It is a regression tripwire, not a target: raise it by hand when a step-change in coverage makes the headroom pointless, never automatically to whatever the last run scored. Don't chase 100% on render-heavy code or `src/setup/`'s OS-interaction branches (hooks, filesystem writes) — a single file below the average is not by itself a problem.

**The coverage job runs the live store.** CI installs `spacetime`, so `tests/spacetime_module.rs` and `tests/memory_caller_conformance.rs` publish real modules under tarpaulin's instrumentation. They are the slowest part of the job and what timed it out on 2026-10-09; budget for them before adding live-store cases.

Coverage is not in the pre-push hook; every *other* CI gate is, and `tests/ci_gates.rs` asserts the hook's script list and the workflow's stay in sync.

## Toolchain and pre-push hook

`rust-toolchain.toml` pins the toolchain CI uses, and only rustup reads it. A distro cargo (Fedora's, no rustup) builds with its own rustc, and clippy then reports lints CI does not have. The hook's first step, `scripts/check-toolchain.sh`, stops the push when the host rustc differs from the pin; a distro toolchain works only while its version equals the pin. On a distro toolchain, install the wasm target with `sudo dnf install rust-std-static-wasm32-unknown-unknown`. The hook's last step builds the SpacetimeDB module and hard-fails without it, with no earlier warning on a docs-only push.

The pre-push hook (`.githooks/pre-push`) calls `scripts/gates.sh --fix`, and the verify command `scripts/verify.sh` calls `scripts/gates.sh --check` then `cargo test --no-fail-fast`, so a rebase wrap-up (which never pushes) meets the same gates; the only difference is `cargo fmt` (hook) versus `cargo fmt --check` (verify). `gates.sh` runs `check-toolchain.sh` (with its self-test), `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `check-rustdoc-links.sh` (rustdoc links in `src/`; its own CI job), the three doc checkers (`check-doc-paths.sh`, `check-doc-symbols.sh`, `check-doc-headings.sh`, each with a self-test), `check-no-test-sleep.sh`, `check-single-write-service.sh` (a `src/service/` function, method or free, with more than one write on a `db.` receiver, including rustfmt-wrapped calls, fails unless `scripts/single-write-service.allow` names it with its exact write count and a reason; a write inside a `for`/`while`/`loop` body counts twice, a `self.<helper>()` call adds the helper's writes one level deep, and a `let store = &self.db;` rebinding is followed; the script header lists what it still misses; its own self-test; CI runs both), `test-fetch-reviews.sh`, `test-fetch-cve.sh`, `check-previous-store.sh` (with its self-test) and the spacetime module check.

## SpacetimeDB module and CI details

**`spacetime/module/` is outside the workspace.** `cargo test` and `cargo clippy --all-targets` stop at the root package. The module is its own crate for wasm32. `./scripts/check-spacetime-module.sh` checks it for wasm, runs its unit tests, then runs the in-memory reducer tests (below). The pre-push hook and CI both run that script. It needs `sudo dnf install rust-std-static-wasm32-unknown-unknown gcc-c++ openssl-devel`.

**Reducer tests run against SpacetimeDB's in-memory store.** `spacetime/module-tests/` is a separate crate that compiles the module's own `src/lib.rs` as a plain rlib with `spacetimedb`'s `test-utils` feature. Its `tests/` call the real reducers as a named client inside one transaction (the harness in `spacetime/module-tests/tests/common/mod.rs`), read the tables back, and move `TestClock` by hand. A reducer's decision stays a pure-function test in `spacetime/module/src/tests/`; a test belongs in `module-tests` when it needs the table writes, the transaction, the caller's identity or a timer.

- **Why a separate crate.** `test-utils` links V8, which cannot link into the module's `cdylib`, and cargo builds every crate type even for a path dependency. The crate has its own `Cargo.lock`, a cold build of several minutes and about 6 GB in `spacetime/module-tests/target`. A warm run takes seconds.
- **Build environment.** A C++ compiler and the system OpenSSL headers, with `OPENSSL_NO_VENDOR=1` (the script sets it). The two ICU crates are capped at `~2.1` in its `Cargo.toml` because V8's date library fails against 2.3; drop the cap when upstream moves.
- **Procedures are wasm-only.** A `#[spacetimedb::procedure]` calls host imports (the `procedure_*_mut_tx` family) that `test-utils` does not provide, so one compiled natively fails the link of every test binary in this crate. Gate each procedure with `#[cfg(target_arch = "wasm32")]` (as `spacetime/module/src/config.rs::own_usage_summary` is) and test its body as a plain function.
- **A module file must not share a table's accessor name.** `mod bookmarks;` beside `#[spacetimedb::table(accessor = bookmarks)]` shadows the generated accessor trait in the glob re-export, and `ctx.db.bookmarks()` stops resolving. Name the file in the singular (`bookmark.rs`).
- **Scheduled reducers.** The in-memory host does not fire timers. A backstop test advances the clock, then calls `expire_dispatch_request` or `expire_session_request` with the timer row as the module's own scheduler, as the live host would. Called as a client, they refuse.
- **Only `pub` items are reachable** from `tests/`. Go through the reducers.
- **Not covered.** Publish and automigration stay in `tests/spacetime_module.rs`; the in-memory store does not exercise either. Store-side changes still need the live-store tests whenever a rule moves into a reducer.
- **What stays live, and why (audit, task #98318).** Every remaining live-instance user needs something the in-memory store cannot give: a published module, the SDK connection, or a real process. None can move.
  - `tests/spacetime_module/publish.rs`: publish and automigration, the store's own judgement.
  - `tests/spacetime_module/subscription.rs` and `mutations.rs`: the SDK's row callbacks and subscription delivery (`src/sync/sdk_connector/`). The reducer a test calls is already covered in `spacetime/module-tests/`; what the live test adds is that the row reaches `SharedRows`.
  - `tests/spacetime_module/data_rules.rs`: a board connecting to a store upgraded by a plain publish. Needs publish plus the SDK handshake.
  - `src/spacetime/tests/managed_store_real.rs`: starts, adopts and stops a real `spacetime` process.
  - `tests/memory_caller_conformance.rs` **stays live.** It diffs the hand-written `MemoryReducerCaller` (`src/sync/memory_caller/`) against the real module through `SdkReducerCaller`. `module-tests` is a separate crate that cannot depend on the board crate, so it cannot host the memory caller. Moving the real-module side in-memory would drop the SDK path, and the SDK path is half of what the diff checks. `module-tests` proves the reducers; the conformance suite proves the board's stand-in matches them. Both are needed.
- **No restamp.** The crate lives outside `spacetime/module/src`, so adding or editing its tests does not change the `module.wasm` source stamp.

Two root-level tests watch the module from outside:

- `src/spacetime/tests/module_schema.rs` reads the module's tables from its source; the snapshot fixtures take their column lists from it.
- `tests/spacetime_module.rs` publishes the module into a throwaway instance when `spacetime` is on `PATH`.

**The board embeds a committed prebuilt `src/spacetime/module.wasm`.** After any module change, run `./scripts/build-managed-module.sh`. The hook, CI and `cargo test` check a source-hash stamp beside the wasm, not the wasm bytes, which differ per machine. The stamp hashes every file under `spacetime/module/src`, test files included, so a test-only change there needs a restamp too.

**That check fails spuriously mid-merge.** It publishes the committed module and automigrates the working tree's over it. With a merge resolved but not committed, it reports every difference the merge brought in. Commit the merge, then re-run. See `spacetime/module/README.md`.

**Changing the module means regenerating the client bindings.** Run `./scripts/regenerate-spacetime-bindings.sh` and commit the result. CI never runs it, because it needs the `spacetime` CLI. `src/spacetime/tests/bindings_parity.rs` fails when a table or reducer exists on one side only. Absence in the module is a sentinel (`""`, `0`), not a null, because SpacetimeDB SQL cannot filter on an optional column. `SharedTable::sentinel_columns` holds the list and the reasoning.

**Known gap: the memory twin does not apply the per-caller views to the board's rows.** `MemoryReducerCaller` pushes every task and epic into `SharedRows` unconditionally (`remove_member`, a private flip and an unfollow all leave the rows on the board), so a test over it cannot fail on a board-side loss-of-sight bug. The view bodies themselves are shared with the module (`src/sync/memory_caller/views.rs`) and tested in `spacetime/module/src/tests/views.rs`; recomputing the board's rows from them after each apply is not done (task 98853, finding Y5).

**A new task-move path goes in the move-path table.** `tests/memory_caller_conformance/move_paths.rs` runs every path that moves a task between epics (`set_task_epic`, `update_task`, the repo reroute, `regroup_epic`, `flatten_epic`) against one set of rules on the module and on the twin. `every_task_move_site_in_the_module_is_in_the_table` fails when a module function writes a task's `epic_id` (a `Task { epic_id, .. }` literal or `.epic_id =`) and no row covers it, so a path that forgets the unseen mark is caught too; inserts are listed in `NOT_A_MOVE` with a reason. Each row's destination cases include an epic another user holds privately, which must be refused like a missing one.

**The live-store tests skip without `spacetime`, except under CI.** `tests/spacetime_module.rs`, `tests/memory_caller_conformance.rs` and `src/spacetime/tests/managed_store_real.rs` skip when `spacetime` is not on `PATH`, and fail instead when `CI` is set (`spacetime_available_or_skip` in `tests/common/spacetime_instance.rs`, the same rule as `tmux_available_or_skip`). The Test and Coverage jobs both install the pinned CLI, so they run for real in both.

**Startup smoke tests.** `src/runtime/tests/startup_smoke.rs` runs the real composition root, `TuiRuntime::bootstrap`, against a throwaway `spacetime start` (`src/runtime/tests/live_store.rs`, same skip rule as above). Every other bootstrap test uses the in-memory stand-in, so these are the only tests that catch a change which makes the operator's next launch abort. Design: `docs/superpowers/specs/2026-10-26-startup-smoke-test-design.md`.

- `a_fresh_install_starts_against_a_real_store`: no `host.json`; startup publishes, connects, mints the identity, and sees a task it wrote again after a restart.
- `an_upgrade_from_the_previous_store_keeps_every_task_and_epic`: starts a copy of `tests/fixtures/previous-store/`, a real store written by `main`'s module, and checks the same user sees exactly the ids in its `expected.json`. It always publishes, as the store's owner. On a branch that changes the module that is a migration; on one that does not, it only republishes, so green there proves nothing about automigration. The test prints which ran.
- **The fixture moves with `main`.** After a module change lands, run `scripts/rotate-previous-store.sh` and commit the result. `scripts/check-previous-store.sh` (pre-push hook, and the CI Gate scripts job, which checks out full history and makes a local `main`; it still skips in any shallow clone or one with no `main`) fails when the fixture holds a module `main` never had, and says when it lags `main`'s current one. The fixture's own README says what each file is.
- **Never pick a port by bind-and-drop.** A port free when picked can be another test's by the time the store binds, and the managed path's readiness probe then adopts that store. Start a standalone store on port 0 and read the port from its stdout (`LiveStore::start`). Where the managed path must be told an address up front, the test proves the listener is its own store (`listener_runs_with_arg`, over `--data-dir=`) and starts over when it is not. A test whose fake spawner never binds uses `UNUSED_ADDRESS`.
- **Never the operator's store or key.** Each store gets a free port, its own data directory, CLI config and JWT key pair. Without explicit key paths `spacetime start` signs with `~/.config/spacetime/id_ecdsa`; `tests/common/spacetime_instance.rs` passes them too.

**They run under tarpaulin too.** Tarpaulin builds the tests with `RUSTFLAGS=-Cinstrument-coverage`, and `spacetime publish -p` builds the module with cargo. Inherited, that flag fails the wasm build with "can't find crate for profiler_builtins" (task #4909). `spacetime_command` drops `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS` and `LLVM_PROFILE_FILE` from every `spacetime` call, so route new calls through it. A local `cargo tarpaulin` with `spacetime` on `PATH` therefore runs them, and is slower.

**Live-only wiring.** The SDK's row callbacks (`src/sync/sdk_connector/wiring.rs`) and the bodies of the reducer calls (`src/sync/sdk_connector/callers.rs`) run only on the SDK's event context, which a live store produces. Only the live-store tests reach them, which is why the Coverage job installs `spacetime`. The row-retire rules they call live on `SharedRows` and are unit-tested there; the "no connection" refusal of every caller is tested in `src/sync/sdk_connector/tests.rs`. The signal handler, health watcher and input thread in `src/runtime/mod.rs` need a real process or terminal and are left to the live tests.

**CI jobs.** `.github/workflows/ci.yml` runs Test, Clippy, Rustdoc links, Format, Coverage, Gate scripts and a release Build per shipped target (the same `cargo build --release --locked` that `release-please.yml` runs, so a link failure shows on the PR). `scripts/check-rustdoc-links.sh` has its own job because it needs the compiler; it runs `cargo doc --no-deps --document-private-items` and fails on an unresolved intra-doc link outside the generated bindings. Gate scripts mirrors every `scripts/*.sh` check in `scripts/gates.sh`, in the same order; `tests/ci_gates.rs` fails if the hook gains a script CI does not run. The Test and Coverage jobs install a pinned `spacetime` CLI and hard-fail if it lands at another version, so the live-store tests run for real.
