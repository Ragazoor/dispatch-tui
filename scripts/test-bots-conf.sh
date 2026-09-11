#!/usr/bin/env bash
# test-bots-conf.sh — shell test for the BOT_AUTHORS validation both readers
# of bots.conf perform after sourcing it.
#
# bots.conf documents exactly one entry form, gh search's app form
# "app/<slug>". An entry written the other natural way ("dependabot[bot]") used
# to be misread silently by BOTH readers — fetch-reviews.sh appended a second
# "[bot]" and matched no author, and the gh queries returned nothing — and the
# symptom was indistinguishable from "this bot has no open PRs". Both readers
# now reject a malformed entry outright. See "A malformed BOT_AUTHORS entry is
# fatal, not silently misread" in docs/specs/feed-scripts.allium.
#
# Asserts, for fetch-reviews.sh and fetch-dependabot.sh alike:
#   - a malformed entry exits non-zero, names itself on stderr, and emits no
#     items on stdout
#   - EVERY malformed entry is named, not just the first
#   - a well-formed list, an empty list and an absent bots.conf all pass
#   - near-miss forms (bare login, wrong case, empty slug, illegal character)
#     are all rejected
#
# Run from the repo root:  bash scripts/test-bots-conf.sh
# Exits 0 on success, non-zero with a diagnostic on the first failed assertion.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

# --- Fake gh: every query answers with an empty array. ---------------------
# The validation must not depend on what gh returns, so the stub is inert.
cat >"$WORKDIR/gh" <<'STUB'
#!/usr/bin/env bash
args="$*"
if [[ "$args" == *"api user"* ]]; then
  printf '%s\n' 'ragge'
elif [[ "$args" == *"api graphql"* ]]; then
  printf '%s\n' '{"data":{"nodes":[]}}'
elif [[ "$args" == *"/repos/"* ]]; then
  printf '%s\n' 'testorg/repo'
else
  printf '%s\n' '[]'
fi
STUB
chmod +x "$WORKDIR/gh"

cp "$SCRIPT_DIR/fetch-reviews.sh" "$SCRIPT_DIR/fetch-dependabot.sh" "$WORKDIR/"
chmod +x "$WORKDIR/fetch-reviews.sh" "$WORKDIR/fetch-dependabot.sh"
echo 'REPOS=("testorg/repo")' >"$WORKDIR/repos.conf"

fail() {
  echo "test-bots-conf: FAIL — $1" >&2
  [[ -n "${2:-}" ]] && { echo "---- captured ----" >&2; printf '%s\n' "$2" >&2; }
  exit 1
}

# Run one reader against the current bots.conf. Sets `rc`, `out`, `err`.
run_reader() {
  local script="$1"
  out="$(PATH="$WORKDIR:$PATH" bash "$WORKDIR/$script" 2>"$WORKDIR/stderr.txt")"
  rc=$?
  err="$(cat "$WORKDIR/stderr.txt")"
}

READERS=("fetch-reviews.sh" "fetch-dependabot.sh")

# --- A malformed entry is fatal for both readers ---------------------------

for script in "${READERS[@]}"; do
  echo 'BOT_AUTHORS=("dependabot[bot]")' >"$WORKDIR/bots.conf"
  run_reader "$script"
  [[ $rc -ne 0 ]] || fail "$script accepted the malformed entry \"dependabot[bot]\"" "$out"
  [[ "$err" == *'dependabot[bot]'* ]] ||
    fail "$script did not name the offending entry on stderr" "$err"
  [[ "$err" == *'BOT_AUTHORS'* ]] ||
    fail "$script's diagnostic does not say which setting is wrong" "$err"
  [[ "$err" == *'app/'* ]] ||
    fail "$script's diagnostic does not show the expected app/<slug> form" "$err"
  # Nothing may reach the board: a partial emission is the silent-misread
  # failure this check exists to prevent, wearing a different hat.
  [[ -z "$(printf '%s' "$out" | tr -d '[:space:]')" ]] ||
    fail "$script emitted items despite a malformed BOT_AUTHORS entry" "$out"
done

# --- Every malformed entry is reported, not just the first -----------------

for script in "${READERS[@]}"; do
  cat >"$WORKDIR/bots.conf" <<'CONF'
BOT_AUTHORS=("app/good" "dependabot[bot]" "renovate")
CONF
  run_reader "$script"
  [[ $rc -ne 0 ]] || fail "$script accepted a list with two malformed entries" "$out"
  [[ "$err" == *'dependabot[bot]'* ]] ||
    fail "$script did not report the first malformed entry" "$err"
  [[ "$err" == *'renovate'* ]] ||
    fail "$script reported only the first malformed entry, not every one" "$err"
done

# --- Near-miss forms are all rejected --------------------------------------

# A bare login is the plain-user-account form, deliberately NOT expressible;
# the rest are ordinary typos.
for bad in "renovate" "App/renovate" "app/" "app/my bot" "app/foo/bar" " app/renovate"; do
  printf 'BOT_AUTHORS=("%s")\n' "$bad" >"$WORKDIR/bots.conf"
  run_reader "fetch-reviews.sh"
  [[ $rc -ne 0 ]] || fail "fetch-reviews.sh accepted the malformed entry \"$bad\"" "$out"
done

# --- Well-formed entries pass ----------------------------------------------

cat >"$WORKDIR/bots.conf" <<'CONF'
BOT_AUTHORS=("app/kognic-renovate" "app/dependabot" "app/my-org.renovate_1")
CONF
for script in "${READERS[@]}"; do
  run_reader "$script"
  [[ $rc -eq 0 ]] || fail "$script rejected a well-formed BOT_AUTHORS list" "$err"
done

# --- An empty list and an absent bots.conf are the shipped states ----------

# fetch-dependabot.sh's empty-list fallback is app/kognic-renovate, itself in
# the legal form, so the fallback path cannot trip the check either.
echo 'BOT_AUTHORS=()' >"$WORKDIR/bots.conf"
for script in "${READERS[@]}"; do
  run_reader "$script"
  [[ $rc -eq 0 ]] || fail "$script failed on an empty BOT_AUTHORS list" "$err"
done

rm "$WORKDIR/bots.conf"
for script in "${READERS[@]}"; do
  run_reader "$script"
  [[ $rc -eq 0 ]] || fail "$script failed with no bots.conf at all" "$err"
done

echo "test-bots-conf: all assertions passed"
