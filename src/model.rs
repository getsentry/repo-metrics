use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Bump when the parser changes shape. A mismatch re-ingests from scratch, which is
/// seconds of work — so there is no backfill problem to engineer around.
pub const PARSER_VERSION: u32 = 2;

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
}

impl Change {
    pub fn churn(&self) -> i64 {
        self.added.max(0) as i64 + self.removed.max(0) as i64
    }
    /// Lines rewritten rather than purely added or deleted. A diff records only
    /// additions and removals, so the overlap between them is the best available
    /// stand-in for an edit in place.
    pub fn modified(&self) -> i64 {
        (self.added.max(0) as i64).min(self.removed.max(0) as i64)
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
    let rd = std::io::BufReader::new(f);
    let c: Cache = match bincode::deserialize_from(rd) {
        Ok(c) => c,
        // A cache we can't read is a cache worth throwing away; re-ingest is cheap.
        Err(_) => Cache {
            version: PARSER_VERSION,
            repos: Vec::new(),
        },
    };
    if c.version != PARSER_VERSION {
        return Ok(Cache {
            version: PARSER_VERSION,
            repos: Vec::new(),
        });
    }
    Ok(c)
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
