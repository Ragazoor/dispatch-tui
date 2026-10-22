#!/usr/bin/env bash
# test-check-toolchain.sh — behavioural test for scripts/check-toolchain.sh.
#
# The checker compares the host `rustc` with the channel pinned in
# rust-toolchain.toml. This repo's own host normally matches, so the failure
# path never runs for real; each case below drives it with a stub `rustc` on
# PATH and a fixture toml instead.
#
# Hermetic: every assertion runs against a temp fixture repo. Checking this
# repo's own toolchain is check-toolchain.sh's own job, run as its own hook step.
#
# Run from the repo root:  bash scripts/test-check-toolchain.sh
# Exits 0 on success, non-zero with a diagnostic on the first failed assertion.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECKER="$SCRIPT_DIR/check-toolchain.sh"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT
mkdir -p "$WORKDIR/repo" "$WORKDIR/bin"

failures=0

# $1 expected exit status, $2 the toml's channel line, $3 the stub rustc's
# --version output, $4 a string the output must contain, $5 the label.
expect() {
    local want="$1" channel="$2" version="$3" needle="$4" label="$5"

    printf '[toolchain]\n%s\ncomponents = ["rustfmt", "clippy"]\n' "$channel" \
        >"$WORKDIR/repo/rust-toolchain.toml"
    printf '#!/bin/sh\necho "%s"\n' "$version" >"$WORKDIR/bin/rustc"
    chmod +x "$WORKDIR/bin/rustc"

    local got=0
    local out
    out="$(cd "$WORKDIR/repo" && PATH="$WORKDIR/bin:$PATH" bash "$CHECKER" 2>&1)" || got=$?

    if [ "$got" -ne "$want" ]; then
        echo "FAIL: $label — expected exit $want, got $got; output:"
        echo "$out"
        failures=$((failures + 1))
    elif ! printf '%s' "$out" | grep -qF -- "$needle"; then
        echo "FAIL: $label — output lacks \"$needle\"; output:"
        echo "$out"
        failures=$((failures + 1))
    else
        echo "ok: $label"
    fi
}

expect 0 'channel = "1.99.0"' 'rustc 1.99.0 (5f94df478 2026-08-27)' '1.99.0' \
    'matching rustup rustc passes'

expect 0 'channel = "1.99.0"' 'rustc 1.99.0 (5f94df478 2026-08-27) (Fedora 1.99.0-1.fc43)' '1.99.0' \
    'matching distro rustc passes'

expect 1 'channel = "1.100.0"' 'rustc 1.99.0 (5f94df478 2026-08-27) (Fedora 1.99.0-1.fc43)' \
    'pins 1.100.0' 'newer pin than host fails'

expect 1 'channel = "1.96.0"' 'rustc 1.99.0 (5f94df478 2026-08-27)' \
    'install rustup' 'older pin than host fails and names the fix'

# The distro version string must not satisfy the check by substring: the pin
# is 1.9.0 and "1.99.0" contains "1.9", so only an exact field compare is right.
expect 1 'channel = "1.9.0"' 'rustc 1.99.0 (5f94df478 2026-08-27)' \
    'pins 1.9.0' 'prefix of the host version is still a mismatch'

expect 0 'channel = "stable"' 'rustc 1.99.0 (5f94df478 2026-08-27)' 'not a version' \
    'a named channel is not checked'

if [ "$failures" -ne 0 ]; then
    echo "$failures assertion(s) failed"
    exit 1
fi
echo "all check-toolchain assertions passed"
