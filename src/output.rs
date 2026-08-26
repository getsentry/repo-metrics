use serde::Serialize;
use std::io::IsTerminal;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Debug)]
pub enum Format {
    /// Terminal table with inline sparklines.
    Table,
    /// Machine-readable, for piping into anything else.
    Json,
    /// One self-contained page, data inlined, no network at render time.
    Html,
}

#[derive(Serialize, Clone)]
pub struct Series {
    pub name: String,
    pub points: Vec<f64>,
}

#[derive(Serialize, Clone)]
#[serde(untagged)]
pub enum Cell {
    Text(String),
    Num(f64),
    Int(i64),
}

impl Cell {
    pub fn as_f64(&self) -> f64 {
        match self {
            Cell::Num(n) => *n,
            Cell::Int(i) => *i as f64,
            Cell::Text(_) => 0.0,
        }
    }
    pub fn render(&self) -> String {
        match self {
            Cell::Text(s) => s.clone(),
            Cell::Int(i) => group(*i),
            Cell::Num(n) => {
                if (n - n.round()).abs() < 1e-9 {
                    group(*n as i64)
                } else {
                    format!("{n:.2}")
                }
            }
        }
    }
}

pub fn group(n: i64) -> String {
    let neg = n < 0;
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

#[derive(Serialize, Clone)]
pub struct TreeNode {
    pub name: String,
    /// A directory can be descended into; a file is a leaf. Size and file count
    /// alone can't tell them apart, since a directory holding one file looks
    /// identical to that file.
    pub dir: bool,
    pub size: u64,
    pub files: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeNode>,
}

/// The exact repository state a view was computed from, so a reader can click
/// through to the commit instead of taking the numbers on trust.
#[derive(Serialize, Clone)]
pub struct SourceRef {
    pub repo: String,
    pub sha: String,
    pub short: String,
    /// Deep link to the commit, when the remote is a recognisable forge URL.
    pub url: Option<String>,
}

impl SourceRef {
    pub fn new(repo: &str, sha: &str, web: &Option<String>) -> Self {
        let short = sha.chars().take(8).collect::<String>();
        Self {
            repo: repo.to_string(),
            sha: sha.to_string(),
            short,
            url: web.as_ref().map(|w| format!("{w}/commit/{sha}")),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct RepoLink {
    pub name: String,
    /// None when the repo has no recognisable forge remote.
    pub url: Option<String>,
}

/// A date boundary of the active filter, resolved to the commit it actually lands
/// on so the header can link out to it.
#[derive(Serialize, Clone)]
pub struct DateLink {
    pub date: String,
    pub sha: Option<String>,
    pub url: Option<String>,
    /// True when no commit fell on the date itself and this is the nearest one
    /// inside the range.
    pub approximate: bool,
}

/// The header, as data rather than a formatted string, so each part can carry its
/// own link.
#[derive(Serialize, Clone)]
pub struct ScopeRef {
    pub repos: Vec<RepoLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<DateLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<DateLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Output {
    /// A time series, optionally several stacked bands sharing one x axis.
    Series {
        title: String,
        subtitle: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<SourceRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        scope: Option<ScopeRef>,
        x: Vec<String>,
        series: Vec<Series>,
        stacked: bool,
        y_label: String,
        /// A second measure on its own scale, drawn against a right-hand axis.
        /// Deliberately not part of `series`: it shares the x axis and nothing else,
        /// and where it crosses the bars is an artefact of the two scales rather
        /// than anything true about the data.
        #[serde(skip_serializing_if = "Option::is_none")]
        overlay: Option<Series>,
        #[serde(skip_serializing_if = "Option::is_none")]
        overlay_label: Option<String>,
    },
    Table {
        title: String,
        subtitle: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<SourceRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        scope: Option<ScopeRef>,
        columns: Vec<String>,
        /// Index of the column to draw an in-cell magnitude bar against.
        bar_column: Option<usize>,
        /// Parallel to `rows`: the directory each row represents, where the row is
        /// one that can be drilled into. Rows that aren't directories carry None,
        /// which is why this isn't just a column index — `compare` mixes summary
        /// measures and directories in one table.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        drill: Vec<Option<String>>,
        rows: Vec<Vec<Cell>>,
    },
    Tree {
        title: String,
        subtitle: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<SourceRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        scope: Option<ScopeRef>,
        /// What `size` counts: files, sloc or bytes. Drives how it's formatted.
        measure: String,
        root: TreeNode,
    },
}

impl Output {
    pub fn title(&self) -> &str {
        match self {
            Output::Series { title, .. } | Output::Table { title, .. } | Output::Tree { title, .. } => title,
        }
    }
    /// Views that already pin themselves to a specific commit (the tree snapshots)
    /// keep theirs; everything else is stamped with the repo HEAD it was read from.
    pub fn set_source_if_unset(&mut self, s: Option<SourceRef>) {
        let slot = match self {
            Output::Series { source, .. }
            | Output::Table { source, .. }
            | Output::Tree { source, .. } => source,
        };
        if slot.is_none() {
            *slot = s;
        }
    }

    pub fn set_scope(&mut self, sc: Option<ScopeRef>) {
        let slot = match self {
            Output::Series { scope, .. }
            | Output::Table { scope, .. }
            | Output::Tree { scope, .. } => scope,
        };
        if slot.is_none() {
            *slot = sc;
        }
    }

    pub fn source(&self) -> Option<&SourceRef> {
        match self {
            Output::Series { source, .. }
            | Output::Table { source, .. }
            | Output::Tree { source, .. } => source.as_ref(),
        }
    }

    pub fn subtitle(&self) -> &str {
        match self {
            Output::Series { subtitle, .. }
            | Output::Table { subtitle, .. }
            | Output::Tree { subtitle, .. } => subtitle,
        }
    }
}

// ---------------------------------------------------------------- terminal

const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub fn sparkline(v: &[f64]) -> String {
    if v.is_empty() {
        return String::new();
    }
    let max = v.iter().cloned().fold(f64::MIN, f64::max);
    if max <= 0.0 {
        return SPARK[0].to_string().repeat(v.len());
    }
    v.iter()
        .map(|x| {
            let i = ((x / max) * (SPARK.len() - 1) as f64).round() as usize;
            SPARK[i.min(SPARK.len() - 1)]
        })
        .collect()
}

struct Style {
    on: bool,
}

impl Style {
    fn new() -> Self {
        let on = std::io::stdout().is_terminal() && std::env::var("NO_COLOR").is_err();
        Self { on }
    }
    fn dim(&self, s: &str) -> String {
        if self.on {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.into()
        }
    }
    fn bold(&self, s: &str) -> String {
        if self.on {
            format!("\x1b[1m{s}\x1b[0m")
        } else {
            s.into()
        }
    }
    fn accent(&self, s: &str) -> String {
        if self.on {
            format!("\x1b[36m{s}\x1b[0m")
        } else {
            s.into()
        }
    }
}

fn width(s: &str) -> usize {
    s.chars().count()
}

fn pad(s: &str, w: usize, right: bool) -> String {
    let n = width(s);
    if n >= w {
        return s.to_string();
    }
    let sp = " ".repeat(w - n);
    if right {
        format!("{sp}{s}")
    } else {
        format!("{s}{sp}")
    }
}

/// A magnitude bar sized against the column max, so a ranked table reads at a
/// glance without needing a chart.
fn barcell(v: f64, max: f64, w: usize) -> String {
    if max <= 0.0 {
        return " ".repeat(w);
    }
    let n = ((v / max) * w as f64).round() as usize;
    let n = n.min(w);
    format!("{}{}", "█".repeat(n), " ".repeat(w - n))
}

pub fn render_term(o: &Output) -> String {
    let st = Style::new();
    let mut out = String::new();
    out.push('\n');
    out.push_str(&st.bold(o.title()));
    out.push('\n');
    if !o.subtitle().is_empty() {
        out.push_str(&st.dim(o.subtitle()));
        out.push('\n');
    }
    if let Some(src) = o.source() {
        // Most terminals turn a bare URL into a clickable link.
        let line = match &src.url {
            Some(u) => format!("{} @ {}  {}", src.repo, src.short, u),
            None => format!("{} @ {}", src.repo, src.short),
        };
        out.push_str(&st.dim(&line));
        out.push('\n');
    }
    out.push('\n');

    match o {
        Output::Series {
            x,
            series,
            y_label,
            overlay,
            overlay_label,
            ..
        } => {
            let namew = series
                .iter()
                .chain(overlay.iter())
                .map(|s| width(&s.name))
                .max()
                .unwrap_or(0)
                .max(6);
            for s in series {
                let total: f64 = s.points.iter().sum();
                out.push_str(&format!(
                    "  {}  {}  {}\n",
                    pad(&s.name, namew, false),
                    st.accent(&sparkline(&s.points)),
                    st.dim(&format!("{} {}", group(total as i64), y_label))
                ));
            }
            if let Some(ov) = overlay {
                // Its own scale, so the total is a peak rather than a sum.
                let peak = ov.points.iter().cloned().fold(0.0f64, f64::max);
                out.push_str(&format!(
                    "  {}  {}  {}\n",
                    pad(&ov.name, namew, false),
                    st.dim(&sparkline(&ov.points)),
                    st.dim(&format!(
                        "peak {} {}",
                        group(peak as i64),
                        overlay_label.as_deref().unwrap_or("")
                    ))
                ));
            }
            if let (Some(first), Some(last)) = (x.first(), x.last()) {
                out.push_str(&format!(
                    "\n  {}\n",
                    st.dim(&format!("{} → {}   ({} buckets)", first, last, x.len()))
                ));
            }
        }
        Output::Table {
            columns,
            rows,
            bar_column,
            ..
        } => {
            let ncol = columns.len();
            let mut w: Vec<usize> = columns.iter().map(|c| width(c)).collect();
            let cells: Vec<Vec<String>> = rows
                .iter()
                .map(|r| r.iter().map(|c| c.render()).collect())
                .collect();
            for r in &cells {
                for (i, c) in r.iter().enumerate() {
                    if i < ncol {
                        w[i] = w[i].max(width(c));
                    }
                }
            }
            let numeric: Vec<bool> = (0..ncol)
                .map(|i| {
                    rows.iter()
                        .all(|r| !matches!(r.get(i), Some(Cell::Text(_))))
                })
                .collect();

            let header: Vec<String> = columns
                .iter()
                .enumerate()
                .map(|(i, c)| pad(c, w[i], numeric[i]))
                .collect();
            out.push_str(&format!("  {}\n", st.dim(&header.join("  "))));
            out.push_str(&format!(
                "  {}\n",
                st.dim(&w.iter().map(|n| "─".repeat(*n)).collect::<Vec<_>>().join("  "))
            ));

            let maxbar = bar_column.as_ref()
                .map(|bc| {
                    rows.iter()
                        .filter_map(|r| r.get(*bc))
                        .map(|c| c.as_f64())
                        .fold(0.0f64, f64::max)
                })
                .unwrap_or(0.0);

            for (ri, r) in cells.iter().enumerate() {
                let line: Vec<String> = r
                    .iter()
                    .enumerate()
                    .map(|(i, c)| pad(c, w[i], numeric[i]))
                    .collect();
                let mut s = format!("  {}", line.join("  "));
                if let Some(bc) = bar_column {
                    let v = rows[ri].get(*bc).map(|c| c.as_f64()).unwrap_or(0.0);
                    s.push_str(&format!("  {}", st.accent(&barcell(v, maxbar, 22))));
                }
                out.push_str(&s);
                out.push('\n');
            }
        }
        Output::Tree { root, measure, .. } => {
            // "size" is whichever measure was requested; only bytes want a unit.
            let fmt_size = |n: u64| match measure.as_str() {
                "bytes" => human_bytes(n),
                "sloc" => format!("{} lines", group(n as i64)),
                _ => group(n as i64),
            };
            // Names are padded to a common column so sizes and percentages line up
            // however deep the tree goes.
            fn label_width(n: &TreeNode, depth: usize, w: &mut usize) {
                *w = (*w).max(depth * 2 + width(&n.name));
                for c in &n.children {
                    label_width(c, depth + 1, w);
                }
            }
            let mut lw = 0;
            label_width(root, 0, &mut lw);

            fn walk(
                n: &TreeNode,
                depth: usize,
                total: u64,
                lw: usize,
                out: &mut String,
                st: &Style,
                fmt_size: &dyn Fn(u64) -> String,
            ) {
                let pct = if total > 0 {
                    (n.size as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                let name = format!("{}{}", "  ".repeat(depth), n.name);
                out.push_str(&format!(
                    "  {}  {}  {}\n",
                    pad(&name, lw, false),
                    pad(&fmt_size(n.size), 14, true),
                    st.dim(&format!("{pct:>5.1}%  {:>7} files", group(n.files as i64)))
                ));
                for c in &n.children {
                    walk(c, depth + 1, total, lw, out, st, fmt_size);
                }
            }
            walk(root, 0, root.size, lw, &mut out, &st, &fmt_size);
        }
    }
    out.push('\n');

    out
}

pub fn human_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

pub fn render_json(o: &Output) -> String {
    serde_json::to_string_pretty(o).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}
