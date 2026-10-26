# Burn `learning_verdicts` and Move the Store to a New Owner — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `dispatch store import` and `dispatch spacetime restore` work on a store that holds `learning_verdicts` rows, then use the import to move the operator's board onto a fresh managed store owned by their current `spacetime` CLI identity.

**Architecture:** `burn_id_sequence` (`spacetime/module/src/sequence.rs`) has one arm per id-generating table. Commit `4cebdfee` added `learning_verdicts` to snapshots, and `SharedTable::id_column` says it generates ids, but the reducer has no arm for it, so every restore or import of a store with verdicts aborts with `unknown table learning_verdicts`. Add the arm, and add a live-store test that calls the reducer for every `SharedTable::ALL` entry so the next table added to snapshots cannot drift the same way. Task 2 is an operator runbook, not code.

**Tech Stack:** Rust, SpacetimeDB 2.12 module (wasm), `tests/spacetime_module.rs` live-store harness.

**Spec:** `docs/specs/spacetime-seed.allium` — rule `BurnIdSequences` and invariant `SequenceForEveryGeneratingTable` already require a burn for every generating table. The code violates the spec; the spec does not change.

## Background (incident, 2026-10-26)

- `spacetime login` (maincloud) on 2026-10-25 replaced the CLI token. The managed database `dispatch` (identity `c200298f…`) is owned by the old local identity `c200975d…`; the CLI is now `c200364d…`. No copy of the old token exists.
- The new build's module differs from the published one, so `dispatch tui` publishes, gets `403 Forbidden … not authorized to perform action on database … update database`, stops the store and exits. Inside tmux this shows only as `[exited]`.
- SpacetimeDB has no ownership transfer, and `delete`/`rename` also need the owner. The way out is a fresh store plus `dispatch store import`, which failed on the missing arm.
- Current state on disk (`~/.local/share/dispatch/`): `spacetime/` is the original, untouched store. `spacetime.failed-import-20261026/` is the partial fresh store from the failed import; discard it.

## Global Constraints

- Root `cargo test --no-fail-fast` must run with `spacetime` and `tmux` on `PATH`, or the live-store test skips and proves nothing.
- After any change under `spacetime/module/src`, run `./scripts/build-managed-module.sh` and commit the new `src/spacetime/module.wasm` and its stamp.
- No `RULES_VERSION` / `BOARD_RULES_VERSION` bump: this is a bug fix inside existing behaviour (`spacetime/module/src/rules.rs` says so).
- No bindings regeneration: the reducer's signature does not change.
- Conventional Commit subjects; run `cargo fmt` and the `scripts/check-*.sh` gates before committing.

## Review Focus

1. **A future generating table added to `SharedTable` without a burn arm.** Expected: a test fails, not an operator's import. Pinned by Task 1's `every_shared_table_is_a_table_the_burn_accepts`.
2. **The burn really advances the `learning_verdicts` counter.** Expected: after burning past 5, the next verdict row gets an id above 5. Pinned by Task 1's `burning_learning_verdicts_moves_its_counter_past_the_ceiling`.
3. **`MemoryStore` hides the gap.** `snapshot_target.rs::MemoryStore::advance_id_sequence_past` accepts any generating table, so in-memory restore tests pass while the real module refuses. Expected: only the live-store test is trusted for this; no change to `MemoryStore`.
4. **Re-running import onto a half-written store.** Expected: the runbook always imports into an empty data directory, never onto `spacetime.failed-import-20261026/`.
5. **Store-identity pin after the move.** Expected: the first `dispatch tui` after import stops with the pin mismatch, naming both identities; `--accept-store-switch` lets it through once and re-pins.

---

### Task 1: Burn arm for `learning_verdicts`, with a guard over every shared table

**Files:**
- Modify: `spacetime/module/src/sequence.rs` (`burn_id_sequence` match)
- Modify: `src/spacetime/module.wasm` and `src/spacetime/module.wasm.source-hash` (rebuilt, not hand-edited)
- Test: `tests/spacetime_module/sequence.rs` (new), registered with `mod sequence;` in `tests/spacetime_module.rs` beside the other `mod` lines

**Interfaces:**
- Consumes: `dispatch_tui::spacetime::SharedTable` (`ALL`, `name()`, `generates_ids()`), `Instance::call(&self, reducer: &str, args: &[&str])`, `Instance::sql(&self, query: &str) -> String`, `published_instance()`, `spacetime_available_or_skip()`, `describe(&Output)` — all already in `tests/spacetime_module.rs` / `tests/common/`.
- Produces: nothing new for later tasks.

- [ ] **Step 1: Write the failing guard test**

In `tests/spacetime_module/sequence.rs`:

