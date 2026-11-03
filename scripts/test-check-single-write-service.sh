#!/usr/bin/env bash
# Behavioural test for scripts/check-single-write-service.sh, against temp
# fixture repos. Run from the repo root: bash scripts/test-check-single-write-service.sh
set -euo pipefail

CHECKER="$(cd "$(dirname "$0")" && pwd)/check-single-write-service.sh"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT
failures=0

# $1 expected exit, $2 service file body, $3 allow-list body, $4 label
expect() {
    local want="$1" body="$2" allow="$3" label="$4"
    rm -rf "${WORKDIR:?}/src" "${WORKDIR:?}/scripts"
    mkdir -p "$WORKDIR/src/service" "$WORKDIR/scripts"
    printf '%s\n' "$body" >"$WORKDIR/src/service/thing.rs"
    printf '%s\n' "$allow" >"$WORKDIR/scripts/single-write-service.allow"
    local got=0 out
    out="$(cd "$WORKDIR" && bash "$CHECKER" 2>&1)" || got=$?
    if [[ "$got" != "$want" ]]; then
        echo "FAIL: $label (expected $want, got $got)" >&2
        echo "  output: $out" >&2
        failures=$((failures + 1))
    fi
}

DOUBLE='impl S {
    pub async fn update_thing(&self) {
        self.db.set_a(1).await?;
        self.db.set_b(2).await?;
    }
}'
expect 1 "$DOUBLE" '' 'a planted double write fails'
expect 0 "$DOUBLE" 'src/service/thing.rs::update_thing  2  exclusive branches' 'an allow-listed double write passes'
expect 1 'impl S {
    async fn one(&self) {
        self.db.set_a(1).await?;
    }
}' 'src/service/thing.rs::one  2  stale' 'a stale allow-list entry fails'
expect 0 'impl S {
    async fn read_then_write(&self) {
        let t = self.db.get_task(1).await;
        let all = self.db.list_epics().await;
        self.db.set_a(1).await?;
    }
}' '' 'reads do not count'
expect 0 'impl S {
    async fn one(&self) {
        self.db.set_a(1).await?;
    }
    async fn two(&self) {
        self.db.set_b(1).await?;
    }
}' '' 'one write in each of two functions passes'
expect 0 'impl S {
    async fn one(&self) {
        // self.db.set_a(1) and self.db.set_b(1) in a comment
        self.db.set_a(1).await?;
    }
}
#[cfg(test)]
mod tests {
    async fn t(&self) { self.db.set_a(1); self.db.set_b(1); }
}' '' 'comments and test modules are ignored'

# G2: wrapped calls, free functions, counts, session_request.
WRAPPED='impl S {
    pub async fn update_thing(&self) {
        self
            .db
            .set_a(1)
            .await?;
        self
            .db
            .set_b(2)
            .await?;
    }
}'
expect 1 "$WRAPPED" '' 'rustfmt-wrapped double write fails'
expect 0 "$WRAPPED" 'src/service/thing.rs::update_thing  2  wrapped but exclusive' 'a wrapped double write passes when listed with its count'
expect 1 "$WRAPPED" 'src/service/thing.rs::update_thing  3  wrong count' 'a wrong allow-list count fails'
expect 1 'pub async fn free_writes(db: &dyn Store) {
    db.set_a(1).await?;
    db.set_b(2).await?;
}' '' 'a free function with two db writes fails'
expect 1 'impl S {
    fn nested(&self) {
        if x {
            self.db.set_a(1);
            self.db.set_b(2);
        }
    }
}' '' 'an indented nested fn with two writes fails'
expect 0 'impl S {
    async fn r(&self) {
        let r = self
            .db
            .session_request(1)
            .await;
        self.db.set_a(1).await?;
        let c = self.db.clone();
    }
}' '' 'session_request and clone are not writes'
expect 0 'impl S {
    async fn r(&self) {
        let x = my_db.set_a(1);
        let y = other.db.set_b(1);
        self.db.set_c(1).await?;
    }
}' '' 'only a bare db or self.db receiver counts'

if [[ "$failures" != 0 ]]; then
    echo "$failures check-single-write-service case(s) failed" >&2
    exit 1
fi
echo "check-single-write-service self-test: ok"
