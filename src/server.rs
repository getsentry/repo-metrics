use crate::cmds;
use crate::git;
use crate::html;
use crate::identity::Identities;
use crate::ingest;
use crate::model::*;
use crate::output::Output;
use crate::proc::{pid_alive, signal};
use crate::query::*;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

struct State {
    cache: RwLock<Cache>,
    ids: RwLock<Identities>,
    /// Bumped whenever new commits land. The page polls it and re-fetches, which is
    /// what makes the views live without a websocket.
    generation: AtomicU64,
    last_refresh: RwLock<String>,
}

fn pid_file() -> PathBuf {
    cache_dir().join("serve.pid")
}
fn log_file() -> PathBuf {
    cache_dir().join("serve.log")
}

fn read_pid() -> Option<i32> {
    std::fs::read_to_string(pid_file())
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
}


pub fn run(port: u16, daemon: bool, stop: bool, status: bool, refresh: u64, no_open: bool) -> Result<()> {
    std::fs::create_dir_all(cache_dir())?;

    if stop {
        match read_pid() {
            Some(pid) if pid_alive(pid) => {
                signal(pid, 15);
                std::fs::remove_file(pid_file()).ok();
                println!("stopped repo-metrics daemon (pid {pid})");
            }
            _ => println!("no daemon running"),
        }
        return Ok(());
    }
    if status {
        match read_pid() {
            Some(pid) if pid_alive(pid) => println!("running (pid {pid}) — log: {}", log_file().display()),
            _ => println!("not running"),
        }
        return Ok(());
    }
    // The detached child re-execs this same binary; it must not mistake the
    // pidfile it is about to own for a rival instance.
    let is_child = std::env::var("REPO_METRICS_DAEMON_CHILD").is_ok();
    if !is_child {
        if let Some(pid) = read_pid() {
            if pid_alive(pid) {
                bail!(
                    "a daemon is already running (pid {pid}); use --stop first, or a different --port"
                );
            }
            std::fs::remove_file(pid_file()).ok();
        }
    }

    if daemon {
        // Re-exec ourselves without --daemon, detached, with output in the log.
        let exe = std::env::current_exe()?;
        let log = std::fs::File::create(log_file())?;
        let errlog = log.try_clone()?;
        let child = std::process::Command::new(exe)
            .arg("serve")
            .arg("--port")
            .arg(port.to_string())
            .arg("--refresh")
            .arg(refresh.to_string())
            .arg("--no-open")
            .env("REPO_METRICS_DAEMON_CHILD", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(log))
            .stderr(std::process::Stdio::from(errlog))
            .spawn()
            .context("failed to start daemon")?;
        println!("repo-metrics daemon started (pid {})", child.id());
        println!("  http://127.0.0.1:{port}");
        println!("  log:  {}", log_file().display());
        println!("  stop: repo-metrics serve --stop");
        return Ok(());
    }

    let cache = load_cache()?;
    if cache.repos.is_empty() {
        bail!("cache is empty — run `repo-metrics ingest <repo-path>` before serving");
    }
    let state = Arc::new(State {
        cache: RwLock::new(cache),
        ids: RwLock::new(Identities::load()),
        generation: AtomicU64::new(1),
        last_refresh: RwLock::new("startup".to_string()),
    });

    // Bound to loopback on purpose: this serves the full contents of your private
    // repositories and has no authentication of its own.
    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("cannot bind 127.0.0.1:{port} (already in use?)"))?;
    // Written only once the port is actually bound, so a pidfile always means a
    // server that is answering.
    std::fs::write(pid_file(), std::process::id().to_string()).ok();

    println!("repo-metrics serving http://127.0.0.1:{port}");
    {
        let c = state.cache.read().unwrap();
        for r in &c.repos {
            println!("  {} — {} commits", r.name, group_u(r.commits.len()));
        }
    }
    if refresh > 0 {
        println!("  watching for new commits every {refresh}s");
        spawn_refresher(state.clone(), refresh);
    }
    if !no_open {
        let _ = std::process::Command::new("open")
            .arg(format!("http://127.0.0.1:{port}"))
            .spawn();
    }

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let st = state.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle(s, st) {
                        eprintln!("request error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    Ok(())
}

fn group_u(n: usize) -> String {
    crate::output::group(n as i64)
}

/// Polls each repo's HEAD and folds in only what is new. An incremental pass is
/// well under a second, so this can run often without being noticed.
fn spawn_refresher(state: Arc<State>, secs: u64) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(secs));
        let paths: Vec<(String, String)> = {
            let c = state.cache.read().unwrap();
            c.repos.iter().map(|r| (r.path.clone(), r.head.clone())).collect()
        };
        let mut changed = false;
        for (path, known_head) in paths {
            let p = Path::new(&path);
            let head = match git::head_sha(p) {
                Ok(h) => h,
                Err(_) => continue,
            };
            if head == known_head {
                continue;
            }
            let mut c = state.cache.write().unwrap();
            match ingest::ingest(&mut c, p, false, true) {
                Ok(()) => {
                    changed = true;
                    eprintln!("refreshed {path} → {}", &head[..8.min(head.len())]);
                }
                Err(e) => eprintln!("refresh failed for {path}: {e}"),
            }
        }
        if changed {
            {
                let c = state.cache.read().unwrap();
                let _ = save_cache(&c);
            }
            // Identity rules are re-read too, so editing identities.toml shows up
            // without restarting the daemon.
            *state.ids.write().unwrap() = Identities::load();
            state.generation.fetch_add(1, Ordering::SeqCst);
            *state.last_refresh.write().unwrap() = now_string();
        }
    });
}

