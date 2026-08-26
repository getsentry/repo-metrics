use crate::github::{self, Repo, Sort};
use crate::picker::{self, ago, Item};
use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// Directories people conventionally keep checkouts in, best guess first.
const ROOT_GUESSES: &[&str] = &["code", "src", "dev", "Projects", "projects", "repos", "work"];

pub struct Opts {
    pub org: Option<String>,
    pub dir: Option<String>,
    pub sort: Sort,
    pub limit: usize,
    pub jobs: usize,
    pub all: bool,
    pub yes: bool,
    pub refresh: bool,
    pub ssh: Option<bool>,
    pub filter: Option<String>,
    pub include_archived: bool,
    pub include_forks: bool,
    pub dry_run: bool,
    pub ingest: bool,
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if s == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(s)
}

fn display_path(p: &Path) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if let Ok(rest) = p.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    p.display().to_string()
}

/// Picks where checkouts should live: an explicit `--dir`, else the first
/// conventional directory that already exists, else the working directory. The
/// guess is offered rather than imposed unless the caller opted out of prompts.
fn resolve_root(opts: &Opts) -> Result<PathBuf> {
    if let Some(d) = &opts.dir {
        let p = expand_tilde(d);
        ensure_dir(&p, opts.yes || opts.all, opts.dry_run)?;
        return Ok(p);
    }

    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let mut guess = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(h) = &home {
        for g in ROOT_GUESSES {
            let c = h.join(g);
            if c.is_dir() {
                guess = c;
                break;
            }
        }
    }

    if opts.yes || opts.all || !picker::is_interactive() {
        return Ok(guess);
    }
    let answer = picker::prompt("Where should the repos live?", &display_path(&guess))?;
    let p = expand_tilde(&answer);
    ensure_dir(&p, false, opts.dry_run)?;
    Ok(p)
}

fn ensure_dir(p: &Path, assume_yes: bool, dry_run: bool) -> Result<()> {
    if p.is_dir() {
        return Ok(());
    }
    // A dry run reports the plan and changes nothing — creating the destination
    // would already be a side effect.
    if dry_run {
        return Ok(());
    }
    if p.exists() {
        bail!("{} exists but is not a directory", p.display());
    }
    let ok = assume_yes
        || !picker::is_interactive()
        || picker::confirm(&format!("{} does not exist — create it?", display_path(p)), true)?;
    if !ok {
        bail!("no directory to clone into");
    }
    std::fs::create_dir_all(p).with_context(|| format!("creating {}", p.display()))?;
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("failed to run git")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[derive(Clone, Copy, PartialEq)]
enum Local {
    Missing,
    Present,
    /// Path is taken by something that is not a git checkout of this repo.
    Conflict,
}

fn local_state(root: &Path, r: &Repo) -> Local {
    let p = root.join(&r.name);
    if !p.exists() {
        return Local::Missing;
    }
    if !p.join(".git").exists() {
        return Local::Conflict;
    }
    Local::Present
}

pub enum Outcome {
    Cloned(f64),
    UpToDate,
    FastForwarded(u64),
    HeldBack(String),
    Failed(String),
}

impl Outcome {
    /// Did anything actually move? Used to decide whether a re-ingest is worth it.
    pub fn changed(&self) -> bool {
        matches!(self, Outcome::Cloned(_) | Outcome::FastForwarded(_))
    }
}

impl Outcome {
    pub fn glyph(&self) -> &'static str {
        match self {
            Outcome::Cloned(_) => "+",
            Outcome::UpToDate => "=",
            Outcome::FastForwarded(_) => "↑",
            Outcome::HeldBack(_) => "!",
            Outcome::Failed(_) => "x",
        }
    }
    pub fn text(&self) -> String {
        match self {
            Outcome::Cloned(s) => format!("cloned in {s:.1}s"),
            Outcome::UpToDate => "up to date".into(),
            Outcome::FastForwarded(n) => format!("fast-forwarded {n} commit{}", if *n == 1 { "" } else { "s" }),
            Outcome::HeldBack(why) => why.clone(),
            Outcome::Failed(e) => format!("failed: {e}"),
        }
    }
}

