# Dispatch

Terminal kanban board for dispatching Claude Code agents into isolated git worktrees via tmux.

**Stack**: Rust (2021 edition), ratatui TUI, SpacetimeDB shared store, Axum HTTP/MCP server, tokio async runtime.

**Live data is in SpacetimeDB, not `tasks.db`.** `tasks.db` is a leftover and not authoritative; query tasks through the MCP tools (`list_tasks`, `get_task`). See "Where the live data is" in [docs/reference.md](docs/reference.md).

`AGENTS.md` is a symlink to this file. Edit `CLAUDE.md`.

## Build & Test

```bash
cargo build
cargo test
cargo run -- tui
```

**This repo has a verify command every dispatched agent must run green before declaring work complete — read it from `get_task`'s *Verify command* line, never from this file.** See "Verify Command" below.

**Dispatch-spawned sessions do not run under Claude Code's sandbox** — see `SandboxDisabledForDockerAndUnixSockets` in `docs/specs/dispatch.allium`. If you've enabled the sandbox yourself outside of dispatch, see "Sandbox (historical)" in [docs/reference.md](docs/reference.md).

**`spacetime/module/` is a separate wasm crate.** Root `cargo test` does not cover it; its checks, the committed `module.wasm` and the client bindings are in [docs/testing.md](docs/testing.md) ("SpacetimeDB module and CI details"). Rebuild with `./scripts/build-managed-module.sh` after a module change, and regenerate bindings with `./scripts/regenerate-spacetime-bindings.sh`.

**The full suite needs `tmux` on `PATH`.** Without it the `tmux_*` targets print `skipping: tmux not available on PATH` and pass, so a green local run isn't proof they ran.

**Don't pipe `cargo test` into `tail`/`head`/`grep`.** A pipeline's exit code is the last command's, so a failing suite reads as a clean pass. Redirect instead: `cargo test > /tmp/t.txt 2>&1; echo $?`.

**Run `cargo test --no-fail-fast`.** Plain `cargo test` stops at the first failing target, so later targets never run. The live-store tests in `tests/spacetime_module.rs` can flake on a one-off `Connection reset by peer`; `Instance::publish` retries once on a transport error, so a failure that survives is real.

**Raw coverage includes generated bindings.** Pass `--exclude-files 'src/spacetime/bindings/*'`, as CI does, or the figure is not comparable to the floor.

**`--data-dir` names a directory** (`DISPATCH_DATA_DIR`). The old `--db` flag is gone.

**The lib target runs in ~10s; a cold full run (including compile) is ~80s.** Run it in the foreground — don't background it. In a *fresh worktree* the first compile is slower than that and a cold `cargo test` can pass 120s, which is Claude Code's default Bash timeout — so pass an explicit `timeout` on the first run of a session rather than letting the harness background it out from under you.

**Local coverage**: `cargo tarpaulin --engine llvm --out stdout`. Always pass `--engine llvm`: the default engine scores ~1.8 points lower than CI's floor assumes. With `spacetime` on `PATH` the live-store tests run under it too, as in CI; that is slower, not broken. Other caveats are in [docs/testing.md](docs/testing.md).

Everything else about tests — the per-target command list, snapshot workflow, where a new test belongs, the no-wall-clock-sleep rule, coverage — is in [docs/testing.md](docs/testing.md).

**`main` moves while you work.** Before wrapping up, run `git log --oneline main..HEAD` **and** `git log --oneline HEAD..main`. If the second is non-empty, merge `main` into your branch and re-run the suite. Compare against local `main`, never `origin/main`, and skim new commits' content, not just their files. Why, and the traps: [docs/conventions.md](docs/conventions.md) ("`main` moves while you work").

### First-time setup

