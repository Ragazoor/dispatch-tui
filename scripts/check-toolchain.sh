#!/usr/bin/env bash
# check-toolchain.sh — fail fast when the host rustc is not the pinned one.
#
# rust-toolchain.toml pins the toolchain CI uses, but only rustup reads it. A
# distro cargo (Fedora's /usr/bin/cargo, no rustup) ignores the pin and builds
# with whatever rustc it ships. When the two drift apart, clippy reports lints
# the pinned toolchain does not have (240 `double_must_use` errors once, from
# a 1.96 pin on a 1.99 host) and dependency version floors pass or fail
# differently from CI. This check turns that into one clear message before the
# hook's slow steps run.
#
# A named channel (`stable`, `nightly`) has no single version to compare
# against, so it is not checked.
#
# Run from the repo root:  ./scripts/check-toolchain.sh
set -euo pipefail

TOML="rust-toolchain.toml"

pinned="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$TOML" | head -n 1)"
if [ -z "$pinned" ]; then
    echo "check-toolchain: no channel in $TOML" >&2
    exit 1
fi

if ! printf '%s' "$pinned" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "check-toolchain: channel \"$pinned\" is not a version; not checked"
    exit 0
fi

# `rustc 1.99.0 (5f94df478 2026-08-27) (Fedora 1.99.0-1.fc43)`: the version is
# the second field. Compare it whole; a substring match would let 1.9.0 pass
# against 1.99.0.
host="$(rustc --version | awk '{print $2}')"

if [ "$host" != "$pinned" ]; then
    cat >&2 <<MSG
check-toolchain: host rustc is $host, but $TOML pins $pinned.
cargo is not using the pinned toolchain, so clippy and the build will not match
CI. Usually this means rustup is missing and a distro cargo is on PATH.
Fix: install rustup (https://rustup.rs) so the pin applies, or install a distro
rustc that matches $pinned.
MSG
    exit 1
fi

echo "check-toolchain: rustc $host matches $TOML"
