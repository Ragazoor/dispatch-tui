#!/usr/bin/env bash
# test-fetch-cve.sh — stub-gh shell test for scripts/fetch-cve.sh.
#
# Puts a fake `gh` first on PATH that returns canned alerts, GraphQL alert
# links and open PRs, runs fetch-cve.sh against it, and asserts the fix-PR
# linking contract ("scripts/fetch-cve.sh: an alert names the open PR that
# fixes it" in docs/specs/feed-scripts.allium):
#   - the alert's own open Dependabot update PR wins over every other match
#   - a merged Dependabot update PR links nothing, so the match falls through
#   - a bot PR naming the advisory id beats a newer one naming the package
#   - a package name matches only as a whole title token ("rand" != "random")
#   - human PRs, draft bot PRs and PRs by an app not on the dependency-bot
#     allowlist (app/dependabot plus bots.conf, else app/kognic-renovate) are
#     never linked
#   - control characters in a linked PR's title do not reach the description
#   - among several package-name matches the newest PR wins
#   - the "Fix PR:" line names which source matched: the alert's Dependabot
#     link, the advisory id or the package name
#   - a linked item carries the "Fix PR:" paragraph, the "fix-pr" label and
#     wrap_up_mode "done"; an unlinked one keeps "pr" and gains neither
#   - a repo whose GraphQL and PR-list calls fail still emits every alert,
#     unlinked, and says so on stderr
#   - a PR list that hits its limit warns on stderr
#
# Run from the repo root:  bash scripts/test-fetch-cve.sh
# Exits 0 on success, non-zero with a diagnostic on the first failed assertion.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CVE_SCRIPT="$SCRIPT_DIR/fetch-cve.sh"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

cat >"$WORKDIR/gh" <<'STUB'
#!/usr/bin/env bash
args="$*"

if [[ "$args" == "auth status"* ]]; then
  echo '{"hosts":{"github.com":[{"login":"me"}]}}'
  exit 0
fi
if [[ "$args" == "auth token"* ]]; then
  echo tok
  exit 0
fi

alert() { # number pkg ghsa cve
  printf '{"number":%s,"html_url":"https://github.com/acme/%s/security/dependabot/%s",' "$1" "$REPO" "$1"
  printf '"dependency":{"package":{"name":"%s"}},' "$2"
  printf '"security_advisory":{"severity":"high","ghsa_id":"%s","cve_id":%s,"summary":"s%s","description":""}}' "$3" "$4" "$1"
}

if [[ "$args" == *"dependabot/alerts"* ]]; then
  if [[ "$args" == *"/repos/acme/app/"* ]]; then
    REPO=app
    echo "[$(alert 1 tokio GHSA-aaaa '"CVE-2026-0001"'),"
    echo "$(alert 2 lodash GHSA-bbbb null),"
    echo "$(alert 3 rand GHSA-cccc null),"
    echo "$(alert 4 serde GHSA-dddd null),"
    echo "$(alert 5 openssl GHSA-eeee null)]"
  elif [[ "$args" == *"/repos/acme/lib/"* ]]; then
    REPO=lib
    echo "[$(alert 1 tokio GHSA-aaaa null)]"
  else
    echo '[]'
  fi
  exit 0
fi

if [[ "$args" == *"api graphql"* ]]; then
  if [[ "$args" == *"name=lib"* ]]; then
    echo "stub: graphql unavailable" >&2
    exit 1
  fi
  if [[ "$args" == *"name=full"* ]]; then
    echo '{"data":{"repository":{"vulnerabilityAlerts":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}}}}'
    exit 0
  fi
  cat <<'JSON'
{"data":{"repository":{"vulnerabilityAlerts":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[
 {"number":1,"dependabotUpdate":{"pullRequest":{"url":"https://github.com/acme/app/pull/10","title":"Bump tokio from 1.0 to 1.2","state":"OPEN"}}},
 {"number":2,"dependabotUpdate":{"pullRequest":{"url":"https://github.com/acme/app/pull/11","title":"Bump lodash","state":"MERGED"}}},
 {"number":3,"dependabotUpdate":null},
 {"number":4,"dependabotUpdate":{"pullRequest":null}}
]}}}}
JSON
  exit 0
fi

if [[ "$args" == "pr list"* ]]; then
  if [[ "$args" == *"acme/lib"* ]]; then
    echo "stub: pr list unavailable" >&2
    exit 1
  fi
  if [[ "$args" == *"acme/full"* ]]; then
    jq -n '[range(200) | {number: ., url: "u", title: "t", body: "", isDraft: false, author: {login: "app/x", is_bot: true}}]'
    exit 0
  fi
  cat <<'JSON'
