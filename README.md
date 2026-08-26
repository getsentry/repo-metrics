# repo-metrics

Charts and metrics for git repositories, read straight from the repo.

Ingest parses history once into a local cache. After that every view is a query
against that cache, fast enough to be interactive. There is no server to deploy
and nothing to configure, beyond the GitHub CLI if you want `sync` to fetch repos
for you.

## Install

```bash
cargo build --release
install -m 755 target/release/repo-metrics ~/.local/bin/
```

Use `install` rather than `cp` when upgrading. Overwriting the binary in place,
while a `serve` daemon or the scheduled job is pointing at it, has produced
processes that die immediately with signal 9; replacing the file cleanly avoids
it.

## Quick start

```bash
repo-metrics sync getsentry --ingest    # pick repos, clone them, load them
repo-metrics hotspots --since 90d       # what's been moving
repo-metrics serve                      # browse it all in a browser
```

## Getting repos

`sync` lists an org's repositories, lets you choose from them, and clones the
selection in parallel. Run it again later to bring everything up to date.

```bash
repo-metrics sync                       # asks for the org and a folder
repo-metrics sync getsentry
repo-metrics sync getsentry --ingest
```

If you don't pass `--dir` it looks for `~/code`, `~/src`, `~/dev`, `~/Projects`
and a few others, and offers the first one that exists.

The list is ordered by most recent push. Large orgs have hundreds of repos and an
alphabetical list puts the interesting ones nowhere near the top. `--sort active`
weighs repository size against how long since anyone pushed, for things that are
both substantial and still moving. `--sort stars`, `size` and `name` also work.

`active` is an approximation. GitHub's org listing has no commit count in it, and
the endpoint that does costs a request per repository, so size stands in for how
much history has accumulated.

In the picker, type to filter and press space to check something. `^a` checks
everything matching the current filter, `^x` clears, enter confirms, esc backs
out. Repos you already have are checked when the picker opens, so the default
action in a folder you've used before is to refresh what's there. You get a total
download size before anything starts.

Archived repos and forks are hidden; pass `--archived` or `--forks` to include
them. Listings are cached for an hour, `--refresh` skips the cache. The first 200
repos are fetched by default and `--limit 0` gets all of them.

### Updating existing checkouts

`sync` always fetches. It fast-forwards only when the working tree is clean, the
branch tracks an upstream, and there are no local commits. In any other case it
reports what it found and leaves the repository alone.

```
= getsentry/relay          up to date
↑ getsentry/sentry         fast-forwarded 14 commits
! getsentry/relay           fetched; 3 behind, working tree dirty
! getsentry/snuba          fetched; 1 local commit not pushed, 2 behind
x getsentry/private-thing  failed: Repository not found
```

It never rebases, never force-updates, and never writes over a path that already
exists but isn't a checkout. `--dry-run` prints the plan and changes nothing.

## Keeping the cache warm

`refresh` fetches every repository already in the cache and folds new commits in.
It's incremental, so a repo whose HEAD hasn't moved costs almost nothing.

```bash
repo-metrics refresh
repo-metrics refresh --dir ~/code       # also pick up newly cloned repos there
```

`schedule` installs a macOS LaunchAgent that runs `refresh` on a timer, so the
cache is current whenever you go to use it.

```bash
repo-metrics schedule --interval 30m    # install
repo-metrics schedule --dir ~/code      # and watch a folder for new checkouts
repo-metrics schedule --status
repo-metrics schedule --now             # run it immediately
repo-metrics schedule --logs
repo-metrics schedule --remove
```

The job runs at low priority, logs one line per run to
`~/.cache/repo-metrics/refresh.log`, and won't start a second run while one is
still going. Point it at a binary somewhere stable rather than `target/release`,
or a rebuild will break it. Linux has no equivalent installer; use a systemd timer
or cron entry that runs `repo-metrics refresh`.

## Views

