# How-To Guides

## Adding a New MCP Tool

1. **Define the handler** in `src/mcp/handlers/tasks/crud.rs` (or `epics.rs` for epic tools; `tasks/wrap_up.rs` for session-lifecycle tools). Follow the pattern: parse args with `types::parse_args`, call `state.db` methods, call `state.notify()` if mutating, return `JsonRpcResponse::ok`.
2. **Add the tool schema** to `tool_definitions()` in `src/mcp/handlers/dispatch.rs` — add a new entry to the `tools` array with `name`, `description`, and `inputSchema`.
3. **Wire the route** in `handle_mcp()` in `src/mcp/handlers/dispatch.rs` — add a match arm in the `tools/call` section mapping the tool name to your handler.
4. **Add types** if needed in `src/mcp/handlers/types.rs` (argument structs with serde derives, use `#[serde(deserialize_with = "deserialize_flexible_i64")]` for integer fields since Claude Code may send them as strings).
5. **Write tests** in `src/mcp/handlers/tests/` (the file matching the tool's domain) using `Database::open_in_memory()`.

## Adding a New TUI View/Mode

1. **Add a `ViewMode` variant** in `src/tui/types.rs` (e.g., `ViewMode::MyNewView { selection, saved_board }`).
2. **Add `Message` variants** for entering/exiting and any view-specific actions.
3. **Add `Command` variants** if the view triggers side effects (DB writes, shell commands).
4. **Handle input** in `src/tui/input.rs` — add key handlers under a new match arm for your `ViewMode`.
5. **Handle messages** in `src/tui/mod.rs` `update()` — process your new messages, return commands.
6. **Render** in the appropriate `src/tui/ui/` module (`kanban.rs`, `review.rs`, or `security.rs`) — add a rendering branch for your view mode in `kanban.rs::render()`.

## Adding a New Entity (with patch builder and sub-trait)

Adding a fully integrated entity involves five layers. Work through them in order:

1. **Domain model** (`src/models/`) — define the struct and any enums in the appropriate domain file. For nullable fields that agents or the TUI can set/clear, plan to use `FieldUpdate` (service layer) and `Option<Option<T>>` double-Option (DB layer); see the [FieldUpdate](conventions.md#fieldupdate--nullable-string-fields) and [TaskPatch/EpicPatch](conventions.md#taskpatch--epicpatch--double-option-in-the-db-layer) conventions.

2. **Database migration** (`src/db/migrations.rs`) — write `migrate_vN_description(conn)` and register it in `MIGRATIONS`. See [Adding a Database Migration](#adding-a-database-migration) for the full procedure.

3. **DB trait and queries** (`src/db/mod.rs`, `src/db/queries/`):
   - Define a narrow sub-trait (e.g., `trait NewEntityCrud`) with CRUD methods. Follow the [trait-narrowing convention](conventions.md#db-trait-narrowing--take-the-narrowest-sub-trait-you-need).
   - Add `NewEntityCrud` as a supertrait of `TaskStore` so existing holders (`McpState`, `TuiRuntime`) get it automatically.
   - Implement `impl NewEntityCrud for Database` under `src/db/queries/` (a new file per domain, wired into `src/db/queries/mod.rs`) using `self.conn()?`.
   - Define a `NewEntityPatch` builder struct with `Option<Option<T>>` for nullable fields; implement the `UPDATE` query.
   - Write a corresponding `NewEntityFilter` if list queries need filtering.

4. **Service layer** (`src/service/<entity>.rs`) — create `NewEntityService` holding `Arc<dyn NewEntityCrud>`. Add `create_`, `get_`, `list_`, `update_`, and any lifecycle methods. Use `ServiceError::Validation` for input errors, `ServiceError::NotFound` for missing rows, and `anyhow` for DB I/O errors. Accept `FieldUpdate` for nullable string fields, map to `Option<Option<T>>` before writing the patch. Declare the new module in `src/service/mod.rs` and add `pub use` re-exports so callers are unaffected.

5. **MCP handler** (if agents need to interact) — follow [Adding a New MCP Tool](#adding-a-new-mcp-tool). For read-only tools, hold the narrowest sub-trait; for mutating tools, call `state.notify()` after the write.

6. **Tests**:
   - DB-layer tests in `src/db/tests/` (the file matching the entity's domain) using `Database::open_in_memory()`.
   - Service-layer tests inline in the corresponding `src/service/<entity>.rs` file.
   - MCP handler tests in `src/mcp/handlers/tests/` (the file matching the tool's domain) for any new tools.

7. **Spec** (`docs/specs/`) — write or extend an Allium spec to document the entity's lifecycle, rules, and invariants. Use the `allium:tend` skill and run `allium check` to validate syntax.

## Adding a Database Migration

Migrations live in `src/db/migrations.rs` as standalone functions. We do **not** squash migrations — see the module-level doc comment in `src/db/migrations.rs` for the policy.

1. **Write the migration function**: `fn migrate_vN_description(conn: &Connection) -> Result<()>` in `src/db/migrations.rs`. Use `ALTER TABLE` for additive changes; for destructive changes (column removal, constraint changes), create a new table, copy data, drop old, rename.
2. **Register it** in the `MIGRATIONS` array in `src/db/migrations.rs`: add `(N, migrate_vN_description)`. The loop in `Database::init_schema()` applies any migration where `current_version < N` and bumps `PRAGMA user_version` after each.
3. **Update the schema test**: `fresh_db_has_latest_schema_version` in `src/db/tests/migrations.rs` asserts the final version number — bump it to match your new N.
4. **Write a migration test** in `src/db/tests/migrations.rs` that creates a DB at the pre-migration schema, inserts test data, runs the migration, and verifies the result.
5. **Cross-reference superseded migrations.** When a later migration drops or replaces a table/column introduced by an earlier one (create-then-drop pattern), add an inline comment on both `MIGRATIONS` entries noting the relationship — e.g. `// superseded by vN` on the original and `// drops table created in vM` on the new one. This prevents agents from trying to re-add something that was intentionally removed.

## Projects Feature

Projects group tasks and epics for board filtering. See `docs/specs/projects.allium` for the full domain specification.

**Filter semantics:**
- `App.active_project: ProjectId` is the active board filter.
- **Default project active** → show all tasks/epics regardless of `project_id` (catch-all view).
- **Any other project active** → show only items where `item.project_id == active_project.id`.
- The filter is applied in `project_matches()` at four call sites in `tui/mod.rs`: task column rendering, epic column rendering, archive view, and search results.

**Default project pinning:**
- The Default project is seeded at DB init (migration v39, `is_default = 1`). There is exactly one default.
- The Default project cannot be deleted. `delete_project_and_move_items` checks `is_default` before proceeding.
- Deleting any other project moves all its tasks and epics to Default in a single DB transaction, preventing orphaned items.
- Users can rename the Default project but cannot change `is_default`.

**Why TUI-only admin state:**
- Projects are never mutated by MCP agents — there are no MCP tools for project management. Only humans create, rename, reorder, and delete projects from the TUI panel.
- The project list is refreshed only after explicit project-mutating commands (`CreateProject`, `RenameProject`, `DeleteProject`, `ReorderProject`), not on every MCP tick.

**Panel behavior:**
- The projects panel is a left-side overlay opened with `h` (or `Left`) from column 0 (Backlog). While visible it intercepts all input before normal board key handling.
- Moving the cursor with `j`/`k` immediately activates the hovered project (hover-to-filter). `Enter`, `g`, `l`, `Right`, and `Esc` close the panel, keeping the currently activated project.
- The panel cursor resets to the active project on each open.

**Delete confirmation:**
- Deleting a project is a two-step confirmation: first `D` opens `ConfirmDeleteProject1`; after confirming, `ConfirmDeleteProject2` shows the count of tasks/epics that will be moved to Default. The user types `y` or presses Enter to proceed.

**Implementation details:**
- `ProjectId = i64` (type alias, not newtype) — simpler rusqlite integration. No FK constraint in the schema; integrity is enforced at the service/runtime layer.
- `exec_refresh_projects_from_db` follows the `exec_refresh_*_from_db` naming pattern (see `src/runtime/tasks.rs`).

## Knowledge Base MCP Tools

Three MCP tools manage the knowledge base from within an agent session:

- **`record_learning`** — record a new entry in the knowledge base (immediately active in future dispatch prompts)
- **`query_learnings`** — retrieve approved entries relevant to the current task's context; supports `tag_filter` and `limit`
- **`rate_learning`** — give feedback on a retrieved entry: `helped` increments `upvote_count`; `wrong` routes an approved entry to `needs_review`

**When to call these tools:**
- Call `query_learnings` at the right moment — not just at task start.
- Call `record_learning` when you discover a pattern worth capturing for future agents (pitfall, convention, landscape, etc.).
- Call `rate_learning` when you act on a retrieved entry — `helped` if it applied, `wrong` if it misled you. Only entries surfaced to you this task (injected or returned by `query_learnings`) can be rated.

**Scope auto-derivation:** omit `scope_ref` — the MCP handler derives it from the task's project, repo, or epic automatically. Pass `scope_ref` explicitly only to override.

**Task-scoped learnings** are not auto-injected into dispatch prompts. Use `query_learnings` with `tag_filter` to retrieve them when needed.

**Scopes at retrieval time**: a `query_learnings` call for a task returns the union of all approved learnings where:
- `scope = user` (always included)
- `scope = repo` and `scope_ref` matches the task's repo path
- `scope = project` and `scope_ref` matches the task's project
- `scope = epic` and `scope_ref` matches the task's epic (only if the task belongs to an epic)

See `docs/reference.md` → *Learning Store* for the full scoping model with examples.
