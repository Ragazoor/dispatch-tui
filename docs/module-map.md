# Module Map

| File | Responsibility |
|------|---------------|
| `src/main.rs` | CLI entry point (clap), subcommand dispatch (`tui`, `setup`, `verify-feed`, …) |
| `src/lib.rs` | Crate root, public module re-exports |
| `src/runtime/mod.rs` | Async event loop (`tokio::select!`), bridges TUI ↔ MCP ↔ shell commands |
| `src/runtime/commands.rs` | `Command` side-effect dispatcher (called by `execute_commands`) |
| `src/runtime/tasks.rs` | Per-command runtime handlers for tasks (refresh, dispatch, finish, etc.) |
| `src/runtime/{agents,epics,learnings,pr,settings,split,editor}.rs` | Domain-specific runtime helpers |
| `src/tui/mod.rs` | `App` struct, lifecycle, `update()` entry point, `column_items_for_status()` render helper |
| `src/tui/dispatcher.rs` | `dispatch(app, msg)` routing table — match arm per `Message` variant |
| `src/tui/update/` | Per-message handlers (`agent.rs`, `epics.rs`, `feeds.rs`, `forms.rs`, `learnings.rs`, `lifecycle.rs`, `main_session.rs`, `navigation.rs`, `pr.rs`, `repo_filter.rs`, `retry.rs`, `selection.rs`, `split_pane.rs`, `system.rs`, `tips_projects.rs`, `wrap_up.rs`) |
| `src/tui/input.rs` | Key event entry point, inline-mutation convention for UI-only state |
| `src/tui/input/` | Per-mode key handlers: `normal.rs`, `confirm.rs`, `projects.rs`, `repo_filter.rs` |
| `src/tui/ui/mod.rs` | Rendering entry point — re-exports `render()`, thin dispatcher |
| `src/tui/ui/kanban/` | Kanban board rendering: `mod.rs` entry, `cards.rs`, `columns.rs`, `status_bar.rs`, `projects_panel.rs`, `popups/` overlays |
| `src/tui/ui/shared.rs` | Cross-board helpers: `render_tab_bar`, `refresh_status`, `truncate`, `push_hint_spans` |
| `src/tui/ui/palette.rs` | Tokyo Night color palette constants |
| `src/tui/ui/{input_form,learnings}.rs` | Overlay renderers (input forms, knowledge base panel) |
| `src/tui/types.rs` | `Message`, `Command`, `ViewMode`, `InputMode`, `AgentTracking` enums and structs |
| `src/tui/tests/` | TUI unit and scenario tests, snapshots, helpers |
| `src/models/mod.rs` | Re-exports of domain types and shared model tests |
| `src/models/tasks.rs` | `Task`, `TaskStatus`, `SubStatus`, `TaskTag`, `DispatchMode::for_task()` tag routing |
| `src/models/{epics,learnings,projects,review}.rs` | Domain types per area |
| `src/service/mod.rs` | Service module root: `ServiceError`, `FieldUpdate`, re-exports of all sub-module types |
| `src/service/tasks/mod.rs` | `TaskService` — task business logic |
| `src/service/tasks/{crud,params,validators}.rs` | Task CRUD methods, `*Params` request types, validation helpers |
| `src/service/epics.rs` | `EpicService`, `UpdateEpicParams`, `CreateEpicParams` — epic business logic |
| `src/service/learnings.rs` | `LearningService`, `CreateLearningParams`, `UpdateLearningParams` — learning business logic |
| `src/db/mod.rs` | `Database` struct, constructor, `TaskStore` trait, `TaskPatch`/`EpicPatch` builders |
| `src/db/migrations.rs` | Versioned schema migrations (`MIGRATIONS` array, `migrate_vN_*` functions) |
| `src/db/queries/mod.rs` | `impl TaskStore for Database` — fans out across the per-domain query files |
| `src/db/queries/{tasks,epics,prs,alerts,projects,learnings,settings}.rs` | CRUD per domain |
| `src/db/tests/mod.rs` | Database unit tests entry point |
| `src/db/tests/{tasks,epics,prs,alerts,projects,learnings,settings,migrations}.rs` | Tests per domain |
| `src/dispatch/mod.rs` | Worktree creation, tmux session management, agent lifecycle (dispatch/brainstorm/plan/resume/review) |
| `src/dispatch/agents.rs` | Agent-specific dispatch helpers |
| `src/dispatch/prompts.rs` | Prompt construction (with-plan, no-plan variants, learning injection) |
| `src/dispatch/worktree.rs` | Worktree creation/teardown |
| `src/dispatch/finish.rs` | Rebase + fast-forward branch onto base branch, kill tmux window (`finish_task`); defines `FinishError` |
| `src/process.rs` | `ProcessRunner` trait + `RealProcessRunner` / `MockProcessRunner` for testable shell execution |
| `src/tmux.rs` | Tmux API: create windows, send keys, capture pane output, kill windows |
| `src/editor.rs` | External `$EDITOR` integration for editing task/epic fields |
| `src/plan.rs` | Plan file parsing (extract title/description from markdown) |
| `src/setup/mod.rs` | First-run setup entry point |
| `src/setup/{config,plugins,hooks}.rs` | MCP config merging, plugin installation, git hook installation |
| `src/mcp/mod.rs` | MCP server bootstrap (Axum router), `McpState`, `McpEvent` notification enum |
| `src/mcp/handlers/dispatch.rs` | JSON-RPC entry point (`handle_mcp`), tool definitions, method routing |
| `src/mcp/handlers/tasks.rs` | Task tool handlers (thin wrappers): parse JSON-RPC args → call `TaskService` → format response |
| `src/mcp/handlers/epics.rs` | Epic tool handlers (thin wrappers): parse JSON-RPC args → call `EpicService` → format response |
| `src/mcp/handlers/learnings.rs` | Knowledge base tool handlers |
| `src/mcp/handlers/types.rs` | JSON-RPC request/response types, flexible integer deserializer |
| `src/mcp/handlers/tests/mod.rs` | MCP handler integration tests entry point |
| `src/mcp/handlers/tests/{tasks,epics,learnings,projects}.rs` | MCP handler tests per domain |
