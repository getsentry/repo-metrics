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
chk "hotspots rows"     3 "^  [a-z.].*[0-9]"             $B hotspots --repo sentry --since 1y --top 3
chk "tree header"       1 "^[0-9,]* files$"               $B tree --repo sentry --depth 1
chk "timeseries header" 1 "Commits over time"     $B timeseries --repo sentry --by month --since 1y
chk "authors header"    1 "^Authors$"             $B authors --repo sentry --top 5
chk "compare header"    1 "^Compare"              $B compare 2025-H2 2026-H1 --repo sentry
chk "flags header"      1 "interesting periods"   $B flags --repo sentry --since 2y
chk "folders header"    1 "Folders over time"     $B folders --repo sentry --since 1y
chk "source link"       1 "https://github.com"    $B hotspots --repo sentry --since 1y --top 3
# tree and radial were once two views rendering identically; radial is now only an
# alias, and must not come back as its own subcommand
[ "$($B --help 2>&1 | grep -cE '^  radial')" = "0" ] || { echo "  FAIL radial is a separate subcommand again"; fail=1; }
diff <($B tree --repo sentry --depth 2 --format json 2>/dev/null) \
     <($B radial --repo sentry --depth 2 --format json 2>/dev/null) >/dev/null \
  || { echo "  FAIL radial alias diverged from tree"; fail=1; }

# json/html stay well-formed
$B hotspots --repo sentry --since 1y --format json 2>/dev/null | python3 -c 'import json,sys; json.load(sys.stdin)' || { echo "  FAIL json invalid"; fail=1; }
$B hotspots --repo sentry --since 1y --format html -o /tmp/r.html >/dev/null 2>&1
node -e 'const h=require("fs").readFileSync("/tmp/r.html","utf8");const m=h.match(/<script>([\s\S]*?)<\/script>/);new Function(m[1]);' || { echo "  FAIL html js"; fail=1; }
[ "$(grep -c '<title>' /tmp/r.html)" = "1" ] || { echo "  FAIL html duplicated"; fail=1; }
# The commits chart carries a second series on its own scale. It must be separate
# from `series`, or it would be stacked with them as if it shared their axis.
$B timeseries --repo sentry --by month --since 2y --format json 2>/dev/null \
  | python3 -c '
import json,sys
d=json.load(sys.stdin)
ov=d.get("overlay"); assert ov, "no authors overlay"
assert len(ov["points"])==len(d["x"]), "overlay length differs from the x axis"
assert any(v>0 for v in ov["points"]), "overlay is all zero"
assert all(s["name"]!=ov["name"] for s in d["series"]), "overlay leaked into series"
' || { echo "  FAIL authors overlay"; fail=1; }
$B timeseries --repo sentry --by month --since 1y --overlay none --format json 2>/dev/null \
  | python3 -c 'import json,sys; assert "overlay" not in json.load(sys.stdin), "--overlay none still emits one"' \
  || { echo "  FAIL --overlay none"; fail=1; }

# Commit-size metrics have to stay internally consistent: every metric divides by
# the same commit count, so added-per-commit plus removed-per-commit is exactly
# churn-per-commit, and modified can never exceed either side of the diff.
python3 - "$B" <<'PYEOF' || { echo "  FAIL commit-size metrics"; fail=1; }
import json,subprocess,sys
B=sys.argv[1]
def pts(metric,per):
    out=subprocess.run([B,"timeseries","--repo","sentry","--by","month","--since","2y",
        "--metric",metric,"--per",per,"--overlay","none","--format","json"],
        capture_output=True,text=True).stdout
    return json.loads(out)["series"][0]["points"]
a=pts("added","commit"); r=pts("removed","commit")
c=pts("churn","commit"); m=pts("modified","commit")
checked=0
for i in range(len(c)):
    if c[i]<=0: continue
    assert abs((a[i]+r[i])-c[i])<0.01, f"bucket {i}: {a[i]}+{r[i]} != {c[i]}"
    assert m[i]<=min(a[i],r[i])+0.01, f"bucket {i}: modified {m[i]} > min({a[i]},{r[i]})"
    checked+=1
assert checked>12, f"only {checked} buckets compared"
PYEOF

# compare puts two different things in one set of columns; they must stay labelled
# and separated, and the directory rows must say what they measure.
$B compare 2025-H2 2026-H1 --repo sentry --top 5 --format json 2>/dev/null \
  | python3 -c '
