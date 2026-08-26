use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

const PER_PAGE: usize = 100;
/// Polite ceiling on concurrent API calls. The rate limit is 5,000/hour, so this is
/// about not hammering the endpoint rather than about staying under quota.
const API_JOBS: usize = 6;
const CACHE_TTL_SECS: i64 = 3600;

#[derive(Clone, Serialize, Deserialize)]
pub struct Repo {
    pub full_name: String,
    pub name: String,
    pub clone_url: String,
    pub ssh_url: String,
    #[serde(default)]
    pub default_branch: String,
    #[serde(default)]
    pub pushed_at: Option<String>,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub stargazers_count: u64,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub fork: bool,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub description: Option<String>,
}

impl Repo {
    pub fn pushed_ts(&self) -> i64 {
        self.pushed_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp())
            .unwrap_or(0)
    }

    pub fn days_since_push(&self) -> f64 {
        let ts = self.pushed_ts();
        if ts == 0 {
            return 99_999.0;
        }
        ((Utc::now().timestamp() - ts) as f64 / 86_400.0).max(0.0)
    }

    /// Rough "how much repo is there, and is it still moving" score.
    ///
    /// GitHub gives no commit count on the org listing — the only per-repo endpoint
    /// that does costs one request each, which is 1,200+ requests for an org this
    /// size. Size on disk is the available proxy for accumulated history, decayed by
    /// how long since anyone pushed.
    pub fn activity_score(&self) -> f64 {
        let bulk = ((self.size as f64) + 10.0).log10();
        let decay = 1.0 / (1.0 + self.days_since_push() / 90.0);
        bulk * decay
    }

    pub fn url_for(&self, ssh: bool) -> &str {
        if ssh {
            &self.ssh_url
        } else {
            &self.clone_url
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Debug)]
pub enum Sort {
    /// Most recently pushed first
    Recent,
    /// Size on disk decayed by time since last push — big and still moving first
    Active,
    /// Most stars first
    Stars,
    /// Largest first
    Size,
    /// Alphabetical
    Name,
}

