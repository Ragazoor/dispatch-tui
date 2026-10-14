"""Running metrics for the learnings store, so each iteration run is comparable.

Run from anywhere: `uv run python -I scripts/learnings-metrics.py [--prs]`.

Reads the live store with `spacetime sql --format json` (its SQL rejects GROUP BY, so
everything is aggregated here). `--prs` also fetches the body of every PR that an
`exit_session` trajectory recorded since 2026-08-21 and counts breaches of the PR-body
rules in `plugin/skills/wrap-up/references/pr.md` (needs `gh`).
"""

import collections
import datetime
import glob
import json
import os
import re
import subprocess
import sys

SERVER = os.environ.get("DISPATCH_SPACETIME_NICKNAME", "local")
DB = "dispatch"
TRAJ = os.path.expanduser("~/.local/share/dispatch/trajectories/*.jsonl")
RULES_FROM = "2026-08-21"  # b1c78a32 added the PR-body rules to the wrap-up skill


def table(name):
    out = subprocess.run(
        ["spacetime", "sql", "-s", SERVER, "--format", "json", DB, f"select * from {name}"],
        capture_output=True, text=True, check=True,
    ).stdout
    res = json.loads(out)[0]
    cols = [e["name"]["some"] for e in res["schema"]["elements"]]
    return [dict(zip(cols, row)) for row in res["rows"]]


def share(counts, n):
    total = sum(counts)
    return round(100 * sum(sorted(counts, reverse=True)[:n]) / total) if total else 0


def store_metrics():
    L, R = table("learnings"), table("learning_retrievals")
    by = {r["id"]: r for r in L}
    hits = collections.Counter(r["learning_id"] for r in R)
    src = collections.Counter(r["source"] for r in R)
    rated = {r["id"] for r in L if r["upvote_count"] != 0}
    month = collections.Counter(r["created_at"][:7] for r in L)
    cutoff = (datetime.datetime.now() - datetime.timedelta(days=30)).strftime("%Y-%m-%d")
    m = {
        "learnings": len(L),
        "by status": dict(collections.Counter(r["status"] for r in L)),
        "never upvoted": sum(1 for r in L if r["upvote_count"] == 0),
        "negative score": sum(1 for r in L if r["upvote_count"] < 0),
        "created per month": dict(sorted(month.items())[-4:]),
        "retrieval rows": len(R),
        "tasks with retrievals": len({r["task_id"] for r in R}),
        "retrievals by source": dict(src),
        "total upvotes": sum(r["upvote_count"] for r in L),
        "top-10 upvote share %": share([max(r["upvote_count"], 0) for r in L], 10),
        "top-30 upvote share %": share([max(r["upvote_count"], 0) for r in L], 30),
        "top-50 retrieval share %": share(list(hits.values()), 50),
        "dangling retrieval ids": sum(1 for i in hits if i not in by),
        "retrieved 20+, never upvoted": sum(
            1 for i, c in hits.items() if c >= 20 and i in by and i not in rated
        ),
        "unrated and older than 30 days": sum(
            1 for r in L if r["upvote_count"] == 0 and r["created_at"][:10] < cutoff
        ),
    }
    for k, v in m.items():
        print(f"{k}: {v}")


def pr_metrics():
    prs = {}
    for f in glob.glob(TRAJ):
        for line in open(f):
            try:
                e = json.loads(line)
            except ValueError:
                continue
            url = (e.get("args") or {}).get("pr_url")
            if e.get("method") == "exit_session" and url and e["timestamp"][:10] >= RULES_FROM:
                prs[url] = e["task_id"]
    fetched = impl = taskid = plan = 0
    for url, task in prs.items():
        r = subprocess.run(["gh", "pr", "view", url, "--json", "body"], capture_output=True, text=True)
        if r.returncode:
            continue
        body = json.loads(r.stdout)["body"] or ""
        fetched += 1
        impl += bool(re.search(r"Implements\s+#\d+", body))
        taskid += bool(re.search(rf"#{task}\b", body))
        plan += bool(re.search(r"(?im)^#+\s*test plan", body))
    print(f"PRs since {RULES_FROM}: {len(prs)} recorded, {fetched} fetched")
    print(f"breaches: 'Implements #N' {impl}, task id {taskid}, Test plan section {plan}")


if __name__ == "__main__":
    store_metrics()
    if "--prs" in sys.argv:
        pr_metrics()