import json,sys
d=json.load(sys.stdin); secs=d.get("sections") or []
assert len(secs)==2, f"expected two sections, got {len(secs)}"
assert secs[0]["start"]==0
assert secs[1]["start"]>0
assert "churn" in secs[1]["label"], f"directory section unlabelled: {secs[1]["label"]!r}"
# a percentage change from zero has to be blank, not 0
for r in d["rows"]:
    if r[1]==0 and r[2]>0:
        assert r[4] is None, f"growth from zero reported as {r[4]}"
' || { echo "  FAIL compare sections"; fail=1; }

# per-human must actually be the raw metric divided by the authors overlay for the
# same bucket, not an independently computed number that could drift from it.
python3 - "$B" <<'PYEOF' || { echo "  FAIL per-human division"; fail=1; }
import json,subprocess,sys
B=sys.argv[1]
def run(*a):
    return json.loads(subprocess.run([B,"timeseries","--repo","sentry","--by","month",
        "--since","2y","--format","json",*a],capture_output=True,text=True).stdout)
raw=run("--metric","churn","--per","total")
per=run("--metric","churn","--per","human")
assert per.get("rate") is True, "per-human should be flagged as a rate"
r=raw["series"][0]["points"]; p=per["series"][0]["points"]; h=raw["overlay"]["points"]
checked=0
for i in range(len(r)):
    if h[i]>0 and r[i]>0:
        assert abs(p[i]-r[i]/h[i])<0.01, f"bucket {i}: {p[i]} != {r[i]}/{h[i]}"
        checked+=1
assert checked>6, f"only {checked} buckets compared"
PYEOF

# Header links: repo names go to the forge, date boundaries to real commits.
$B hotspots --repo sentry --since 2026-05-28 --until 2026-06-30 --format json 2>/dev/null \
  | python3 -c '
import json,sys
d=json.load(sys.stdin); sc=d.get("scope") or {}
assert sc.get("repos") and sc["repos"][0].get("url"), "repo link missing"
for k in ("since","until"):
    assert sc.get(k,{}).get("url"), k+" commit link missing"
' || { echo "  FAIL header scope links"; fail=1; }

# Directory rows carry drill targets; compare must not mark its summary rows.
$B hotspots --repo sentry --since 1y --format json 2>/dev/null \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("drill") and any(d["drill"]), "no drill targets"' \
  || { echo "  FAIL hotspots drill targets"; fail=1; }
$B compare 2025-H2 2026-H1 --repo sentry --format json 2>/dev/null \
  | python3 -c '
import json,sys
d=json.load(sys.stdin); dr=d.get("drill") or []
assert dr and dr[0] is None, "summary rows should not be drillable"
assert any(x for x in dr), "no folder rows drillable"
' || { echo "  FAIL compare drill targets"; fail=1; }

# Folder sizes must support all three measures and report which one it used.
for m in files sloc bytes; do
  $B tree --repo sentry --depth 1 --measure $m --format json 2>/dev/null \
    | python3 -c "import json,sys; d=json.load(sys.stdin); assert d['measure']=='$m', d['measure']" \
    || { echo "  FAIL tree --measure $m"; fail=1; }
done

# The web app keeps its state in the URL so views can be bookmarked. Only checked
# when a server happens to be running; check.sh does not start one.
PORT="${PORT:-7777}"
if curl -s -o /dev/null --max-time 2 "localhost:$PORT/" 2>/dev/null; then
  page=$(curl -s "localhost:$PORT/")
  for fn in readUrl syncUrl applyUrl navigate popstate scopeHtml drillBar __drill caveat; do
    echo "$page" | grep -q "$fn" || { echo "  FAIL app lost $fn (URL state)"; fail=1; }
  done
  # The URL is the single source of truth. Mutating `state` directly and
  # re-rendering is what let the path input drift out of step with the chart.
  for bad in 'state\[inp.dataset.f\]=' 'state\[drillField()\]='; do
    echo "$page" | grep -q "$bad" && { echo "  FAIL app mutates state directly ($bad)"; fail=1; }
  done
  code=$(curl -s -o /dev/null -w '%{http_code}' "localhost:$PORT/?view=hotspots&repo=x&since=90d")
  [ "$code" = "200" ] || { echo "  FAIL app does not serve a query-string URL ($code)"; fail=1; }
else
  echo "  (no server on :$PORT — skipped URL-state checks)"
fi

echo "  output regression failures: $fail"
