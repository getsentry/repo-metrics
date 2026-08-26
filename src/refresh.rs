use crate::model::{self, cache_dir};
use crate::proc::pid_alive;
use crate::sync::{update_checkout, Outcome};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Above this the log is trimmed. launchd opens the file in append mode, so
/// truncating from under it is safe — writes still land at the end.
const LOG_MAX_BYTES: u64 = 2 * 1024 * 1024;
const LOG_KEEP_LINES: usize = 400;

pub struct Opts {
    pub dir: Option<String>,
    pub jobs: usize,
    pub quiet: bool,
    pub no_fetch: bool,
}

fn lock_path() -> PathBuf {
    cache_dir().join("refresh.lock")
}

pub fn log_path() -> PathBuf {
    cache_dir().join("refresh.log")
}

/// Stops a slow run from overlapping the next scheduled one. A stale lock left by a
/// killed process is reclaimed rather than blocking forever.
struct Lock(PathBuf);

impl Lock {
    fn acquire() -> Result<Option<Self>> {
        let p = lock_path();
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Ok(pid) = s.trim().parse::<i32>() {
                if pid_alive(pid) {
                    return Ok(None);
                }
            }
        }
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).ok();
        }
        std::fs::write(&p, std::process::id().to_string())?;
        Ok(Some(Lock(p)))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

pub fn trim_log() {
    let p = log_path();
    let big = std::fs::metadata(&p)
        .map(|m| m.len() > LOG_MAX_BYTES)
        .unwrap_or(false);
    if !big {
        return;
    }
    if let Ok(text) = std::fs::read_to_string(&p) {
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(LOG_KEEP_LINES);
        let kept = lines[start..].join("\n");
        std::fs::write(&p, format!("{kept}\n")).ok();
    }
}

/// Every git checkout one level below `root`. One level is deliberate: it matches how
/// `sync` lays repos out, and avoids walking into node_modules or vendored trees.
fn scan_dir(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && p.join(".git").exists() {
                found.push(p);
            }
        }
    }
    found
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(s)
}

pub fn run(opts: Opts) -> Result<()> {
    trim_log();
    let _lock = match Lock::acquire()? {
        Some(l) => l,
        None => {
            if !opts.quiet {
                eprintln!("another refresh is already running");
            }
            return Ok(());
        }
    };

    let started = Instant::now();
    let mut cache = model::load_cache()?;

    // Everything already ingested, plus anything new sitting in the watched folder —
    // so repos cloned since the last run get picked up without reconfiguring.
    let mut targets: Vec<PathBuf> = cache.repos.iter().map(|r| PathBuf::from(&r.path)).collect();
    if let Some(d) = &opts.dir {
        for p in scan_dir(&expand_tilde(d)) {
            targets.push(p);
        }
    }
    targets.retain(|p| p.join(".git").exists());
    targets.sort();
    targets.dedup();

    if targets.is_empty() {
        if !opts.quiet {
            eprintln!("nothing to refresh — ingest a repo or pass --dir");
        }
        return Ok(());
    }

    let printer = Mutex::new(());
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.jobs.clamp(1, 32))
        .build()
        .context("could not build refresh pool")?;

    let outcomes: Vec<(PathBuf, Outcome)> = if opts.no_fetch {
        targets
            .iter()
            .map(|p| (p.clone(), Outcome::UpToDate))
            .collect()
    } else {
        pool.install(|| {
            targets
                .par_iter()
                .map(|p| {
                    let o = update_checkout(p);
                    if !opts.quiet {
                        let _l = printer.lock().unwrap();
                        eprintln!("  {} {:<40} {}", o.glyph(), short(p), o.text());
                    }
                    (p.clone(), o)
                })
                .collect()
        })
    };

    // Ingest is incremental and returns immediately when HEAD has not moved, so this
    // walks everything rather than only what fetched — it also picks up repos that
    // are new to the cache.
    let mut ingested = 0;
    let mut failures = Vec::new();
    for (p, _) in &outcomes {
        match crate::ingest::ingest(&mut cache, p, false, true) {
            Ok(()) => ingested += 1,
            Err(e) => failures.push(format!("{}: {e}", short(p))),
        }
    }
    model::save_cache(&cache)?;

    let updated = outcomes.iter().filter(|(_, o)| o.changed()).count();
    let failed = outcomes
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::Failed(_)))
        .count();

    // One line per run even in quiet mode: this is what the scheduled job leaves in
    // the log, and a silent job is one nobody can debug.
    println!(
        "[{}] refreshed {} repos · {updated} updated · {ingested} ingested · {failed} failed · {:.1}s",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        outcomes.len(),
        started.elapsed().as_secs_f64()
    );
    for f in &failures {
        println!("    ingest failed — {f}");
    }
    for (p, o) in &outcomes {
        if let Outcome::Failed(e) = o {
            println!("    fetch failed — {}: {e}", short(p));
        }
    }
    Ok(())
}

fn short(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}
