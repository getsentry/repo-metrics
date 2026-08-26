mod cmds;
mod git;
mod github;
mod html;
mod identity;
mod ingest;
mod model;
mod output;
mod picker;
mod proc;
mod query;
mod refresh;
mod schedule;
mod server;
mod sync;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use identity::Identities;
use model::*;
use output::*;
use query::*;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "repo-metrics",
    about = "Repository metrics and visualizations, straight from the git repo",
    long_about = "Reads git history directly — no server, no warehouse, no credentials.\n\
                  Ingest once into a local cache, then every view is a millisecond query.",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct Scope {
    /// Limit to repos whose name or path contains this string
    #[arg(long, global = true)]
    repo: Option<String>,
    /// Start of the window: YYYY-MM-DD, 90d, 12w, 6m, 2y, or a year
    #[arg(long, global = true)]
    since: Option<String>,
    /// End of the window
    #[arg(long, global = true)]
    until: Option<String>,
    /// Restrict to a path prefix, e.g. src/sentry/api
    #[arg(long, global = true)]
    path: Option<String>,
    /// How to render: table (terminal), json, or a self-contained html page
    #[arg(long, short, global = true, value_enum, default_value = "table")]
    format: Format,
    /// Write html output here instead of stdout
    #[arg(long, short, global = true)]
    out: Option<PathBuf>,
}