```bash
repo-metrics timeseries --repo sentry --by week --split assist
repo-metrics folders    --repo sentry --depth 2 --metric churn --since 2y
repo-metrics hotspots   --repo sentry --since 90d --top 20
repo-metrics tree       --repo sentry --at 2025-06-01 --subpath src/sentry --depth 2
repo-metrics compare    2025-H2 2026-H1 --repo sentry
repo-metrics flags      --repo sentry --z 2.5 --min-churn 200
repo-metrics assist     --repo sentry --by month
repo-metrics authors    --repo sentry --since 6m --top 25
```

| Command | Answers |
|---|---|
| `timeseries` | Commits over time, with a second line for how many humans were active |
| `folders` | Commits or churn per folder over time |
| `hotspots` | Which directories are moving fastest |
| `tree` | Folder sizes at a point in time, as an indented tree or a sunburst |
| `compare` | Two periods side by side |
| `flags` | Weeks where a folder broke out of its own trailing baseline |
| `assist` | Human vs agent-assisted vs bot over time |
| `authors` | Who is committing, and whether an agent helped |

Every view takes `--repo`, `--since`, `--until` and `--path`. Dates can be
`YYYY-MM-DD`, `90d`, `12w`, `6m`, `2y` or a year. `compare` also understands
`2025-H1`, `2025-Q3` and `start:end`.

`tree` renders as an indented list in the terminal and as a sunburst in HTML;
`--depth` controls how many levels deep it goes. `radial` is accepted as an alias
for it.

`--measure` decides what a folder's size means:

| | |
|---|---|
| `files` *(default)* | Number of files |
| `sloc` | Lines of text, binaries excluded |
| `bytes` | Bytes on disk |

Bytes flatter whatever is bulky rather than whatever is code. In sentry, the
translation catalogue under `src/sentry/locale` is 65% of the byte weight and 67%
of the line count from 90 files, so it swamps a byte- or line-sized tree while
`files` puts it at under 2% and the real subsystems surface. SLOC comes from
`git grep -c`, which counts and skips binaries itself and costs about a third of
a second on a 20,000-file tree.

`timeseries` draws a second line for the number of distinct humans who authored or
co-authored in each bucket, against its own axis on the right (`--overlay none`
turns it off). Co-authors count, since pairing and agent-assisted work both put a
real person on a commit they worked on, and bots and agents are excluded.

Read the two lines separately. They share only the x axis, so where the dashed
line crosses the solid one is a consequence of the two scales and means nothing on
its own. What it is good for is telling apart "more people" from "more per
person": if commits climb while the author line stays flat, output per person rose.

`repos` lists what's in the cache.

### Output

```bash
--format table              # terminal, with sparklines and inline bars (default)
--format json               # for piping
--format html -o out.html   # a single file with the data inlined
```

The HTML is self-contained and makes no network requests, so it survives being
committed, attached to a pull request or emailed.

### Server

```bash
repo-metrics serve
repo-metrics serve --daemon
repo-metrics serve --status
repo-metrics serve --stop
```

Holds the cache in memory and answers the same queries over HTTP, which makes the
filters feel direct rather than like submitting a form. It re-checks each repo's
HEAD periodically and folds in new commits, so a page left open follows the repo
as you work. Every view links its commit back to the forge.

Repository names in the header link to the forge, and the `since` and `until`
dates link to the commits they resolve to — the earliest and latest commit of
that day. A date with no commits on it links to the nearest commit inside the
range, marked with an asterisk.

Folder sizes, fastest-moving and compare let you click a folder to descend into
it, with a `..` button above the table to come back up. In the folder view the
sunburst arcs are clickable too. Drilling is a navigation, so Back undoes it and
the path lands in the URL like everything else.

The URL is the single source of truth for what you're looking at. Every
interaction writes the URL first, and the controls and the chart are both rebuilt
from it, so the address bar, the filter inputs and what's on screen can't drift
apart. A view you've drilled into can be bookmarked and comes back exactly as you
left it:

