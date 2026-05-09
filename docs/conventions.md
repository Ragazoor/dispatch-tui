# Code Conventions

## Rendering purity

Code under `src/tui/ui/` must be pure: it reads `App` and shared helpers, writes ratatui buffers, and does nothing else.

**Allowed:**
- Immutable reads of `App` fields and shared helpers from `src/tui/ui/shared.rs` and `src/tui/ui/palette.rs`
- Writes to the ratatui `Buffer` / `Frame` passed in by the caller
- Pure formatting (`format!`, `truncate`, span construction)

**Forbidden:**
- Database access (no `self.db`, no `Database::*`, no `rusqlite`)
- File I/O (`std::fs`, `std::io`, `tokio::fs`)
- Process spawning (`std::process::Command`, `tokio::process`)
- Async runtime calls (`tokio::*`, `block_on`, channel sends/receives)
- MCP calls or network I/O
- `unwrap()` / `expect()` / `panic!` outside `#[cfg(test)]` — render must never crash the TUI

If a render path needs data that isn't on `App`, compute it in the runtime/update layer and stash the result on `App` before rendering — do not reach for it from `src/tui/ui/`.

## Soft-fail decoding

Schema enum values may be added in a migration before all rows are upgraded. Row decoders in `src/db/queries/` default unknown values and emit `tracing::warn!` rather than panicking — see `row_to_task` in `src/db/queries/mod.rs:20-25` for the canonical example.

Never `panic!` (or `unwrap()`/`expect()`) on an unknown enum value read from the DB. Use `Enum::parse(&s).unwrap_or_else(|| { tracing::warn!(...); Enum::Default })`. This keeps an old DB readable after a partial migration and prevents a poisoned row from killing the TUI.

**Decode-fallback counter:** every soft-fail in `row_to_task`/`row_to_epic` and `read_json_string_vec` bumps a process-wide `AtomicU64` exposed as `crate::db::decode_fallback_count()`. The counter value is included in the `tracing::warn!` message (`count=N`) so the warns are greppable in aggregate, and the accessor lets tests and ad-hoc debugging detect slow-bleeding decode bugs without chasing log lines. When you add a new soft-fail branch, bump the counter from `db::queries::bump_decode_fallback()`.

## Border parsing

Untrusted inputs — MCP JSON-RPC arguments, editor output, feed-script JSON, plan files — must be parsed into typed domain enums **at the boundary**. Business logic should never see raw `serde_json::Value` or `String` for fields that have a typed shape.

- MCP handlers in `src/mcp/handlers/` parse to typed `*Args` structs (with serde derives) before calling into the service layer.
- Feed scripts produce `FeedItem` JSON which is parsed into the typed struct in `verify-feed` and at runtime ingest.
- Plan files are parsed by `src/plan.rs` into a typed plan structure.

Parse failures must surface to the caller as a `ServiceError::Validation` (or `-32602` at the MCP layer). Silent fallback to a default value is forbidden — if the input is invalid, the caller needs to know.

## `FieldUpdate` — nullable string fields

`FieldUpdate` (`src/service/mod.rs`) replaces the `Option<String>` + empty-string sentinel anti-pattern for fields that need three states: "don't touch", "set to value", "clear to NULL":

```rust
pub enum FieldUpdate {
    Set(String),  // set the field to this value
    Clear,        // set the field to NULL
}
```

Used in `UpdateTaskParams` for `pr_url`, `worktree`, and `tmux_window`. When adding a new nullable field to `UpdateTaskParams`, use `Option<FieldUpdate>` rather than `Option<Option<String>>`.

**When to use:** if the caller can clear the field to NULL (nullable column, user-clearable), use `FieldUpdate`. If the field is non-nullable or the update path only ever sets a value, use a plain `String` (or `Option<String>` to mean "don't touch / set"). Reserve the three-state pattern for genuinely tri-valued updates.

## `TaskPatch` / `EpicPatch` — double-Option in the DB layer

`TaskPatch` and `EpicPatch` (`src/db/mod.rs`) use `Option<Option<T>>` for nullable fields — the DB-layer equivalent of `FieldUpdate`:

| Value | Meaning |
|-------|---------|
| `None` | Don't touch this field |
| `Some(None)` | Set the field to NULL |
| `Some(Some(v))` | Set the field to `v` |

The service layer bridges the two patterns before writing a patch: `FieldUpdate::Set(v)` becomes `Some(Some(v))` and `FieldUpdate::Clear` becomes `Some(None)`. When adding a new nullable field, use `FieldUpdate` in `UpdateTaskParams`/`UpdateEpicParams` and double-Option in the corresponding patch struct.

## DB trait narrowing — take the narrowest sub-trait you need