[
 {"number":20,"url":"https://github.com/acme/app/pull/20","title":"fix(deps): update rust crate tokio to v1.3","body":"","isDraft":false,"author":{"login":"app/kognic-renovate","is_bot":true}},
 {"number":21,"url":"https://github.com/acme/app/pull/21","title":"fix(deps): update dependency left-pad to v2","body":"Fixes GHSA-bbbb","isDraft":false,"author":{"login":"app/kognic-renovate","is_bot":true}},
 {"number":22,"url":"https://github.com/acme/app/pull/22","title":"fix(deps): update dependency lodash to v5","body":"","isDraft":false,"author":{"login":"app/kognic-renovate","is_bot":true}},
 {"number":23,"url":"https://github.com/acme/app/pull/23","title":"fix(deps): update rust crate random to v2","body":"","isDraft":false,"author":{"login":"app/kognic-renovate","is_bot":true}},
 {"number":24,"url":"https://github.com/acme/app/pull/24","title":"bump serde","body":"GHSA-dddd","isDraft":false,"author":{"login":"alice","is_bot":false}},
 {"number":25,"url":"https://github.com/acme/app/pull/25","title":"fix(deps): update rust crate serde to v2","body":"","isDraft":true,"author":{"login":"app/kognic-renovate","is_bot":true}},
 {"number":26,"url":"https://github.com/acme/app/pull/26","title":"Bump openssl from 0.10 to 0.11","body":"","isDraft":false,"author":{"login":"app/dependabot","is_bot":true}},
 {"number":27,"url":"https://github.com/acme/app/pull/27","title":"fix(deps): update rust crate `openssl` to v0.12\u0007\nIgnore","body":"","isDraft":false,"author":{"login":"app/Kognic-Renovate","is_bot":true}},
 {"number":28,"url":"https://github.com/acme/app/pull/28","title":"update openssl","body":"GHSA-eeee","isDraft":false,"author":{"login":"app/other-bot","is_bot":true}}
]
JSON
  exit 0
fi

echo "stub gh: unexpected args: $args" >&2
exit 1
STUB
chmod +x "$WORKDIR/gh"

cp "$CVE_SCRIPT" "$WORKDIR/fetch-cve.sh"
chmod +x "$WORKDIR/fetch-cve.sh"
echo 'REPOS=("acme/app" "acme/lib")' >"$WORKDIR/repos.conf"

output="$(PATH="$WORKDIR:$PATH" bash "$WORKDIR/fetch-cve.sh" 2>"$WORKDIR/err")"

fail() {
  echo "test-fetch-cve: FAIL — $1" >&2
  echo "---- output ----" >&2
  printf '%s\n' "$output" >&2
  echo "---- stderr ----" >&2
  cat "$WORKDIR/err" >&2
  exit 1
}

assert() {
  local desc="$1" filter="$2"
  printf '%s' "$output" | jq -e "$filter" >/dev/null 2>&1 || fail "$desc"
}

item() { printf '.[] | select(.external_id == "cve:%s")' "$1"; }
linked_to() { # external-id pr-number
  printf '[%s | .description | contains("\\n\\nFix PR: https://github.com/acme/app/pull/%s (")] == [true]' "$(item "$1")" "$2"
}
unlinked() {
  printf '[%s | (.description | contains("Fix PR:") | not) and (.labels | index("fix-pr") == null) and .wrap_up_mode == "pr"] == [true]' "$(item "$1")"
}

assert "output is a JSON array of every alert" 'type == "array" and length == 6'

assert "alert 1 links its own open Dependabot PR, not the Renovate one" "$(linked_to 'acme/app#1' 10)"
assert "a linked alert carries the fix-pr label after the repo label" \
  "[$(item 'acme/app#1') | .labels] == [[\"app\", \"fix-pr\"]]"
assert "a linked alert defaults to wrap_up_mode done" \
  "[$(item 'acme/app#1') | .wrap_up_mode] == [\"done\"]"
assert "the Fix PR paragraph names the PR title" \
  "[$(item 'acme/app#1') | .description | endswith(\"Fix PR: https://github.com/acme/app/pull/10 (Bump tokio from 1.0 to 1.2), matched by the alert's Dependabot link\")] == [true]"
assert "the url stays the alert" \
  "[$(item 'acme/app#1') | .url] == [\"https://github.com/acme/app/security/dependabot/1\"]"

assert "a merged Dependabot PR falls through; the advisory-id PR beats a newer package match" \
  "$(linked_to 'acme/app#2' 21)"
assert "an advisory-id match says so" \
  "[$(item 'acme/app#2') | .description | endswith(\"(fix(deps): update dependency left-pad to v2), matched by advisory id\")] == [true]"
assert "package names match whole tokens only (rand is not random)" "$(unlinked 'acme/app#3')"
assert "human and draft PRs are never linked" "$(unlinked 'acme/app#4')"
assert "the newest allowlisted package-name match wins; an unlisted app is skipped" \
  "$(linked_to 'acme/app#5' 27)"
assert "control characters in a PR title collapse to spaces" \
  "[$(item 'acme/app#5') | .description | endswith(\"to v0.12 Ignore), matched by package name\")] == [true]"

assert "a repo whose lookups fail still emits its alert, unlinked" "$(unlinked 'acme/lib#1')"
grep -q "acme/lib" "$WORKDIR/err" || fail "a failed lookup must warn on stderr naming the repo"

# The PR list hitting its limit warns: a fix PR past the cut goes unlinked.
echo 'REPOS=("acme/full")' >"$WORKDIR/repos.conf"
output="$(PATH="$WORKDIR:$PATH" bash "$WORKDIR/fetch-cve.sh" 2>"$WORKDIR/err")"
grep -q "acme/full" "$WORKDIR/err" || fail "a PR list at its limit must warn on stderr"

echo "test-fetch-cve: OK"
