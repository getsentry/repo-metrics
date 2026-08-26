use anyhow::{bail, Context, Result};
use std::io::Write;
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

pub fn head_sha(repo: &Path) -> Result<String> {
    Ok(git(repo, &["rev-parse", "HEAD"])?.trim().to_string())
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

/// Newest commit at or before `date`, for point-in-time tree snapshots.
pub fn rev_before(repo: &Path, date: &str) -> Result<Option<String>> {
    let before = format!("--before={date}");
    let out = git(repo, &["rev-list", "-1", &before, "HEAD"])?;
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

/// Lines per file at a commit, via `git grep -c`. Git does the counting and the
/// binary detection itself and parallelises internally, which makes this far
/// cheaper than streaming every blob out and counting here.
///
/// Bytes are the wrong measure for a code repo — translation catalogues and
/// vendored assets dominate a byte-weighted tree while contributing no code.
pub fn sloc(repo: &Path, sha: &str) -> Result<std::collections::HashMap<String, u64>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        // -I skips binary files; ^ matches every line.
        .args(["grep", "-c", "-I", "-E", "^", sha])
        .output()
        .context("failed to run git grep")?;
    // Exit code 1 just means nothing matched, which is a legitimately empty tree.
    if !out.status.success() && out.status.code() != Some(1) {
        bail!(
            "git grep failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let prefix = format!("{sha}:");
    let mut map = std::collections::HashMap::new();
    for line in text.lines() {
        let rest = match line.strip_prefix(&prefix) {
            Some(r) => r,
            None => continue,
        };
        // Split from the right: a path may contain a colon, the count cannot.
        if let Some((path, n)) = rest.rsplit_once(':') {
            if let Ok(n) = n.parse::<u64>() {
                map.insert(path.to_string(), n);
            }
        }
    }
    Ok(map)
}

pub fn canonical(path: &str) -> Result<PathBuf> {
    std::fs::canonicalize(path).with_context(|| format!("no such path: {path}"))
}
