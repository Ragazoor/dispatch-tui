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
# And for RELEASE_BOT_AUTHORS (task 53281):
#   - fetch-reviews.sh validates its form exactly like BOT_AUTHORS, naming the
#     list on each offender's line, and walks both lists before exiting
#   - fetch-dependabot.sh does not validate its form and never queries by it
#   - an entry on both lists is fatal in BOTH readers, compared
#     case-insensitively, every overlap named, nothing emitted
#   - lists are compared as configured: an empty BOT_AUTHORS overlaps nothing
#   - empty lists, an old conf without the array, and the shipped conf pass
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
printf '%s\n' "$args" >>"$(dirname "$0")/gh-args.log"
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

# ===========================================================================
# RELEASE_BOT_AUTHORS (task 53281). See "Release bots get their own tag",
# "A malformed BOT_AUTHORS entry is fatal, not silently misread" and "A bot
# on both lists is fatal" in docs/specs/feed-scripts.allium.
# ===========================================================================

# Is stdout empty of items? A refused cycle emits NOTHING, not even "[]".
emitted_nothing() {
  [[ -z "$(printf '%s' "$out" | tr -d '[:space:]')" ]]
}

# --- A malformed RELEASE_BOT_AUTHORS entry is fatal in fetch-reviews.sh -----
# fetch-reviews.sh is the reader that uses the release list's entries, so it
# validates their form exactly as it validates BOT_AUTHORS.

cat >"$WORKDIR/bots.conf" <<'CONF'
BOT_AUTHORS=("app/kognic-renovate")
RELEASE_BOT_AUTHORS=("kognic-gha-release-bot[bot]")
CONF
run_reader "fetch-reviews.sh"
[[ $rc -ne 0 ]] ||
  fail "fetch-reviews.sh accepted the malformed RELEASE_BOT_AUTHORS entry \"kognic-gha-release-bot[bot]\"" "$out"
[[ "$err" == *'kognic-gha-release-bot[bot]'* ]] ||
  fail "fetch-reviews.sh did not name the malformed release entry on stderr" "$err"
grep -F 'kognic-gha-release-bot[bot]' <<<"$err" | grep -q 'RELEASE_BOT_AUTHORS' ||
  fail "the malformed release entry's line must name the list it sits in (RELEASE_BOT_AUTHORS)" "$err"
[[ "$err" == *'app/'* ]] ||
  fail "fetch-reviews.sh's release diagnostic does not show the expected app/<slug> form" "$err"
emitted_nothing ||
  fail "fetch-reviews.sh emitted items despite a malformed RELEASE_BOT_AUTHORS entry" "$out"

# --- Near-miss release forms are all rejected ------------------------------

for bad in "kognic-gha-release-bot" "App/kognic-gha-release-bot" "app/" "app/release bot" "app/a/b" " app/kognic-gha-release-bot"; do
  printf 'RELEASE_BOT_AUTHORS=("%s")\n' "$bad" >"$WORKDIR/bots.conf"
  run_reader "fetch-reviews.sh"
  [[ $rc -ne 0 ]] ||
    fail "fetch-reviews.sh accepted the malformed RELEASE_BOT_AUTHORS entry \"$bad\"" "$out"
done

# --- Every offender in BOTH lists is named in one run ----------------------
# Both lists are walked before the script exits, so a conf with a typo in
# each costs one round-trip, and each line names the list its entry sits in.

cat >"$WORKDIR/bots.conf" <<'CONF'
BOT_AUTHORS=("app/good" "dependabot[bot]")
RELEASE_BOT_AUTHORS=("app/fine-release" "release-please[bot]" "kognic-gha-release-bot")
CONF
run_reader "fetch-reviews.sh"
[[ $rc -ne 0 ]] || fail "fetch-reviews.sh accepted malformed entries in both lists" "$out"
for offender in 'dependabot[bot]' 'release-please[bot]' 'kognic-gha-release-bot'; do
  grep -qF "\"$offender\"" <<<"$err" ||
    fail "fetch-reviews.sh did not name every offender — missing \"$offender\"" "$err"
done
grep -F '"dependabot[bot]"' <<<"$err" | grep -q 'BOT_AUTHORS' ||
  fail "the BOT_AUTHORS offender's line must name BOT_AUTHORS" "$err"
if grep -F '"dependabot[bot]"' <<<"$err" | grep -q 'RELEASE_BOT_AUTHORS'; then
  fail "the BOT_AUTHORS offender's line must not claim it sits in RELEASE_BOT_AUTHORS" "$err"
fi
for offender in 'release-please[bot]' 'kognic-gha-release-bot'; do
  grep -F "\"$offender\"" <<<"$err" | grep -q 'RELEASE_BOT_AUTHORS' ||
    fail "the release offender \"$offender\"'s line must name RELEASE_BOT_AUTHORS" "$err"
done
[[ "$err" != *'app/fine-release'* ]] ||
  fail "a well-formed release entry must not be reported as an offender" "$err"
emitted_nothing || fail "fetch-reviews.sh emitted items despite malformed entries" "$out"

# --- fetch-dependabot.sh does not validate the release list's form ---------
# It reads RELEASE_BOT_AUTHORS for the overlap check alone; a malformed release
# entry is fetch-reviews.sh's to report.

cat >"$WORKDIR/bots.conf" <<'CONF'
BOT_AUTHORS=("app/kognic-renovate")
RELEASE_BOT_AUTHORS=("kognic-gha-release-bot[bot]")
CONF
run_reader "fetch-dependabot.sh"
[[ $rc -eq 0 ]] ||
  fail "fetch-dependabot.sh must not validate RELEASE_BOT_AUTHORS' form" "$err"

