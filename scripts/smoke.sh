#!/bin/bash
# Exercises every view against a real repository. Defaults to this checkout, which
# means CI has something with genuine history to analyse without cloning anything.
#
#   ./scripts/smoke.sh [repo-path]
set -uo pipefail

B="${B:-$(dirname "$0")/../target/release/repo-metrics}"
REPO="${1:-$(dirname "$0")/..}"
CACHE="$(mktemp -d)"
export REPO_METRICS_CACHE="$CACHE"
trap 'rm -rf "$CACHE"' EXIT

fail=0
run() { # run <description> <min-output-bytes> <args...>
  local desc="$1" min="$2"; shift 2
  local out; out=$("$B" "$@" 2>&1)
  local rc=$? len=${#out}
  if [ $rc -ne 0 ]; then
    echo "  FAIL $desc (exit $rc)"; echo "$out" | head -3 | sed 's/^/        /'; fail=1
  elif [ "$len" -lt "$min" ]; then
    echo "  FAIL $desc (only $len bytes of output)"; fail=1
  else
    echo "  ok   $desc"
  fi
}

echo "smoke: $B against $REPO"
run "ingest"      10 ingest "$REPO"
run "repos"       40 repos
run "timeseries" 100 timeseries --by month
run "per commit" 100 timeseries --by month --metric churn --per commit
run "per human"  100 timeseries --by month --metric added --per human
run "modified"   100 timeseries --by month --metric modified
run "split"      100 timeseries --by month --split assist
run "folders"    100 folders --by month --depth 1
run "hotspots"   100 hotspots --depth 1
run "tree"       100 tree --depth 1
run "tree sloc"  100 tree --depth 1 --measure sloc
run "authors"    100 authors --top 5
run "assist"     100 assist --by month
run "flags"       40 flags --z 1.5 --min-churn 1
run "json"       100 hotspots --format json
run "html"      2000 hotspots --format html

# The generated page has to be valid JSON and parseable JS, not just non-empty.
"$B" hotspots --format json 2>/dev/null | python3 -c 'import json,sys; json.load(sys.stdin)' \
  || { echo "  FAIL json is not valid"; fail=1; }

echo "smoke failures: $fail"
exit $fail
