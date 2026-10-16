#!/usr/bin/env bash
# Regenerate the README screenshots from a made-up demo project.
#
#   scripts/readme-demo/run.sh            # board, epic, package PNGs + nav GIF
#   KEEP=1 scripts/readme-demo/run.sh     # leave the demo board up afterwards
#
# Everything runs in isolation: a throwaway SpacetimeDB store on 127.0.0.1:3099,
# a board on MCP port 8899, a private tmux socket (`-L dispatch-demo`), and a
# data dir, demo repos and output under target/readme-demo/. The board's
# startup offer to rewrite the Claude Code configuration is declined.
#
# No agent is launched. Agents get their MCP config from ~/.claude.json, which
# points at the operator's real board, so the Running and Review cards are
# staged: real worktrees and tmux windows, agent activity sent through the same
# `dispatch hook` commands Claude Code's hooks run, and Review states through
# `update_tasks`. Only worktree, tmux_window and host are written by SQL; no
# tool sets them outside a real dispatch.
#
# Needs: cargo, spacetime, tmux, jq, curl, uv, google-chrome, magick, ffmpeg.
set -euo pipefail

REPO=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
HERE=$REPO/scripts/readme-demo
D=$REPO/target/readme-demo
OUT=$D/out
STORE_ADDR=127.0.0.1:3099
STORE=http://$STORE_ADDR
PORT=8899
MCP=http://127.0.0.1:$PORT/mcp
SOCK=dispatch-demo
COLS=170
ROWS=34
BIN=$REPO/target/debug/dispatch

tm() { tmux -L "$SOCK" "$@"; }

# Poll until `cmd` succeeds, up to $1 tenths of a second.
wait_for() {
    local tries=$1
    shift
    for _ in $(seq 1 "$tries"); do
        "$@" >/dev/null 2>&1 && return 0
        sleep 0.1
    done
    echo "timed out waiting for: $*" >&2
    return 1
}
pane_has() { tm capture-pane -p -t "${2:-dispatch:0}" | grep -q -- "$1"; }

mcp_post() {
    curl -sf -X POST "$MCP" -H 'Content-Type: application/json' -H 'X-Caller-Kind: session' -d "$1"
}
mcp() {
    mcp_post "$(jq -nc --arg n "$1" --argjson a "$2" \
        '{jsonrpc:"2.0",id:1,method:"tools/call",params:{name:$n,arguments:$a}}')" \
        | jq -r '.result.content[0].text // .error.message'
}
sql() { spacetime sql -s "$STORE" dispatch "$1" 2>/dev/null; }
# The rows of a query, header and rule dropped, quotes and padding stripped.
sql_rows() { sql "$1" | tail -n +3 | tr -d ' "'; }
# Hook events go to the demo board only: both addresses are named explicitly,
# overriding any DISPATCH_PORT / DISPATCH_SPACETIME_SERVER in the environment.
hook() { "$BIN" "$@" --port "$PORT" --spacetime-server "$STORE" >/dev/null; }

stop_store() {
    local pids
    # Match the demo's data dir too, so the operator's own store is never hit.
    pids=$(pgrep -f -- "--listen-addr $STORE_ADDR --data-dir $D/stdb" || true)
    [ -n "$pids" ] && kill $pids
    return 0
}
teardown() {
    tm kill-server 2>/dev/null || true
    stop_store
}

# ---- fresh state -----------------------------------------------------------
teardown
rm -rf "$D/data" "$D/stdb" "$D/repos" "$OUT"
mkdir -p "$D/data" "$D/stdb" "$OUT"
(cd "$REPO" && cargo build --quiet)

for r in storefront payments-api; do
    mkdir -p "$D/repos/$r"
    git -C "$D/repos/$r" init -q -b main
    echo "# $r" >"$D/repos/$r/README.md"
    git -C "$D/repos/$r" add .
    git -C "$D/repos/$r" -c user.name=demo -c user.email=demo@example.com commit -qm init
done

setsid spacetime start --listen-addr "$STORE_ADDR" --data-dir "$D/stdb" --non-interactive \
    >"$D/stdb.log" 2>&1 </dev/null &
wait_for 100 curl -sf "$STORE/v1/ping"
spacetime publish -p "$REPO/spacetime/module" -s "$STORE" --yes dispatch >/dev/null 2>&1

# The board runs with cwd $D, so fastembed's model cache lands on disk there.
tm new-session -d -s dispatch -x "$COLS" -y "$ROWS" -c "$D" \
    "$BIN --data-dir $D/data --spacetime-server $STORE tui --port $PORT; sleep 100000"
wait_for 300 pane_has "Update it now"
tm send-keys -t dispatch:0 n Enter
wait_for 1200 pane_has "Name for this machine"
tm send-keys -t dispatch:0 laptop Enter
wait_for 300 mcp_post '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'

# ---- seed ------------------------------------------------------------------
SF=$D/repos/storefront
PA=$D/repos/payments-api
epic() { mcp create_epic "$1" >/dev/null; }
task() {
    mcp create_task "$(jq -nc --arg t "$1" --arg r "$2" --argjson e "$3" --arg g "$4" --arg d "$5" \
        '{title:$t,repo_path:$r,epic_id:$e,tag:$g,description:$d}')" >/dev/null
}

epic '{"title":"Checkout v2","description":"Rebuild checkout as one page with saved payment methods."}' # 1
epic '{"title":"Payment methods","parent_epic_id":1,"description":"Cards, wallets and pay-later."}' # 2
epic '{"title":"Order confirmation","parent_epic_id":1,"description":"What the customer sees after paying."}' # 3
epic '{"title":"Search revamp","description":"Typo-tolerant, faceted product search."}' # 4

