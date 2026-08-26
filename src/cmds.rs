use crate::git;
use crate::identity::{AssistKind, Identities};
use crate::model::*;
use crate::output::*;
use crate::query::*;
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::Path;

// ------------------------------------------------------------- commits over time

pub fn timeseries(
    cache: &Cache,
    ids: &Identities,
    f: &Filter,
    b: Bucket,
    m: Metric,
    split: Split,
    top: usize,
) -> Output {
    let repos = select_repos(cache, f);
    let mut buckets: HashMap<(String, i32), f64> = HashMap::new();
    let mut keys: Vec<i32> = Vec::new();
    let mut totals: HashMap<String, f64> = HashMap::new();

    for r in &repos {
        for res in resolve(r, ids, f, false) {
            let (k, _) = bucket_key(res.commit.days, b);
            keys.push(k);
            for (name, v) in split_values(&res, split, m, &f.path) {
                if v == 0.0 {
                    continue;
                }
                *buckets.entry((name.clone(), k)).or_insert(0.0) += v;
                *totals.entry(name).or_insert(0.0) += v;
            }
        }
    }

    let ax = axis(&keys, b);
    let names = rank_names(&totals, top);
    let series = build_series(&buckets, &ax, &names);
    Output::Series {
        title: format!("Commits over time — {}", m.label()),
        subtitle: range_label(f, &repos),
        source: None,
        x: ax.iter().map(|(_, l)| l.clone()).collect(),
        series,
        stacked: split != Split::None,
        y_label: m.label().to_string(),
    }
}

fn split_values(r: &Resolved, split: Split, m: Metric, path: &Option<String>) -> Vec<(String, f64)> {
    match split {
        Split::None => vec![("all".to_string(), metric_value(r, m, path))],
        Split::Assist => vec![(r.assist.label().to_string(), metric_value(r, m, path))],
        Split::Author => vec![(r.author().to_string(), metric_value(r, m, path))],
        Split::Tool => {
            let v = metric_value(r, m, path);
            if r.tools.is_empty() {
                vec![(
                    if r.assist == AssistKind::Bot { "bot" } else { "none" }.to_string(),
                    v,
                )]
            } else {
                // Split the commit's weight across its tools rather than counting it
                // once per tool, so stacked bands still sum to the real total.
                let share = v / r.tools.len() as f64;
                r.tools.iter().map(|t| (t.clone(), share)).collect()
            }
        }
        Split::Language => {
            // Languages are a property of the files, not the commit, so this one
            // attributes per change instead of per commit.
            let mut acc: HashMap<String, f64> = HashMap::new();
            for c in r.repo.changes_of(r.commit) {
                let p = r.repo.s(c.path);
                if !path_matches(r.repo.s(c.dir), p, path) {
                    continue;
                }
                let v = match m {
                    Metric::Commits | Metric::Files => 1.0,
                    Metric::Churn => c.churn() as f64,
                    Metric::Added => c.added.max(0) as f64,
                    Metric::Removed => c.removed.max(0) as f64,
                };
                *acc.entry(language_of(p).to_string()).or_insert(0.0) += v;
            }
            acc.into_iter().collect()
        }
    }
}

fn rank_names(totals: &HashMap<String, f64>, top: usize) -> Vec<String> {
    let mut v: Vec<(&String, &f64)> = totals.iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    v.into_iter().take(top.max(1)).map(|(k, _)| k.clone()).collect()
}

// --------------------------------------------------------- commits by folder

pub fn folders(
    cache: &Cache,
    ids: &Identities,
    f: &Filter,
    b: Bucket,
    m: Metric,
    depth: usize,
    top: usize,
) -> Output {
    let repos = select_repos(cache, f);
    let mut buckets: HashMap<(String, i32), f64> = HashMap::new();
    let mut totals: HashMap<String, f64> = HashMap::new();
    let mut keys: Vec<i32> = Vec::new();

    for r in &repos {
        for res in resolve(r, ids, f, false) {
            let (k, _) = bucket_key(res.commit.days, b);
            keys.push(k);
            // A commit touching three files in one folder counts once for the
            // commits metric — otherwise "commits by folder" silently becomes
            // "file touches by folder".
            let mut seen: HashMap<String, f64> = HashMap::new();
            for c in r.changes_of(res.commit) {
                let dir = r.s(c.dir);
                let p = r.s(c.path);
                if !path_matches(dir, p, &f.path) {
                    continue;
                }
                let key = {
                    let d = dir_at_depth(dir, depth);
                    if d.is_empty() { "(root)" } else { d }
                }
                .to_string();
                let v = match m {
                    Metric::Commits => 1.0,
                    Metric::Files => 1.0,
                    Metric::Churn => c.churn() as f64,
                    Metric::Added => c.added.max(0) as f64,
                    Metric::Removed => c.removed.max(0) as f64,
                };
                if m == Metric::Commits {
                    seen.insert(key, 1.0);
                } else {
                    *seen.entry(key).or_insert(0.0) += v;
                }
            }
            for (key, v) in seen {
                *buckets.entry((key.clone(), k)).or_insert(0.0) += v;
                *totals.entry(key).or_insert(0.0) += v;
            }
        }
    }

    let ax = axis(&keys, b);
    let names = rank_names(&totals, top);
    let series = build_series(&buckets, &ax, &names);
    Output::Series {
        title: format!("Folders over time — {} (depth {depth})", m.label()),
        subtitle: range_label(f, &repos),
        source: None,
        x: ax.iter().map(|(_, l)| l.clone()).collect(),
        series,
        stacked: true,
        y_label: m.label().to_string(),
    }
}

