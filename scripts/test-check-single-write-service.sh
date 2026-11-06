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

# G4: loops, helpers, rebinding and every `fn` header form. Each probe below is
# a bug the checker used to miss; mutation-test by reverting the matching part
# of the checker and watching the case fail.
LOOPED='impl S {
    pub async fn bulk(&self) {
        for id in ids {
            self.db.set_a(id).await?;
        }
    }
}'
expect 1 "$LOOPED" '' 'a write inside a for loop counts as many'
expect 0 "$LOOPED" 'src/service/thing.rs::bulk  2  one batch call per id, each independent' 'a looped write passes when listed with count 2'
expect 1 'impl S {
    async fn bulk(&self) {
        while let Some(id) = next() {
            self.db.set_a(id).await?;
        }
    }
}' '' 'a write inside a while loop counts as many'
expect 1 'impl S {
    async fn bulk(&self) {
        loop {
            self.db.set_a(1).await?;
            break;
        }
    }
}' '' 'a write inside a loop block counts as many'
expect 1 'impl S {
    async fn bulk(&self) {
        for id in ids { self.db.set_a(id).await?; }
    }
}' '' 'a one-line loop with a write counts as many'
expect 0 'impl S {
    async fn bulk(&self) {
        for id in ids {
            let t = self.db.get_task(id).await?;
        }
        self.db.set_a(1).await?;
    }
}' '' 'reads in a loop plus one write after it pass'
expect 1 'impl S {
    async fn bulk(&self) {
        for id in ids {
            if id > 1 {
                self.db.set_a(id).await?;
            }
        }
    }
}' '' 'a write in a nested block inside a loop counts as many'
expect 0 'impl S {
    async fn bulk(&self) {
        for id in ids {
            noop(id);
        }
        self.db.set_a(1).await?;
    }
}' '' 'a write after a loop closes counts once'

HELPER='impl S {
    async fn put(&self, v: i64) {
        self.db.set_a(v).await?;
    }
    pub async fn twice(&self) {
        self.put(1).await?;
        self.put(2).await?;
    }
}'
expect 1 "$HELPER" '' 'a helper that writes, called twice, is flagged'
expect 0 "$HELPER" 'src/service/thing.rs::twice  2  two independent puts' 'a listed helper-driven double write passes'
expect 1 'impl S {
    async fn put(&self, v: i64) {
        self.db.set_a(v).await?;
    }
    pub async fn mixed(&self) {
        self.db.set_b(1).await?;
        self.put(2).await?;
    }
}' '' 'a direct write plus a writing helper is flagged'
expect 0 'impl S {
    async fn put(&self, v: i64) {
        self.db.set_a(v).await?;
    }
    pub async fn once(&self) {
        self.put(1).await?;
    }
}' '' 'one call to a one-write helper passes'
expect 0 'impl S {
    async fn look(&self) -> i64 {
        self.db.get_a().await
    }
    pub async fn twice(&self) {
        self.look().await;
        self.look().await;
        self.db.set_a(1).await?;
    }
}' '' 'a helper that only reads does not count'
expect 1 'impl S {
    async fn put(&self, v: i64) {
        self.db.set_a(v).await?;
    }
    pub async fn bulk(&self) {
        for v in vs {
            self.put(v).await?;
        }
    }
}' '' 'a writing helper called in a loop counts as many'

expect 1 'impl S {
    pub async fn rebound(&self) {
        let store = &self.db;
        store.set_a(1).await?;
        store.set_b(2).await?;
    }
}' '' 'a rebound db receiver is followed'
expect 1 'impl S {
    pub async fn cloned(&self) {
        let s = self.db.clone();
        s.set_a(1).await?;
        self.db.set_b(2).await?;
    }
}' '' 'a cloned db receiver is followed'
expect 0 'impl S {
    pub async fn rebound(&self) {
        let store = &self.db;
        store.get_a().await?;
        store.set_b(2).await?;
    }
}' '' 'a rebound receiver used for one write passes'

expect 1 'impl S {
    pub(in crate::service) async fn scoped(&self) {
        self.db.set_a(1).await?;
        self.db.set_b(2).await?;
    }
}' '' 'a pub(in path) fn header is recognised'
expect 1 'impl S {
    pub(super) fn sup(&self) {
        self.db.set_a(1);
        self.db.set_b(2);
    }
}' '' 'a pub(super) fn header is recognised'
expect 1 'impl S {
    pub const unsafe fn weird(&self) {
        self.db.set_a(1);
        self.db.set_b(2);
    }
}' '' 'const/unsafe qualifiers are recognised'
expect 0 'impl S {
    pub(in crate::service) async fn first(&self) {
        self.db.set_a(1).await?;
    }
    pub(in crate::service) async fn second(&self) {
        self.db.set_b(1).await?;
    }
}' '' 'two pub(in path) fns with one write each stay separate'

# G6: `#[cfg(test)]` skips only the item it marks, not the rest of the file.
expect 1 'use a::b;
#[cfg(test)]
pub use crate::fixtures::Fx;
impl S {
    pub async fn update_thing(&self) {
        self.db.set_a(1).await?;
        self.db.set_b(2).await?;
    }
}' '' 'a cfg(test) use above a double write does not hide it'
expect 1 'impl S {
    async fn ok(&self) { self.db.set_a(1).await?; }
}
#[cfg(test)]
mod tests {
    fn t() { let s = "{"; self.db.set_a(1); self.db.set_b(1); }
}
impl T {
    pub async fn update_thing(&self) {
        self.db.set_a(1).await?;
        self.db.set_b(2).await?;
    }
}' '' 'a function after a cfg(test) mod is still checked'
expect 0 'impl S {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn only_in_tests(&self) {
        self.db.set_a(1);
        self.db.set_b(2);
    }
    pub async fn real(&self) { self.db.set_a(1).await?; }
}' '' 'a cfg(test) fn is skipped'
expect 1 '#[cfg(test)]
pub use crate::fixtures::{
    A,
    B,
};
pub async fn two(db: &dyn Store) {
    db.set_a(1).await?;
    db.set_b(2).await?;
}' '' 'a multi-line cfg(test) use ends at its semicolon'

# G7: an Arc::clone rebinding, a `//` inside a string, same-named helpers in
# two impl blocks.
expect 1 'impl S {
    pub async fn arced(&self) {
        let store = Arc::clone(&self.db);
        store.set_a(1).await?;
        store.set_b(2).await?;
    }
}' '' 'an Arc::clone rebound receiver is followed'
expect 1 'impl S {
    pub async fn url(&self) {
        self.db.set_a("http://x").await?; self.db.set_b(1).await?;
    }
}' '' 'a // inside a string does not hide the writes after it'
expect 0 'impl A {
    async fn put(&self) -> i64 {
        self.db.get_a().await
    }
    pub async fn once(&self) {
        self.put().await;
        self.db.set_a(1).await?;
    }
}
impl B {
    async fn put(&self) {
        self.db.set_b(1).await?;
    }
}' '' 'a same-named helper in another impl block is not charged'

if [[ "$failures" != 0 ]]; then
    echo "$failures check-single-write-service case(s) failed" >&2
    exit 1
fi
echo "check-single-write-service self-test: ok"