task "Add Apple Pay and Google Pay buttons" "$SF" 2 feature "Wallet buttons above the card form." # 1
task "Store card tokens through payments-api" "$SF" 2 feature "Tokenise in the browser." # 2
task "Tokenise endpoint for saved cards" "$PA" 2 feature "POST /tokens, idempotent." # 3
task "Support Klarna pay-later" "$SF" 2 feature "Behind a per-market flag." # 4
task "Send order confirmation email" "$SF" 3 feature "Line items and delivery estimate." # 5
task "Show an order timeline on the receipt" "$SF" 3 feature "Paid, packed, shipped, delivered." # 6
task "Typo-tolerant product search" "$SF" 4 feature "Match 'snekers' to 'sneakers'." # 7
task "Index product variants separately" "$SF" 4 feature "Size and colour variants." # 8
task "Facet filters on category pages" "$SF" 4 feature "Brand, price, size." # 9
task "Fix flaky cart total test" "$SF" null bug "Fails one run in ten." # 10
task "Rate-limit the refund endpoint" "$PA" null bug "No limit today." # 11
task "Compare CDN image resizing options" "$SF" null research "Cost and latency." # 12
task "Upgrade to Node 22" "$SF" null chore "Drop the Node 18 polyfills." # 13
task "Review PR #212: currency rounding" "$PA" null pr-review "Rounding in refunds." # 14
task "Retry webhooks with backoff" "$PA" null feature "A failed webhook is dropped today." # 15
task "Remember the last used payment method" "$SF" 2 feature "Preselect it next time." # 16
task "Search suggestions as you type" "$SF" 4 feature "Top five products." # 17
task "Move session store to Redis" "$SF" null chore "Sticky sessions block autoscaling." # 18
task "Add refund reason codes" "$PA" null feature "For the finance report." # 19
task "Fix currency rounding in refunds" "$PA" null bug "JPY rounds to cents." # 20
task "Single-page checkout layout" "$SF" 1 feature "One page, three steps." # 21

mcp create_work_package '{"title":"Black Friday launch","description":"Must ship before 28 November.","link_epic_ids":[1,4],"link_task_ids":[11]}' >/dev/null
mcp update_tasks '{"task_ids":[5,13,18,19,20,21],"status":"done"}' >/dev/null
mcp update_tasks '{"task_ids":[1,7,10,15],"status":"running"}' >/dev/null
mcp update_tasks '{"task_ids":[2,8,12],"status":"review"}' >/dev/null

# Stage the agents: a worktree and a tmux window per card, then the hook
# events a live agent would send.
HOST=$(sql_rows "SELECT id FROM hosts")
sql_rows "SELECT id, alias, repo_path FROM tasks WHERE status = 'running' OR status = 'review'" |
    while IFS='|' read -r id alias repo; do
        wt=$repo/.worktrees/$id-$alias
        git -C "$repo" worktree add -q "$wt" -b "$id-$alias"
        tm new-window -d -t dispatch -n "task-$id" -c "$wt"
        sql "UPDATE tasks SET worktree = '$wt', tmux_window = 'task-$id', host = '$HOST' WHERE id = $id"
    done
for id in 1 7 10 15; do hook hook "$id" pre_tool_use; done
hook hook 10 notification --kind permission_prompt
for a in explore-1 explore-2; do hook hook-subagent 1 start --agent-id "$a" --session-id demo; done
mcp update_tasks '{"task_ids":[2],"sub_status":"approved"}' >/dev/null
mcp update_tasks '{"task_ids":[8],"sub_status":"changes_requested"}' >/dev/null
mcp update_tasks '{"task_ids":[12],"sub_status":"awaiting_review"}' >/dev/null
wait_for 100 pane_has "Retry webhooks"

# ---- capture -----------------------------------------------------------------
# frame <name>: capture the board pane and render $OUT/<name>.png
frame() {
    tm capture-pane -e -p -t dispatch:0 >"$OUT/$1.ansi"
    uv run --no-project python -I "$HERE/ansi2html.py" "$OUT/$1.ansi" "$OUT/$1.html" "dispatch tui"
    google-chrome --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=2 \
        --window-size=1900,1300 --screenshot="$OUT/$1.raw.png" "file://$OUT/$1.html" 2>/dev/null
    magick "$OUT/$1.raw.png" -trim +repage -bordercolor '#0f0f14' -border 24 "$OUT/$1.png"
    rm "$OUT/$1.raw.png"
}
key() {
    tm send-keys -t dispatch:0 "$1"
    sleep 0.6
}

# Board, cursor on the work package card.
key '['
frame board
key j
frame nav-01
key Space # into Checkout v2
frame nav-02
key Space # into Payment methods
frame nav-03
cp "$OUT/nav-03.png" "$OUT/epic-nested.png"
key Q
frame nav-04
key k
key Space # into the Black Friday launch package
frame nav-05
cp "$OUT/nav-05.png" "$OUT/package.png"
key Q
frame nav-06

# Stills for the README, at a width GitHub will not downscale into mush.
mkdir -p "$REPO/docs/images"
for s in board epic-nested package; do
    magick "$OUT/$s.png" -resize 1800x "$REPO/docs/images/$s.png"
done
# The tour GIF: 1.6 s per frame.
ffmpeg -y -loglevel error -framerate 0.625 -i "$OUT/nav-%02d.png" \
    -vf "scale=1200:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=none" \
    -loop 0 "$REPO/docs/images/tour.gif"

[ "${KEEP:-0}" = 1 ] || teardown
echo "wrote docs/images/{board,epic-nested,package}.png and docs/images/tour.gif"
