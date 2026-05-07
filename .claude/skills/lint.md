---
name: lint
description: Consult before wrapping up work in the dispatch repo — explains custom Clippy lint rules, how to add new ones, and how to run clippy locally.
---

# Lint Conventions for the Dispatch Repo

## What custom lint rules are

This repo maintains a `[lints.clippy]` section in `Cargo.toml`. Rules here define which Clippy lints are active and at what level. They are enforced on every push by the pre-push hook:

```bash
cargo clippy --all-targets --fix -- -D warnings
```

The `--fix` flag auto-applies fixable suggestions. The `-D warnings` flag escalates all warnings to hard errors, so any rule set to `"warn"` becomes a push blocker.

**Important:** the hook uses `--fix`, so it will silently modify source files during a push. Run clippy manually first (without `--fix`) to see violations before the hook fixes them for you:

```bash
cargo clippy --all-targets -- -D warnings
```

## Adding a new rule

When you discover a pattern worth enforcing across the codebase, add it to `[lints.clippy]` in `Cargo.toml` as part of your PR. Include a structured comment:

```toml
[lints.clippy]
# Added by task #NNN (YYYY-MM-DD): <reason this rule matters>
your_lint_name = "warn"
```

Always use `"warn"` — not `"deny"`. The pre-push hook's `-D warnings` already makes all warns into errors. Adding `"deny"` directly is redundant and harder to suppress.

## Fixing violations

For each violation:
- **Production code:** use `?` propagation or a `match`/`if let` instead of `.unwrap()` or `.expect()`. Note that `expect_used` is also enabled, so replacing `.unwrap()` with `.expect()` just moves the violation.
- **Test helpers that intentionally panic** (e.g. `MockProcessRunner` internals, test fixtures): suppress with `#[allow(clippy::unwrap_used)]` on the specific function — not the whole module.

## Current rules

See `[lints.clippy]` in `Cargo.toml` for the full list with rationale comments.
