#!/bin/bash
# Verifies OUTPUT, not just exit status — an exit-code-only check missed a bug
# where every terminal render printed twice.
B="${B:-$(dirname "$0")/../target/release/repo-metrics}"
fail=0
chk(){ # chk <desc> <expected-count> <pattern> <cmd...>
  local desc="$1" want="$2" pat="$3"; shift 3
  local got; got=$("$@" 2>/dev/null | grep -c "$pat")
  if [ "$got" != "$want" ]; then echo "  FAIL $desc (wanted $want '$pat', got $got)"; fail=1; fi
}
# each header/label must appear exactly once
chk "assist header"     1 "Authorship over time"  $B assist --repo sentry --since 2026-06-01
chk "assist bands"      4 "commits$"              $B assist --repo sentry --since 2026-06-01
chk "hotspots header"   1 "Fastest-moving"        $B hotspots --repo sentry --since 1y --top 3
chk "hotspots rows"     3 "^  [a-z.]"             $B hotspots --repo sentry --since 1y --top 3
chk "tree header"       1 "files ·"               $B tree --repo sentry --depth 1
chk "timeseries header" 1 "Commits over time"     $B timeseries --repo sentry --by month --since 1y
chk "authors header"    1 "^Authors$"             $B authors --repo sentry --top 5
chk "compare header"    1 "^Compare"              $B compare 2025-H2 2026-H1 --repo sentry
chk "flags header"      1 "interesting periods"   $B flags --repo sentry --since 2y
chk "folders header"    1 "Folders over time"     $B folders --repo sentry --since 1y
chk "source link"       1 "https://github.com"    $B hotspots --repo sentry --since 1y --top 3
# json/html stay well-formed
$B hotspots --repo sentry --since 1y --format json 2>/dev/null | python3 -c 'import json,sys; json.load(sys.stdin)' || { echo "  FAIL json invalid"; fail=1; }
$B hotspots --repo sentry --since 1y --format html -o /tmp/r.html >/dev/null 2>&1
node -e 'const h=require("fs").readFileSync("/tmp/r.html","utf8");const m=h.match(/<script>([\s\S]*?)<\/script>/);new Function(m[1]);' || { echo "  FAIL html js"; fail=1; }
[ "$(grep -c '<title>' /tmp/r.html)" = "1" ] || { echo "  FAIL html duplicated"; fail=1; }
echo "  output regression failures: $fail"
