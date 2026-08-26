use crate::identity::{AssistKind, Identities};
use crate::model::*;
use crate::output::*;
use anyhow::{bail, Result};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Debug)]
pub enum Bucket {
    Day,
    Week,
    Month,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Debug)]
pub enum Metric {
    Commits,
    Churn,
    Added,
    Removed,
    Files,
}

impl Metric {
    pub fn label(&self) -> &'static str {
        match self {
            Metric::Commits => "commits",
            Metric::Churn => "lines churned",
            Metric::Added => "lines added",
            Metric::Removed => "lines removed",
            Metric::Files => "files touched",
        }
    }
}

/// What a folder's "size" means. Bytes are free but misleading for code — a
/// translation catalogue or a vendored asset outweighs a whole subsystem.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Debug)]
pub enum Measure {
    /// Number of files
    Files,
    /// Lines of text, binaries excluded
    Sloc,
    /// Bytes on disk
    Bytes,
}

impl Measure {
    pub fn label(&self) -> &'static str {
        match self {
            Measure::Files => "files",
            Measure::Sloc => "sloc",
            Measure::Bytes => "bytes",
        }
    }
}

/// Whether a metric is reported raw or divided by the people who produced it.
/// Raw totals conflate a bigger team with a more productive one.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Debug)]
pub enum Per {
    /// Raw totals
    Total,
    /// Divided by the distinct humans active in the same bucket
    Human,
}

/// A second measure drawn against its own axis on the commits chart.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Debug)]
pub enum Overlay {
    /// No second line
    None,
    /// Distinct humans who authored or co-authored in each bucket
    Authors,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Debug)]
pub enum Split {
    None,
    Assist,
    Tool,
    Author,
    Language,
}

#[derive(Clone, Default)]
pub struct Filter {
    pub repo: Option<String>,
    pub since: Option<i32>,
    pub until: Option<i32>,
    pub path: Option<String>,
}

impl Filter {
    pub fn covers(&self, days: i32) -> bool {
        self.since.map_or(true, |s| days >= s) && self.until.map_or(true, |u| days <= u)
    }
}

/// Accepts absolute `YYYY-MM-DD`, relative `90d` / `12w` / `6m` / `2y`, and the
/// half-year and quarter shorthands people reach for when comparing periods.
pub fn parse_date(s: &str) -> Result<i32> {
    let s = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(date_to_days(d));
    }
    let today = Utc::now().date_naive();
    if let Some(num) = s.strip_suffix(|c| matches!(c, 'd' | 'w' | 'm' | 'y')) {
        if let Ok(n) = num.parse::<i64>() {
            let unit = s.chars().last().unwrap();
            let days = match unit {
                'd' => n,
                'w' => n * 7,
                'm' => n * 30,
                _ => n * 365,
            };
            return Ok(date_to_days(today - Duration::days(days)));
        }
    }
    if let Ok(y) = s.parse::<i32>() {
        if (1970..2200).contains(&y) {
            return Ok(date_to_days(NaiveDate::from_ymd_opt(y, 1, 1).unwrap()));
        }
    }
    bail!("cannot parse date {s:?} (want YYYY-MM-DD, 90d, 12w, 6m, 2y, or a year)")
}

/// `2025-H1`, `2025-Q3`, `2025`, or a bare `YYYY-MM-DD:YYYY-MM-DD` range.
pub fn parse_period(s: &str) -> Result<(i32, i32, String)> {
    let s = s.trim();
    if let Some((a, b)) = s.split_once(':') {
        return Ok((parse_date(a)?, parse_date(b)?, s.to_string()));
    }
    let up = s.to_uppercase();
    if let Some((y, part)) = up.split_once('-') {
        if let Ok(year) = y.parse::<i32>() {
            let months: Option<(u32, u32)> = match part {
                "H1" => Some((1, 6)),
                "H2" => Some((7, 12)),
                "Q1" => Some((1, 3)),
                "Q2" => Some((4, 6)),
                "Q3" => Some((7, 9)),
                "Q4" => Some((10, 12)),
                _ => None,
            };
            if let Some((m0, m1)) = months {
                let start = NaiveDate::from_ymd_opt(year, m0, 1).unwrap();
                let end = last_day_of(year, m1);
                return Ok((date_to_days(start), date_to_days(end), s.to_string()));
            }
        }
    }
    if let Ok(year) = s.parse::<i32>() {
        if (1970..2200).contains(&year) {
            return Ok((
                date_to_days(NaiveDate::from_ymd_opt(year, 1, 1).unwrap()),
                date_to_days(last_day_of(year, 12)),
                s.to_string(),
            ));
        }
    }
    let d = parse_date(s)?;
    Ok((d, d, s.to_string()))
}

fn last_day_of(y: i32, m: u32) -> NaiveDate {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - Duration::days(1)
}

