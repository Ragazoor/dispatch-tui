#!/usr/bin/env bash
# test-fetch-reviews.sh — stub-gh shell test for scripts/fetch-reviews.sh.
#
# Puts a fake `gh` first on PATH that returns canned JSON per search qualifier,
# runs fetch-reviews.sh against it, and asserts the single-emission +
# signal-merging contract:
#   - a PR matched by two queries collapses to ONE item carrying BOTH signals
#   - bot-authored PRs (renovate/dependabot) are included with author-bot +
#     tag "dependabot" (no longer excluded)
#   - a PR authored by the gh user carries the author-me signal
#   - draft PRs are excluded
#   - the output parses as a JSON array
#
# Run from the repo root:  bash scripts/test-fetch-reviews.sh
# Exits 0 on success, non-zero with a diagnostic on the first failed assertion.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REVIEWS_SCRIPT="$SCRIPT_DIR/fetch-reviews.sh"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

# --- Fake gh: dispatch on the search qualifier in its arguments. -----------
# NOTE: "user-review-requested:@me" contains "review-requested:@me" as a
# substring, so it MUST be matched first.
cat >"$WORKDIR/gh" <<'STUB'
#!/usr/bin/env bash
args="$*"

if [[ "$args" == *"api user"* ]]; then
  printf '%s\n' "ragge"
  exit 0
fi

if [[ "$args" == *"user-review-requested:@me"* ]]; then
  printf '%s\n' '[]'
elif [[ "$args" == *"review-requested:@me"* ]]; then
  cat <<'JSON'
[
  {"number":1,"title":"Add feature","body":"d","url":"https://github.com/testorg/repo/pull/1","repository":{"name":"repo","nameWithOwner":"testorg/repo"},"isDraft":false,"author":{"login":"alice"}},
  {"number":2,"title":"Bump dep","body":"","url":"https://github.com/testorg/repo/pull/2","repository":{"name":"repo","nameWithOwner":"testorg/repo"},"isDraft":false,"author":{"login":"kognic-renovate[bot]"}},
  {"number":5,"title":"Draft PR","body":"","url":"https://github.com/testorg/repo/pull/5","repository":{"name":"repo","nameWithOwner":"testorg/repo"},"isDraft":true,"author":{"login":"bob"}}
]
JSON
elif [[ "$args" == *"reviewed-by:@me"* ]]; then
  cat <<'JSON'
[
  {"number":1,"title":"Add feature","body":"d","url":"https://github.com/testorg/repo/pull/1","repository":{"name":"repo","nameWithOwner":"testorg/repo"},"isDraft":false,"author":{"login":"alice"}},
  {"number":3,"title":"Bump lib","body":"","url":"https://github.com/testorg/repo/pull/3","repository":{"name":"repo","nameWithOwner":"testorg/repo"},"isDraft":false,"author":{"login":"dependabot[bot]"}}
]
JSON
elif [[ "$args" == *"commenter:@me"* ]]; then
  cat <<'JSON'
[
  {"number":4,"title":"My own PR","body":"","url":"https://github.com/testorg/repo/pull/4","repository":{"name":"repo","nameWithOwner":"testorg/repo"},"isDraft":false,"author":{"login":"ragge"}}
]
JSON
else
  printf '%s\n' '[]'
fi
STUB
chmod +x "$WORKDIR/gh"

# --- Script copy with a non-empty ORGS so it actually queries. -------------
sed 's/^ORGS=()/ORGS=("testorg")/' "$REVIEWS_SCRIPT" >"$WORKDIR/fetch-reviews.sh"

output="$(PATH="$WORKDIR:$PATH" bash "$WORKDIR/fetch-reviews.sh")"

fail() {
  echo "test-fetch-reviews: FAIL — $1" >&2
  echo "---- output ----" >&2
  printf '%s\n' "$output" >&2
  exit 1
}

assert() {
  local desc="$1" filter="$2"
  printf '%s' "$output" | jq -e "$filter" >/dev/null 2>&1 || fail "$desc"
}

# Output is a JSON array.
assert "output is a JSON array" 'type == "array"'

# Exactly four PRs survive (PR1 deduped across two queries; draft PR5 excluded).
assert "exactly 4 items after dedup + draft exclusion" 'length == 4'

# PR1 matched by review-requested AND reviewed-by -> one item, both signals.
assert "PR1 carries team-request" \
  'map(select(.url | endswith("/pull/1"))) | .[0].signals | index("team-request")'
assert "PR1 carries reviewed" \
  'map(select(.url | endswith("/pull/1"))) | .[0].signals | index("reviewed")'
assert "PR1 appears exactly once" \
  '[.[] | select(.url | endswith("/pull/1"))] | length == 1'
assert "PR1 keeps tag pr-review" \
  'map(select(.url | endswith("/pull/1"))) | .[0].tag == "pr-review"'

# Renovate bot PR included, author-bot + tag dependabot.
assert "renovate PR2 tagged dependabot" \
  'map(select(.url | endswith("/pull/2"))) | .[0].tag == "dependabot"'
assert "renovate PR2 carries author-bot" \
  'map(select(.url | endswith("/pull/2"))) | .[0].signals | index("author-bot")'

# Dependabot bot PR included, author-bot + tag dependabot.
assert "dependabot PR3 tagged dependabot" \
  'map(select(.url | endswith("/pull/3"))) | .[0].tag == "dependabot"'
assert "dependabot PR3 carries author-bot" \
  'map(select(.url | endswith("/pull/3"))) | .[0].signals | index("author-bot")'

# Self-authored PR carries author-me (so route() keeps it out of My Reviews).
assert "self-authored PR4 carries author-me" \
  'map(select(.url | endswith("/pull/4"))) | .[0].signals | index("author-me")'
assert "self-authored PR4 carries commented" \
  'map(select(.url | endswith("/pull/4"))) | .[0].signals | index("commented")'

# Draft PR5 excluded entirely.
assert "draft PR5 excluded" \
  '[.[] | select(.url | endswith("/pull/5"))] | length == 0'

echo "test-fetch-reviews: all assertions passed"