fn clone_one(root: &Path, r: &Repo, ssh: bool) -> Outcome {
    let t = Instant::now();
    let dest = root.join(&r.name);
    // Full history: --numstat diffs blob contents, so a blobless or shallow clone
    // would refetch during ingest and be far slower than cloning properly once.
    let out = Command::new("git")
        .arg("clone")
        .arg("--quiet")
        .arg(r.url_for(ssh))
        .arg(&dest)
        .output();
    match out {
        Ok(o) if o.status.success() => Outcome::Cloned(t.elapsed().as_secs_f64()),
        Ok(o) => Outcome::Failed(
            String::from_utf8_lossy(&o.stderr)
                .lines()
                .last()
                .unwrap_or("clone failed")
                .trim()
                .to_string(),
        ),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

fn update_one(root: &Path, r: &Repo) -> Outcome {
    update_checkout(&root.join(&r.name))
}

/// Fetches, then fast-forwards only when it is unambiguously safe: a clean tree, a
/// tracking branch, and no local commits. Anything else is reported and left alone —
/// nothing here should ever be able to lose someone's work.
pub fn update_checkout(dir: &Path) -> Outcome {
    let dir = dir.to_path_buf();
    if let Err(e) = git(&dir, &["fetch", "--prune", "--quiet"]) {
        return Outcome::Failed(e.to_string().lines().last().unwrap_or("fetch failed").into());
    }
    let upstream = match git(&dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]) {
        Ok(u) => u,
        Err(_) => return Outcome::HeldBack("fetched; no upstream branch".into()),
    };
    let count = |range: &str| -> u64 {
        git(&dir, &["rev-list", "--count", range])
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    };
    let behind = count(&format!("HEAD..{upstream}"));
    let ahead = count(&format!("{upstream}..HEAD"));

    if behind == 0 && ahead == 0 {
        return Outcome::UpToDate;
    }
    if ahead > 0 {
        return Outcome::HeldBack(format!(
            "fetched; {ahead} local commit{} not pushed{}",
            if ahead == 1 { "" } else { "s" },
            if behind > 0 { format!(", {behind} behind") } else { String::new() }
        ));
    }
    let dirty = git(&dir, &["status", "--porcelain"]).map(|s| !s.is_empty()).unwrap_or(true);
    if dirty {
        return Outcome::HeldBack(format!("fetched; {behind} behind, working tree dirty"));
    }
    match git(&dir, &["merge", "--ff-only", "--quiet", &upstream]) {
        Ok(_) => Outcome::FastForwarded(behind),
        Err(e) => Outcome::HeldBack(format!(
            "fetched; {behind} behind, {}",
            e.to_string().lines().last().unwrap_or("could not fast-forward")
        )),
    }
}

fn human_kb(kb: u64) -> String {
    let b = kb as f64 * 1024.0;
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", U[i])
}

pub fn run(opts: Opts) -> Result<()> {
    github::check_auth()?;

    let org = match &opts.org {
        Some(o) => o.clone(),
        None => {
            if !picker::is_interactive() {
                bail!("which GitHub org? pass it as an argument, e.g. `repo-metrics sync getsentry`");
            }
            let a = picker::prompt("Which GitHub org or user?", "getsentry")?;
            if a.trim().is_empty() {
                bail!("no org given");
            }
            a.trim().to_string()
        }
    };

    let root = resolve_root(&opts)?;

    eprint!("Listing repositories in {org}… ");
    let listing = github::list_repos(&org, opts.limit, opts.refresh)?;
    let mut repos = listing.repos;
    eprintln!(
        "{} found{}{}",
        repos.len(),
        if listing.complete { "" } else { " (truncated — use --limit 0 for all)" },
        if listing.from_cache { " [cached]" } else { "" }
    );

    // Archived repos and forks are noise for most people and are excluded unless
    // explicitly asked for.
    let before = repos.len();
    repos.retain(|r| (opts.include_archived || !r.archived) && (opts.include_forks || !r.fork));
    if let Some(f) = &opts.filter {
        let f = f.to_lowercase();
        repos.retain(|r| r.name.to_lowercase().contains(&f) || r.full_name.to_lowercase().contains(&f));
    }
    let hidden = before - repos.len();
    if repos.is_empty() {
        bail!("no repositories left after filtering ({before} before filters)");
    }
    github::sort_repos(&mut repos, opts.sort);

    let states: Vec<Local> = repos.iter().map(|r| local_state(&root, r)).collect();
    let ssh = opts.ssh.unwrap_or_else(github::prefers_ssh);

    // Selection. Already-cloned repos start checked, so the default action on a
    // populated directory is "keep what I have current".
    let chosen: Vec<usize> = if opts.all {
        (0..repos.len()).collect()
    } else if !picker::is_interactive() {
        bail!("not a terminal — pass --all, or run this from an interactive shell");
    } else {
        let mut items: Vec<Item> = repos
            .iter()
            .zip(states.iter())
            .map(|(r, st)| Item {
                label: r.full_name.clone(),
                meta: format!("{:>9}  {}", human_kb(r.size), ago(r.days_since_push())),
                note: match st {
                    Local::Present => Some("cloned".into()),
                    Local::Conflict => Some("path taken".into()),
                    Local::Missing => None,
                },
                selected: *st == Local::Present,
            })
            .collect();
        let title = format!("{org} → {}", display_path(&root));
        let hint = format!(
            "{} repos, {} first{}. Cloned ones are pre-selected so they get updated.",
            repos.len(),
            match opts.sort {
                Sort::Recent => "most recently pushed",
                Sort::Active => "biggest and still moving",
                Sort::Stars => "most starred",
                Sort::Size => "largest",
                Sort::Name => "alphabetical",
            },
            if hidden > 0 { format!(" · {hidden} archived/forks hidden") } else { String::new() }
        );
        match picker::pick(&title, &hint, &mut items)? {
            None => {
                eprintln!("cancelled");
                return Ok(());
            }
            Some(sel) => sel,
        }
    };

    // A destination that exists but is not a checkout is never overwritten. Say so
    // rather than dropping it silently — a selected repo that just vanishes from the
    // run is indistinguishable from a bug.
    let blocked: Vec<usize> = chosen.iter().copied().filter(|&i| states[i] == Local::Conflict).collect();
    let work: Vec<usize> = chosen.into_iter().filter(|&i| states[i] != Local::Conflict).collect();
    if !blocked.is_empty() {
        eprintln!();
        eprintln!("Skipping {} — the path exists but is not a git checkout:", blocked.len());
        for &i in &blocked {
            eprintln!("  ! {}  ({})", repos[i].full_name, display_path(&root.join(&repos[i].name)));
        }
    }
    if work.is_empty() {
        eprintln!("nothing to do");
        return Ok(());
    }

    let to_clone: Vec<usize> = work.iter().copied().filter(|&i| states[i] == Local::Missing).collect();
    let to_update: Vec<usize> = work.iter().copied().filter(|&i| states[i] == Local::Present).collect();
    let download_kb: u64 = to_clone.iter().map(|&i| repos[i].size).sum();

    eprintln!();
    eprintln!(
        "{} to clone (~{} to download), {} to update, into {}",
        to_clone.len(),
        human_kb(download_kb),
        to_update.len(),
        display_path(&root)
    );
    if opts.dry_run {
        for &i in &to_clone {
            println!("clone   {}  ({})", repos[i].full_name, human_kb(repos[i].size));
        }
        for &i in &to_update {
            println!("update  {}", repos[i].full_name);
        }
        return Ok(());
    }
    if !opts.yes && !opts.all && picker::is_interactive() && !picker::confirm("Proceed?", true)? {
        eprintln!("cancelled");
        return Ok(());
    }
    eprintln!();

    let done = AtomicUsize::new(0);
    let total = work.len();
    let printer = Mutex::new(());
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.jobs.clamp(1, 32))
        .build()
        .context("could not build clone pool")?;

    let started = Instant::now();
    let results: Vec<(usize, Outcome)> = pool.install(|| {
        work.par_iter()
            .map(|&i| {
                let r = &repos[i];
                let outcome = if states[i] == Local::Missing {
                    clone_one(&root, r, ssh)
                } else {
                    update_one(&root, r)
                };
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                {
                    let _lock = printer.lock().unwrap();
                    eprintln!(
                        "  {} {:<44} {:<40} [{n}/{total}]",
                        outcome.glyph(),
                        r.full_name,
                        outcome.text()
                    );
                }
                (i, outcome)
            })
            .collect()
    });

    let mut cloned = 0;
    let mut ffwd = 0;
    let mut current = 0;
    let mut held = 0;
    let mut failed: Vec<(String, String)> = Vec::new();
    for (i, o) in &results {
        match o {
            Outcome::Cloned(_) => cloned += 1,
            Outcome::FastForwarded(_) => ffwd += 1,
            Outcome::UpToDate => current += 1,
            Outcome::HeldBack(_) => held += 1,
            Outcome::Failed(e) => failed.push((repos[*i].full_name.clone(), e.clone())),
        }
    }

    eprintln!();
    eprintln!(
        "{cloned} cloned · {ffwd} updated · {current} already current · {held} left alone · {} failed  ({:.1}s)",
        failed.len(),
        started.elapsed().as_secs_f64()
    );
    if !failed.is_empty() {
        eprintln!();
        for (name, e) in &failed {
            eprintln!("  x {name}: {e}");
        }
    }
    if held > 0 {
        eprintln!("\n  \"left alone\" means fetched but not merged — local commits or a dirty tree.");
    }

    if opts.ingest {
        let paths: Vec<PathBuf> = work
            .iter()
            .map(|&i| root.join(&repos[i].name))
            .filter(|p| p.join(".git").exists())
            .collect();
        eprintln!("\nIngesting {} repos into the metrics cache…", paths.len());
        let mut cache = crate::model::load_cache()?;
        for p in &paths {
            if let Err(e) = crate::ingest::ingest(&mut cache, p, false, true) {
                eprintln!("  ingest failed for {}: {e}", display_path(p));
            }
        }
        crate::model::save_cache(&cache)?;
        eprintln!("done — try `repo-metrics hotspots --since 90d`");
    }

    Ok(())
}