A fresh clone must point git at the tracked hooks once: `git config core.hooksPath .githooks`. It also needs the wasm target (`sudo dnf install rust-std-static-wasm32-unknown-unknown`, or `rustup target add wasm32-unknown-unknown`) — the hook's last step builds the SpacetimeDB module and hard-fails without it, so a docs-only push fails at push time with no earlier warning. Nothing does this for you, and until it is run the whole gate below is silently inert locally — CI runs the same checks (see "CI" below), so skipping the setup costs you the fast local feedback, not the enforcement. Don't add hooks to `.git/hooks/` directly — that directory is untracked and shared across all worktrees, so changes there aren't version-controlled or reviewed.

**Install `sccache`** (`sudo dnf install sccache`): dispatch sets `RUSTC_WRAPPER=sccache` on agent windows when it is on `PATH` (`DispatchedAgentsShareASccacheNotATargetDir` in `docs/specs/dispatch.allium`). Do not share a `CARGO_TARGET_DIR` across worktrees. Raise the cache cap with `SCCACHE_CACHE_SIZE` (e.g. `50G`).

The pre-push hook (`.githooks/pre-push`) runs `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, the three doc checkers (`check-doc-paths.sh`, `check-doc-symbols.sh`, `check-doc-headings.sh`, each with a self-test), `check-no-test-sleep.sh`, `test-fetch-reviews.sh` and the spacetime module check. `cargo test` is not part of the hook; run it yourself before pushing. To exempt a deliberate citation, annotate it with `allow-phantom-symbol: <why>` or `allow-phantom-heading: <why>`.

**That `cargo fmt` step has no `--check`.** Pushing reformats your working tree in place, so a push can leave you with unstaged changes you did not make. Run `cargo fmt` yourself before committing and the step becomes a no-op.

Cite `path::symbol` (`src/feed/exec.rs::exec_feed_command`) rather than `file:NN` in docs: a symbol is checked against the real file, a line number only for existence. See "`file:NN` vs `path::symbol` citations" in `docs/conventions.md`.

### CI

CI runs Test, Clippy, Format, Coverage and Gate scripts (a mirror of the pre-push hook). Coverage is gated. Details: [docs/testing.md](docs/testing.md).

## Running & Debugging Locally

`cargo run -- tui` needs `tmux` on `PATH`. Outside tmux it starts its own `dispatch` session; where one exists it restarts the board and the agent windows survive. See `docs/specs/startup.allium`.

**Point dev runs at a throwaway data directory, store and port**: `cargo run -- --data-dir /tmp/scratch --spacetime-server http://127.0.0.1:3099 tui --port 8899` (stand up the store as "Running & Debugging Locally" in `docs/reference.md` describes). **A bare `cargo run -- tui` runs the *managed* store: it adopts whatever answers on `127.0.0.1:3000` (your real store) and stops it on exit.**

**A throwaway `--data-dir` does not sandbox the run.** It redirects the data directory only; the startup configuration check still rewrites the real Claude Code configuration under `$HOME`. Verify configuration behaviour through the tests, which use temp directories, not by running the TUI.

Logs do not go to stderr — stderr belongs to the TUI. They append to `app.log` in the data directory; `tail -f ~/.local/share/dispatch/app.log`. Data directory, port, environment variables, the full CLI subcommand list, and troubleshooting are in [docs/reference.md](docs/reference.md); driving MCP by hand is in [docs/mcp.md](docs/mcp.md).

**Never run `tmux kill-server`, and never drive tmux by hand without an explicit `-L <unique-socket>`.** You run inside the operator's own tmux server, next to every other agent. A glob loop that matches nothing falls back to the default socket and kills them all. Test real tmux through `tests/tmux_harness/mod.rs`, which gives each test a private socket.

**`tmux display-message -p` without `-t` answers about the session's *active* window, not yours.** Pass `tmux::self_pane_id()` (`$TMUX_PANE`) as the target. Read the env at the entry point and pass it down; mock tests cannot catch this.

## External Dependencies