pub fn bucket_key(days: i32, b: Bucket) -> (i32, String) {
    let d = days_to_date(days);
    match b {
        Bucket::Day => (days, d.format("%Y-%m-%d").to_string()),
        Bucket::Week => {
            // ISO weeks start Monday; normalise to the Monday so buckets line up
            // across repos regardless of when each one's history starts.
            let dow = d.weekday().num_days_from_monday() as i64;
            let monday = d - Duration::days(dow);
            (date_to_days(monday), monday.format("%Y-%m-%d").to_string())
        }
        Bucket::Month => {
            let first = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap();
            (date_to_days(first), first.format("%Y-%m").to_string())
        }
    }
}

/// One commit, with its identity already resolved.
pub struct Resolved<'a> {
    pub commit: &'a Commit,
    pub repo: &'a RepoData,
    pub assist: AssistKind,
    pub tools: Vec<String>,
}

impl<'a> Resolved<'a> {
    pub fn author(&self) -> &str {
        self.repo.s(self.commit.author)
    }
}

/// Resolving identities is the only per-commit work that touches strings, so it is
/// memoised on the interned ids. Editing the identity table and re-running is a
/// fraction of a second on a full history — which is the point of deriving these
/// labels on read instead of freezing them in at ingest.
pub fn resolve<'a>(
    repo: &'a RepoData,
    ids: &Identities,
    f: &Filter,
    include_merges: bool,
) -> Vec<Resolved<'a>> {
    let mut memo: HashMap<(u32, u32, u64), (AssistKind, Vec<String>)> = HashMap::new();
    let mut out = Vec::new();
    for c in &repo.commits {
        if !f.covers(c.days) {
            continue;
        }
        if c.is_merge && !include_merges {
            continue;
        }
        let mut h: u64 = 0;
        for (n, e) in &c.coauthors {
            h = h.wrapping_mul(1000003).wrapping_add(*n as u64);
            h = h.wrapping_mul(1000003).wrapping_add(*e as u64);
        }
        let key = (c.author, c.email, h);
        let (assist, tools) = memo
            .entry(key)
            .or_insert_with(|| {
                let co: Vec<(String, String)> = c
                    .coauthors
                    .iter()
                    .map(|(n, e)| (repo.s(*n).to_string(), repo.s(*e).to_string()))
                    .collect();
                ids.classify(repo.s(c.author), repo.s(c.email), &co)
            })
            .clone();
        out.push(Resolved {
            commit: c,
            repo,
            assist,
            tools,
        });
    }
    out
}

/// Repo selection tries progressively looser matches and stops at the first that
/// hits anything. A plain substring test alone is wrong here: `--repo sentry` would
/// also select `getsentry/relay`, because "getsentry" contains "sentry".
pub fn select_repos<'a>(cache: &'a Cache, f: &Filter) -> Vec<&'a RepoData> {
    let want = match &f.repo {
        None => return cache.repos.iter().collect(),
        Some(w) => w.trim().to_lowercase(),
    };
    let short = |r: &RepoData| r.name.rsplit('/').next().unwrap_or("").to_lowercase();

    let tiers: [Box<dyn Fn(&RepoData) -> bool>; 4] = [
        Box::new({
            let want = want.clone();
            move |r: &RepoData| r.name.to_lowercase() == want
        }),
        Box::new({
            let want = want.clone();
            move |r: &RepoData| short(r) == want
        }),
        Box::new({
            let want = want.clone();
            move |r: &RepoData| short(r).contains(&want)
        }),
        Box::new({
            let want = want.clone();
            move |r: &RepoData| {
                r.name.to_lowercase().contains(&want) || r.path.to_lowercase().contains(&want)
            }
        }),
    ];
    for t in tiers.iter() {
        let hit: Vec<&RepoData> = cache.repos.iter().filter(|r| t(r)).collect();
        if !hit.is_empty() {
            return hit;
        }
    }
    Vec::new()
}

pub fn path_matches(dir: &str, path: &str, prefix: &Option<String>) -> bool {
    match prefix {
        None => true,
        Some(p) => {
            let p = p.trim_end_matches('/');
            p.is_empty() || dir == p || dir.starts_with(&format!("{p}/")) || path.starts_with(&format!("{p}/"))
        }
    }
}

/// Sum a metric over one commit, honouring the active path filter.
pub fn metric_value(r: &Resolved, m: Metric, path: &Option<String>) -> f64 {
    let changes = r.repo.changes_of(r.commit);
    match m {
        Metric::Commits => {
            if path.is_none() {
                return 1.0;
            }
            let hit = changes
                .iter()
                .any(|c| path_matches(r.repo.s(c.dir), r.repo.s(c.path), path));
            if hit {
                1.0
            } else {
                0.0
            }
        }
        _ => changes
            .iter()
            .filter(|c| path_matches(r.repo.s(c.dir), r.repo.s(c.path), path))
            .map(|c| match m {
                Metric::Churn => c.churn() as f64,
                Metric::Added => c.added.max(0) as f64,
                Metric::Removed => c.removed.max(0) as f64,
                Metric::Files => 1.0,
                Metric::Commits => 0.0,
            })
            .sum(),
    }
}

