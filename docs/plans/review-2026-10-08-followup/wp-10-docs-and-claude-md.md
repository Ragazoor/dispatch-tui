# Docs, Stale Comments and CLAUDE.md

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove stale SQLite references outside the store/sync code, catch broken rustdoc links in CI, and add the missing context to `CLAUDE.md`.

## Context

This work package addresses A3 (outside `src/store`, `src/sync`, `src/runtime/mod.rs`, which WP1 owns) and section 7 of `docs/plans/review-2026-10-08-followup/report.md`. `CLAUDE.md` is loaded into every agent's context — keep additions to a sentence or two each and link out to `docs/`.

## Findings

### 💡 Broken intra-doc citations (`src/spacetime/snapshot.rs:452`, `:688`)

**Issue:** Comments cite `dump::is_sqlite_backed` and `super::dump_from_sqlite`; neither exists.

**Fix:** Rewrite the comments to describe the current snapshot source.

### 💡 Other stale SQLite comments (`src/mcp/mod.rs`, `src/cli/commands.rs`, `src/service/epics.rs::EpicService`)

**Issue:** `src/mcp/mod.rs` ("parent of the SQLite DB"), `src/cli/commands.rs` ("tests hand them SQLite… until Phase 12b"), `src/service/epics.rs` (learnings described as "the local half"; learnings are shared now). 62 production comment lines mention SQLite in total.

**Fix:** `grep -rn 'SQLite\|local store' src --include=*.rs` outside WP1's directories; rewrite each that describes SQLite as live.

### 💡 `docs/module-map.md` cites a removed symbol

**Issue:** The `src/sync/decode.rs` row cites `db::queries::row_to_task`, which no longer exists — the doc checker missed it.

**Fix:** Correct the row; find out why `check-doc-symbols.sh` did not flag it (wrong module prefix form?) and fix the checker if so.

### 💡 Rustdoc links are not checked

**Issue:** The doc checkers scan `docs/` and `CLAUDE.md`, not rustdoc in `src/`, so stale citations survive.

**Fix:** Add `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps` to CI and the pre-push hook (fix what it flags), or extend `check-doc-symbols.sh` to `src/` comments.

### 💡 `CLAUDE.md` gaps

**Issue / Fix:**
- Add one line on store layering: where `store::Store`, the `sync` adapters, `spacetime/` (server lifecycle, bindings, snapshot tooling) and `sync/memory_caller/` (in-memory test double, conformance-checked by `tests/memory_caller_conformance.rs`) sit. Write it after WP1/WP5 land if they are in flight, so it names the final types.
- Say that the coverage job never runs the live SpacetimeDB tests, so new `sdk_connector` code adds uncovered lines by construction (adjust if WP2 changes this).
- Note that rustdoc comments are not covered by the doc checkers (or that they now are, after the fix above).
- Reconcile the timing figure ("lib target ~10s, cold full run ~80s") with knowledge-base entry #428 ("15–20s"): measure once and keep one number; rate or update #428 accordingly.

## Changes

| File | Change |
|------|--------|
| `src/spacetime/snapshot.rs`, `src/mcp/mod.rs`, `src/cli/commands.rs`, `src/service/epics.rs`, others found by grep | Comment fixes |
| `docs/module-map.md` | Fix row |
| `scripts/check-doc-symbols.sh` or `.github/workflows/ci.yml` + `.githooks/pre-push` | Rustdoc link check |
| `CLAUDE.md` | Additions above |

## Verification

- [ ] `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps` passes (if adopted)
- [ ] All `scripts/check-doc-*.sh` and their self-tests pass
- [ ] `cargo test --no-fail-fast` — all pass
