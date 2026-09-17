#!/usr/bin/env bash
# Build and test the SpacetimeDB module crate.
#
# WHY THIS SCRIPT EXISTS
#
# `spacetime/module/` is deliberately outside the dispatch workspace, so
# `cargo build`, `cargo test` and `cargo clippy --all-targets` at the repo root
# all stop before reaching it. Nothing is wrong with that — it targets wasm and
# is not linked into the dispatch binary — but it means the crate had no gate at
# all: it could stop compiling, and every check in the repo would still be
# green. This is that gate.
#
# Two checks, because they fail differently:
#
#   1. `cargo check --target wasm32-unknown-unknown` — the target it is actually
#      published for. A dependency or an API that works on the host and not on
#      wasm fails only here.
#   2. `cargo test` on the HOST target — the module's own unit tests, which
#      cover the validators (`validate_task_ownership`). They need a test
#      harness to link against, which is why the crate is `cdylib` + `rlib`.
#
# Not covered here: publishing against a live SpacetimeDB instance. That needs
# the `spacetime` binary and a running server, and lives in
# `tests/spacetime_module.rs`, which skips when neither is present.

set -euo pipefail

cd "$(dirname "$0")/.."
MODULE_DIR="spacetime/module"

if [ ! -f "$MODULE_DIR/Cargo.toml" ]; then
  echo "error: no module crate at $MODULE_DIR" >&2
  exit 1
fi

# The wasm target is a rustup component, not part of a default toolchain. A
# missing one is reported rather than skipped: skipping would mean the one check
# that exercises the real publish target passes silently on any machine that
# never installed it, which is the failure mode this whole script is about.
if ! rustc --print target-list | grep -qx wasm32-unknown-unknown; then
  echo "error: this rustc does not know the wasm32-unknown-unknown target" >&2
  exit 1
fi

echo "spacetime module: cargo check --target wasm32-unknown-unknown"
if ! (cd "$MODULE_DIR" && cargo check --target wasm32-unknown-unknown); then
  cat >&2 <<'EOF'

The module failed to build for wasm32-unknown-unknown.

If the error is a missing standard library for the target, install it:
  Fedora:  sudo dnf install rust-std-static-wasm32-unknown-unknown
  rustup:  rustup target add wasm32-unknown-unknown
EOF
  exit 1
fi

echo "spacetime module: cargo test"
(cd "$MODULE_DIR" && cargo test)
