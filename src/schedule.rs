use crate::proc::uid;
use crate::refresh;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

pub const LABEL: &str = "com.repo-metrics.refresh";

pub struct Opts {
    pub interval: String,
    pub dir: Option<String>,
    pub jobs: usize,
    pub remove: bool,
    pub status: bool,
    pub now: bool,
    pub logs: bool,
    pub at_load: bool,
}

fn plist_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("no HOME")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

fn domain() -> String {
    format!("gui/{}", uid())
}

fn service() -> String {
    format!("{}/{}", domain(), LABEL)
}

/// Accepts `30m`, `2h`, `90s`, `1d`, or a bare number of seconds.
fn parse_interval(s: &str) -> Result<u64> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u64>() {
        return ok_interval(n);
    }
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: u64 = num
        .parse()
        .with_context(|| format!("cannot parse interval {s:?} (try 30m, 2h, 1d)"))?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => bail!("unknown interval unit {unit:?} (use s, m, h, or d)"),
    };
    ok_interval(secs)
}

fn ok_interval(n: u64) -> Result<u64> {
    // Below a minute launchd throttles anyway, and a fetch storm helps nobody.
    if n < 60 {
        bail!("interval must be at least 60s");
    }
    Ok(n)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// launchd starts jobs with a minimal PATH, so git and gh must be findable. The
/// installing shell's PATH is the one known to work, with the usual locations added
/// in case it was unusually sparse.
fn agent_path() -> String {
    let mut parts: Vec<String> = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    for extra in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        if !parts.iter().any(|p| p == extra) {
            parts.push(extra.to_string());
        }
    }
    parts.join(":")
}

fn require_tool(name: &str) -> Result<()> {
    let found = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !found {
        bail!("{name} is not on PATH — the scheduled job would fail");
    }
    Ok(())
}

fn build_plist(exe: &str, interval: u64, dir: &Option<String>, jobs: usize, at_load: bool) -> String {
    let log = refresh::log_path();
    let log = log.display().to_string();
    let mut args = vec![
        exe.to_string(),
        "refresh".into(),
        "--quiet".into(),
        "--jobs".into(),
        jobs.to_string(),
    ];
    if let Some(d) = dir {
        args.push("--dir".into());
        args.push(d.clone());
    }
    let arg_xml: String = args
        .iter()
        .map(|a| format!("    <string>{}</string>\n", xml_escape(a)))
        .collect();
    let home = std::env::var("HOME").unwrap_or_default();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{arg_xml}  </array>
  <key>StartInterval</key>
  <integer>{interval}</integer>
  <key>RunAtLoad</key>
  <{at_load}/>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>{path}</string>
    <key>HOME</key>
    <string>{home}</string>
  </dict>
  <key>ProcessType</key>
  <string>Background</string>
  <key>LowPriorityIO</key>
  <true/>
  <key>Nice</key>
  <integer>5</integer>
</dict>
</plist>
"#,
        label = LABEL,
        arg_xml = arg_xml,
        interval = interval,
        at_load = if at_load { "true" } else { "false" },
        log = xml_escape(&log),
        path = xml_escape(&agent_path()),
        home = xml_escape(&home),
    )
}

fn launchctl(args: &[&str]) -> (bool, String) {
    match Command::new("launchctl").args(args).output() {
        Ok(o) => (
            o.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
            .trim()
            .to_string(),
        ),
        Err(e) => (false, e.to_string()),
    }
}

fn is_loaded() -> bool {
    launchctl(&["print", &service()]).0
}

fn unload_quietly() {
    launchctl(&["bootout", &service()]);
}