# --- A bot on both lists is fatal, in BOTH readers -------------------------
# Compared case-insensitively, as an author is matched. Every overlapping entry
# is named, the exit is non-zero and nothing is emitted.

cat >"$WORKDIR/bots.conf" <<'CONF'
BOT_AUTHORS=("app/kognic-renovate" "app/shared-bot" "app/twice-bot")
RELEASE_BOT_AUTHORS=("app/Shared-Bot" "app/kognic-gha-release-bot" "app/twice-bot")
CONF
for script in "${READERS[@]}"; do
  run_reader "$script"
  [[ $rc -ne 0 ]] ||
    fail "$script accepted a bot listed on both BOT_AUTHORS and RELEASE_BOT_AUTHORS" "$out"
  grep -qi 'shared-bot' <<<"$err" ||
    fail "$script did not name the case-insensitive overlap app/shared-bot" "$err"
  grep -qi 'twice-bot' <<<"$err" ||
    fail "$script named only the first overlapping entry, not every one" "$err"
  [[ "$err" == *'RELEASE_BOT_AUTHORS'* ]] ||
    fail "$script's overlap diagnostic does not name the lists involved" "$err"
  emitted_nothing || fail "$script emitted items despite an overlap between the lists" "$out"
done

# The overlap alone, with exact case, is fatal too.
cat >"$WORKDIR/bots.conf" <<'CONF'
BOT_AUTHORS=("app/kognic-gha-release-bot")
RELEASE_BOT_AUTHORS=("app/kognic-gha-release-bot")
CONF
for script in "${READERS[@]}"; do
  run_reader "$script"
  [[ $rc -ne 0 ]] || fail "$script accepted an exact-case overlap between the lists" "$out"
  [[ "$err" == *'app/kognic-gha-release-bot'* ]] ||
    fail "$script did not name the overlapping entry" "$err"
  emitted_nothing || fail "$script emitted items despite an exact-case overlap" "$out"
done

# --- Lists are compared as configured, before any fallback -----------------
# An empty BOT_AUTHORS overlaps nothing: fetch-dependabot.sh's app/kognic-renovate
# default is a default, not a listing, so a release list naming it is legal.

cat >"$WORKDIR/bots.conf" <<'CONF'
BOT_AUTHORS=()
RELEASE_BOT_AUTHORS=("app/kognic-renovate")
CONF
for script in "${READERS[@]}"; do
  run_reader "$script"
  [[ $rc -eq 0 ]] ||
    fail "$script treated its empty-list fallback as a listing and refused the conf" "$err"
done

# --- Well-formed, disjoint lists pass both readers -------------------------

cat >"$WORKDIR/bots.conf" <<'CONF'
BOT_AUTHORS=("app/kognic-renovate" "app/dependabot")
RELEASE_BOT_AUTHORS=("app/kognic-gha-release-bot" "app/release-please_2.x")
CONF
for script in "${READERS[@]}"; do
  run_reader "$script"
  [[ $rc -eq 0 ]] || fail "$script rejected well-formed, disjoint bot lists" "$err"
done

# fetch-dependabot.sh never lists PRs by a release bot: its gh calls name the
# dependency bots only.
: >"$WORKDIR/gh-args.log"
run_reader "fetch-dependabot.sh"
if grep -q 'kognic-gha-release-bot\|release-please_2.x' "$WORKDIR/gh-args.log"; then
  fail "fetch-dependabot.sh queried gh for a RELEASE_BOT_AUTHORS bot" "$(cat "$WORKDIR/gh-args.log")"
fi
grep -q 'kognic-renovate' "$WORKDIR/gh-args.log" ||
  fail "fetch-dependabot.sh stopped listing its dependency bots" "$(cat "$WORKDIR/gh-args.log")"

# --- Empty lists never fail ------------------------------------------------

cat >"$WORKDIR/bots.conf" <<'CONF'
BOT_AUTHORS=()
RELEASE_BOT_AUTHORS=()
CONF
for script in "${READERS[@]}"; do
  run_reader "$script"
  [[ $rc -eq 0 ]] || fail "$script failed on two empty lists" "$err"
done

# --- An old conf without the release array still works --------------------
# The conf is installed create-new-only, so an existing deployment's copy has
# no RELEASE_BOT_AUTHORS. Both readers pre-declare it, so this reads as empty
# rather than tripping an unbound-variable error.

echo 'BOT_AUTHORS=("app/kognic-renovate")' >"$WORKDIR/bots.conf"
for script in "${READERS[@]}"; do
  run_reader "$script"
  [[ $rc -eq 0 ]] || fail "$script failed on an old bots.conf with no RELEASE_BOT_AUTHORS" "$err"
  [[ "$err" != *'unbound'* ]] ||
    fail "$script tripped on an undeclared RELEASE_BOT_AUTHORS" "$err"
done

# --- The shipped bots.conf ships RELEASE_BOT_AUTHORS empty -----------------

cp "$SCRIPT_DIR/bots.conf" "$WORKDIR/bots.conf"
shipped_release_count="$(bash -c 'set -u; RELEASE_BOT_AUTHORS=("sentinel"); source "$1"; echo "${#RELEASE_BOT_AUTHORS[@]}"' _ "$WORKDIR/bots.conf")"
[[ "$shipped_release_count" == "0" ]] ||
  fail "the shipped bots.conf must set RELEASE_BOT_AUTHORS to an empty list, got $shipped_release_count entries"
grep -q '^#.*app/kognic-gha-release-bot' "$SCRIPT_DIR/bots.conf" ||
  fail "the shipped bots.conf must document app/kognic-gha-release-bot as the release-bot example"
for script in "${READERS[@]}"; do
  run_reader "$script"
  [[ $rc -eq 0 ]] || fail "$script failed on the shipped bots.conf" "$err"
done

echo "test-bots-conf: all assertions passed"