```
http://127.0.0.1:7777/?view=tree&repo=getsentry/sentry&subpath=src/sentry&at=2025-06-01&depth=2
```

Switching tabs adds a history entry so Back returns to the previous view;
adjusting a filter replaces the current one, so a session doesn't fill the
history with every keystroke. The tab title names the view and repo, which is
what a bookmark gets called.

It binds to 127.0.0.1 only. It serves the full contents of your private
repositories and has no authentication of its own.

## How things are counted

Author date, not committer date. Rebases, squash-merges and cherry-picks all
rewrite the committer date, which would move a week of work onto the day it
landed.

Merges contribute no file rows. `git log --numstat` prints nothing for a merge
unless you force it, and forcing it double-counts every line already attributed
to the branch commits. Merges are excluded from churn.

Binary files are recorded, not skipped. They report `-` for both columns and are
stored with null line counts.

Renames count against the new path, so a rename doesn't look like a directory
that churned and then vanished.

Tree snapshots are taken on demand rather than stored. `ls-tree` is quick enough
that keeping a snapshot table would make it the largest thing in the cache for
very little benefit. It reports bytes, not lines: git's tree data has no line
count in it, and computing real SLOC would mean reading every blob.

## Authorship

The cache stores raw author and co-author strings. Labels are worked out at query
time, so recognising a new tool is a config change and a re-run rather than a
re-ingest.

Commits fall into four kinds:

| Kind | Means |
|---|---|
| `human` | A person wrote it, with no agent credited |
| `agent_assisted` | A person authored it and credited an agent as co-author |
| `agent` | No human author: an agent opened the commit itself, or a bot landed an agent's work |
| `bot` | Automation that doesn't write code: dependency bumps, releases, reverts, CI, scanners |

Several things this gets right that a naive version doesn't:

Agents show up both ways. Claude, Codex and Cursor usually sit in `Co-authored-by`
trailers while the author stays the person who ran them, so looking only at the
author field finds almost no agent activity. But agents also open pull requests
under their own identity, and treating every `[bot]` as automation files that work
alongside Dependabot.

Not every bot is a bot in the same sense. A revert bot and a license-header bumper
are automation; Seer and Junior are writing code. They're counted separately.

Identities are keyed on the numeric GitHub user id where there is one, because
bots get renamed. One id shows up in its history under four different names.

`@users.noreply.github.com` is not a bot signal. It's the default privacy address
for ordinary accounts, and treating it as one classifies most of a team as robots.

Coding agents and infrastructure bots are counted separately. Dependabot and CI
bots are not AI assistants, and folding them together roughly quadruples the
reported agent-assisted share of a busy year.

Add your own rules in `~/.config/repo-metrics/identities.toml`:

```toml
[[identity]]
match_kind  = "email_domain"   # github_user_id | email | email_domain | name_prefix
match_value = "windsurf.com"
tool        = "windsurf"
kind        = "agent"          # agent writes code, infra does chores
```

More specific rules win. An id beats an email, an email beats a domain, a domain
beats a name prefix, and a longer prefix beats a shorter one.

## Development

```bash
cargo build --release
./scripts/check.sh      # checks command output, not just exit codes
```

## Files

```
~/.cache/repo-metrics/cache.bin      parsed history
~/.cache/repo-metrics/refresh.log    scheduled job output
~/.cache/repo-metrics/serve.log      server output
~/.config/repo-metrics/identities.toml
```

Set `REPO_METRICS_CACHE` to move the cache. It's keyed on a parser version, and
bumping that re-ingests from scratch, which is cheap enough not to plan around.

## Limits

You need the repositories cloned. `--numstat` diffs blob contents, so a blobless
or shallow clone would refetch during ingest and end up slower than cloning
properly once.

The server is loopback-only, so there's no link to send anyone. Use
`--format html` for something you can hand over.

Freshness is whenever you last ran `ingest` or `refresh`, or whatever the
scheduled job has picked up.
