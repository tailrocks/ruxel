#!/usr/bin/env bash
# Poll PR #96 unresolved review threads (paginated) + CI check conclusions.
set -uo pipefail
REPO="tailrocks/ruxel"
PR=96
OUT="${1:-/tmp/pr96_poll.json}"

PAGE='query($owner:String!, $name:String!, $pr:Int!, $cursor:String) {
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
}'

CURSOR="null"
PAGES=0
: > /tmp/pr96_threads.jsonl
while : ; do
  resp=$(gh api graphql -f query="$PAGE" -f owner="tailrocks" -f name="ruxel" -F pr="$PR" ${CURSOR:+-f cursor="$CURSOR"} 2>&1)
  if [[ "$resp" == *"error"* ]]; then
    echo "GRAPHQL_ERROR: $resp" >&2
    exit 1
  fi
  echo "$resp" >> /tmp/pr96_threads.jsonl
  PAGES=$((PAGES+1))
  hasnext=$(echo "$resp" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["data"]["repository"]["pullRequest"]["reviewThreads"]["pageInfo"]["hasNextPage"])')
  cursor=$(echo "$resp" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["data"]["repository"]["pullRequest"]["reviewThreads"]["pageInfo"]["endCursor"])')
  if [[ "$hasnext" != "True" ]]; then break; fi
  CURSOR="$cursor"
  if [[ $PAGES -gt 40 ]]; then echo "MAX_PAGES" >&2; break; fi
done

python3 - "$OUT" <<'PY'
import json,sys,glob
out=sys.argv[1]
head=None; total=None; threads=[]
for f in ["/tmp/pr96_threads.jsonl"]:
    for line in open(f):
        line=line.strip()
        if not line: continue
        d=json.loads(line)["data"]["repository"]["pullRequest"]
        head=d["headRefOid"]; total=d["reviewThreads"]["totalCount"]
        threads.extend(d["reviewThreads"]["nodes"])
unresolved=[t for t in threads if not t["isResolved"]]
res={"head_sha":head,"total_threads":total,"fetched":len(threads),"unresolved_count":len(unresolved),
     "unresolved":[{"id":t["id"],"path":t["path"],"line":t["line"],"outdated":t["isOutdated"],
                    "author":(t["comments"]["nodes"][0]["author"]["login"] if t["comments"]["nodes"] else None),
                    "body":(t["comments"]["nodes"][0]["body"][:200] if t["comments"]["nodes"] else "")}
                   for t in unresolved]}
json.dump(res,open(out,"w"),indent=2)
print(json.dumps(res,indent=2)[:4000])
PY