**tmux**, **git**, **gh**, and **claude** must be on `PATH` at runtime. There is **no startup preflight** — nothing checks binary availability, so a missing binary surfaces as a failed shell command mid-operation. Per-binary detail (which module calls what, and the two load-bearing `claude` flags) is under "External Dependencies" in [docs/reference.md](docs/reference.md).

Dispatched agents do not run under Claude Code's sandbox mode (see "Build & Test" above). `bubblewrap` and `socat` on `PATH` (`sudo dnf install bubblewrap socat` on Fedora) only matter if you re-enable the sandbox yourself — see `SandboxedAgentExecution` in `docs/specs/dispatch.allium`; if either is missing, Claude Code warns and silently falls back to running unsandboxed rather than failing to start.

POSIX-only. Embeddings/RAG (`src/service/embeddings.rs`) run **locally** — `fastembed` does inference in-process, no API key, no per-call network I/O. The only network activity is a one-time model download on first init.

## Verify Command

Read it from `get_task`'s "Verify command" line, never from this file; how it is stored and surfaced is in [docs/reference.md](docs/reference.md) ("Verify Command").

## Working With the User

The most important thing is to stay aligned with the user. The Allium specs in `docs/specs/` are the shared source of truth that alignment is expressed in — when the spec and your intent agree, you are aligned; when they don't, one of them is wrong and it must be resolved before code is written.

- **Ambiguity is a stop condition, not a judgement call.** If the spec is silent, contradictory, or open to more than one reading, ask. Do not pick the plausible interpretation and proceed.
- **Behaviour changes start in the spec.** Spec first, then tests, then code. This applies to UI and interaction behaviour too — that is a first-class Allium surface, not a prose note.
- **Agreement gets recorded, in one of two places.** A decision about *what the system does* goes into the relevant `docs/specs/*.allium` file. A decision about *how to work in this repo* — a convention, a pitfall, a gotcha that would trip the next agent — goes into the knowledge base via `record_learning`. A decision that lives only in the conversation is lost the moment the session ends.

## Allium Specification

The Allium specs in `docs/specs/` are the **source of truth** for domain and interaction behaviour. Each filename names its domain — `tasks.allium`, `dispatch.allium`, `epics.allium`, `feeds.allium`, and so on; `core.allium` holds the shared domain model. Consult the relevant spec before changing core behaviour, and use the `allium:tend` and `allium:weed` skills to keep spec and code aligned.

## MCP Tools for Agents

The `dispatch` MCP server's tool descriptions say what each tool does; call `tools/list`, or see `docs/specs/mcp-task-tools.allium`. `create_task`/`create_epic` matter mainly to orchestrating agents, not to one executing a single task.

## Agent-Facing Skill Copy

`plugin/skills/*/SKILL.md` is the source of truth for the skills agents run; edit it, never the installed copy under `~/.claude/plugins/`. A skill must be a directory containing `SKILL.md` (a bare `.md` is never loaded). Install path, test gates and frontmatter contract: [docs/reference.md](docs/reference.md) ("Agent-Facing Skill Copy").

## Agent Working Directory

Dispatched agents always work from their worktree folder. Every prompt includes an instruction to stay in the worktree and not `cd` to the parent repo. The tmux window's *starting* cwd is test-covered: `src/dispatch/tests/agent_launch.rs::dispatch_agent_opens_tmux_window_in_worktree_not_parent_repo` asserts the window opens inside the task worktree, never the bare parent repo. Runtime `cd`-escape prevention — an agent later `cd`ing out of the worktree — remains prompt-instruction only, with no test asserting against it.

<!-- allow-phantom-symbol: file_path names a Claude Code tool parameter, not a repo symbol -->
**A second, easier way to leave the worktree: an absolute Read/Edit/Write `file_path` missing the `.worktrees/<id>-<slug>/` segment.** The parent repo's path and the worktree's path both look like valid absolute paths and differ only by that one segment, but using the former silently edits the parent checkout instead of the worktree — the tool reports success and even Read echoes the change back, so nothing looks wrong until a shell command (`git status`, `cargo build`) run against the *worktree* path shows no change. If the parent checkout has any auto-commit/snapshot tooling watching it, a stray edit like this can land on shared `main` before anyone notices. Always build `file_path` from the worktree's own absolute path (e.g. from `pwd`), never assume the parent repo's path with the task ID spliced in.

