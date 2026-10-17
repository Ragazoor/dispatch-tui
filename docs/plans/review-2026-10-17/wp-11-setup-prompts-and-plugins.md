# Setup Prompts and Plugin Install

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Setup never treats a missing answer as consent, never deletes outside its own files, and never loses its script manifest.

## Context

This work package addresses findings 4, 26, 27, 38 and 40 of the 2026-10-17
code review (`docs/plans/2026-10-17-codebase-review.md`). Do not run
`dispatch setup` or the TUI to verify — it rewrites real config under
`$HOME`; use the tests' temp directories. Spec: `docs/specs/startup.allium`,
`docs/specs/settings.allium`.

## Findings

### 🟡 EOF at a yes/no prompt counts as "yes" (`src/setup/confirm.rs:33`)

**Issue:** `StdinConfirmer::prompt` treats `read_line` → `Ok(0)` as `""`,
i.e. the default. The startup "Update it now?" defaults to yes, so Ctrl-D or
a closed stdin rewrites `~/.claude.json`, plugins and tmux.conf.
`src/startup/config.rs:119` says the opposite must hold.

**Fix:** On `Ok(0)` return `Err` (or `false`) in `confirm`,
`confirm_dangerous` and `prompt_text`. Make the read testable (inject a
reader) if it is not.

### 🟡 Shipped-script install error skips the manifest (`src/setup/plugins.rs:548`)

**Issue:** `install_one_shipped_script` documents "never fails the run" but
`confirm_dangerous(...)?` propagates; the loop returns early and
`write_script_manifest` is skipped, losing provenance digests.

**Fix:** Map a prompt error to `Kept`/`Failed` for that script; always write
the manifest.

### 🟡 Stale-plugin cleanup follows symlinks (`src/setup/plugins.rs:79`)

**Issue:** `remove_stale_recursive` uses `path.is_dir()`, which follows
symlinks, and deletes every non-embedded file. A symlinked skill directory
has its target's files deleted.

**Fix:** Use `entry.file_type()`; unlink a symlink without recursing.
Consider listing files to be deleted in the drift prompt.

### 🟢 Low findings

- `src/setup/config.rs:108` (`merge_mcp_config`) — a non-object
  `mcpServers` never gets the entry but reports `changed: true`, so the user
  is prompted every launch. Replace with `{}` or return an error; same for a
  non-object top level.
- `src/editor.rs:31` (`resolve_editor`) — `split_whitespace` breaks quoted
  `$EDITOR`/`$VISUAL`. Use shell-word splitting (`shlex`) or run via `sh -c`.

## Changes

| File | Change |
|------|--------|
| `src/setup/confirm.rs` | EOF is not consent |
| `src/setup/plugins.rs` | Manifest always written; symlink-safe cleanup |
| `src/setup/config.rs` | Non-object `mcpServers` |
| `src/editor.rs` | Quoted editor commands |

## Verification

- [ ] Tests first: EOF reader → not confirmed; symlinked dir under the plugin root keeps its target's files; prompt error still writes the manifest
- [ ] `cargo test --no-fail-fast` and the task's verify command pass