`TaskStore` is a supertrait of `TaskAndEpicStore + PrStore + AlertStore + SettingsStore`. New consumers should hold the narrowest sub-trait they actually call:

| Consumer | Holds |
|----------|-------|
| `TaskService` | `Arc<dyn TaskAndEpicStore>` |
| `EpicService` | `Arc<dyn EpicCrud>` |
| `McpState`, `TuiRuntime` | `Arc<dyn TaskStore>` (fans out to all sub-traits) |

`Arc<dyn TaskStore>` coerces to any narrower trait object at call sites via Rust's trait-object upcasting (stabilised in 1.86). If you need to split a wide `Arc<dyn TaskStore>` into a narrower one, use a typed `let` binding: `let d: Arc<dyn EpicCrud> = task_store_arc.clone();`.

## DB access — `conn()?` and `db_call`

`Database` (`src/db/mod.rs`) currently holds **two** connection handles to the same SQLite database:

- `self.conn()?` returns a guarded `MutexGuard<Connection>` — the legacy sync path. Used by every `*Store` impl that has not yet been migrated to async, by `init_schema`, and by sync-only helpers. Locks the mutex and propagates a `Result` error if the lock is poisoned, rather than panicking. Never call `self.conn.lock().unwrap()` directly.
- `self.db_call(|conn| { … }).await` runs a synchronous closure on a dedicated `tokio_rusqlite::Connection`, lazily opened against the same SQLite database via a shared-cache URI. New async trait impls (WP-2..WP-6 of the DB-async migration; see issue #681) move onto this helper so async MCP/runtime callers stop blocking the Tokio worker thread. The closure must be `Send + 'static` — clone any borrowed `&str`/slice arguments to owned values before moving them in.

Once all `*Store` impls are async (end of WP-6), the sync `Mutex<Connection>` field and `conn()?` helper will be removed.

## Inline-mutation boundary

Key handlers in `src/tui/input.rs` follow two different patterns:

- **Mutate inline, return `vec![]`** — for UI-only state with no side effects (cursor position, `input.mode`, selected index, text buffer). These changes don't need to be auditable and touching the DB/processes isn't required.
- **Return a `Command`** — for anything that needs a side effect: DB write, process spawn, network call, or waking the runtime.

The rule: if you're only changing what the screen looks like without touching external state, mutate inline. If the change needs to outlast the current render cycle or involve I/O, return a `Command`.

## Intentional `let _ =`

`let _ = expr` silences the `#[must_use]` warning on a result or value. In this codebase it appears in two patterns — neither is a bug:

- **Fire-and-forget channel sends** — `let _ = tx.send(McpEvent::Refresh)` in `src/mcp/mod.rs`: the send can only fail if the receiver has dropped (TUI exited), which is fine to ignore
- **Non-critical side-effect patches** — `let _ = self.db.patch_epic(...)` where the caller cannot usefully recover from a transient DB error on a non-primary write

If you see `let _ =` and are unsure whether it's intentional, check the surrounding comment or commit message. Add a comment when adding a new one.

## `#[allow(dead_code)]`

Avoid `#[allow(dead_code)]` — dead code should be removed, not suppressed. If a type or function is unused today but is part of an in-progress feature, document it with a comment pointing at the relevant issue/task rather than silencing the warning.

## Sub-status validation TOCTOU

`TaskService::update_task()` (`src/service/tasks/crud.rs`) reads the existing task to validate the requested sub-status before applying the patch. This is a TOCTOU window: a concurrent MCP call could change the task status between the read and the write. This is intentional and accepted — simultaneous status changes from two agents on the same task are considered a user error, and the window is too small to be worth a transaction-level fix.

## Immutable `parent_epic_id`

`EpicPatch` intentionally omits `parent_epic_id`. Reparenting an epic is not supported: the parent is set at creation time and never changed. This keeps the parent chain immutable and prevents accidental cycle introduction. The database enforces `CHECK (parent_epic_id != id)` (migration v35) as a final guard. See the doc comment at `src/db/mod.rs` (`EpicPatch` definition) for the full rationale.

## Clippy lint rules

Custom lint rules are configured in `[lints.clippy]` in `Cargo.toml`. The pre-push hook enforces them via `cargo clippy --all-targets --fix -- -D warnings`. When you discover a pattern worth enforcing, add a new entry with a structured comment explaining why. Consult the `/lint` skill for the full workflow.

## Visibility convention

`App` fields use `pub(in crate::tui)` to restrict mutation to the TUI module. External code (runtime, MCP handlers) can only change `App` state by sending a `Message` through `app.update()`, which returns `Command`s. This keeps state transitions auditable in one place and prevents scattered mutation from outside the TUI boundary.