## Documentation

This file is intentionally slim — it is loaded into every agent's context. Seams where code that compiles and passes tests can still be wrong. Each has a paragraph in [docs/invariants.md](docs/invariants.md) — read the one that matches what you are touching:

- **`FieldUpdate`/`TaskPatch`** nullable mutations — also read [docs/conventions.md](docs/conventions.md) before writing an update handler.
- **Mutation boundary** — task/epic writes go through `TaskServiceApi`/`EpicServiceApi`, never `state.db`.
- **Dispatch seam** — launching an agent is `TaskService::dispatch`; extend it, never re-derive it.
- **Layout-cache coherence**, **board read source**, **one store, no routing layer**, **DB connection model** — the TUI and storage seams.
- **Render-panic policy**, **unsafe policy**, **tag system**, **read-side layering**, **`#[cfg(test)]` gating**, **workhorse macros**, **timing constants**.
- Bare `unwrap()`/`expect()` outside tests only hard-fail under `cargo clippy --all-targets -- -D warnings` (the pre-push hook).

Read these on demand:

- [docs/invariants.md](docs/invariants.md) — the seams and policies listed above, one paragraph each
- [docs/testing.md](docs/testing.md) — running tests, snapshot workflow, where a new test goes, the no-sleep rule, coverage
- [docs/architecture.md](docs/architecture.md) — Message→Command, ProcessRunner, command queue draining, editor session invariant, layout-cache coherence, render dirty flag, error handling, quick dispatch
- [docs/conventions.md](docs/conventions.md) — the full convention set: `FieldUpdate`/`TaskPatch` double-Option, DB/service trait narrowing, the `run_bounded` primitive, keybinding telemetry, Clippy/visibility rules, tag system, and more
- [docs/module-map.md](docs/module-map.md) — module and subsystem responsibilities
- [docs/how-to.md](docs/how-to.md) — adding an MCP tool, TUI view, entity; knowledge base MCP tools
- [docs/mcp.md](docs/mcp.md) — MCP notification flow, error codes, debugging handlers, feed epics, knowledge base flow
- [docs/reference.md](docs/reference.md) — key bindings, CLI subcommands, configuration, environment variables, troubleshooting, learning store
- [docs/specs/](docs/specs/) — Allium specifications for domain logic (one file per domain: `tasks`, `dispatch`, `epics`, `feeds`, `learnings`, `sync`, `storage`, `startup`, …; `core` holds the shared model — `ls docs/specs/`)
- [docs/plans/](docs/plans/) — implementation plans and one-off analysis/review docs. **This repo commits them** (the policy was reversed on 2026-07-26); older entries are filed under `docs/plans/archive/`. Neither doc checker scans them, so treat their contents as dated snapshots.

Subsystem entry points (no dedicated doc page — read the source):

- `src/feed/mod.rs` — feed system: `FeedRunner` poll loop, exec/parse/ingest pipeline that upserts tasks from external commands
- `src/cli/` — CLI subcommand implementations, one module per subcommand (`ls src/cli/`)
- `src/mcp/trajectory.rs` — agent trajectory capture (records the agent's tool-call history for a task)
- `src/repo_sync.rs` — local-first repo sync: `ahead_behind` drift measurement and `sync_repo` (fetch, merge `origin/<base>`, push). See `docs/specs/repo-sync.allium`
- `src/sync/` — the shared store: the connection loop, the identity handshake, and **where the board's cards come from**. `board_reads::BoardReads` is that seam, served by the subscription (`SubscriptionBoardReads`), or by the in-memory store a test handle owns. See `docs/specs/sync.allium`