pub fn run(opts: Opts) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("LaunchAgents are macOS-only; on Linux use a systemd timer or cron running `repo-metrics refresh`");
    }
    let path = plist_path()?;

    if opts.logs {
        let lp = refresh::log_path();
        if !lp.exists() {
            println!("no log yet at {}", lp.display());
            return Ok(());
        }
        let text = std::fs::read_to_string(&lp)?;
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(40);
        println!("{}", lines[start..].join("\n"));
        return Ok(());
    }

    if opts.remove {
        if !path.exists() && !is_loaded() {
            println!("no scheduled refresh installed");
            return Ok(());
        }
        unload_quietly();
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        println!("removed the scheduled refresh");
        return Ok(());
    }

    if opts.status {
        println!("label    {LABEL}");
        println!("plist    {}", path.display());
        if !path.exists() {
            println!("state    not installed — run `repo-metrics schedule` to add it");
            return Ok(());
        }
        let (loaded, out) = launchctl(&["print", &service()]);
        if !loaded {
            println!("state    installed but not loaded");
            return Ok(());
        }
        let field = |k: &str| {
            out.lines()
                .find(|l| l.trim_start().starts_with(k))
                .map(|l| l.split('=').nth(1).unwrap_or("").trim().to_string())
        };
        println!("state    loaded");
        if let Some(v) = field("state ") {
            println!("running  {v}");
        }
        if let Some(v) = field("last exit code") {
            println!("last run exit {v}");
        }
        // The interval lives in the plist we wrote, which is easier to read back
        // than launchctl's output.
        if let Ok(p) = std::fs::read_to_string(&path) {
            if let Some(i) = p.split("<key>StartInterval</key>").nth(1) {
                if let Some(v) = i.split("<integer>").nth(1).and_then(|x| x.split('<').next()) {
                    let secs: u64 = v.trim().parse().unwrap_or(0);
                    println!("interval every {}", human_secs(secs));
                }
            }
        }
        println!("log      {}", refresh::log_path().display());
        return Ok(());
    }

    if opts.now {
        if !path.exists() {
            bail!("nothing scheduled yet — run `repo-metrics schedule` first");
        }
        let (ok, out) = launchctl(&["kickstart", "-k", &service()]);
        if !ok {
            bail!("could not start the job: {out}");
        }
        println!("kicked off a refresh; watch it with `repo-metrics schedule --logs`");
        return Ok(());
    }

    // --- install ---
    require_tool("git")?;
    let interval = parse_interval(&opts.interval)?;
    let exe = std::env::current_exe()
        .context("cannot determine this binary's path")?
        .canonicalize()
        .context("cannot resolve this binary's path")?;
    let exe = exe.display().to_string();
    if exe.contains("/target/") {
        eprintln!(
            "note: scheduling {exe}\n      that path is a build directory, so `cargo clean` or a rebuild will break the job.\n      consider installing the binary somewhere stable first."
        );
    }

    let body = build_plist(&exe, interval, &opts.dir, opts.jobs, opts.at_load);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    // Replace rather than edit in place: launchd caches by label, so an old copy has
    // to be booted out before the new one is bootstrapped.
    unload_quietly();
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    let (ok, out) = launchctl(&["bootstrap", &domain(), &path.to_string_lossy()]);
    if !ok {
        // Older macOS wants the deprecated verb.
        let (ok2, out2) = launchctl(&["load", "-w", &path.to_string_lossy()]);
        if !ok2 {
            bail!("could not load the LaunchAgent: {out}\n{out2}");
        }
    }

    println!("scheduled a refresh every {}", human_secs(interval));
    println!("  plist  {}", path.display());
    println!("  log    {}", refresh::log_path().display());
    if let Some(d) = &opts.dir {
        println!("  watching {d} for new checkouts");
    }
    println!();
    println!("  repo-metrics schedule --status    see whether it is running");
    println!("  repo-metrics schedule --now       run it immediately");
    println!("  repo-metrics schedule --logs      recent output");
    println!("  repo-metrics schedule --remove    uninstall");
    Ok(())
}

fn human_secs(s: u64) -> String {
    if s % 86_400 == 0 && s >= 86_400 {
        let d = s / 86_400;
        format!("{d} day{}", if d == 1 { "" } else { "s" })
    } else if s % 3600 == 0 && s >= 3600 {
        let h = s / 3600;
        format!("{h} hour{}", if h == 1 { "" } else { "s" })
    } else if s % 60 == 0 {
        let m = s / 60;
        format!("{m} minute{}", if m == 1 { "" } else { "s" })
    } else {
        format!("{s}s")
    }
}
