use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Run a git command and return stdout as a String.
pub fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn git {args:?}"))?;
    if !out.status.success() {
        bail!(
            "git {args:?} failed in {}: {}",
            repo.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run `git log` feeding revisions on stdin. This is how we shard: `rev-list` gives
/// us every sha, we chunk it, and each worker asks for exactly its chunk. Slicing by
/// date instead would silently drop commits whose author date sits outside the range.
pub fn git_log_stdin(repo: &Path, args: &[&str], revs: &[String]) -> Result<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn git log --stdin")?;
    {
        let mut stdin = child.stdin.take().context("no stdin on git log")?;
        let mut buf = String::with_capacity(revs.len() * 41);
        for r in revs {
            buf.push_str(r);
            buf.push('\n');
        }
        stdin.write_all(buf.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "git log --stdin failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn is_repo(path: &Path) -> bool {
    git(path, &["rev-parse", "--git-dir"]).is_ok()
}

/// The ref every metric is read from.
///
/// Deliberately not HEAD. A checkout is a working state — a feature branch, a
/// detached HEAD, a half-finished rebase — and ingesting it folds private commits
/// into charts that claim to describe the project. Reading the default branch makes
/// the numbers independent of whatever the working copy happens to be doing.
///
/// `origin/HEAD` is the authoritative answer where it exists. The rest of the ladder
/// covers clones made without it and repos that have no remote at all.
pub fn default_ref(repo: &Path) -> String {
    if let Ok(s) = git(
        repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        let s = s.trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    for r in [
        "refs/remotes/origin/main",
        "refs/remotes/origin/master",
        "refs/heads/main",
        "refs/heads/master",
    ] {
        if git(repo, &["rev-parse", "--verify", "--quiet", r]).is_ok() {
            return r.to_string();
        }
    }
    // Nothing conventional to find. A repo with no remote and no main/master still
    // has to chart something, so fall back to the checkout.
    "HEAD".to_string()
}

/// Resolve a ref to the commit it names.
pub fn sha_of(repo: &Path, r: &str) -> Result<String> {
    Ok(git(repo, &["rev-parse", &format!("{r}^{{commit}}")])?
        .trim()
        .to_string())
}

/// The commit the default branch points at.
pub fn default_sha(repo: &Path) -> Result<String> {
    sha_of(repo, &default_ref(repo))
}

/// Whether `a` is reachable from `b`. A cached checkpoint that fails this test
/// describes history that has since been rewritten, and every commit ingested on
/// top of it is now unverifiable.
pub fn is_ancestor(repo: &Path, a: &str, b: &str) -> bool {
    git(repo, &["merge-base", "--is-ancestor", a, b]).is_ok()
}

/// Prefer the origin remote's path (`getsentry/sentry`) so charts are labelled the
/// way people talk about the repo, and fall back to the directory name.
pub fn repo_name(repo: &Path) -> String {
    if let Ok(url) = git(repo, &["config", "--get", "remote.origin.url"]) {
        let url = url.trim().trim_end_matches(".git");
        if !url.is_empty() {
            let tail = url.rsplit(':').next().unwrap_or(url);
            let parts: Vec<&str> = tail.trim_matches('/').split('/').collect();
            if parts.len() >= 2 {
                return parts[parts.len() - 2..].join("/");
            }
        }
    }
    repo.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.display().to_string())
}

/// Turns an origin remote into a browsable base URL. Handles the scp-style
/// `git@host:owner/repo.git` and the https form; anything unrecognised yields None
/// rather than a guess that 404s.
pub fn web_url(repo: &Path) -> Option<String> {
    let raw = git(repo, &["config", "--get", "remote.origin.url"]).ok()?;
    let raw = raw.trim().trim_end_matches('/').trim_end_matches(".git");
    if raw.is_empty() {
        return None;
    }
    if let Some(rest) = raw.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return Some(format!("https://{host}/{}", path.trim_start_matches('/')));
    }
    for pre in ["ssh://git@", "ssh://", "git://", "https://", "http://"] {
        if let Some(rest) = raw.strip_prefix(pre) {
            let rest = rest.strip_prefix("git@").unwrap_or(rest);
            return Some(format!("https://{rest}"));
        }
    }
    None
}

pub fn rev_list(repo: &Path, range: &str) -> Result<Vec<String>> {
    let out = git(repo, &["rev-list", range])?;
    Ok(out.lines().map(|s| s.to_string()).collect())
}

/// Newest commit at or before `date`, for point-in-time tree snapshots. Walks back
/// from `from` — the branch the cache was built from, not the checkout.
pub fn rev_before(repo: &Path, date: &str, from: &str) -> Result<Option<String>> {
    let before = format!("--before={date}");
    let out = git(repo, &["rev-list", "-1", &before, from])?;
    let s = out.trim();
    Ok(if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    })
}

pub struct TreeEntry {
    pub path: String,
    pub size: u64,
}

/// `ls-tree -r --long` at a commit. The `--long` column is the blob size in BYTES,
/// not lines: there is no line count anywhere in git's tree data, and getting true
/// SLOC would mean reading every blob. Bytes track line count closely enough to
/// size a treemap wedge, and they come free with the listing.
pub fn ls_tree(repo: &Path, sha: &str) -> Result<Vec<TreeEntry>> {
    let out = git(repo, &["ls-tree", "-r", "--long", sha])?;
    let mut entries = Vec::new();
    for line in out.lines() {
        // <mode> <type> <sha> <size>\t<path>
        let (meta, path) = match line.split_once('\t') {
            Some(x) => x,
            None => continue,
        };
        let size = meta
            .split_whitespace()
            .nth(3)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        entries.push(TreeEntry {
            path: path.to_string(),
            size,
        });
    }
    Ok(entries)
}

/// Lines per file at a commit, split into code, comment and blank.
///
/// Bytes are the wrong measure for a code repo — translation catalogues and
/// vendored assets dominate a byte-weighted tree while contributing no code.
///
/// This streams every blob in the tree and classifies it, rather than asking
/// `git grep -c` for a line count. Grep can only answer "how many lines", which is
/// what made the old `sloc` measure a plain `wc -l` wearing a better name. Reading
/// a whole file also means block comments are tracked exactly, unlike the diff side
/// where only a hunk is visible. On sentry's 20k-file tree the whole pass is under
/// a second, so the honesty is close to free.
pub fn line_counts(repo: &Path, sha: &str) -> Result<HashMap<String, (u64, u64, u64)>> {
    let listing = git(repo, &["ls-tree", "-r", sha])?;
    // <mode> <type> <sha>\t<path>
    let mut blobs: Vec<(String, String)> = Vec::new();
    for line in listing.lines() {
        let (meta, path) = match line.split_once('\t') {
            Some(x) => x,
            None => continue,
        };
        let mut it = meta.split_whitespace();
        let (_mode, kind, oid) = match (it.next(), it.next(), it.next()) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => continue,
        };
        if kind != "blob" {
            continue;
        }
        blobs.push((oid.to_string(), path.to_string()));
    }
    if blobs.is_empty() {
        return Ok(HashMap::new());
    }

    // One `cat-file --batch` for the whole tree: a process per file would cost far
    // more than the reading does.
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn git cat-file")?;

    // The request list runs to hundreds of kilobytes, far past a pipe buffer, so it
    // has to be written from its own thread. Writing it inline deadlocks: we block
    // filling git's stdin while git blocks filling a stdout nobody is draining yet.
    let mut stdin = child.stdin.take().context("no stdin on git cat-file")?;
    let oids: Vec<String> = blobs.iter().map(|(o, _)| o.clone()).collect();
    let writer = std::thread::spawn(move || {
        let mut buf = String::with_capacity(oids.len() * 41);
        for o in &oids {
            buf.push_str(o);
            buf.push('\n');
        }
        let _ = stdin.write_all(buf.as_bytes());
        // Dropping stdin closes it, which is what ends the batch.
    });

    let mut rd = BufReader::new(child.stdout.take().context("no stdout on git cat-file")?);
    let mut map = HashMap::with_capacity(blobs.len());
    let mut body: Vec<u8> = Vec::new();
    for (_, path) in &blobs {
        // "<oid> <type> <size>\n", then <size> bytes, then a newline. A missing
        // object answers "<oid> missing" with no body, so the two lists stay in step.
        let mut header = String::new();
        if rd.read_line(&mut header)? == 0 {
            break;
        }
        let size: usize = match header
            .trim_end()
            .rsplit(' ')
            .next()
            .and_then(|s| s.parse().ok())
        {
            Some(n) => n,
            None => continue,
        };
        body.clear();
        body.resize(size, 0);
        if rd.read_exact(&mut body).is_err() {
            break;
        }
        let mut nl = [0u8; 1];
        let _ = rd.read_exact(&mut nl);
        // A NUL byte is how git itself decides a blob is binary; such a file has no
        // line count worth reporting, so it is left out rather than counted as one.
        if !body.contains(&0) {
            let text = String::from_utf8_lossy(&body);
            map.insert(path.clone(), crate::lines::count_file(path, &text));
        }
    }
    let _ = writer.join();
    let _ = child.wait();
    Ok(map)
}

pub fn canonical(path: &str) -> Result<PathBuf> {
    std::fs::canonicalize(path).with_context(|| format!("no such path: {path}"))
}