```rust
use super::*;
use dispatch_tui::spacetime::SharedTable;

/// Every table a snapshot carries must be one `burn_id_sequence` accepts,
/// because `restore.rs::burn_id_sequences` calls it for each generating table.
/// A table added to `SharedTable` without an arm used to surface only as an
/// operator's failed import (`unknown table learning_verdicts`, 2026-10-26).
#[test]
fn every_shared_table_is_a_table_the_burn_accepts() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    for table in SharedTable::ALL {
        let out = instance.call("burn_id_sequence", &[table.name(), "3"]);
        assert!(out.status.success(), "{}: {}", table.name(), describe(&out));
    }
}
```

Check the argument encoding against `src/spacetime/cli_store.rs::advance_id_sequence_past`, which already calls this reducer through the CLI; match how it quotes the table name.

- [ ] **Step 2: Write the counter test**

```rust
#[test]
fn burning_learning_verdicts_moves_its_counter_past_the_ceiling() {
    // burn_id_sequence("learning_verdicts", 5), then create one verdict
    // through the normal reducer path and read its id back with
    // `SELECT id FROM learning_verdicts`.
    // assert!(id > 5)
}
```

Create the verdict the way `spacetime/module-tests/tests/learning_verdicts.rs` does (a learning, then `apply_learning_verdicts` with a rating task). Delete nothing; the instance is private to the test.

- [ ] **Step 3: Run both and see them fail**

Run: `cargo test --test spacetime_module sequence:: > /tmp/seq.txt 2>&1; echo $?`
Expected: non-zero; the guard names `learning_verdicts` with `unknown table learning_verdicts`. If the output says `skipping`, `spacetime` is not on `PATH` — fix that first.

- [ ] **Step 4: Add the arm**

In `burn_id_sequence`, after the `learning_retrievals` arm:

```rust
"learning_verdicts" => burn_table!(learning_verdicts, blank_learning_verdict()),
```

`blank_learning_verdict()` already exists in `spacetime/module/src/blanks.rs`. Add a short comment in the style of the `poll_owners` and `retired_feed_items` arms: added to snapshots in `4cebdfee` without an arm, found by an operator's import on 2026-10-26.

- [ ] **Step 5: Rebuild the module**

Run: `./scripts/build-managed-module.sh`
Expected: `src/spacetime/module.wasm` and its stamp change; `git status` shows both.

- [ ] **Step 6: Run the tests and the full suite**

Run: `cargo test --test spacetime_module sequence:: > /tmp/seq.txt 2>&1; echo $?` → `0`, both tests listed as `ok`.
Then the repo's verify command (from `get_task`), or `cargo test --no-fail-fast > /tmp/t.txt 2>&1; echo $?` → `0`.

- [ ] **Step 7: Commit**

```bash
cargo fmt
for s in scripts/check-*.sh; do "$s" || break; done
git add spacetime/module/src/sequence.rs src/spacetime/module.wasm src/spacetime/module.wasm.source-hash \
        tests/spacetime_module.rs tests/spacetime_module/sequence.rs
git commit -m "fix(spacetime): burn learning_verdicts' id sequence on restore and import"
```

---

### Task 2: Move the operator's board to a fresh store (runbook, operator runs it)

Run only after Task 1 is merged and installed (`cargo install --path .`, then `dispatch --version`). Nothing here edits the repo.

- [ ] **Step 1: Confirm nothing runs on the store**

`ss -ltnH 'sport = :3000'` prints nothing, and no board is running.

- [ ] **Step 2: Clear the way**

```bash
cd ~/.local/share/dispatch
rm -rf spacetime.failed-import-20261026        # the partial store from the failed run
mv spacetime spacetime.old-owner-20261026      # the original; keep it until Step 5 checks out
```

- [ ] **Step 3: Import**

```bash
dispatch store import --from ~/.local/share/dispatch/spacetime.old-owner-20261026
```

Expected: exit 0 and an `ImportReport` summary with row counts. The command starts a fresh managed store in `spacetime/`, publishes the module as the current CLI identity, writes, and stops the store.

- [ ] **Step 4: First launch**

```bash
dispatch tui --accept-store-switch
```

Expected: the board opens with your tasks, epics and learnings. Without the flag it stops on the store-identity pin, naming the old and new database identities; that is correct.

- [ ] **Step 5: Check, then tidy**

- Task and epic counts match the old board; create one task and check its id is above the highest imported id.
- Rate a learning from any task and check it lands (this writes a `learning_verdicts` row with a burned id).
- After a few days of normal use, delete `spacetime.old-owner-20261026/`.

**Rollback** at any point before Step 5's delete: stop the board, `mv spacetime spacetime.import-rejected && mv spacetime.old-owner-20261026 spacetime`, and run a build whose module matches the old store (it skips the publish).

---

## Out of scope (separate follow-up task)

Dispatch publishes the managed module with the operator's personal `spacetime` CLI config, so any `spacetime login` locks the operator out of their own store. Give the managed store its own CLI config (the `with_config_path` hooks in `src/spacetime/managed_store.rs` already exist) and decide how an existing store's owner carries over. That change needs a spec decision in `startup.allium` first.
