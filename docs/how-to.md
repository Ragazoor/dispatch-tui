# How-To Guides

## Adding a New MCP Tool

The registry is **generated**. `tool_definitions()`, the `tools/call` dispatch arm, and `TOOL_NAMES` all expand from the `mcp_tools!` macro's single declarative list (`src/mcp/handlers/dispatch.rs::mcp_tools`) — do not hand-edit any of the three, and read the macro's doc comment before adding an entry.

1. **Add the argument struct** next to its siblings — task tools in `src/mcp/handlers/tasks/mod.rs`, other domains in their own handler module. Derive `Deserialize`, and annotate every integer field with a flexible deserializer from `src/mcp/handlers/types.rs`, since Claude Code may send integers as strings: `deserialize_flexible_id` / `deserialize_optional_flexible_id` / `deserialize_nullable_flexible_id` for entity ids (the field's type is the newtype itself — `TaskId`, `EpicId`, `LearningId` — never a bare `i64`), and the `*_flexible_i64` trio for genuine integers like `sort_order` or `limit`. Parse enum-valued fields into their model type at the boundary too (`status`, `tag`, `sub_status`, `url_type`) rather than carrying a `String` inward.

   Build errors, not JSON-RPC errors, are the goal here: an args struct's field types are the only thing standing between a mistyped handler and a `-32602` the caller only sees at run time.

2. **Define the handler** in the module matching the tool's domain (`src/mcp/handlers/tasks/crud.rs`, `tasks/dispatch.rs`, `tasks/wrap_up.rs`, `tasks/watch.rs`, `epics.rs`, `learnings.rs`, `managed_feeds.rs`). The signature is fixed by the macro:

   ```rust
   pub(crate) async fn handle_my_tool(
       state: &McpState,
       id: Option<Value>,
       _identity: &CallerIdentity,
       args: Value,
   ) -> JsonRpcResponse {
       let parsed: MyToolArgs = match parse_args(&id, args) { Ok(v) => v, Err(e) => return e };
       // …
   }
   ```

   Reads may go through `state.db`. **Mutations must not** — task and epic writes go through `state.task_svc` / `state.epic_svc` (`TaskServiceApi` / `EpicServiceApi`), because the service layer owns invariants like epic-status recalculation. `McpState.db` is typed `Arc<dyn store::TaskReadStore>` (`src/mcp/mod.rs::McpState`), so `state.db.patch_task(…)` is a **compile error**, locked in by a `compile_fail` doctest on `src/store/mod.rs::TaskReadStore`. <!-- allow-phantom-symbol: compile_fail is a rustdoc attribute, not our symbol --> Map service errors with `service_err_to_response`, and call `state.notify()` after a successful mutation so the TUI refreshes. See the [service mutation boundary](conventions.md#service-layer-is-the-mutation-boundary).

3. **Register the tool** by adding one entry to the `mcp_tools!` list in `src/mcp/handlers/dispatch.rs`: the `sync`/`async` kind, the tool name, the handler path, the description string, and the JSON input schema.

   ```rust
   async "my_tool" => tasks::handle_my_tool,
       "What the tool does, written for the agent that will read it.",
       {
           "type": "object",
           "properties": {
               "task_id": { "type": "integer", "description": "The task ID" }
           },
           "required": ["task_id"]
       };
   ```

   Use named error-code constants (`INVALID_PARAMS`, `INTERNAL_ERROR`, `INVALID_REQUEST`, `METHOD_NOT_FOUND`, `NOT_FOUND_CODE` — all in `src/mcp/handlers/types.rs`) in the handler rather than bare `-32602`/`-32603` literals. Where the failure is already a `ServiceError`, route it through `service_err_to_response` instead of hand-picking a code.

4. **Consider generating the boundary instead.** For a tool with a large, growing field set, declare it once with `mcp_args!` (`src/mcp/handlers/args.rs`) rather than hand-writing the struct, the schema and the mapping separately. One field list expands to the `Deserialize` struct, the tool's input schema (a `fn` you reference from `mcp_tools!` as `(tasks::update_task_schema())`), the arg→params mapping, and the `FIELD_NAMES`/`manual_fields()` introspection the parity tests derive from. `update_task` — fifteen fields — is the worked example; read the macro's module docs for the grammar and for when `[manual]` is the right mode.

   The macro only fits a tool whose service params type has chained setters, since every mode emits `params = params.setter(…)`. `UpdateTaskParams` qualifies; `UpdateEpicParams` and `CreateTaskParams` are built by struct literal, so converting those tools needs a service-layer builder or a new struct-literal mode first — a design step, not a rename.

5. **Write tests** in `src/mcp/handlers/tests/` (the file matching the tool's domain) using the helpers from `src/mcp/handlers/tests/mod.rs`. The canonical pattern:

   ```rust
   // For tools that only read state — use test_state():
   #[tokio::test]
   // allow-phantom-symbol: illustrative test name, not a real test
   async fn my_tool_returns_expected_data() {
       let state = test_state().await;
       let resp = call(&state, "tools/call",
           Some(json!({ "name": "my_tool", "arguments": { "id": 1 } }))).await;
       assert!(resp.error.is_none());
   }

   // For tools that trigger a fire-and-forget background write (e.g. usage
   // recording) — use test_state_with_bg_done() and await the signal
   // deterministically instead of sleeping:
   #[tokio::test]
   async fn my_tool_records_usage() {
       let (state, mut bg_done) = test_state_with_bg_done().await;
       call(&state, "tools/call",
           Some(json!({ "name": "my_tool", "arguments": {} }))).await;
       // Blocks until the spawned write completes — no tokio::time::sleep needed.
       bg_done.recv().await.expect("bg write signal lost");
   }
   ```

   Never use `tokio::time::sleep` in handler tests — the pre-push hook rejects it. See the "No `tokio::time::sleep` in tests" section of `docs/conventions.md` for the full rationale.

## Removing an MCP Tool

Reverse the steps above, but note that one test does **not** self-heal from the `mcp_tools!` macro like `tools_list_returns_tools` does: `every_tool_with_args_rejects_unknown_field` (`src/mcp/handlers/tests/mod.rs`) hand-lists every tool name in its `payloads`/`no_arg_tools` arrays. Deleting a tool's macro entry without also deleting its line there fails that test's `covered == all_tools` assertion at run time, not at compile time — `cargo build`/`cargo clippy` stay green.

1. Delete the `mcp_tools!` entry in `src/mcp/handlers/dispatch.rs` and the handler function/module.
2. Delete the tool's entry from `every_tool_with_args_rejects_unknown_field`'s `payloads` (or `no_arg_tools`) list.
3. Delete the tool's dedicated test file/module, and any doc references (`docs/module-map.md`, `CLAUDE.md`, relevant `docs/specs/*.allium`).
4. Run the full `cargo test` suite — grepping for the tool name is not enough to catch every reference; this hardcoded list is the proof.

## Adding a New TUI View/Mode

<!-- allow-phantom-symbol: `MyNewView` is the placeholder name for the variant you are adding -->
1. **Add a `ViewMode` variant** in `src/tui/types.rs` (e.g., `ViewMode::MyNewView { selection, saved_board }`).
2. **Add `Message` variants** for entering/exiting and any view-specific actions.
3. **Add `Command` variants** if the view triggers side effects (DB writes, shell commands).
4. **Handle input** in `src/tui/input.rs` — add key handlers under a new match arm for your `ViewMode`.
5. **Handle messages** in `src/tui/mod.rs` `update()` — process your new messages, return commands.
6. **Render** in the appropriate `src/tui/ui/` module — the board renderer is the `src/tui/ui/kanban/` directory (`mod.rs` holds `render()`, with `cards.rs`, `columns.rs`, `status_bar.rs`, and `popups/` beneath it); full-screen overlays live in `src/tui/ui/` (e.g. `input_form.rs`). Add a rendering branch for your view mode in `kanban::render()`.

## Adding a New Entity (with patch builder and sub-trait)

Adding a fully integrated entity involves five layers. Work through them in order:

1. **Domain model** (`src/models/`) — define the struct and any enums in the appropriate domain file. For nullable fields that agents or the TUI can set/clear, plan to use `FieldUpdate` (service layer) and `Option<Option<T>>` double-Option (DB layer); see the [FieldUpdate](conventions.md#fieldupdate--nullable-string-fields) and [TaskPatch/EpicPatch](conventions.md#taskpatch--epicpatch--double-option-in-the-db-layer) conventions.

2. **Store table and reducers** (`spacetime/module/src/`) — add the table and a reducer per write, mirror them in the in-memory store (`src/sync/memory_caller/`, which `Store::open_in_memory` runs), then rebuild with `./scripts/build-managed-module.sh` and regenerate the client bindings with `./scripts/regenerate-spacetime-bindings.sh`. SpacetimeDB automigrates a new table or an appended column; it cannot drop a table that holds rows. See [docs/testing.md](testing.md) ("SpacetimeDB module and CI details").

3. **DB trait and queries** (`src/store/mod.rs`, `src/store/queries/`):
   - Define a narrow sub-trait (e.g., `trait NewEntityCrud`) with CRUD methods. Follow the [trait-narrowing convention](conventions.md#db-trait-narrowing--take-the-narrowest-sub-trait-you-need).
   - Add `NewEntityCrud` to `TaskStore`'s member list, and give every read and write a store path: a table in the SpacetimeDB module, reducers for the writes (a `SharedWriter` method), and the reads on a reader port over `SharedRows` — each routed with `self.shared_writer()?` / `self.shared_reader()?`. See "The store seam" in [conventions.md](conventions.md#the-store-seam--retired-and-where-reads-and-writes-go-instead): there is no local fallback, so a method that is not routed has nothing to answer from.
   - Add `NewEntityCrud` as a supertrait of the store the holders actually carry. `McpState` and `TuiRuntime` hold `Arc<dyn TaskReadStore>` (`src/mcp/mod.rs::McpState`), so a **read** trait belongs on `TaskReadStore`; a **mutating** trait belongs on `TaskStore` and stays out of `TaskReadStore` — that split is what makes bypassing the service layer a compile error.
   - Implement `impl NewEntityCrud for Store` under `src/store/queries/` (a new file per domain, wired into `src/store/queries/mod.rs`). Each method hands the call to the attached port; there is no local connection.
   - Define a `NewEntityPatch` builder struct with `Option<Option<T>>` for nullable fields; carry it through the port's reducer call.
   - Write a corresponding `NewEntityFilter` if list queries need filtering.

4. **Service layer** (`src/service/<entity>.rs`) — create `NewEntityService` holding `Arc<dyn NewEntityCrud>`. Add `create_`, `get_`, `list_`, `update_`, and any lifecycle methods. Use `ServiceError::Validation` for input errors, `ServiceError::NotFound` for missing rows, and `anyhow` for DB I/O errors. Accept `FieldUpdate` for nullable string fields, map to `Option<Option<T>>` before writing the patch. Declare the new module in `src/service/mod.rs` and add `pub use` re-exports so callers are unaffected.

5. **MCP handler** (if agents need to interact) — follow [Adding a New MCP Tool](#adding-a-new-mcp-tool). For read-only tools, hold the narrowest sub-trait; for mutating tools, route the write through the service layer (never `state.db`) and call `state.notify()` afterwards.

6. **Tests**:
   - DB-layer tests in `src/store/tests/` (the file matching the entity's domain) using `Store::open_in_memory()` (the in-memory store).
   - Service-layer tests inline in the corresponding `src/service/<entity>.rs` file.
   - MCP handler tests in `src/mcp/handlers/tests/` (the file matching the tool's domain) for any new tools.

7. **Spec** (`docs/specs/`) — write or extend an Allium spec to document the entity's lifecycle, rules, and invariants. Use the `allium:tend` skill and run `allium check` to validate syntax.

## Knowledge Base MCP Tools

Four MCP tools manage the knowledge base from within an agent session:

- **`record_learning`** — record a new entry in the knowledge base (immediately active in future dispatch prompts)
- **`query_learnings`** — retrieve approved entries relevant to the current task's context; supports `tag_filter` and `limit`
- **`rate_learning`** — give feedback on a retrieved entry: `helped` increments `upvote_count`; `wrong` decrements it (a downvote; may go negative) without changing status
- **`delete_learning`** — permanently delete a knowledge base entry by ID; returns an error if the ID does not exist

**When to call these tools:**
- Call `query_learnings` at the right moment — not just at task start.
- Call `record_learning` when you discover a pattern worth capturing for future agents (pitfall, convention, landscape, etc.).
- Call `rate_learning` when you act on a retrieved entry — `helped` if it applied, `wrong` if it misled you. Only entries surfaced to you this task (injected or returned by `query_learnings`) can be rated.

**Scope auto-derivation:** omit `scope_ref` — the MCP handler derives it from the task's repo or epic automatically. Pass `scope_ref` explicitly only to override.

**Task-scoped learnings** are not auto-injected into dispatch prompts. Use `query_learnings` with `tag_filter` to retrieve them when needed.

**Scopes at retrieval time**: a `query_learnings` call for a task returns the union of all approved learnings where:
- `scope = user` (always included)
- `scope = repo` and `scope_ref` matches the task's repo path
- `scope = epic` and `scope_ref` matches the task's epic (only if the task belongs to an epic)

See `docs/reference.md` → *Learning Store* for the full scoping model with examples.
