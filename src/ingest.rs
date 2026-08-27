use crate::git;
use crate::lines;
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
///
/// Kept low deliberately: the diff pass asks for patch text, which runs about fifty
/// times the volume of a numstat, and every worker holds its whole chunk in memory
/// at once. Smaller shards trade a few milliseconds of process startup for a peak
/// that stays in the tens of megabytes.
const CHUNK: usize = 500;

/// One file's entry in a diff. `added` and `removed` come from the numstat block
/// and are -1 for binaries, which report `-` rather than a count. The comment and
/// blank counts are classified from the patch text for the same lines.
pub struct FileStat {
    pub added: i32,
    pub removed: i32,
    pub added_comment: i32,
    pub added_blank: i32,
    pub removed_comment: i32,
    pub removed_blank: i32,
    pub path: String,
}
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

/// Classify the changed lines of one file's patch.
///
/// The numstat block already gives authoritative totals, so this only has to split
/// them into comments and blanks. Both sides are walked at once: a hunk's context
/// lines belong to the old file *and* the new one, so feeding them through both
/// block-comment states keeps a `/* ... */` or a docstring recognised across the
/// lines that follow it. State still restarts at each hunk, since a hunk carries no
/// knowledge of the file above it — the counts are close, not exact, which is why
/// only comments and blanks are derived this way and the totals never are.
fn classify_patch(path: &str, body: &[&str]) -> (i32, i32, i32, i32) {
    let style = lines::style_of(path);
    let (mut ac, mut ab, mut rc, mut rb) = (0i32, 0i32, 0i32, 0i32);
    let (mut new_block, mut old_block): (lines::Block, lines::Block) = (None, None);
    let mut in_hunk = false;
    for l in body {
        if l.starts_with("@@") {
            in_hunk = true;
            new_block = None;
            old_block = None;
            continue;
        }
        if !in_hunk {
            continue;
        }
        // "\ No newline at end of file" is a note about the diff, not a line of it.
        if l.starts_with('\\') {
            continue;
        }
        let (mark, text) = l.split_at(l.chars().next().map(|c| c.len_utf8()).unwrap_or(0));
        match mark {
            "+" => match lines::classify(style, text, &mut new_block) {
                lines::Kind::Comment => ac += 1,
                lines::Kind::Blank => ab += 1,
                lines::Kind::Code => {}
            },
            "-" => match lines::classify(style, text, &mut old_block) {
                lines::Kind::Comment => rc += 1,
                lines::Kind::Blank => rb += 1,
                lines::Kind::Code => {}
            },
            // Context belongs to both sides; advance both, count neither.
            " " => {
                lines::classify(style, text, &mut new_block);
                lines::classify(style, text, &mut old_block);
            }
            _ => {}
        }
    }
    (ac, ab, rc, rb)
}

/// Parse one `--numstat -p` run.
///
/// The numstat rows and the patch sections describe the same files in the same
/// order, so they are matched by position. Nothing is read out of the `diff --git`
/// header: its shape depends on the repo's own diff config, while the ordering does
/// not. A section that fails to line up leaves the file with totals and no
/// breakdown, rather than a breakdown attributed to the wrong file.
fn parse_diff(out: &str) -> Vec<ShaFiles> {
    let mut result = Vec::new();
    for rec in out.split(REC) {
        if rec.trim().is_empty() {
            continue;
        }
        let all: Vec<&str> = rec.lines().collect();
        let sha = match all.first() {
            Some(s) => s.trim().to_string(),
            None => continue,
        };
        if sha.is_empty() {
            continue;
        }
        let split = all
            .iter()
            .position(|l| l.starts_with("diff --git"))
            .unwrap_or(all.len());

        let mut files = Vec::new();
        for l in &all[1..split] {
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
            files.push(FileStat {
                added,
                removed,
                added_comment: 0,
                added_blank: 0,
                removed_comment: 0,
                removed_blank: 0,
                path: normalize_path(p),
            });
        }

        let mut sections: Vec<&[&str]> = Vec::new();
        let mut start = None;
        for (i, l) in all.iter().enumerate().skip(split) {
            if l.starts_with("diff --git") {
                if let Some(s) = start {
                    sections.push(&all[s..i]);
                }
                start = Some(i);
            }
        }
        if let Some(s) = start {
            sections.push(&all[s..]);
        }

        if sections.len() == files.len() {
            for (f, sec) in files.iter_mut().zip(sections) {
                if f.added < 0 {
                    continue; // binary: nothing to classify
                }
                let (ac, ab, rc, rb) = classify_patch(&f.path, sec);
                f.added_comment = ac;
                f.added_blank = ab;
                f.removed_comment = rc;
                f.removed_blank = rb;
            }
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
                    // The patch is what makes a comment distinguishable from a line
                    // of code; numstat alone only ever gives a total.
                    "-p",
                    "--format=%x1e%H",
                ],
                chunk,
            )
            .map(|o| parse_diff(&o))
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
        for f in files {
            let path_id = interner.intern(&f.path);
            let dir_id = interner.intern(dir_of(&f.path));
            changes.push(Change {
                path: path_id,
                dir: dir_id,
                added: f.added,
                removed: f.removed,
                added_comment: f.added_comment,
                added_blank: f.added_blank,
                removed_comment: f.removed_comment,
                removed_blank: f.removed_blank,
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
    // The default branch, never the checkout. See git::default_ref.
    let branch = git::default_ref(repo_path);
    let head = git::sha_of(repo_path, &branch)?;

    let idx = cache.repos.iter().position(|r| r.path == path);
    let prev_head = idx.and_then(|i| {
        let r = &cache.repos[i];
        // A checkpoint only holds if it is still on the branch we are about to walk.
        // Mere existence is not enough: a commit left behind by a rebase, a
        // force-push or an ingest taken from another ref still resolves, and
        // appending `{that}..{head}` to the cache never removes what it left there.
        // Anything but a clean ancestor is re-read from scratch.
        if git::is_ancestor(repo_path, &r.head, &head) {
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
        // Pin both ends to shas. The branch can move while we work, and a range that
        // re-resolves mid-run would record a head we never actually walked.
        (Some(h), true) => format!("{h}..{head}"),
        _ => head.clone(),
    };

    if !quiet {
        eprintln!(
            "{name}: {} ingest ({branch} @ {})",
            if incremental { "incremental" } else { "full" },
            &head[..8.min(head.len())]
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
