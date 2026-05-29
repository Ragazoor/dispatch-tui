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

### OwnedTaskPatch (and OwnedCreateTaskRequest)

`db_call` closures must be `Send + 'static`, so borrowed fields from `TaskPatch<'_>` cannot
cross the boundary. `OwnedTaskPatch` and `OwnedCreateTaskRequest` in `src/db/queries/tasks.rs`
are owned mirrors that exist solely to satisfy this constraint. Convert via the `From` impl:
`OwnedTaskPatch::from(patch)`.

**Parity is compiler-enforced.** Both `From` impls use an exhaustive destructuring of the
source struct (no `..`), so adding a field to `TaskPatch` or `CreateTaskRequest` without
also updating the owned mirror and its `From` impl is a **compile error**. When you add a
field, name it in the destructuring pattern and add it to the `Self { … }` construction; the
compiler rejects anything less.

`OwnedTaskPatch` deliberately omits `labels` — labels are pre-serialised to JSON before
entering `db_call` and handled via `labels_json` in `patch_task`. The `labels: _` binding in
the `From` impl keeps the exhaustive pattern intact despite the omission.

## DB trait narrowing — take the narrowest sub-trait you need

`TaskStore` is a supertrait of `TaskAndEpicStore + PrStore + AlertStore + SettingsStore`. New consumers should hold the narrowest sub-trait they actually call:

| Consumer | Holds |
|----------|-------|
| `TaskService` | `Arc<dyn TaskAndEpicStore>` |
| `EpicService` | `Arc<dyn EpicCrud>` |
| `McpState`, `TuiRuntime` | `Arc<dyn TaskStore>` (fans out to all sub-traits) |

`Arc<dyn TaskStore>` coerces to any narrower trait object at call sites via Rust's trait-object upcasting (stabilised in 1.86). If you need to split a wide `Arc<dyn TaskStore>` into a narrower one, use a typed `let` binding: `let d: Arc<dyn EpicCrud> = task_store_arc.clone();`.

## Service trait narrowing — `Arc<dyn TaskServiceApi>` / `Arc<dyn EpicServiceApi>`

Parallel to DB trait narrowing, the service layer exposes two traits defined in `src/service/api.rs`:

| Trait | Production impl | Where held |
|-------|----------------|------------|
| `TaskServiceApi` | `TaskService` | `TuiRuntime::task_svc`, `McpState::task_svc` |
| `EpicServiceApi` | `EpicService` | `TuiRuntime::epic_svc`, `McpState::epic_svc` |

Consumers that call task or epic operations should hold `Arc<dyn TaskServiceApi>` / `Arc<dyn EpicServiceApi>` rather than the concrete struct. This lets unit tests inject a mock service without a real database — construct `McpState` directly (all fields are `pub` or `pub(crate)`) and pass a custom `Arc<dyn TaskServiceApi>`.

The concrete structs (`TaskService`, `EpicService`) delegate via UFCS (`TaskService::method(self, …)`) inside the `impl` blocks to avoid shadowing the inherent methods.

## DB access — `db_call`

`Database` (`src/db/mod.rs`) wraps a single [`tokio_rusqlite::Connection`] — a dedicated worker thread owning the underlying `rusqlite::Connection`. There is no sync handle or mutex; every store impl, schema init, and migration runs through that worker.

- `Database::open(path).await` / `Database::open_in_memory().await` open the connection and run the migration chain on the worker thread.
- `self.db_call(|conn| { … }).await` is the single entry point for all SQL. The closure receives a `&mut rusqlite::Connection`, must be `Send + 'static`, and returns `Result<R>`. Errors are routed back through `tokio_rusqlite::Error::Other` and surfaced as `anyhow::Error`. Clone any borrowed `&str`/slice arguments to owned values before moving them into the closure.

Every `*Store` trait method is `async fn` and uses `db_call` internally. Callers `.await` each store call.

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

## Performance footguns

Two patterns have already caused bugs and must not be repeated:

- **`column_items_for_status` is test-only.** It calls `column_items_for_status_with_stats(status, None)`, which derives epic sort order by cloning subtasks on every invocation. In production render paths, always call `column_items_for_status_with_stats(status, Some(&stats))` with a pre-computed `EpicStatsMap` to avoid per-frame allocations.

- **No `std::fs` inside async handlers.** Blocking I/O on the async executor stalls the tokio thread pool. Any file-system operation inside an `async fn` must use `tokio::fs` or be wrapped in `tokio::task::spawn_blocking`.