fn now_string() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let h = |c: u8| (c as char).to_digit(16);
                match (h(b[i + 1]), h(b[i + 2])) {
                    (Some(a), Some(c)) => {
                        out.push((a * 16 + c) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(b[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((pct_decode(k), pct_decode(v)))
        })
        .collect()
}

fn respond(mut s: TcpStream, code: &str, ctype: &str, body: &str) -> Result<()> {
    let head = format!(
        "HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.as_bytes().len()
    );
    s.write_all(head.as_bytes())?;
    s.write_all(body.as_bytes())?;
    s.flush()?;
    Ok(())
}

fn handle(stream: TcpStream, state: Arc<State>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut rd = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if rd.read_line(&mut line)? == 0 {
        return Ok(());
    }
    // Drain the headers so the client sees a clean response.
    let mut h = String::new();
    loop {
        h.clear();
        if rd.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    if method != "GET" {
        return respond(stream, "405 Method Not Allowed", "text/plain", "GET only");
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let q = parse_query(query);

    match path {
        "/" => respond(stream, "200 OK", "text/html; charset=utf-8", &app_page()),
        "/api/meta" => {
            let c = state.cache.read().unwrap();
            let repos: Vec<serde_json::Value> = c
                .repos
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "name": r.name,
                        "path": r.path,
                        "commits": r.commits.len(),
                        "changes": r.changes.len(),
                        "head": r.head,
                        "first": r.commits.first().map(|x| days_to_date(x.days).to_string()),
                        "last": r.commits.last().map(|x| days_to_date(x.days).to_string()),
                    })
                })
                .collect();
            let body = serde_json::json!({
                "repos": repos,
                "generation": state.generation.load(Ordering::SeqCst),
                "last_refresh": *state.last_refresh.read().unwrap(),
            });
            respond(stream, "200 OK", "application/json", &body.to_string())
        }
        "/api/view" => {
            let t = std::time::Instant::now();
            match build_view(&state, &q) {
                Ok(o) => {
                    let mut v = serde_json::to_value(&o).unwrap_or(serde_json::Value::Null);
                    if let Some(m) = v.as_object_mut() {
                        m.insert("ms".into(), serde_json::json!(t.elapsed().as_secs_f64() * 1000.0));
                        m.insert(
                            "generation".into(),
                            serde_json::json!(state.generation.load(Ordering::SeqCst)),
                        );
                    }
                    respond(stream, "200 OK", "application/json", &v.to_string())
                }
                Err(e) => respond(
                    stream,
                    "400 Bad Request",
                    "application/json",
                    &serde_json::json!({ "error": e.to_string() }).to_string(),
                ),
            }
        }
        _ => respond(stream, "404 Not Found", "text/plain", "not found"),
    }
}

