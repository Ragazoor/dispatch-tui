#!/usr/bin/env bash
# Run the agent-behaviour evals in plugin/evals/ with `claude plugin eval`.
#
# Local only: every run spends the operator's own Claude credentials. It is not
# in CI and not in the pre-push hook. See "Agent-behaviour evals" in
# docs/testing.md.
#
# Usage: scripts/run-evals.sh [--runs N] [--max-cost-usd USD] [extra claude plugin eval flags...]
#
# Defaults: 3 runs per case, a $3 cost cap. Env: EVAL_RUNS, EVAL_MAX_COST_USD.
# Anything else is passed through, e.g. `--case 'wrap-up-*'` or `--judge-model sonnet`.
set -euo pipefail

cd "$(dirname "$0")/.."

runs="${EVAL_RUNS:-3}"
max_cost="${EVAL_MAX_COST_USD:-3}"
extra=()
while [ $# -gt 0 ]; do
    case "$1" in
        --runs) runs="${2:?--runs needs a value}"; shift 2 ;;
        --max-cost-usd) max_cost="${2:?--max-cost-usd needs a value}"; shift 2 ;;
        *) extra+=("$1"); shift ;;
    esac
done

if ! command -v claude >/dev/null 2>&1; then
    echo "run-evals: 'claude' is not on PATH" >&2
    exit 1
fi

# GRADLE_USER_HOME=./.gradle makes the eval sandbox refuse any run that grants
# Bash, so drop it. --ablation none: the mocked MCP tools are visible to the
# no-plugin arm, so a with-without score shows no plugin effect. --no-publish
# keeps the HTML report on this machine.
exec env -u GRADLE_USER_HOME claude plugin eval plugin \
    --runs "$runs" \
    --max-cost-usd "$max_cost" \
    --ablation none \
    --no-publish \
    ${extra[@]+"${extra[@]}"}
