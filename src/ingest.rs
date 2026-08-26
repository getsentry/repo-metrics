use crate::git;
use crate::model::*;
use anyhow::Result;
use chrono::DateTime;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

const REC: char = '\x1e';
const FLD: char = '\x1f';
/// Enough shas per worker that process startup is noise, small enough that 14 cores
/// all stay busy to the end of a 109k-commit history.
const CHUNK: usize = 3000;

/// One file's entry in a numstat block: added, removed, path. Added and removed
/// are -1 for binaries, which report `-` rather than a count.
pub type FileStat = (i32, i32, String);
/// A commit's sha paired with the files it touched.
pub type ShaFiles = (String, Vec<FileStat>);

pub struct RawCommit {
    pub sha: String,
    pub days: i32,
    pub ts: i64,
    pub author: String,
    pub email: String,
    pub is_merge: bool,
    pub coauthors: Vec<(String, String)>,
}

/// `git log --numstat` renders renames inline, in two shapes:
///   `old/path.txt => new/path.txt`  and  `dir/{old => new}/file.txt`
/// Both should count against the path the file now has, or a rename shows up as a
/// directory that churns and then vanishes.
fn normalize_path(p: &str) -> String {
    if !p.contains(" => ") {
        return p.to_string();
    }
    if let (Some(open), Some(close)) = (p.find('{'), p.find('}')) {
        if open < close {
            let inner = &p[open + 1..close];
            let new = inner.split(" => ").nth(1).unwrap_or(inner);
            let joined = format!("{}{}{}", &p[..open], new, &p[close + 1..]);
            // `dir/{ => new}/f` collapses to a doubled slash once the braces go.
            return joined.replace("//", "/");
        }
    }
    p.split(" => ").nth(1).unwrap_or(p).to_string()
}

fn parse_meta(out: &str) -> Vec<RawCommit> {
    let mut commits = Vec::new();
    for rec in out.split(REC) {
        if rec.trim().is_empty() {
            continue;
        }
        let mut it = rec.splitn(6, FLD);
        let (sha, date, name, email, parents, body) = match (
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
        ) {
            (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) => (a, b, c, d, e, f),
            _ => continue,
        };
        let sha = sha.trim_start_matches('\n').trim();
        if sha.is_empty() {
            continue;
        }
        let dt = match DateTime::parse_from_rfc3339(date.trim()) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let mut coauthors = Vec::new();
        for line in body.lines() {
            let l = line.trim();
            // Compare as bytes: commit bodies are full of multibyte characters and
            // slicing a str by a fixed byte count will land mid-codepoint.
            const CO: &[u8] = b"co-authored-by:";
            let lb = l.as_bytes();
            if lb.len() <= CO.len() || !lb[..CO.len()].eq_ignore_ascii_case(CO) {
                continue;
            }
            let v = l[CO.len()..].trim();
            let (n, e) = match (v.find('<'), v.rfind('>')) {
                (Some(a), Some(b)) if a < b => (v[..a].trim(), &v[a + 1..b]),
                _ => (v, ""),
            };
            if !n.is_empty() || !e.is_empty() {
                coauthors.push((n.to_string(), e.to_string()));
            }
        }
        commits.push(RawCommit {
            sha: sha.to_string(),
            days: date_to_days(dt.date_naive()),
            ts: dt.timestamp(),
            author: name.to_string(),
            email: email.to_string(),
            is_merge: parents.split_whitespace().count() > 1,
            coauthors,
        });
    }
    commits
}

fn parse_numstat(out: &str) -> Vec<ShaFiles> {
    let mut result = Vec::new();
    for rec in out.split(REC) {
        if rec.trim().is_empty() {
            continue;
        }
        let mut lines = rec.lines();
        let sha = match lines.next() {
            Some(s) => s.trim().to_string(),
            None => continue,
        };
        if sha.is_empty() {
            continue;
        }
        let mut files = Vec::new();
        for l in lines {
            if l.is_empty() {
                continue;
            }
            let mut parts = l.splitn(3, '\t');
            let (a, r, p) = match (parts.next(), parts.next(), parts.next()) {
                (Some(a), Some(r), Some(p)) => (a, r, p),
                _ => continue,
            };
            // Binary files report `-` for both columns. Recorded as touched with
            // null line counts, not skipped.
            let added: i32 = if a == "-" { -1 } else { a.parse().unwrap_or(0) };
            let removed: i32 = if r == "-" { -1 } else { r.parse().unwrap_or(0) };
            files.push((added, removed, normalize_path(p)));
        }
        result.push((sha, files));
    }
    result
}