/// Dense bucket axis: every bucket between first and last gets a slot, so a gap in
/// activity reads as a gap rather than silently closing up.
pub fn axis(keys: &[i32], b: Bucket) -> Vec<(i32, String)> {
    if keys.is_empty() {
        return Vec::new();
    }
    let min = *keys.iter().min().unwrap();
    let max = *keys.iter().max().unwrap();
    let mut out = Vec::new();
    let mut cur = min;
    let mut guard = 0;
    while cur <= max && guard < 200_000 {
        out.push(bucket_key(cur, b));
        let d = days_to_date(cur);
        cur = match b {
            Bucket::Day => cur + 1,
            Bucket::Week => cur + 7,
            Bucket::Month => {
                let (y, m) = if d.month() == 12 {
                    (d.year() + 1, 1)
                } else {
                    (d.year(), d.month() + 1)
                };
                date_to_days(NaiveDate::from_ymd_opt(y, m, 1).unwrap())
            }
        };
        guard += 1;
    }
    out.dedup_by_key(|(k, _)| *k);
    out
}

pub fn build_series(
    buckets: &HashMap<(String, i32), f64>,
    axis: &[(i32, String)],
    names: &[String],
) -> Vec<Series> {
    names
        .iter()
        .map(|n| Series {
            name: n.clone(),
            points: axis
                .iter()
                .map(|(k, _)| *buckets.get(&(n.clone(), *k)).unwrap_or(&0.0))
                .collect(),
        })
        .collect()
}

pub fn range_label(f: &Filter, repos: &[&RepoData]) -> String {
    let names: Vec<&str> = repos.iter().map(|r| r.name.as_str()).collect();
    let scope = if names.is_empty() {
        "no repos".to_string()
    } else {
        names.join(", ")
    };
    let when = match (f.since, f.until) {
        (Some(a), Some(b)) => format!("{} → {}", days_to_date(a), days_to_date(b)),
        (Some(a), None) => format!("since {}", days_to_date(a)),
        (None, Some(b)) => format!("through {}", days_to_date(b)),
        (None, None) => "full history".to_string(),
    };
    let path = f
        .path
        .as_ref()
        .map(|p| format!(" · {p}"))
        .unwrap_or_default();
    format!("{scope} · {when}{path}")
}

/// Stamps a view with the repo state it was read from. Only meaningful when the
/// filter narrowed to a single repo — a chart spanning several has no one commit.
pub fn stamp_source(o: &mut Output, cache: &Cache, f: &Filter) {
    let repos = select_repos(cache, f);
    if let [r] = repos.as_slice() {
        o.set_source_if_unset(Some(SourceRef::new(&r.name, &r.head, &r.web)));
    }
    o.set_scope(Some(build_scope(&repos, f)));
}

/// The commit a date boundary lands on: the earliest commit of that day for the
/// start of a range, the latest for the end. When nothing was committed on the day
/// itself, the nearest commit *inside* the range is used instead, since that is the
/// one the filter actually begins or ends at.
fn boundary_commit(repo: &RepoData, day: i32, earliest: bool) -> Option<(&Commit, bool)> {
    let on_day = repo.commits.iter().filter(|c| c.days == day);
    let exact = if earliest {
        on_day.min_by_key(|c| c.ts)
    } else {
        on_day.max_by_key(|c| c.ts)
    };
    if let Some(c) = exact {
        return Some((c, false));
    }
    let near = if earliest {
        repo.commits.iter().filter(|c| c.days > day).min_by_key(|c| (c.days, c.ts))
    } else {
        repo.commits.iter().filter(|c| c.days < day).max_by_key(|c| (c.days, c.ts))
    };
    near.map(|c| (c, true))
}

fn date_link(repos: &[&RepoData], day: i32, earliest: bool) -> DateLink {
    let date = days_to_date(day).to_string();
    // Only one repo can supply a commit for a date. With several in scope the date
    // would point at a different commit in each, so it stays plain text.
    if let [r] = repos {
        if let Some((c, approximate)) = boundary_commit(r, day, earliest) {
            return DateLink {
                date,
                sha: Some(c.sha.clone()),
                url: r.web.as_ref().map(|w| format!("{w}/commit/{}", c.sha)),
                approximate,
            };
        }
    }
    DateLink {
        date,
        sha: None,
        url: None,
        approximate: false,
    }
}

fn build_scope(repos: &[&RepoData], f: &Filter) -> ScopeRef {
    ScopeRef {
        repos: repos
            .iter()
            .map(|r| RepoLink {
                name: r.name.clone(),
                url: r.web.clone(),
            })
            .collect(),
        since: f.since.map(|d| date_link(repos, d, true)),
        until: f.until.map(|d| date_link(repos, d, false)),
        path: f.path.clone(),
    }
}

pub fn cell_text(s: &str) -> Cell {
    Cell::Text(s.to_string())
}
