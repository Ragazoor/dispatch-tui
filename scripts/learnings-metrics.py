"""Running metrics for the learnings store, so each iteration run is comparable.

Run from anywhere: `uv run python -I scripts/learnings-metrics.py [--prs]`.

Reads the live store with `spacetime sql --format json` (its SQL rejects GROUP BY, so
everything is aggregated here). `--prs` also fetches the body of every PR that an
`exit_session` trajectory recorded since 2026-08-21 and counts breaches of the PR-body
rules in `plugin/skills/wrap-up/references/pr.md` (needs `gh`).

It always prints the repeated-mistakes section too: from the MCP trajectories, the share
of learnings-tool calls refused for their arguments, per ISO week, and the error classes
that the most distinct tasks hit in the last 14 days. This is the outcome signal: a
mistake many tasks repeat, and whether a change makes it fall.
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
    off_scope_metrics(L, R)


def opt(v):
    """A SpacetimeDB Option column: [0, value] is Some, [1, []] is None."""
    return v[1] if isinstance(v, list) and len(v) == 2 and v[0] == 0 else None


def off_scope_metrics(L, R):
    """Share of repo/epic-scoped retrievals outside the task's scope union.

    learnings.allium says retrieval returns user + the task's repo + the task's epic.
    Before 2026-10-16 the ranker only left others unboosted. Rows on deleted tasks
    cannot be judged and are skipped, so read this as a ratio, not a count.
    """
    by = {r["id"]: r for r in L}
    tasks = {t["id"]: t for t in table("tasks")}
    weeks = collections.defaultdict(lambda: [0, 0])
    for r in R:
        l, t = by.get(r["learning_id"]), tasks.get(r["task_id"])
        if not l or not t or l["scope"] not in ("repo", "epic"):
            continue
        want = t["repo_path"] if l["scope"] == "repo" else str(t["epic_id"])
        y, w, _ = datetime.date.fromisoformat(r["retrieved_at"][:10]).isocalendar()
        c = weeks[f"{y}-W{w:02d}"]
        c[0] += 1
        c[1] += opt(l["scope_ref"]) != want
    print("repo/epic-scoped retrievals outside the task's scope, per week (off/rows):")
    for wk in sorted(weeks)[-4:]:
        off, n = weeks[wk][1], weeks[wk][0]
        print(f"  {wk}  {off:>5}/{n:<5} {100 * off // n}%")


LEARNING_TOOLS = ("query_learnings", "record_learning", "rate_learning")


def trajectory_events():
    for f in glob.glob(TRAJ):
        for line in open(f):
            try:
                yield json.loads(line)
            except ValueError:
                continue


def error_text(e):
    r = e.get("result")
    return r["error"] if isinstance(r, dict) and isinstance(r.get("error"), str) else None


def mistakes_metrics():
    weeks = collections.defaultdict(lambda: collections.Counter())
    classes = collections.defaultdict(set)
    recent = (datetime.datetime.now() - datetime.timedelta(days=14)).strftime("%Y-%m-%d")
    for e in trajectory_events():
        err = error_text(e)
        if e["method"] in LEARNING_TOOLS:
            y, w, _ = datetime.date.fromisoformat(e["timestamp"][:10]).isocalendar()
            c = weeks[f"{y}-W{w:02d}"]
            c[e["method"]] += 1
            c[e["method"] + " bad"] += bool(err and err.startswith("Invalid arguments"))
        if err and e["timestamp"][:10] >= recent:
            classes[(e["method"], re.sub(r"\d+", "N", err)[:90])].add(e["task_id"])
    print("learnings-tool calls refused for their arguments, per week (bad/calls):")
    print("  week      " + "  ".join(f"{t:>16}" for t in LEARNING_TOOLS))
    for wk in sorted(weeks)[-4:]:
        c = weeks[wk]
        print(f"  {wk}  " + "  ".join(f"{c[t + ' bad']:>7}/{c[t]:<8}" for t in LEARNING_TOOLS))
    print(f"errors hit by the most distinct tasks since {recent}:")
    for (method, msg), tasks in sorted(classes.items(), key=lambda kv: -len(kv[1]))[:8]:
        print(f"  {len(tasks):>4} tasks  {method}: {msg}")


LEARNING_TOOLS = ("query_learnings", "record_learning", "rate_learning")


def trajectory_events():
    for f in glob.glob(TRAJ):
        for line in open(f):
            try:
                yield json.loads(line)
            except ValueError:
                continue


def error_text(e):
    r = e.get("result")
    return r["error"] if isinstance(r, dict) and isinstance(r.get("error"), str) else None


def mistakes_metrics():
    weeks = collections.defaultdict(lambda: collections.Counter())
    classes = collections.defaultdict(set)
    recent = (datetime.datetime.now() - datetime.timedelta(days=14)).strftime("%Y-%m-%d")
    for e in trajectory_events():
        err = error_text(e)
        if e["method"] in LEARNING_TOOLS:
            y, w, _ = datetime.date.fromisoformat(e["timestamp"][:10]).isocalendar()
            c = weeks[f"{y}-W{w:02d}"]
            c[e["method"]] += 1
            c[e["method"] + " bad"] += bool(err and err.startswith("Invalid arguments"))
        if err and e["timestamp"][:10] >= recent:
            classes[(e["method"], re.sub(r"\d+", "N", err)[:90])].add(e["task_id"])
    print("learnings-tool calls refused for their arguments, per week (bad/calls):")
    print("  week      " + "  ".join(f"{t:>16}" for t in LEARNING_TOOLS))
    for wk in sorted(weeks)[-4:]:
        c = weeks[wk]
        print(f"  {wk}  " + "  ".join(f"{c[t + ' bad']:>7}/{c[t]:<8}" for t in LEARNING_TOOLS))
    print(f"errors hit by the most distinct tasks since {recent}:")
    for (method, msg), tasks in sorted(classes.items(), key=lambda kv: -len(kv[1]))[:8]:
        print(f"  {len(tasks):>4} tasks  {method}: {msg}")


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
    mistakes_metrics()
    if "--prs" in sys.argv:
        pr_metrics()