pub fn sort_repos(repos: &mut [Repo], by: Sort) {
    match by {
        Sort::Recent => repos.sort_by_key(|r| std::cmp::Reverse(r.pushed_ts())),
        Sort::Active => repos.sort_by(|a, b| {
            b.activity_score()
                .partial_cmp(&a.activity_score())
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        Sort::Stars => repos.sort_by_key(|r| std::cmp::Reverse(r.stargazers_count)),
        Sort::Size => repos.sort_by_key(|r| std::cmp::Reverse(r.size)),
        Sort::Name => repos.sort_by_key(|a| a.name.to_lowercase()),
    }
}

#[derive(Serialize, Deserialize)]
struct CachedListing {
    fetched_at: i64,
    complete: bool,
    repos: Vec<Repo>,
}

fn cache_file(org: &str) -> PathBuf {
    // Org names are restricted to alphanumerics and hyphens, so this cannot escape
    // the directory — but sanitise anyway rather than trust the argument.
    let safe: String = org
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    crate::model::cache_dir()
        .join("orgs")
        .join(format!("{safe}.json"))
}

fn gh(args: &[String]) -> Result<String> {
    let out = Command::new("gh")
        .args(args)
        .output()
        .context("could not run `gh` — install the GitHub CLI and run `gh auth login`")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("gh {}: {}", args.join(" "), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn check_auth() -> Result<()> {
    let out = Command::new("gh")
        .args(["auth", "status"])
        .output()
        .context("could not run `gh` — install the GitHub CLI (https://cli.github.com)")?;
    if !out.status.success() {
        bail!("gh is not authenticated — run `gh auth login` first");
    }
    Ok(())
}

/// Whether the user's gh is configured for ssh or https git operations, so clones
/// match whatever credential setup they already have working.
pub fn prefers_ssh() -> bool {
    Command::new("gh")
        .args(["config", "get", "git_protocol"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "ssh")
        .unwrap_or(false)
}

/// Total repos the owner has, so pages can be fetched in parallel instead of
/// walking `--paginate` one request at a time.
fn owner_repo_count(org: &str) -> Option<(usize, bool)> {
    if let Ok(s) = gh(&[
        "api".into(),
        format!("orgs/{org}"),
        "--jq".into(),
        "[(.public_repos // 0), (.total_private_repos // 0)] | add".into(),
    ]) {
        if let Ok(n) = s.trim().parse::<usize>() {
            return Some((n, true));
        }
    }
    if let Ok(s) = gh(&[
        "api".into(),
        format!("users/{org}"),
        "--jq".into(),
        "(.public_repos // 0)".into(),
    ]) {
        if let Ok(n) = s.trim().parse::<usize>() {
            return Some((n, false));
        }
    }
    None
}

fn fetch_page(org: &str, is_org: bool, page: usize) -> Result<Vec<Repo>> {
    let kind = if is_org { "orgs" } else { "users" };
    // sort=pushed matters even though we re-sort locally: it means a truncated fetch
    // still returns the repos most likely to be wanted.
    let path = format!("{kind}/{org}/repos?per_page={PER_PAGE}&sort=pushed&type=all&page={page}");
    let body = gh(&["api".into(), path])?;
    let repos: Vec<Repo> = serde_json::from_str(&body)
        .with_context(|| format!("unexpected response for {org} page {page}"))?;
    Ok(repos)
}

pub struct Listing {
    pub repos: Vec<Repo>,
    pub complete: bool,
    pub from_cache: bool,
}

/// Fetches an owner's repositories, newest-push first.
///
/// `limit` caps how many are pulled; because the API sorts server-side by push
/// date, a capped fetch still yields the active repos rather than an arbitrary
/// slice. Pass 0 for everything.
pub fn list_repos(org: &str, limit: usize, refresh: bool) -> Result<Listing> {
    let cf = cache_file(org);
    if !refresh {
        if let Ok(text) = std::fs::read_to_string(&cf) {
            if let Ok(c) = serde_json::from_str::<CachedListing>(&text) {
                let age = Utc::now().timestamp() - c.fetched_at;
                let enough = c.complete || limit == 0 || c.repos.len() >= limit;
                if age < CACHE_TTL_SECS && enough {
                    return Ok(Listing {
                        repos: c.repos,
                        complete: c.complete,
                        from_cache: true,
                    });
                }
            }
        }
    }

    let (total, is_org) =
        owner_repo_count(org).with_context(|| format!("no such GitHub org or user: {org}"))?;

    // The count endpoints undercount slightly against `type=all`; ask for a couple
    // of extra pages and stop when one comes back short.
    let want = if limit == 0 {
        total + 2 * PER_PAGE
    } else {
        limit.min(total + 2 * PER_PAGE)
    };
    let pages = want.div_ceil(PER_PAGE).max(1);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(API_JOBS)
        .build()
        .context("could not build API thread pool")?;
    let results: Vec<Result<Vec<Repo>>> = pool.install(|| {
        (1..=pages)
            .into_par_iter()
            .map(|p| fetch_page(org, is_org, p))
            .collect()
    });

    let mut repos = Vec::new();
    let mut short_page = false;
    let mut first_err = None;
    for r in results {
        match r {
            Ok(page) => {
                if page.len() < PER_PAGE {
                    short_page = true;
                }
                repos.extend(page);
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    if repos.is_empty() {
        if let Some(e) = first_err {
            return Err(e);
        }
        bail!("{org} has no repositories visible to this account");
    }

    repos.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    repos.dedup_by(|a, b| a.full_name == b.full_name);
    sort_repos(&mut repos, Sort::Recent);

    // Complete only if we asked for everything and saw the end of the list.
    let complete = limit == 0 && short_page;
    if limit != 0 && repos.len() > limit {
        repos.truncate(limit);
    }

    if let Some(dir) = cf.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let payload = CachedListing {
        fetched_at: Utc::now().timestamp(),
        complete,
        repos: repos.clone(),
    };
    if let Ok(j) = serde_json::to_string(&payload) {
        std::fs::write(&cf, j).ok();
    }

    Ok(Listing {
        repos,
        complete,
        from_cache: false,
    })
}
