# repo-metrics

Repository metrics and visualizations read straight from git. No warehouse, no cloud
project, no credentials — one binary, one local cache, and a server you can run when
you want the views to be interactive.

This is the local counterpart to a GCP pipeline design (BigQuery + Cloud Run + Cloud
Scheduler + Firestore + IAP). The analysis is the same; where it runs is not.

## Why local works

Measured on this machine against `getsentry/sentry` — 108,973 commits, 780 MB of
`.git`, 20,594 files at HEAD:

| Operation | Time |
|---|---|
| Full history ingest (metadata + numstat, sharded across cores) | **10.6 s** |
| Incremental ingest, one day of commits | **0.1 s** |
| Any view over the full cache | **1–5 ms** |
| Point-in-time tree snapshot (`git ls-tree -r --long`) | **0.08 s** |
| Cache on disk, sentry + seer together | **23 MB** |

563,383 file-change rows for sentry, 25,609 for seer. That is not a warehouse — it is
a file that fits in memory twice over. Most of the machinery a remote pipeline needs
(bundle caches, checkpoints, staging tables, `MERGE` dedup, partition-filter guards,
schedulers, load balancers) exists to avoid re-doing a ten-second computation.

## Install

```bash
cargo build --release
cp target/release/repo-metrics ~/.local/bin/     # or anywhere on PATH
```

## Use

```bash
repo-metrics ingest ~/code/sentry ~/code/seer    # full first time, incremental after
repo-metrics repos                               # what's in the cache

repo-metrics timeseries --repo sentry --by week --split assist
repo-metrics folders    --repo sentry --depth 2 --metric churn --since 2y
repo-metrics hotspots   --repo sentry --since 90d --top 20
repo-metrics tree       --repo sentry --at 2025-06-01 --subpath src/sentry
repo-metrics radial     --repo sentry --depth 3
repo-metrics compare    2025-H2 2026-H1 --repo sentry
repo-metrics flags      --repo sentry --z 2.5 --min-churn 200
repo-metrics assist     --repo sentry --by month
repo-metrics authors    --repo sentry --since 6m --top 25
```

Every view takes `--repo`, `--since`, `--until`, `--path`, and `--format`.
Dates accept `YYYY-MM-DD`, `90d`, `12w`, `6m`, `2y`, or a bare year; `compare`
additionally takes `2025-H1`, `2025-Q3`, `2024`, or `start:end`.

### Output formats

```bash
--format table            # terminal, with sparklines and in-row bars (default)
--format json             # pipe it anywhere
--format html -o out.html # one self-contained file; no network at render time
```

### Interactive server

```bash
repo-metrics serve                    # foreground, opens a browser
repo-metrics serve --daemon           # detach; logs to the cache dir
repo-metrics serve --status
repo-metrics serve --stop
```

Binds **127.0.0.1 only** — it serves the full contents of your private repositories
and has no authentication of its own.

The daemon re-checks each repo's HEAD every 30s (`--refresh N`, `0` disables) and
folds in new commits incrementally, so an open page follows the repo as you work.
Every view links its commit out to the forge.

## Views

| Command | What it answers |
|---|---|
| `timeseries` | Commits over time, splittable by assist kind, tool, author, or language |
| `folders` | Commits or churn by folder over time, as stacked bands |
| `hotspots` | Fastest-moving directories, ranked by churn |
| `tree` | Folder sizes at a point in time, one level at a time |
| `radial` | Full-depth tree at a point in time, as a sunburst |
| `compare` | Two periods side by side, as a delta |
| `flags` | Weeks where a folder broke out of its own trailing baseline |
| `assist` | Human vs agent-assisted vs bot, over time |
| `authors` | Who is committing, and whether an agent helped |

## Decisions that change the numbers

**Author date, not committer date.** Rebases, squash-merges and cherry-picks rewrite
the committer date, collapsing a week of work onto the day it landed.

**Merges contribute no file rows.** `git log --numstat` emits nothing for a merge
unless forced, which would double-count every line already attributed to the branch
commits. Merges are excluded from churn aggregates.

**Binary files are recorded, not skipped.** They report `-` for both columns and are
stored with null line counts.

**Tree snapshots are taken on demand, never stored.** `ls-tree --long` is ~0.1 s, so a
snapshot table would be the largest thing in the system in exchange for very little.
It also reports **bytes**, not lines — there is no line count anywhere in git's tree
data, and true SLOC would mean reading every blob.

**Renames count against the new path**, so a rename does not read as a directory that
churns and then vanishes.

### Authorship is derived on read

The cache stores raw author and co-author strings and nothing else. Human / agent /
bot labels are computed at query time against a rule table, so a newly-recognised
agent is a config edit and a re-run, not a re-ingest.

The rules that hold up:

- **The coding agents are co-authors, not authors.** Claude, Codex and Cursor sit in
  `Co-authored-by` trailers while the author stays the human who ran them. Classifying
  on the author field alone reports almost no agent activity.
- **Key on the numeric GitHub user id.** Bots get renamed — id `157164994` appears in
  its history under four different names.
- **`@users.noreply.github.com` is not a bot signal.** It is the default privacy
  address for ordinary accounts; treating it as one files most of the team as robots.
- **`agent` and `infra` are different.** Dependabot and getsantry are bots, but they
  are not AI coding assistants. Conflating them roughly quadruples the reported
  agent-assisted share in 2025.

Override or extend the table at `~/.config/repo-metrics/identities.toml`:

```toml
[[identity]]
match_kind  = "email_domain"   # github_user_id | email | email_domain | name_prefix
match_value = "windsurf.com"
tool        = "windsurf"
kind        = "agent"          # agent (writes code) | infra (deps, reverts)
```

More specific rules win: an id beats an email, an email beats a domain, a domain beats
a name prefix, and a longer prefix beats a shorter one.

## Files

```
~/.cache/repo-metrics/cache.bin     parsed history (bincode, interned strings)
~/.cache/repo-metrics/serve.pid     daemon pid
~/.cache/repo-metrics/serve.log     daemon output
~/.config/repo-metrics/identities.toml   optional identity overrides
```

Cache is keyed on a parser version; bumping it re-ingests from scratch, which is ten
seconds and therefore not an event worth engineering around. Set `REPO_METRICS_CACHE`
to relocate it.

## Limits

- **Needs the repos cloned.** `--numstat` diffs blob contents, so a blobless partial
  clone would lazily refetch and be far slower. Budget the full `.git`.
- **No shared URL.** The server is loopback-only. Use `--format html` for something
  you can hand to someone.
- **Nothing is scheduled.** Freshness is whenever you last ran it, or whatever the
  daemon's refresh interval catches.
