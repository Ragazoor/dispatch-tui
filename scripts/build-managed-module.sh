#!/usr/bin/env bash
# Build the SpacetimeDB module that `dispatch tui` publishes to its managed
# store, and commit it as `src/spacetime/module.wasm`.
#
#   scripts/build-managed-module.sh           rebuild the .wasm and its stamp
#   scripts/build-managed-module.sh --check   verify the stamp, build nothing
#
# WHY THE MODULE IS COMMITTED
#
# The board embeds the prebuilt module (`include_bytes!`) and publishes it to
# the managed store when the store's recorded module differs
# (`docs/specs/startup.allium`: PublishTheEmbeddedModuleWhenTheStoreDiffers).
# Building it needs the `spacetime` CLI and the wasm32 target, which a plain
# `cargo build` must not require -- the same reason the client bindings in
# `src/spacetime/bindings/` are committed.
#
# WHY THE GATE IS A SOURCE STAMP, NOT A REBUILD-AND-COMPARE
#
# A wasm build is NOT byte-reproducible across machines: the binary embeds the
# absolute path of the cargo registry (`/home/<user>/.cargo/registry/...`) in
# its panic locations, and `spacetime build` runs `wasm-opt` only when it is
# installed, so the same source yields different bytes on a laptop and on a CI
# runner. Rebuilding in CI and comparing would fail on every run.
#
# So the committed `src/spacetime/module.wasm.source-hash` records a hash of
# the module's SOURCE (src/**, Cargo.toml, Cargo.lock), taken when the .wasm was
# built. `--check` recomputes it and fails when the source has moved on without
# a rebuild. It needs neither `spacetime` nor a toolchain, so it runs in the
# hook and in CI. `src/spacetime/tests/managed_store_real.rs` computes the same
# hash in Rust, so `cargo test` catches the drift too. What it cannot catch is a
# .wasm that was swapped by hand without a source change; review does.
#
# THE STAMP RECIPE (the Rust test mirrors it exactly): every file under
# spacetime/module/src plus spacetime/module/Cargo.toml and Cargo.lock, in
# bytewise path order, each as "<sha256>  <repo-relative path>\n", and the
# sha256 of that listing.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
module="$root/spacetime/module"
wasm="$root/src/spacetime/module.wasm"
stamp="$root/src/spacetime/module.wasm.source-hash"

source_hash() {
    (
        cd "$root"
        {
            find spacetime/module/src -type f
            echo spacetime/module/Cargo.toml
            echo spacetime/module/Cargo.lock
        } | LC_ALL=C sort | while read -r f; do
            printf '%s  %s\n' "$(sha256sum "$f" | cut -d' ' -f1)" "$f"
        done | sha256sum | cut -d' ' -f1
    )
}

current="$(source_hash)"

if [ "${1:-}" = "--check" ]; then
    recorded="$(tr -d '[:space:]' < "$stamp" 2>/dev/null || true)"
    if [ "$recorded" != "$current" ]; then
        echo "build-managed-module: src/spacetime/module.wasm is stale against spacetime/module." >&2
        echo "Run ./scripts/build-managed-module.sh and commit the result." >&2
        exit 1
    fi
    echo "build-managed-module: the embedded module matches spacetime/module."
    exit 0
fi

if ! command -v spacetime >/dev/null 2>&1; then
    echo "build-managed-module: \`spacetime\` is not on PATH." >&2
    echo "Install it from https://spacetimedb.com/install and re-run." >&2
    exit 1
fi

spacetime build -p "$module"
built="$module/target/wasm32-unknown-unknown/release/dispatch_spacetime_module.wasm"
if [ ! -f "$built" ]; then
    echo "build-managed-module: expected $built after the build." >&2
    exit 1
fi
cp "$built" "$wasm"
printf '%s\n' "$current" > "$stamp"
echo "build-managed-module: wrote $(basename "$wasm") ($(wc -c < "$wasm") bytes), source hash $current"
