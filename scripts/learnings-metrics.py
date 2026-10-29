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
PROSE_RULE_AT = "2026-10-28 17:26"  # 215c89e2: pr.md says PR bodies are prose, no bullet lists

# Run 5 (task 94228) moved these user-scoped entries to repo scope in place, keeping their
# ids. The store keeps only the current scope, so a retrieval before this time is read
# with the scope the entry had then (user); otherwise the rescope would rewrite history.
RESCOPED_AT = "2026-10-22 11:30"
# Run 7 (task 98511) did the same for #565 and #500 on 2026-10-28; read it with the same table.
RESCOPED_FROM_USER = {
    565, 500,
    574, 578, 579, 580, 598, 599, 600, 601, 606, 614, 617, 621, 630, 632, 633, 637, 670,
    671, 683, 700, 701, 724, 737, 742, 767, 770, 772, 775, 776, 777, 780, 791, 831, 4107,
    12292, 12304, 69635, 69637,
}


def scope_at(learning, retrieval):
    """The (scope, scope_ref) a learning had when the retrieval happened."""
    if learning["id"] in RESCOPED_FROM_USER and retrieval["retrieved_at"] < (
        "2026-10-28 18:00" if learning["id"] in (565, 500) else RESCOPED_AT
    ):
        return "user", None
    return learning["scope"], opt(learning["scope_ref"])


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


def repo_ref_covers(ref, repo):
    """Mirror of `repo_ref_covers` in src/embeddings.rs: equal, or a parent directory."""
    return ref == repo or repo.startswith(ref.rstrip("/") + "/")


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
        if not l or not t:
            continue
        sc, ref = scope_at(l, r)
        if sc not in ("repo", "epic"):
            continue
        if sc == "repo":
            inside = repo_ref_covers(ref, t["repo_path"])
        else:
            inside = ref == str(t["epic_id"])
        y, w, _ = datetime.date.fromisoformat(r["retrieved_at"][:10]).isocalendar()
        c = weeks[f"{y}-W{w:02d}"]
        c[0] += 1
        c[1] += not inside
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


def user_scope_metrics():
    """User-scoped injection rows no agent rated helped, per week.

    Injected entries are rated at wrap-up almost by default, so an injection row without
    a `helped` verdict is the best "this did not apply here" signal there is. User scope
    is the only scope that spans repos, so this is where repo-specific facts leak. Read it
    with the top-30 share: a few entries cause most of it.
    """
    L, R, tasks = table("learnings"), table("learning_retrievals"), {t["id"]: t for t in table("tasks")}
    by = {r["id"]: r for r in L}
    helped, raters = set(), set()
    for e in trajectory_events():
        if e["method"] == "rate_learning":
            raters.add(e["task_id"])
            if (e.get("args") or {}).get("verdict") == "helped":
                helped.add((e["task_id"], (e.get("args") or {}).get("learning_id")))
    weeks = collections.defaultdict(lambda: [0, 0, 0, 0])
    per_entry = collections.Counter()
    for r in R:
        l = by.get(r["learning_id"])
        if r["source"] != "prompt_injection" or not l or scope_at(l, r)[0] != "user":
            continue
        if r["task_id"] not in tasks:
            continue
        y, w, _ = datetime.date.fromisoformat(r["retrieved_at"][:10]).isocalendar()
        c = weeks[f"{y}-W{w:02d}"]
        c[0] += 1
        rated = r["task_id"] in raters
        c[2] += rated
        if (r["task_id"], r["learning_id"]) not in helped:
            c[1] += 1
            c[3] += rated
            per_entry[r["learning_id"]] += 1
    print("user-scoped injection rows with no helped verdict, per week (unhelped/rows):")
    print("  (all rows; then only rows on tasks that rated something, where 'no helped' is a")
    print("   real signal. Before run 8 there was no 'not_applicable' verdict, so an unrated")
    print("   task counts as unhelped; verdict_coverage_metrics splits the two from W45.)")
    for wk in sorted(weeks)[-4:]:
        n, bad, rn, rbad = weeks[wk]
        print(f"  {wk}  {bad:>5}/{n:<5} {100 * bad // n}%   rated tasks only {rbad:>4}/{rn:<4} {100 * rbad // max(rn, 1)}%")
    total = sum(per_entry.values())
    top = sum(v for _, v in per_entry.most_common(30))
    print(f"  top-30 entries cause {100 * top // max(total, 1)}% of the unhelped rows ({total} rows)")


