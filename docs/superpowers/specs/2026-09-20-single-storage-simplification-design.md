# Consolidating dispatch onto a single storage backend

**Status:** proposed — supersedes the storage split in
[2026-09-17-spacetimedb-migration-design.md](2026-09-17-spacetimedb-migration-design.md),
whose ownership, host-gating and subscription design still stands.
**Task:** #4912, epic #325 (SpacetimeDB)
**Decision owner:** ragge
**Date:** 2026-09-20

## TL;DR

The 2026-09-17 design kept SQLite forever for "what SpacetimeDB cannot hold: the
knowledge base, which needs a vector index, and local UI preferences." That
premise doesn't hold up:

- Learnings' semantic search is not a SpacetimeDB-side vector index today and
  never was — it fetches candidate rows and ranks them by cosine similarity in
  Rust, in process (`src/service/embeddings.rs::rag_rank_learnings`). Any store
  that can hand back rows can serve it.
- Each learning's embedding is ~1.5 KB (384 floats). This is nothing like
  embedding a codebase. (A codebase-embedding feature was designed in
  `docs/superpowers/plans/2026-05-22-rag-code-files.md` but never merged — it
  doesn't exist in the code today and isn't a factor here.)
- Learnings are scoped `user | repo | epic` — designed to be knowledge shared
  across a team on a repo. Filing them under "SQLite, per machine, forever"
  (`docs/conventions.md`'s store seam table) means two teammates on the same
  repo never see each other's learnings. That was never a deliberate choice,
  just a classification nobody revisited.

Conclusion: there is no remaining technical reason to keep two storage
backends. Dispatch moves to **one storage backend, SpacetimeDB, for everything
except two pieces of bootstrap-only local state.**

## What moves

Everything currently classified `LocalStore` in `docs/conventions.md`'s store
seam table, plus `task_usage`:

- `settings` (except the two exceptions below) and `filter_presets` — scoped
  per host/user, the same way `todos.owner` was added in Phase 5, so a
  teammate's filter presets and preferences don't broadcast to your board.
- `learnings`, their embeddings, `learning_retrievals`, `learning_verdicts` —
  unscoped (or scoped per their existing `scope`/`scope_ref`), because these
  are meant to be shared.
- `usage_events`, `task_usage` — shared. This was an open question in the
  original plan ("defaulting to local"); resolved here as shared, consistent
  with everything else moving.

This is in addition to the shared tables already migrated or in flight
(`tasks`, `epics`, `todos`, hosts, subscriptions, and the remaining Phase
6b/6c/7/8 work).

## What stays local — and why these two are not a judgment call

1. **The user identity credential** (`user_identity_token` /
   `IdentityCredentialStore`). Already documented in
   `src/db/queries/settings.rs`: "a credential in a shared store is a
   credential everybody on the board can use." A secret cannot live inside the
   store it authenticates you to.
2. **The server address** (`--spacetime-server` / `DISPATCH_SPACETIME_SERVER`).
   Already never persisted — it's a CLI flag / env var today. You need
   something local to say where to connect before you can read anything from
   the connection.

Nothing else is exempt. `host_id` and `host_label` — currently also in
`settings` — move too; they're already mirrored into the shared `Host` table
today, so this drops a local copy rather than creating a new shared one.

## SpacetimeDB becomes mandatory

A single-machine install no longer has a fully offline, no-server mode. Every
install — solo or team — runs or connects to a SpacetimeDB instance. A solo
dev runs one locally (`spacetime start`, loopback).

This retires:

- `docs/specs/sync.allium`'s "a board with no store configured is not a
  degraded board" branch and everything conditioned on it.
- The swappable-backend abstraction Phase 3/3b built
  (`SharedDomainStore`/`LocalStore` trait split, the in-memory conformance
  store, `db::SharedWriter`/`sync::ReducerWriter`, `SHARED_WRITES_ARE_COMPLETE`)
  — once there is exactly one backend, the seam that exists to let a second
  backend implement only half the traits has nothing left to seam.
- `docs/specs/storage.allium` in its entirety (`LocalStore`, `DatabaseConnection`,
  wal/rollback journal modes) — there is no more local SQLite process store to
  describe, only the two local settings above (which need no journal-mode
  story; they're a tiny file, not a database).

## Consequences to work through in the implementation phases

- **Cross-table relationships enforced by SQLite FKs today must become
  explicit reducer logic.** `learnings.source_task_id` (`ON DELETE SET NULL`)
  and `task_usage.task_id` (`ON DELETE CASCADE`) both reference `tasks(id)`;
  SpacetimeDB has no cross-table cascade, so a task-delete reducer needs to
  do this explicitly, the way `recalculate_epic_status` already does cascades
  by hand.
- **Test infrastructure.** Today's unit tests lean heavily on
  `Database::open_in_memory()` (in-memory SQLite). Phase 3b's in-memory
  `SharedDomainStore` conformance store is the closer analogue and needs
  extending to cover the newly-moved tables; this is a large mechanical
  rewrite across the suite, not a design question, but it's the biggest
  reason this won't land in one phase.
- **Migration/backfill.** An existing install's local SQLite data (settings,
  learnings, usage history) needs a path into a freshly-provisioned
  SpacetimeDB instance. The Phase 0 dump/restore tooling
  (`docs/specs/spacetime-seed.allium`) is the existing precedent to extend
  rather than a new mechanism.
- **Embedding encoding.** Confirm SATS represents the embedding blob as
  `Vec<u8>` (bytes, as SQLite stores it today) rather than `Vec<f32>`, during
  the schema phase — mechanical, not a design fork either way.

## Ask

Approve this shape so the phase plan (new "Phase N" subtasks under epic #325,
following the pattern of Phases 0–8) can be written against it.