fn getn(q: &HashMap<String, String>, k: &str, d: usize) -> usize {
    q.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
}
fn getf(q: &HashMap<String, String>, k: &str, d: f64) -> f64 {
    q.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
}
fn opt(q: &HashMap<String, String>, k: &str) -> Option<String> {
    q.get(k).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn bucket_of(s: &str) -> Bucket {
    match s {
        "day" => Bucket::Day,
        "month" => Bucket::Month,
        _ => Bucket::Week,
    }
}
fn metric_of(s: &str) -> Metric {
    match s {
        "churn" => Metric::Churn,
        "added" => Metric::Added,
        "removed" => Metric::Removed,
        "files" => Metric::Files,
        _ => Metric::Commits,
    }
}
fn split_of(s: &str) -> Split {
    match s {
        "assist" => Split::Assist,
        "tool" => Split::Tool,
        "author" => Split::Author,
        "language" => Split::Language,
        _ => Split::None,
    }
}

fn build_view(state: &State, q: &HashMap<String, String>) -> Result<Output> {
    let cache = state.cache.read().unwrap();
    let ids = state.ids.read().unwrap();
    let f = Filter {
        repo: opt(q, "repo"),
        since: opt(q, "since").as_deref().map(parse_date).transpose()?,
        until: opt(q, "until").as_deref().map(parse_date).transpose()?,
        path: opt(q, "path"),
    };
    let by = bucket_of(q.get("by").map(|s| s.as_str()).unwrap_or("week"));
    let metric = metric_of(q.get("metric").map(|s| s.as_str()).unwrap_or("commits"));
    let split = split_of(q.get("split").map(|s| s.as_str()).unwrap_or("none"));
    let depth = getn(q, "depth", 1);
    let top = getn(q, "top", 12);

    let view = q.get("view").map(|s| s.as_str()).unwrap_or("timeseries");
    let mut out = match view {
        "timeseries" => cmds::timeseries(&cache, &ids, &f, by, metric, split, top),
        "folders" => cmds::folders(&cache, &ids, &f, by, metric, depth.max(1), top),
        "hotspots" => cmds::hotspots(&cache, &ids, &f, depth.max(1), top.max(5)),
        "flags" => cmds::flags(
            &cache,
            &ids,
            &f,
            depth.max(1),
            getf(q, "z", 2.5),
            getf(q, "min_churn", 200.0),
            getn(q, "window", 12),
            getn(q, "min_baseline", 8),
            top.max(5),
        ),
        "authors" => cmds::authors(&cache, &ids, &f, top.max(5)),
        "assist" => cmds::assist_mix(&cache, &ids, &f, by),
        "compare" => cmds::compare(
            &cache,
            &ids,
            &f,
            &opt(q, "a").unwrap_or_else(|| "2025-H2".into()),
            &opt(q, "b").unwrap_or_else(|| "2026-H1".into()),
            depth.max(1),
            top,
        )?,
        // `radial` was a second view that rendered identically to `tree`; it stays
        // accepted so old links keep working, but there is only one view now.
        "tree" | "radial" => cmds::tree(
            &cache,
            &f,
            opt(q, "at").as_deref(),
            &opt(q, "subpath").unwrap_or_default(),
            depth.max(1),
        )?,
        other => bail!("unknown view {other:?}"),
    };
    stamp_source(&mut out, &cache, &f);
    Ok(out)
}

fn app_page() -> String {
    format!(
        r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>repo-metrics</title>
<style>{css}
.bar{{display:flex;flex-wrap:wrap;gap:.55rem;align-items:center;margin-bottom:1.1rem}}
.bar label{{display:flex;flex-direction:column;gap:.2rem;font-size:.68rem;text-transform:uppercase;
  letter-spacing:.07em;color:var(--muted);font-weight:600}}
select,input{{font:inherit;font-size:.83rem;padding:.32rem .45rem;border:1px solid var(--border);
  border-radius:6px;background:var(--surface);color:var(--text);min-width:6.5rem}}
input{{font-family:ui-monospace,Menlo,monospace}}
select:focus-visible,input:focus-visible,button:focus-visible{{outline:2px solid var(--accent);outline-offset:1px}}
.tabs{{display:flex;flex-wrap:wrap;gap:.3rem;margin-bottom:1rem;border-bottom:1px solid var(--border);padding-bottom:.7rem}}
.tabs button{{font:inherit;font-size:.83rem;padding:.34rem .7rem;border-radius:6px;border:1px solid transparent;
  background:transparent;color:var(--muted);cursor:pointer}}
.tabs button:hover{{background:var(--surface2);color:var(--text)}}
.tabs button[aria-selected="true"]{{background:var(--surface);border-color:var(--border);color:var(--text);font-weight:600}}
.status{{display:flex;gap:1rem;align-items:center;font-size:.74rem;color:var(--faint);margin-top:.9rem;flex-wrap:wrap}}
.dot{{width:7px;height:7px;border-radius:50%;background:var(--accent);display:inline-block;margin-right:.35rem}}
.dot.stale{{background:var(--c1)}}
.hide{{display:none!important}}
</style></head>
<body><div class="wrap">
<h1>repo-metrics</h1>
<div class="head"><p class="sub" id="scope">loading…</p><span id="src"></span></div>

<div class="tabs" role="tablist" id="tabs"></div>
<div class="bar" id="controls"></div>

<div class="card"><div id="chart"><div class="empty">loading…</div></div></div>
<div class="status">
  <span><span class="dot" id="dot"></span><span id="live">connecting</span></span>
  <span id="timing"></span>
</div>
</div>
<script>{js}

const VIEWS=[
  ['timeseries','Commits over time',   ['repo','since','until','path','by','metric','split','top'],{{}}],
  ['folders',   'Folders over time',   ['repo','since','until','path','by','metric','depth','top'],{{depth:'1'}}],
  ['hotspots',  'Fastest-moving',      ['repo','since','until','path','depth','top'],{{depth:'2',top:'20'}}],
  ['tree',      'Folder sizes',        ['repo','at','subpath','depth'],{{depth:'2'}}],
  ['compare',   'Compare periods',     ['repo','a','b','depth','top'],{{depth:'1'}}],
  ['flags',     'Interesting periods', ['repo','since','until','path','depth','z','min_churn','top'],{{depth:'1',top:'30'}}],
  ['assist',    'Human vs agent',      ['repo','since','until','path','by'],{{by:'month'}}],
  ['authors',   'Authors',             ['repo','since','until','path','top'],{{top:'25'}}],
];
// Per-view defaults win over the field default, but never over something the
// reader has already chosen for that field.
function viewDef(f){{const v=VIEWS.find(x=>x[0]===view); return (v[3]||{{}})[f]; }}
const FIELDS={{
  repo:  {{t:'select',label:'repo',opts:[]}},
  since: {{t:'text',label:'since',ph:'2024-01-01 / 90d'}},
  until: {{t:'text',label:'until',ph:''}},
  path:  {{t:'text',label:'path',ph:'src/sentry'}},
  subpath:{{t:'text',label:'subtree',ph:'src/sentry'}},
  at:    {{t:'text',label:'as of',ph:'2025-06-01'}},
  a:     {{t:'text',label:'period A',ph:'2025-H2',def:'2025-H2'}},
  b:     {{t:'text',label:'period B',ph:'2026-H1',def:'2026-H1'}},
  by:    {{t:'select',label:'bucket',opts:['day','week','month'],def:'week'}},
  metric:{{t:'select',label:'metric',opts:['commits','churn','added','removed','files'],def:'commits'}},
  split: {{t:'select',label:'split by',opts:['none','assist','tool','author','language'],def:'none'}},
  depth: {{t:'select',label:'depth',opts:['1','2','3'],def:'1'}},
  top:   {{t:'select',label:'top',opts:['5','8','12','20','30','50'],def:'12'}},
  z:     {{t:'text',label:'z >',ph:'2.5',def:'2.5'}},
  min_churn:{{t:'text',label:'min churn',ph:'200',def:'200'}},
}};

let view='timeseries', gen=0, state={{}};
const el=id=>document.getElementById(id);

function buildTabs(){{
  el('tabs').innerHTML=VIEWS.map(([k,label])=>
    `<button role="tab" data-v="${{k}}" aria-selected="${{k===view}}">${{label}}</button>`).join('');
  el('tabs').querySelectorAll('button').forEach(b=>b.onclick=()=>{{view=b.dataset.v;buildTabs();buildControls();load();}});
}}

function buildControls(){{
  const fields=VIEWS.find(v=>v[0]===view)[2];
  el('controls').innerHTML=fields.map(f=>{{
    const d=FIELDS[f]; if(!d)return '';
    const val=state[f]!==undefined?state[f]:(viewDef(f)||d.def||'');
    if(d.t==='select'){{
      const opts=(f==='repo'?state.__repos||[]:d.opts);
      const list=(f==='repo'?['<option value="">all repos</option>']:[]).concat(
        opts.map(o=>`<option value="${{esc(o)}}"${{String(val)===String(o)?' selected':''}}>${{esc(o)}}</option>`));
      return `<label>${{d.label}}<select data-f="${{f}}">${{list.join('')}}</select></label>`;
    }}
    return `<label>${{d.label}}<input data-f="${{f}}" value="${{esc(val)}}" placeholder="${{esc(d.ph||'')}}" size="12"></label>`;
  }}).join('');
  el('controls').querySelectorAll('[data-f]').forEach(inp=>{{
    const ev=inp.tagName==='SELECT'?'change':'change';
    inp.addEventListener(ev,()=>{{state[inp.dataset.f]=inp.value;load();}});
  }});
}}

function params(){{
  const fields=VIEWS.find(v=>v[0]===view)[2];
  const p=new URLSearchParams({{view}});
  for(const f of fields){{
    const d=FIELDS[f]; if(!d)continue;
    const v=state[f]!==undefined?state[f]:(viewDef(f)||d.def||'');
    if(v!=='')p.set(f,v);
  }}
  return p.toString();
}}

let inflight=null;
async function load(){{
  const url='/api/view?'+params();
  if(inflight)inflight.abort();
  const ac=new AbortController(); inflight=ac;
  try{{
    const r=await fetch(url,{{signal:ac.signal}});
    const d=await r.json();
    if(d.error){{el('chart').innerHTML=`<div class="empty">${{esc(d.error)}}</div>`;el('timing').textContent='';return;}}
    window.__data=d;
    window.__rerender=()=>render(el('chart'),d);
    window.__rerender();
    el('scope').textContent=d.subtitle||'';
    el('src').innerHTML=sourceHtml(d);
    el('timing').textContent=`${{d.title}} · ${{d.ms.toFixed(0)}} ms`;
  }}catch(e){{ if(e.name!=='AbortError') el('chart').innerHTML=`<div class="empty">${{esc(e.message)}}</div>`; }}
}}

async function poll(){{
  try{{
    const m=await (await fetch('/api/meta')).json();
    if(!state.__repos){{
      state.__repos=m.repos.map(r=>r.name);
      buildControls();
    }}
    el('live').textContent=`live · ${{m.repos.length}} repo${{m.repos.length===1?'':'s'}} · last change ${{m.last_refresh}}`;
    el('dot').classList.remove('stale');
    // A new generation means the daemon folded in new commits; refetch the view.
    if(gen&&m.generation!==gen)load();
    gen=m.generation;
  }}catch(e){{
    el('live').textContent='daemon unreachable';
    el('dot').classList.add('stale');
  }}
}}

buildTabs(); buildControls(); poll().then(load); setInterval(poll,5000);
</script></body></html>"##,
        css = html::CSS,
        js = html::CHART_JS
    )
}