NOT_APPLICABLE_FROM = "2026-10-29"  # run 8 (task 98645) added the not_applicable verdict


def verdict_coverage_metrics():
    """Injected (task, learning) pairs by the task's standing verdict, per week, all scopes.

    The outcome signal run 8 added: before it, "no helped" mixed "read, did not fit" with
    "never rated". Read the split: mostly not_applicable means a precision problem (rescope
    or prune those entries); mostly unrated means agents skip rating (a prompt problem).
    """
    R, V = table("learning_retrievals"), table("learning_verdicts")
    standing = {(v["task_id"], v["learning_id"]): v["verdict"] for v in V}
    pairs = {}
    for r in R:
        if r["source"] == "prompt_injection" and r["retrieved_at"] >= NOT_APPLICABLE_FROM:
            pairs.setdefault((r["task_id"], r["learning_id"]), r["retrieved_at"])
    kinds = ("helped", "wrong", "not_applicable", "unrated")
    weeks = collections.defaultdict(collections.Counter)
    na = collections.Counter()
    for key, at in pairs.items():
        y, w, _ = datetime.date.fromisoformat(at[:10]).isocalendar()
        v = standing.get(key, "unrated")
        weeks[f"{y}-W{w:02d}"][v] += 1
        if v == "not_applicable":
            na[key[1]] += 1
    print(f"injected (task, learning) pairs by standing verdict, since {NOT_APPLICABLE_FROM}:")
    print("  week      " + "".join(f"{k:>16}" for k in kinds))
    for wk in sorted(weeks)[-4:]:
        c, n = weeks[wk], sum(weeks[wk].values())
        print(f"  {wk}  " + "".join(f"{c[k]:>8}/{n:<4}{100 * c[k] // n:>3}%" for k in kinds))
    if na:
        print("  most often not_applicable: " + ", ".join(f"#{i} ({n})" for i, n in na.most_common(10)))


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
                prs[url] = (e["task_id"], e["timestamp"])
    fetched = impl = taskid = plan = 0
    bullets = collections.Counter()  # (after the prose rule?, has a bullet) -> PRs
    for url, (task, ts) in prs.items():
        r = subprocess.run(["gh", "pr", "view", url, "--json", "body"], capture_output=True, text=True)
        if r.returncode:
            continue
        body = json.loads(r.stdout)["body"] or ""
        fetched += 1
        impl += bool(re.search(r"Implements\s+#\d+", body))
        taskid += bool(re.search(rf"#{task}\b", body))
        plan += bool(re.search(r"(?im)^#+\s*test plan", body))
        bullets[(ts.replace("T", " ")[:16] >= PROSE_RULE_AT, bool(re.search(r"(?m)^\s*(?:[-*]|\d+\.)\s", body)))] += 1
    print(f"PRs since {RULES_FROM}: {len(prs)} recorded, {fetched} fetched")
    print(f"breaches: 'Implements #N' {impl}, task id {taskid}, Test plan section {plan}")
    for after in (False, True):
        n = bullets[(after, True)] + bullets[(after, False)]
        print(f"PRs with a bullet or numbered list, {'after' if after else 'before'} the prose rule: {bullets[(after, True)]}/{n}")


if __name__ == "__main__":
    store_metrics()
    mistakes_metrics()
    user_scope_metrics()
    verdict_coverage_metrics()
    if "--prs" in sys.argv:
        pr_metrics()
