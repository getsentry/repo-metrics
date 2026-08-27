use crate::lines::Lines;
use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Bump when the parser changes shape, or when the set of commits we ingest changes.
/// A mismatch re-ingests from scratch, which is seconds of work — so there is no
/// backfill problem to engineer around.
///
/// 3: read the default branch instead of HEAD. Caches written before this may hold
/// commits from whatever branch happened to be checked out at ingest time, and
/// nothing short of a re-read can tell those apart from real history.
/// 4: store the comment/blank split of every change, so line metrics can be asked
/// for everything, for source and comments, or for source alone. The counts are
/// only obtainable from diff content, which earlier caches never recorded.
pub const PARSER_VERSION: u32 = 4;

#[derive(Serialize, Deserialize, Default)]
pub struct Cache {
    pub version: u32,
    pub repos: Vec<RepoData>,
}

#[derive(Serialize, Deserialize)]
pub struct RepoData {
    pub name: String,
    pub path: String,
    pub head: String,
    /// Browsable base URL for the origin remote, so views can link a commit out to
    /// the forge. None when the remote is missing or an unfamiliar shape.
    pub web: Option<String>,
    /// Interned strings: paths, directories, author names and emails all repeat
    /// heavily across a history, so ids keep the cache small and comparisons cheap.
    pub strings: Vec<String>,
    pub commits: Vec<Commit>,
    pub changes: Vec<Change>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Commit {
    pub sha: String,
    /// Author date, not committer date. Rebases, squash-merges and cherry-picks all
    /// rewrite the committer date, which would collapse a week of work onto the day
    /// it happened to land.
    pub days: i32,
    pub ts: i64,
    pub author: u32,
    pub email: u32,
    pub is_merge: bool,
    /// Raw co-author identities, stored unclassified. Labels are derived on read so
    /// a newly-recognised agent is a config edit, not a re-ingest.
    pub coauthors: Vec<(u32, u32)>,
    pub change_start: u32,
    pub change_len: u32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Change {
    pub path: u32,
    pub dir: u32,
    /// -1 means binary: git reports `-` for both columns, which is a file that was
    /// touched with no meaningful line count, not a file to skip.
    pub added: i32,
    pub removed: i32,
    /// How the lines counted above split by kind. Only the comment and blank parts
    /// are stored; code is the remainder. Keeping the total authoritative and
    /// deriving code from it means the breakdown can never drift away from what git
    /// itself reported, however the classifier is later changed.
    pub added_comment: i32,
    pub added_blank: i32,
    pub removed_comment: i32,
    pub removed_blank: i32,
}

impl Change {
    pub fn added_of(&self, l: Lines) -> i64 {
        l.of(
            self.added.max(0) as i64,
            self.added_comment.max(0) as i64,
            self.added_blank.max(0) as i64,
        )
    }
    pub fn removed_of(&self, l: Lines) -> i64 {
        l.of(
            self.removed.max(0) as i64,
            self.removed_comment.max(0) as i64,
            self.removed_blank.max(0) as i64,
        )
    }
    pub fn churn_of(&self, l: Lines) -> i64 {
        self.added_of(l) + self.removed_of(l)
    }
    /// Lines rewritten rather than purely added or deleted. A diff records only
    /// additions and removals, so the overlap between them is the best available
    /// stand-in for an edit in place.
    pub fn modified_of(&self, l: Lines) -> i64 {
        self.added_of(l).min(self.removed_of(l))
    }
    /// Binary files are touched-but-uncounted, which callers need to distinguish
    /// from a real zero-line change.
    #[allow(dead_code)]
    pub fn is_binary(&self) -> bool {
        self.added < 0
    }
}

impl RepoData {
    pub fn s(&self, id: u32) -> &str {
        &self.strings[id as usize]
    }
    pub fn changes_of(&self, c: &Commit) -> &[Change] {
        let a = c.change_start as usize;
        let b = a + c.change_len as usize;
        &self.changes[a..b]
    }
}

pub struct Interner {
    map: HashMap<String, u32>,
    pub list: Vec<String>,
}

impl Interner {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            list: Vec::new(),
        }
    }
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.map.get(s) {
            return i;
        }
        let i = self.list.len() as u32;
        self.list.push(s.to_string());
        self.map.insert(s.to_string(), i);
        i
    }
}

pub fn days_to_date(days: i32) -> NaiveDate {
    NaiveDate::from_num_days_from_ce_opt(days).unwrap_or_default()
}