// ------------------------------------------------------- fastest-moving parts

struct Roll {
    commits: f64,
    added: f64,
    removed: f64,
    files: f64,
    authors: std::collections::HashSet<u32>,
}

impl Default for Roll {
    fn default() -> Self {
        Self {
            commits: 0.0,
            added: 0.0,
            removed: 0.0,
            files: 0.0,
            authors: Default::default(),
        }
    }
}

fn rollup_dirs(
    cache: &Cache,
    ids: &Identities,
    f: &Filter,
    depth: usize,
) -> Vec<(String, Roll)> {
    let repos = select_repos(cache, f);
    let mut acc: HashMap<String, Roll> = HashMap::new();
    for r in &repos {
        for res in resolve(r, ids, f, false) {
            let mut touched: HashMap<String, (f64, f64, f64)> = HashMap::new();
            for c in r.changes_of(res.commit) {
                let dir = r.s(c.dir);
                let p = r.s(c.path);
                if !path_matches(dir, p, &f.path) {
                    continue;
                }
                let key = {
                    let d = dir_at_depth(dir, depth);
                    if d.is_empty() { "(root)" } else { d }
                }
                .to_string();
                let e = touched.entry(key).or_insert((0.0, 0.0, 0.0));
                e.0 += c.added.max(0) as f64;
                e.1 += c.removed.max(0) as f64;
                e.2 += 1.0;
            }
            for (key, (a, rm, fl)) in touched {
                let e = acc.entry(key).or_default();
                e.commits += 1.0;
                e.added += a;
                e.removed += rm;
                e.files += fl;
                e.authors.insert(res.commit.author);
            }
        }
    }
    let mut v: Vec<(String, Roll)> = acc.into_iter().collect();
    v.sort_by(|a, b| {
        (b.1.added + b.1.removed)
            .partial_cmp(&(a.1.added + a.1.removed))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    v
}

pub fn hotspots(
    cache: &Cache,
    ids: &Identities,
    f: &Filter,
    depth: usize,
    top: usize,
) -> Output {
    let repos = select_repos(cache, f);
    let rolled = rollup_dirs(cache, ids, f, depth);
    let rows: Vec<Vec<Cell>> = rolled
        .iter()
        .take(top)
        .map(|(dir, r)| {
            vec![
                cell_text(dir),
                Cell::Int((r.added + r.removed) as i64),
                Cell::Int(r.added as i64),
                Cell::Int(r.removed as i64),
                Cell::Int(r.commits as i64),
                Cell::Int(r.authors.len() as i64),
            ]
        })
        .collect();
    Output::Table {
        title: format!("Fastest-moving parts (depth {depth})"),
        subtitle: range_label(f, &repos),
        source: None,
        columns: vec![
            "directory".into(),
            "churn".into(),
            "added".into(),
            "removed".into(),
            "commits".into(),
            "authors".into(),
        ],
        bar_column: Some(1),
        rows,
    }
}

// ------------------------------------------------------------ timeframe compare

pub fn compare(
    cache: &Cache,
    ids: &Identities,
    base: &Filter,
    a: &str,
    b: &str,
    depth: usize,
    top: usize,
) -> Result<Output> {
    let (a0, a1, alab) = parse_period(a)?;
    let (b0, b1, blab) = parse_period(b)?;
    let fa = Filter { since: Some(a0), until: Some(a1), ..base.clone() };
    let fb = Filter { since: Some(b0), until: Some(b1), ..base.clone() };

    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let push = |label: String, x: f64, y: f64, rows: &mut Vec<Vec<Cell>>| {
        let d = y - x;
        let pct = if x > 0.0 { (d / x) * 100.0 } else { 0.0 };
        rows.push(vec![
            cell_text(&label),
            Cell::Int(x as i64),
            Cell::Int(y as i64),
            Cell::Int(d as i64),
            Cell::Num((pct * 10.0).round() / 10.0),
        ]);
    };

    let sa = summarize(cache, ids, &fa);
    let sb = summarize(cache, ids, &fb);
    push("commits".into(), sa.0, sb.0, &mut rows);
    push("lines added".into(), sa.1, sb.1, &mut rows);
    push("lines removed".into(), sa.2, sb.2, &mut rows);
    push("files touched".into(), sa.3, sb.3, &mut rows);
    push("distinct authors".into(), sa.4, sb.4, &mut rows);
    push("agent-assisted commits".into(), sa.5, sb.5, &mut rows);

    // Then the folders that moved most between the two windows.
    let ra: HashMap<String, f64> = rollup_dirs(cache, ids, &fa, depth)
        .into_iter()
        .map(|(k, v)| (k, v.added + v.removed))
        .collect();
    let rb: HashMap<String, f64> = rollup_dirs(cache, ids, &fb, depth)
        .into_iter()
        .map(|(k, v)| (k, v.added + v.removed))
        .collect();
    let mut all: Vec<String> = ra.keys().chain(rb.keys()).cloned().collect();
    all.sort();
    all.dedup();
    let mut deltas: Vec<(String, f64, f64)> = all
        .into_iter()
        .map(|k| {
            let x = *ra.get(&k).unwrap_or(&0.0);
            let y = *rb.get(&k).unwrap_or(&0.0);
            (k, x, y)
        })
        .collect();
    deltas.sort_by(|p, q| {
        (q.2 - q.1)
            .abs()
            .partial_cmp(&(p.2 - p.1).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (k, x, y) in deltas.into_iter().take(top) {
        push(format!("  {k}"), x, y, &mut rows);
    }

    let repos = select_repos(cache, base);
    Ok(Output::Table {
        title: format!("Compare {alab} → {blab}"),
        subtitle: format!(
            "{} · churn by directory below the summary",
            range_label(&Filter { since: None, until: None, ..base.clone() }, &repos)
        ),
        source: None,
        columns: vec![
            "measure".into(),
            alab,
            blab,
            "Δ".into(),
            "Δ%".into(),
        ],
        bar_column: None,
        rows,
    })
}

fn summarize(cache: &Cache, ids: &Identities, f: &Filter) -> (f64, f64, f64, f64, f64, f64) {
    let repos = select_repos(cache, f);
    let (mut commits, mut added, mut removed, mut files, mut agent) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let mut authors = std::collections::HashSet::new();
    for r in &repos {
        for res in resolve(r, ids, f, false) {
            let mut hit = f.path.is_none();
            for c in r.changes_of(res.commit) {
                if !path_matches(r.s(c.dir), r.s(c.path), &f.path) {
                    continue;
                }
                hit = true;
                added += c.added.max(0) as f64;
                removed += c.removed.max(0) as f64;
                files += 1.0;
            }
            if hit {
                commits += 1.0;
                authors.insert((r.name.clone(), res.commit.author));
                if res.assist == AssistKind::AgentAssisted {
                    agent += 1.0;
                }
            }
        }
    }
    (commits, added, removed, files, authors.len() as f64, agent)
}

// -------------------------------------------------------- anomalous periods

pub fn flags(
    cache: &Cache,
    ids: &Identities,
    f: &Filter,
    depth: usize,
    z_min: f64,
    min_churn: f64,
    window: usize,
    min_baseline: usize,
    top: usize,
) -> Output {
    let repos = select_repos(cache, f);
    // (repo, dir) -> week -> churn
    let mut acc: HashMap<(String, String), HashMap<i32, f64>> = HashMap::new();
    for r in &repos {
        for res in resolve(r, ids, f, false) {
            let (wk, _) = bucket_key(res.commit.days, Bucket::Week);
            for c in r.changes_of(res.commit) {
                let dir = r.s(c.dir);
                let p = r.s(c.path);
                if !path_matches(dir, p, &f.path) {
                    continue;
                }
                let key = {
                    let d = dir_at_depth(dir, depth);
                    if d.is_empty() { "(root)" } else { d }
                }
                .to_string();
                *acc.entry((r.name.clone(), key))
                    .or_default()
                    .entry(wk)
                    .or_insert(0.0) += c.churn() as f64;
            }
        }
    }

    let mut hits: Vec<(f64, String, String, String, f64, f64)> = Vec::new();
    for ((repo, dir), weeks) in acc {
        let mut ws: Vec<(i32, f64)> = weeks.into_iter().collect();
        ws.sort_by_key(|(k, _)| *k);
        for i in 0..ws.len() {
            // Trailing window, strictly preceding — a week is never part of its own
            // baseline.
            let lo = i.saturating_sub(window);
            let base: Vec<f64> = ws[lo..i].iter().map(|(_, v)| *v).collect();
            // Fewer than min_baseline preceding weeks means the standard deviation
            // is meaningless, and every folder would light up in the weeks after it
            // is created.
            if base.len() < min_baseline {
                continue;
            }
            let v = ws[i].1;
            // Churn is heavily right-skewed: a folder averaging 3 lines a week is
            // four sigma out at 12 lines, which is true and uninteresting.
            if v < min_churn {
                continue;
            }
            let n = base.len() as f64;
            let mean = base.iter().sum::<f64>() / n;
            let var = base.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
            let sd = var.sqrt();
            if sd <= 0.0 {
                continue;
            }
            let z = (v - mean) / sd;
            if z > z_min {
                hits.push((
                    z,
                    repo.clone(),
                    dir.clone(),
                    days_to_date(ws[i].0).to_string(),
                    v,
                    mean,
                ));
            }
        }
    }
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let rows: Vec<Vec<Cell>> = hits
        .iter()
        .take(top)
        .map(|(z, repo, dir, week, v, mean)| {
            vec![
                cell_text(week),
                cell_text(repo),
                cell_text(dir),
                Cell::Int(*v as i64),
                Cell::Int(*mean as i64),
                Cell::Num((z * 10.0).round() / 10.0),
            ]
        })
        .collect();

    Output::Table {
        title: "Auto-detected interesting periods".into(),
        subtitle: format!(
            "{} · z > {z_min}, churn ≥ {}, {window}-week trailing baseline (min {min_baseline})",
            range_label(f, &repos),
            min_churn as i64
        ),
        source: None,
        columns: vec![
            "week".into(),
            "repo".into(),
            "directory".into(),
            "churn".into(),
            "baseline".into(),
            "z".into(),
        ],
        bar_column: Some(5),
        rows,
    }
}

// ------------------------------------------------------------------- authors

pub fn authors(cache: &Cache, ids: &Identities, f: &Filter, top: usize) -> Output {
    let repos = select_repos(cache, f);
    let mut acc: HashMap<String, (f64, f64, f64, AssistKind, Vec<String>)> = HashMap::new();
    for r in &repos {
        for res in resolve(r, ids, f, false) {
            let mut added = 0.0;
            let mut removed = 0.0;
            let mut hit = f.path.is_none();
            for c in r.changes_of(res.commit) {
                if !path_matches(r.s(c.dir), r.s(c.path), &f.path) {
                    continue;
                }
                hit = true;
                added += c.added.max(0) as f64;
                removed += c.removed.max(0) as f64;
            }
            if !hit {
                continue;
            }
            let e = acc
                .entry(res.author().to_string())
                .or_insert((0.0, 0.0, 0.0, res.assist, Vec::new()));
            e.0 += 1.0;
            e.1 += added;
            e.2 += removed;
            for t in &res.tools {
                if !e.4.contains(t) {
                    e.4.push(t.clone());
                }
            }
        }
    }
    let mut v: Vec<(String, (f64, f64, f64, AssistKind, Vec<String>))> = acc.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.partial_cmp(&a.1 .0).unwrap_or(std::cmp::Ordering::Equal));

    let rows: Vec<Vec<Cell>> = v
        .iter()
        .take(top)
        .map(|(name, (c, a, r, kind, tools))| {
            vec![
                cell_text(name),
                cell_text(kind.label()),
                cell_text(&if tools.is_empty() { "—".to_string() } else { tools.join(",") }),
                Cell::Int(*c as i64),
                Cell::Int(*a as i64),
                Cell::Int(*r as i64),
            ]
        })
        .collect();

    Output::Table {
        title: "Authors".into(),
        subtitle: range_label(f, &repos),
        source: None,
        columns: vec![
            "author".into(),
            "kind".into(),
            "tools".into(),
            "commits".into(),
            "added".into(),
            "removed".into(),
        ],
        bar_column: Some(3),
        rows,
    }
}

/// Share of commits by human / agent_assisted / bot over time.
pub fn assist_mix(cache: &Cache, ids: &Identities, f: &Filter, b: Bucket) -> Output {
    let repos = select_repos(cache, f);
    let mut buckets: HashMap<(String, i32), f64> = HashMap::new();
    let mut keys = Vec::new();
    for r in &repos {
        for res in resolve(r, ids, f, false) {
            let (k, _) = bucket_key(res.commit.days, b);
            keys.push(k);
            *buckets.entry((res.assist.label().to_string(), k)).or_insert(0.0) += 1.0;
        }
    }
    let ax = axis(&keys, b);
    let names: Vec<String> = AssistKind::all().iter().map(|a| a.label().to_string()).collect();
    let series = build_series(&buckets, &ax, &names);
    Output::Series {
        title: "Human vs agent-assisted vs bot".into(),
        subtitle: range_label(f, &repos),
        source: None,
        x: ax.iter().map(|(_, l)| l.clone()).collect(),
        series,
        stacked: true,
        y_label: "commits".into(),
    }
}

// --------------------------------------------------------------- tree / radial

fn resolve_repo<'a>(cache: &'a Cache, f: &Filter) -> Result<&'a RepoData> {
    let repos = select_repos(cache, f);
    match repos.len() {
        0 => bail!("no ingested repo matches; run `repo-metrics ingest <path>` first"),
        1 => Ok(repos[0]),
        _ => bail!(
            "tree views need one repo; got {}. Narrow with --repo",
            repos.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Builds a directory tree from a point-in-time listing. Snapshots are taken on
/// demand rather than stored: `ls-tree` is ~0.1s even on a 20k-file repo, so a
/// snapshot table would be the largest thing in the system in exchange for very
/// little.
pub fn build_tree(entries: &[git::TreeEntry], root_path: &str, max_depth: usize) -> TreeNode {
    let root_path = root_path.trim_end_matches('/');
    let mut root = TreeNode {
        name: if root_path.is_empty() { "/".into() } else { root_path.to_string() },
        size: 0,
        files: 0,
        children: Vec::new(),
    };
    // index -> children, keyed by relative path segment
    let mut index: HashMap<String, TreeNode> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for e in entries {
        let rel = if root_path.is_empty() {
            e.path.as_str()
        } else if e.path.starts_with(&format!("{root_path}/")) {
            &e.path[root_path.len() + 1..]
        } else {
            continue;
        };
        root.size += e.size;
        root.files += 1;
        let seg: Vec<&str> = rel.split('/').collect();
        let key = if seg.len() > 1 {
            seg[0].to_string()
        } else {
            format!("\u{1}{}", seg[0]) // leading marker keeps files sorted apart from dirs
        };
        let node = index.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            TreeNode {
                name: seg[0].to_string(),
                size: 0,
                files: 0,
                children: Vec::new(),
            }
        });
        node.size += e.size;
        node.files += 1;
    }

    let mut children: Vec<TreeNode> = order
        .into_iter()
        .filter_map(|k| index.remove(&k))
        .collect();
    children.sort_by(|a, b| b.size.cmp(&a.size));

    if max_depth > 1 {
        for c in children.iter_mut() {
            if c.files > 1 {
                let sub = if root_path.is_empty() {
                    c.name.clone()
                } else {
                    format!("{root_path}/{}", c.name)
                };
                let built = build_tree(entries, &sub, max_depth - 1);
                if !built.children.is_empty() {
                    c.children = built.children;
                }
            }
        }
    }
    root.children = children;
    root
}

pub fn tree(
    cache: &Cache,
    f: &Filter,
    date: Option<&str>,
    path: &str,
    depth: usize,
) -> Result<Output> {
    let rd = resolve_repo(cache, f)?;
    let repo_path = Path::new(&rd.path);
    let (sha, when) = match date {
        Some(d) => match git::rev_before(repo_path, d)? {
            Some(s) => (s, d.to_string()),
            None => bail!("no commit at or before {d} in {}", rd.name),
        },
        None => (rd.head.clone(), "HEAD".to_string()),
    };
    let entries = git::ls_tree(repo_path, &sha)?;
    let root = build_tree(&entries, path, depth);
    Ok(Output::Tree {
        title: format!("{} — {} @ {}", rd.name, if path.is_empty() { "/" } else { path }, when),
        subtitle: format!("{} files · {}", group(root.files as i64), human_bytes(root.size)),
        // The snapshot commit, which is not HEAD whenever --at is used.
        source: Some(SourceRef::new(&rd.name, &sha, &rd.web)),
        root,
    })
}
