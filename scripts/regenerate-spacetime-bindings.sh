#!/usr/bin/env bash
# Regenerate the SpacetimeDB client bindings in `src/spacetime/bindings/`.
#
# The bindings are the SDK's view of `spacetime/module/` — one Rust module per
# table and per reducer — and they are GENERATED, not written. Editing them by
# hand is editing a build artifact: the header of every file says so, and the
# next run of this script discards the edit.
#
# They are committed rather than generated at build time because generating
# them needs the `spacetime` CLI, which is not a build dependency of this repo
# and is not installed in CI. Committing them keeps `cargo build` working with
# nothing but cargo, at the cost of a step to remember after a module change —
# which is what `src/spacetime/tests/bindings_parity.rs` exists to catch.
#
# Run this after ANY change to `spacetime/module/src/lib.rs`, then commit the
# result alongside it.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/src/spacetime/bindings"

if ! command -v spacetime >/dev/null 2>&1; then
    echo "regenerate-spacetime-bindings: \`spacetime\` is not on PATH." >&2
    echo "Install it from https://spacetimedb.com/install and re-run." >&2
    exit 1
fi

# Generated afresh rather than merged over what is there: a table removed from
# the module leaves a stale file behind otherwise, and a stale binding compiles
# perfectly well.
rm -rf "$out"
mkdir -p "$out"
spacetime generate --lang rust --module-path "$root/spacetime/module" --out-dir "$out"

echo "regenerate-spacetime-bindings: wrote $(find "$out" -name '*.rs' | wc -l) files to src/spacetime/bindings"