fn ingest_range(repo: &Path, range: &str, quiet: bool) -> Result<Vec<(RawCommit, Vec<FileStat>)>> {
    let shas = git::rev_list(repo, range)?;
    if shas.is_empty() {
        return Ok(Vec::new());
    }
    let t0 = Instant::now();

    // Pass one: metadata only. No diffing, so it runs over the whole history in
    // about a second even on sentry — cheap enough not to bother sharding.
    let meta_out = git::git_log_stdin(
        repo,
        &[
            "log",
            "--stdin",
            "--no-walk",
            "--format=%x1e%H%x1f%aI%x1f%an%x1f%ae%x1f%P%x1f%B",
        ],
        &shas,
    )?;
    let commits = parse_meta(&meta_out);
    drop(meta_out);
    if !quiet {
        eprintln!(
            "  metadata: {} commits in {:.2}s",
            commits.len(),
            t0.elapsed().as_secs_f64()
        );
    }

    // Pass two: the numstat diff, which is the expensive half. Sharded by explicit
    // sha list so every commit lands in exactly one chunk.
    let t1 = Instant::now();
    let chunks: Vec<&[String]> = shas.chunks(CHUNK).collect();
    let outs: Vec<Vec<ShaFiles>> = chunks
        .par_iter()
        .map(|chunk| {
            git::git_log_stdin(
                repo,
                &[
                    "log",
                    "--stdin",
                    "--no-walk",
                    "--numstat",
                    "--format=%x1e%H",
                ],
                chunk,
            )
            .map(|o| parse_numstat(&o))
            .unwrap_or_default()
        })
        .collect();

    let mut by_sha: HashMap<String, Vec<FileStat>> = HashMap::with_capacity(shas.len());
    let mut nfiles = 0usize;
    for o in outs {
        for (sha, files) in o {
            nfiles += files.len();
            by_sha.insert(sha, files);
        }
    }
    if !quiet {
        eprintln!(
            "  file changes: {} rows in {:.2}s ({} shards)",
            nfiles,
            t1.elapsed().as_secs_f64(),
            chunks.len()
        );
    }

    Ok(commits
        .into_iter()
        .map(|c| {
            let f = by_sha.remove(&c.sha).unwrap_or_default();
            (c, f)
        })
        .collect())
}

fn build_repo_data(
    name: String,
    path: String,
    head: String,
    web: Option<String>,
    mut existing: Option<RepoData>,
    fresh: Vec<(RawCommit, Vec<FileStat>)>,
) -> RepoData {
    let mut interner = Interner::new();
    let (mut commits, mut changes) = match existing.take() {
        Some(prev) => {
            for s in &prev.strings {
                interner.intern(s);
            }
            (prev.commits, prev.changes)
        }
        None => (Vec::new(), Vec::new()),
    };

    for (c, files) in fresh {
        let start = changes.len() as u32;
        for (added, removed, p) in files {
            let path_id = interner.intern(&p);
            let dir_id = interner.intern(dir_of(&p));
            changes.push(Change {
                path: path_id,
                dir: dir_id,
                added,
                removed,
            });
        }
        let author = interner.intern(&c.author);
        let email = interner.intern(&c.email);
        let coauthors = c
            .coauthors
            .iter()
            .map(|(n, e)| (interner.intern(n), interner.intern(e)))
            .collect();
        commits.push(Commit {
            sha: c.sha,
            days: c.days,
            ts: c.ts,
            author,
            email,
            is_merge: c.is_merge,
            coauthors,
            change_start: start,
            change_len: (changes.len() as u32) - start,
        });
    }

    commits.sort_by_key(|c| c.ts);
    RepoData {
        name,
        path,
        head,
        web,
        strings: interner.list,
        commits,
        changes,
    }
}

pub fn ingest(cache: &mut Cache, repo_path: &Path, force: bool, quiet: bool) -> Result<()> {
    let name = git::repo_name(repo_path);
    let path = repo_path.display().to_string();
    let head = git::head_sha(repo_path)?;

    let idx = cache.repos.iter().position(|r| r.path == path);
    let prev_head = idx.and_then(|i| {
        let r = &cache.repos[i];
        // A checkpoint that has been rebased out from under us is not a checkpoint.
        if git::git(
            repo_path,
            &["cat-file", "-e", &format!("{}^{{commit}}", r.head)],
        )
        .is_ok()
        {
            Some(r.head.clone())
        } else {
            None
        }
    });

    let incremental = !force && prev_head.is_some();
    let range = match (&prev_head, incremental) {
        (Some(h), true) if *h == head => {
            if !quiet {
                eprintln!("{name}: already current at {}", &head[..8.min(head.len())]);
            }
            return Ok(());
        }
        (Some(h), true) => format!("{h}..HEAD"),
        _ => "HEAD".to_string(),
    };

    if !quiet {
        eprintln!(
            "{name}: {} ingest ({range})",
            if incremental { "incremental" } else { "full" }
        );
    }

    let fresh = ingest_range(repo_path, &range, quiet)?;
    let existing = if incremental {
        idx.map(|i| cache.repos.remove(i))
    } else {
        if let Some(i) = idx {
            cache.repos.remove(i);
        }
        None
    };

    let web = git::web_url(repo_path);
    let rd = build_repo_data(name, path, head, web, existing, fresh);
    if !quiet {
        eprintln!(
            "  total: {} commits, {} file changes",
            rd.commits.len(),
            rd.changes.len()
        );
    }
    cache.repos.push(rd);
    Ok(())
}