pub fn date_to_days(d: NaiveDate) -> i32 {
    d.num_days_from_ce()
}

pub fn cache_dir() -> PathBuf {
    if let Ok(x) = std::env::var("REPO_METRICS_CACHE") {
        return PathBuf::from(x);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".cache").join("repo-metrics")
}

pub fn cache_path() -> PathBuf {
    cache_dir().join("cache.bin")
}

pub fn load_cache() -> Result<Cache> {
    let p = cache_path();
    if !p.exists() {
        return Ok(Cache {
            version: PARSER_VERSION,
            repos: Vec::new(),
        });
    }
    let f = std::fs::File::open(&p).with_context(|| format!("opening {}", p.display()))?;
    let mut rd = std::io::BufReader::new(f);

    // Read the version on its own, before anything else is decoded.
    //
    // It has to happen in this order. The version is the first field, and bincode
    // lays fields out positionally, so a file written by an older parser decodes as
    // whatever the current structs happen to describe. Checking `c.version` after a
    // full deserialize is checking a field that was only reachable if the decode
    // already succeeded — and it does not: reading a stale layout makes bincode take
    // some unrelated bytes for a length and abort the process trying to allocate
    // exabytes. That is not an error this or any caller can catch.
    let mut head = [0u8; 4];
    if std::io::Read::read_exact(&mut rd, &mut head).is_err()
        || u32::from_le_bytes(head) != PARSER_VERSION
    {
        return Ok(Cache {
            version: PARSER_VERSION,
            repos: Vec::new(),
        });
    }
    std::io::Seek::seek(&mut rd, std::io::SeekFrom::Start(0))?;

    // A cache we can't read is a cache worth throwing away; re-ingest is cheap.
    Ok(bincode::deserialize_from(rd).unwrap_or_else(|_| Cache {
        version: PARSER_VERSION,
        repos: Vec::new(),
    }))
}

pub fn save_cache(c: &Cache) -> Result<()> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("cache.bin.tmp");
    {
        let f = std::fs::File::create(&tmp)?;
        let w = std::io::BufWriter::new(f);
        bincode::serialize_into(w, c)?;
    }
    std::fs::rename(&tmp, cache_path())?;
    Ok(())
}

/// Extension -> language. Anything unmapped becomes "other" rather than blocking
/// ingestion; the long tail of one-off extensions is not worth a lookup table entry.
pub fn language_of(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    if !path.contains('.') {
        return "other";
    }
    match ext {
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "py" | "pyi" => "Python",
        "rs" => "Rust",
        "go" => "Go",
        "rb" => "Ruby",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" => "C++",
        "cs" => "C#",
        "php" => "PHP",
        "scala" => "Scala",
        "sh" | "bash" | "zsh" => "Shell",
        "sql" => "SQL",
        "html" | "htm" => "HTML",
        "css" | "scss" | "sass" | "less" => "CSS",
        "json" => "JSON",
        "yml" | "yaml" => "YAML",
        "toml" => "TOML",
        "md" | "mdx" => "Markdown",
        "graphql" | "gql" => "GraphQL",
        "proto" => "Protobuf",
        "lock" => "Lockfile",
        "svg" | "png" | "jpg" | "jpeg" | "gif" | "ico" | "woff" | "woff2" | "ttf" => "Asset",
        _ => "other",
    }
}

pub fn dir_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

/// Trim a directory path to `depth` segments, for rolling files up to the level
/// someone actually wants to look at.
pub fn dir_at_depth(dir: &str, depth: usize) -> &str {
    if depth == 0 || dir.is_empty() {
        return dir;
    }
    let mut n = 0;
    for (i, ch) in dir.char_indices() {
        if ch == '/' {
            n += 1;
            if n == depth {
                return &dir[..i];
            }
        }
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `load_cache` reads the version out of the first four bytes without decoding
    /// anything after it. That only works while the version stays the leading field
    /// in the layout bincode writes — if it ever moves, a stale cache goes back to
    /// aborting the process instead of being discarded.
    #[test]
    fn version_is_the_first_four_bytes_of_a_cache() {
        let c = Cache {
            version: PARSER_VERSION,
            repos: Vec::new(),
        };
        let bytes = bincode::serialize(&c).expect("cache serialises");
        let head = u32::from_le_bytes(bytes[..4].try_into().expect("four bytes"));
        assert_eq!(head, PARSER_VERSION);
    }
}