impl Scope {
    fn filter(&self) -> Result<Filter> {
        Ok(Filter {
            repo: self.repo.clone(),
            since: self.since.as_deref().map(parse_date).transpose()?,
            until: self.until.as_deref().map(parse_date).transpose()?,
            path: self.path.clone(),
        })
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Parse git history into the local cache (incremental after the first run)
    Ingest {
        /// Repo paths; defaults to the current directory
        paths: Vec<String>,
        /// Re-parse from scratch instead of only new commits
        #[arg(long)]
        force: bool,
    },
    /// List what is in the cache
    Repos,
    /// Commits over time
    Timeseries {
        #[arg(long, value_enum, default_value = "week")]
        by: Bucket,
        #[arg(long, value_enum, default_value = "commits")]
        metric: Metric,
        #[arg(long, value_enum, default_value = "none")]
        split: Split,
        /// Second line on its own axis: distinct humans active per bucket
        #[arg(long, value_enum, default_value = "authors")]
        overlay: Overlay,
        #[arg(long, default_value = "8")]
        top: usize,
        #[command(flatten)]
        scope: Scope,
    },
    /// Commits or churn by folder over time
    Folders {
        #[arg(long, value_enum, default_value = "week")]
        by: Bucket,
        #[arg(long, value_enum, default_value = "commits")]
        metric: Metric,
        /// Directory depth to roll up to (1 = top-level folders)
        #[arg(long, default_value = "1")]
        depth: usize,
        #[arg(long, default_value = "8")]
        top: usize,
        #[command(flatten)]
        scope: Scope,
    },
    /// Fastest-moving parts of the codebase, ranked by churn
    Hotspots {
        #[arg(long, default_value = "2")]
        depth: usize,
        #[arg(long, default_value = "20")]
        top: usize,
        #[command(flatten)]
        scope: Scope,
    },
    /// Two timeframes side by side, as a delta
    Compare {
        /// First period: 2025-H1, 2025-Q3, 2024, or start:end
        a: String,
        /// Second period
        b: String,
        #[arg(long, default_value = "1")]
        depth: usize,
        #[arg(long, default_value = "12")]
        top: usize,
        #[command(flatten)]
        scope: Scope,
    },
    /// Periods where a folder broke out of its own trailing baseline
    Flags {
        #[arg(long, default_value = "1")]
        depth: usize,
        /// Z-score threshold
        #[arg(long, default_value = "2.5")]
        z: f64,
        /// Ignore weeks quieter than this, however many sigma out they land
        #[arg(long = "min-churn", default_value = "200")]
        min_churn: f64,
        /// Trailing baseline length, in weeks
        #[arg(long, default_value = "12")]
        window: usize,
        /// Minimum preceding weeks before a folder can be flagged at all
        #[arg(long = "min-baseline", default_value = "8")]
        min_baseline: usize,
        #[arg(long, default_value = "30")]
        top: usize,
        #[command(flatten)]
        scope: Scope,
    },
    /// Folder sizes at a point in time, as a tree (terminal) or sunburst (html)
    #[command(alias = "radial")]
    Tree {
        /// Snapshot date; defaults to HEAD
        #[arg(long)]
        at: Option<String>,
        /// Subtree to show
        #[arg(long, default_value = "")]
        subpath: String,
        #[arg(long, default_value = "1")]
        depth: usize,
        /// What folder size means: files, sloc, or bytes
        #[arg(long, value_enum, default_value = "files")]
        measure: Measure,
        #[command(flatten)]
        scope: Scope,
    },
    /// Who is committing, and whether an agent helped
    Authors {
        #[arg(long, default_value = "25")]
        top: usize,
        #[command(flatten)]
        scope: Scope,
    },
    /// Share of commits by human / agent_assisted / bot over time
    Assist {
        #[arg(long, value_enum, default_value = "month")]
        by: Bucket,
        #[command(flatten)]
        scope: Scope,
    },
    /// Clone an org's repos and keep the ones you have up to date
    Sync {
        /// GitHub org or user; prompted for if omitted
        org: Option<String>,
        /// Where checkouts live; guessed from ~/code and friends if omitted
        #[arg(long)]
        dir: Option<String>,
        /// Ordering in the picker — alphabetical is rarely what you want
        #[arg(long, value_enum, default_value = "recent")]
        sort: github::Sort,
        /// How many repos to list; 0 for every one (slower on large orgs)
        #[arg(long, default_value = "200")]
        limit: usize,
        /// Concurrent clones
        #[arg(long, short = 'j', default_value = "8")]
        jobs: usize,
        /// Take every listed repo without showing the picker
        #[arg(long)]
        all: bool,
        /// Assume yes for prompts
        #[arg(long, short = 'y')]
        yes: bool,
        /// Ignore the cached org listing
        #[arg(long)]
        refresh: bool,
        /// Clone over ssh (defaults to whatever `gh config get git_protocol` says)
        #[arg(long)]
        ssh: bool,
        /// Clone over https
        #[arg(long, conflicts_with = "ssh")]
        https: bool,
        /// Only list repos whose name contains this
        #[arg(long)]
        filter: Option<String>,
        /// Include archived repositories
        #[arg(long = "archived")]
        include_archived: bool,
        /// Include forks
        #[arg(long = "forks")]
        include_forks: bool,
        /// Print what would happen and stop
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Ingest everything synced into the metrics cache afterwards
        #[arg(long)]
        ingest: bool,
    },
    /// Fetch every known repo and fold new commits into the cache
    Refresh {
        /// Also pick up any git checkouts directly inside this folder
        #[arg(long)]
        dir: Option<String>,
        /// Concurrent fetches
        #[arg(long, short = 'j', default_value = "8")]
        jobs: usize,
        /// Only print the one-line summary
        #[arg(long, short = 'q')]
        quiet: bool,
        /// Re-ingest without fetching first
        #[arg(long = "no-fetch")]
        no_fetch: bool,
    },
    /// Install a macOS LaunchAgent that runs `refresh` in the background
    Schedule {
        /// How often: 30m, 2h, 1d, or seconds
        #[arg(long, default_value = "30m")]
        interval: String,
        /// Folder to watch for newly cloned repos
        #[arg(long)]
        dir: Option<String>,
        /// Concurrent fetches in the scheduled job
        #[arg(long, short = 'j', default_value = "4")]
        jobs: usize,
        /// Uninstall it
        #[arg(long)]
        remove: bool,
        /// Report whether it is installed and running
        #[arg(long)]
        status: bool,
        /// Run it right now
        #[arg(long)]
        now: bool,
        /// Show recent output
        #[arg(long)]
        logs: bool,
        /// Skip the run that would otherwise happen at login
        #[arg(long = "no-run-at-load")]
        no_run_at_load: bool,
    },
    /// Run a local server so the views are interactive
    Serve {
        #[arg(long, default_value = "7777")]
        port: u16,
        /// Detach and keep running in the background
        #[arg(long)]
        daemon: bool,
        /// Stop a running daemon
        #[arg(long)]
        stop: bool,
        /// Report whether a daemon is running
        #[arg(long)]
        status: bool,
        /// Re-check the repos for new commits every N seconds (0 disables)
        #[arg(long = "refresh", default_value = "30")]
        refresh: u64,
        /// Don't open a browser on start
        #[arg(long = "no-open")]
        no_open: bool,
    },
}

fn emit(o: &Output, scope: &Scope) -> Result<()> {
    let text = match scope.format {
        Format::Table => render_term(o),
        Format::Json => render_json(o),
        Format::Html => html::page(o),
    };
    match &scope.out {
        Some(p) => {
            std::fs::write(p, &text)?;
            eprintln!("wrote {}", p.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

fn load() -> Result<(Cache, Identities)> {
    let c = load_cache()?;
    if c.repos.is_empty() {
        eprintln!("note: cache is empty — run `repo-metrics ingest <repo-path>` first");
    }
    Ok((c, Identities::load()))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Ingest { paths, force } => {
            let paths = if paths.is_empty() {
                vec![".".to_string()]
            } else {
                paths
            };
            let mut cache = load_cache()?;
            let t = std::time::Instant::now();
            for p in &paths {
                let abs = git::canonical(p)?;
                if !git::is_repo(&abs) {
                    bail!("{} is not a git repository", abs.display());
                }
                ingest::ingest(&mut cache, &abs, force, false)?;
            }
            save_cache(&cache)?;
            eprintln!(
                "cache: {} ({:.2}s)",
                cache_path().display(),
                t.elapsed().as_secs_f64()
            );
        }
        Cmd::Repos => {
            let cache = load_cache()?;
            if cache.repos.is_empty() {
                println!("nothing ingested yet");
                return Ok(());
            }
            let rows: Vec<Vec<Cell>> = cache
                .repos
                .iter()
                .map(|r| {
                    let first = r.commits.first().map(|c| days_to_date(c.days).to_string());
                    let last = r.commits.last().map(|c| days_to_date(c.days).to_string());
                    vec![
                        cell_text(&r.name),
                        Cell::Int(r.commits.len() as i64),
                        Cell::Int(r.changes.len() as i64),
                        cell_text(&first.unwrap_or_default()),
                        cell_text(&last.unwrap_or_default()),
                        cell_text(&r.head[..8.min(r.head.len())]),
                    ]
                })
                .collect();
            let o = Output::Table {
                title: "Ingested repositories".into(),
                subtitle: cache_path().display().to_string(),
                source: None,
        scope: None,
                columns: vec![
                    "repo".into(),
                    "commits".into(),
                    "file changes".into(),
                    "first".into(),
                    "last".into(),
                    "head".into(),
                ],
                bar_column: None,
                drill: Vec::new(),
                rows,
            };
            print!("{}", render_term(&o));
        }
        Cmd::Timeseries { by, metric, split, overlay, top, scope } => {
            let (c, ids) = load()?;
            let f = scope.filter()?;
            let mut o = cmds::timeseries(&c, &ids, &f, by, metric, split, top, overlay);
            stamp_source(&mut o, &c, &f);
            emit(&o, &scope)?;
        }
        Cmd::Folders { by, metric, depth, top, scope } => {
            let (c, ids) = load()?;
            let f = scope.filter()?;
            let mut o = cmds::folders(&c, &ids, &f, by, metric, depth, top);
            stamp_source(&mut o, &c, &f);
            emit(&o, &scope)?;
        }
        Cmd::Hotspots { depth, top, scope } => {
            let (c, ids) = load()?;
            let f = scope.filter()?;
            let mut o = cmds::hotspots(&c, &ids, &f, depth, top);
            stamp_source(&mut o, &c, &f);
            emit(&o, &scope)?;
        }
        Cmd::Compare { a, b, depth, top, scope } => {
            let (c, ids) = load()?;
            let f = scope.filter()?;
            let mut o = cmds::compare(&c, &ids, &f, &a, &b, depth, top)?;
            stamp_source(&mut o, &c, &f);
            emit(&o, &scope)?;
        }
        Cmd::Flags { depth, z, min_churn, window, min_baseline, top, scope } => {
            let (c, ids) = load()?;
            let f = scope.filter()?;
            let mut o = cmds::flags(&c, &ids, &f, depth, z, min_churn, window, min_baseline, top);
            stamp_source(&mut o, &c, &f);
            emit(&o, &scope)?;
        }
        Cmd::Tree { at, subpath, depth, measure, scope } => {
            let (c, _) = load()?;
            let f = scope.filter()?;
            let mut o = cmds::tree(&c, &f, at.as_deref(), &subpath, depth, measure)?;
            stamp_source(&mut o, &c, &f);
            emit(&o, &scope)?;
        }
        Cmd::Authors { top, scope } => {
            let (c, ids) = load()?;
            let f = scope.filter()?;
            let mut o = cmds::authors(&c, &ids, &f, top);
            stamp_source(&mut o, &c, &f);
            emit(&o, &scope)?;
        }
        Cmd::Assist { by, scope } => {
            let (c, ids) = load()?;
            let f = scope.filter()?;
            let mut o = cmds::assist_mix(&c, &ids, &f, by);
            stamp_source(&mut o, &c, &f);
            emit(&o, &scope)?;
        }
        Cmd::Sync {
            org, dir, sort, limit, jobs, all, yes, refresh, ssh, https, filter,
            include_archived, include_forks, dry_run, ingest,
        } => {
            sync::run(sync::Opts {
                org,
                dir,
                sort,
                limit,
                jobs,
                all,
                yes,
                refresh,
                // None means "follow the gh config"; the flags force it either way.
                ssh: if ssh { Some(true) } else if https { Some(false) } else { None },
                filter,
                include_archived,
                include_forks,
                dry_run,
                ingest,
            })?;
        }
        Cmd::Refresh { dir, jobs, quiet, no_fetch } => {
            refresh::run(refresh::Opts { dir, jobs, quiet, no_fetch })?;
        }
        Cmd::Schedule {
            interval, dir, jobs, remove, status, now, logs, no_run_at_load,
        } => {
            schedule::run(schedule::Opts {
                interval,
                dir,
                jobs,
                remove,
                status,
                now,
                logs,
                at_load: !no_run_at_load,
            })?;
        }
        Cmd::Serve { port, daemon, stop, status, refresh, no_open } => {
            server::run(port, daemon, stop, status, refresh, no_open)?;
        }
    }
    Ok(())
}
