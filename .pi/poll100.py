#!/usr/bin/env python3
"""Poll PR #100 (tailrocks/ruxel) for unresolved review threads (paginated) and CI check conclusions."""
import json
import subprocess
import sys

REPO_OWNER = "tailrocks"
REPO_NAME = "ruxel"
PR = 100

QUERY = """
query($owner:String!, $name:String!, $pr:Int!, $cursor:String) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$pr) {
      headRefOid
      reviewThreads(first:100, after:$cursor) {
        pageInfo { hasNextPage endCursor }
        totalCount
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          comments(first:1) { nodes { body author { login } createdAt } }
        }
      }
    }
  }
}
"""


def graphql(cursor):
    args = [
        "gh", "api", "graphql",
        "-f", "query=" + QUERY,
        "-f", "owner=" + REPO_OWNER,
        "-f", "name=" + REPO_NAME,
        "-F", "pr=%d" % PR,
    ]
    if cursor:
        args += ["-f", "cursor=" + cursor]
    out = subprocess.run(args, capture_output=True, text=True)
    if out.returncode != 0:
        raise SystemExit("gh api failed: %s %s" % (out.stderr.strip(), out.stdout[:200]))
    data = json.loads(out.stdout)
    if "errors" in data:
        raise SystemExit("graphql errors: %s" % json.dumps(data["errors"])[:400])
    return data["data"]["repository"]["pullRequest"]


def fetch_threads():
    cursor = None
    head = None
    total = None
    pages = 0
    threads = []
    seen = set()
    while True:
        pr = graphql(cursor)
        head = pr["headRefOid"]
        rt = pr["reviewThreads"]
        total = rt["totalCount"]
        for n in rt["nodes"]:
            if n["id"] not in seen:
                seen.add(n["id"])
                threads.append(n)
        pages += 1
        if not rt["pageInfo"]["hasNextPage"]:
            break
        cursor = rt["pageInfo"]["endCursor"]
        if pages > 50:
            raise SystemExit("too many pages")
    return head, total, pages, threads


def checks():
    out = subprocess.run(
        ["gh", "pr", "checks", str(PR), "--repo", "%s/%s" % (REPO_OWNER, REPO_NAME)],
        capture_output=True, text=True)
    rows = []
    for line in out.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) >= 2:
            rows.append({"name": parts[0], "conclusion": parts[1],
                         "url": parts[-1].strip()})
    return rows, out.stdout.strip(), out.stderr.strip()


def main():
    head, total, pages, threads = fetch_threads()
    unresolved = [t for t in threads if not t["isResolved"]]
    rows, raw, err = checks()
    result = {
        "head_sha": head,
        "total_threads": total,
        "fetched_threads": len(threads),
        "pages": pages,
        "unresolved_count": len(unresolved),
        "unresolved": [
            {
                "id": t["id"],
                "path": t["path"],
                "line": t["line"],
                "outdated": t["isOutdated"],
                "author": (t["comments"]["nodes"][0]["author"]["login"]
                           if t["comments"]["nodes"] else None),
                "body": (t["comments"]["nodes"][0]["body"].splitlines()[0][:160]
                         if t["comments"]["nodes"] else ""),
            } for t in unresolved
        ],
        "checks": rows,
        "checks_not_success": [r for r in rows if r["conclusion"] not in
                               ("success", "pass", "neutral", "skipped")],
        "checks_raw": raw,
        "checks_stderr": err,
    }
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
