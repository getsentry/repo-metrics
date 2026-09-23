//! Value-add versus muda: how much of a repo's effort goes into new capability, and
//! how much into reworking, deleting and maintaining what is already there.
//!
//! Lines are a measure of cost, not of value. Every line kept is a line someone has
//! to maintain, so the bands count lines; but a large feature is not worth more than
//! a small one, so the value-add percentage counts commits.

use crate::identity::Identities;
use crate::lines::Lines;
use crate::model::*;
use crate::output::*;
use crate::query::*;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Intent {
    ValueAdd,
    Muda,
}

/// The author's own label for a commit, from a conventional-commit prefix.
///
/// Only `feat` is value-add. Fixes, refactors, performance work, tests, builds and
/// dependency bumps can all be necessary, but they keep existing capability working
/// rather than adding to it. None means the subject carries no recognisable label.
pub fn intent_of_subject(subject: &str) -> Option<Intent> {
    let s = subject.trim();
    if s.starts_with("Revert ") || s.starts_with("Bump ") {
        return Some(Intent::Muda);
    }
    let kind_end = s
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(s.len());
    let (kind, mut rest) = s.split_at(kind_end);
    if kind.is_empty() {
        return None;
    }
    if rest.starts_with('(') {
        rest = &rest[rest.find(')')? + 1..];
    }
    let rest = rest.strip_prefix('!').unwrap_or(rest);
    if !rest.starts_with(':') {
        return None;
    }
    match kind.to_ascii_lowercase().as_str() {
        "feat" | "feature" => Some(Intent::ValueAdd),
        "fix" | "ref" | "refactor" | "chore" | "test" | "tests" | "perf" | "build" | "ci"
        | "docs" | "doc" | "style" | "deps" | "revert" | "release" | "meta" | "lint"
        | "cleanup" => Some(Intent::Muda),
        _ => None,
    }
}

/// Files whose line counts say nothing about effort: lockfiles, translation
/// catalogues, snapshots, migrations and generated or minified output. Any of them
/// can outweigh a month of hand-written code in a single commit.
pub fn is_excluded(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let in_dir = |d: &str| path.split('/').rev().skip(1).any(|seg| seg == d);
    language_of(path) == "Lockfile"
        || matches!(
            name,
            "package-lock.json" | "npm-shrinkwrap.json" | "pnpm-lock.yaml" | "go.sum"
        )
        || [
            ".po", ".pot", ".mo", ".snap", ".pysnap", ".min.js", ".min.css", ".map",
        ]
        .iter()
        .any(|ext| name.ends_with(ext))
        || name.contains(".generated.")
        || name.ends_with("_pb2.py")
        || name.ends_with(".pb.go")
        || in_dir("migrations")
        || in_dir("__snapshots__")
        || in_dir("generated")
}

/// A commit's churn, split by what it did to the code. The three parts sum to churn.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    /// Lines that grew a file: added beyond what was removed from it.
    pub new: i64,
    /// Lines rewritten in place, counting both the removed and the added side.
    pub rework: i64,
    /// Lines removed beyond what was added back.
    pub deleted: i64,
}

impl Shape {
    pub fn churn(&self) -> i64 {
        self.new + self.rework + self.deleted
    }

    fn muda(&self) -> i64 {
        self.rework + self.deleted
    }

    fn add(&mut self, o: Shape) {
        self.new += o.new;
        self.rework += o.rework;
        self.deleted += o.deleted;
    }
}

pub fn shape_of(repo: &RepoData, c: &Commit, path: &Option<String>, l: Lines) -> Shape {
    let mut s = Shape::default();
    for ch in repo.changes_of(c) {
        let p = repo.s(ch.path);
        if is_excluded(p) || !path_matches(repo.s(ch.dir), p, path) {
            continue;
        }
        let (a, r) = (ch.added_of(l), ch.removed_of(l));
        let m = a.min(r);
        s.new += a - m;
        s.rework += 2 * m;
        s.deleted += r - m;
    }
    s
}

/// Annual maintenance each line of growth is expected to cost, by its age in days.
fn upkeep(age_days: f64, year1: f64, after: f64) -> f64 {
    if age_days < 365.25 {
        year1
    } else {
        after
    }
}

#[derive(Default)]
struct Tally {
    commits: f64,
    labelled: f64,
    value_add: f64,
}

impl Tally {
    /// Value-add is only measurable where most commits say what they are. Below
    /// that, the labelled few are not a sample of the rest.
    fn measurable(&self) -> bool {
        self.labelled > 0.0 && self.labelled * 2.0 >= self.commits
    }
}

pub fn value(
    cache: &Cache,
    ids: &Identities,
    f: &Filter,
    b: Bucket,
    l: Lines,
    year1: f64,
    after: f64,
) -> Output {
    let repos = select_repos(cache, f);
    // Code written before the window is still being maintained inside it, so the
    // model reads growth from the start of history and only reports the window.
    let history = Filter {
        since: None,
        ..f.clone()
    };
    // Two maps because a bucket can straddle `--since`: the model needs everything
    // before the window, the bands only what is inside it.
    let mut growth: HashMap<i32, Shape> = HashMap::new();
    let mut shown: HashMap<i32, Shape> = HashMap::new();
    let mut tally: HashMap<i32, Tally> = HashMap::new();
    let mut keys = Vec::new();
    let mut whole = Tally::default();

    for r in &repos {
        for res in resolve(r, ids, &history, false) {
            if !touches_path(&res, &f.path) {
                continue;
            }
            let c = res.commit;
            let (k, _) = bucket_key(c.days, b);
            let shape = shape_of(r, c, &f.path, l);
            growth.entry(k).or_default().add(shape);
            if !f.covers(c.days) {
                continue;
            }
            keys.push(k);
            shown.entry(k).or_default().add(shape);
            let label = intent_of_subject(r.s(c.subject));
            for t in [tally.entry(k).or_default(), &mut whole] {
                t.commits += 1.0;
                if label.is_some() {
                    t.labelled += 1.0;
                }
                if label == Some(Intent::ValueAdd) {
                    t.value_add += 1.0;
                }
            }
        }
    }

    let ax = axis(&keys, b);
    let band = |pick: fn(&Shape) -> i64| -> Vec<f64> {
        ax.iter()
            .map(|(k, _)| shown.get(k).map(|s| pick(s) as f64).unwrap_or(0.0))
            .collect()
    };

    let mut grown: Vec<(i32, f64)> = growth.iter().map(|(k, s)| (*k, s.new as f64)).collect();
    grown.sort_by_key(|(k, _)| *k);
    let expected: Vec<f64> = ax
        .iter()
        .map(|(k, _)| {
            let span = (next_bucket(*k, b) - k) as f64 / 365.25;
            grown
                .iter()
                .take_while(|(g, _)| g < k)
                .map(|(g, n)| n * upkeep((k - g) as f64, year1, after) * span)
                .sum()
        })
        .collect();

    // Value-add is read from the author's own label: whether a change adds capability
    // is a question of intent, which the shape of a diff can't answer. A month where
    // most commits are unlabelled is a gap in the line, not a zero.
    let overlay = whole.measurable().then(|| Series {
        name: "value-add % (feat: commits)".into(),
        points: ax
            .iter()
            .map(|(k, _)| match tally.get(k) {
                Some(t) if t.measurable() => 100.0 * t.value_add / t.labelled,
                _ => f64::NAN,
            })
            .collect(),
    });

    let window = shown.values().fold(Shape::default(), |mut acc, s| {
        acc.add(*s);
        acc
    });
    let share = |n: f64, d: f64| if d > 0.0 { 100.0 * n / d } else { 0.0 };
    let mut note = if whole.measurable() {
        format!(
            "{:.0}% of the {} commits with a conventional-commit prefix are feat: \
             (value-add); {:.0}% of all commits carry one.",
            share(whole.value_add, whole.labelled),
            group(whole.labelled as i64),
            share(whole.labelled, whole.commits),
        )
    } else {
        format!(
            "Only {:.0}% of {} commits carry a conventional-commit prefix, too few to \
             measure value-add.",
            share(whole.labelled, whole.commits),
            group(whole.commits as i64),
        )
    };
    let expected_total: f64 = expected.iter().sum();
    if expected_total > 0.0 {
        note.push_str(&format!(
            " Rework and deletion came to {:.2}× the expected maintenance.",
            window.muda() as f64 / expected_total,
        ));
    }
    note.push_str(&format!(
        " Net-new lines are {:.0}% of churn. Expected maintenance assumes each new line \
         costs {year1:?} lines of rework or deletion in its first year and {after:?} a \
         year after that. Lockfiles, migrations, test snapshots and generated files are \
         left out.",
        share(window.new as f64, window.churn() as f64),
    ));

    let unit = l.label();
    Output::Series {
        title: "Value-add vs muda".into(),
        subtitle: range_label(f, &repos),
        source: None,
        scope: None,
        x: ax.iter().map(|(_, lb)| lb.clone()).collect(),
        // Muda at the bottom of the stack, so the expected-maintenance line is read
        // against the top of the bands it models.
        series: vec![
            Series {
                name: format!("reworked {unit}"),
                points: band(|s| s.rework),
            },
            Series {
                name: format!("deleted {unit}"),
                points: band(|s| s.deleted),
            },
            Series {
                name: format!("net-new {unit}"),
                points: band(|s| s.new),
            },
        ],
        stacked: true,
        y_label: unit.into(),
        rate: false,
        overlay,
        overlay_label: Some("% of labelled commits".into()),
        overlay_rate: true,
        reference: vec![Series {
            name: "expected maintenance".into(),
            points: expected,
        }],
        note: Some(note),
        cite: Some(Cite {
            label: "Method: James Shore, Measuring AI's Unintended Consequences".into(),
            url: "https://www.jamesshore.com/v2/blog/2026/measuring-ais-unintended-consequences"
                .into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_labels() {
        use Intent::*;
        assert_eq!(
            intent_of_subject("feat(issues): add a filter"),
            Some(ValueAdd)
        );
        assert_eq!(intent_of_subject("feat!: drop the old api"), Some(ValueAdd));
        assert_eq!(intent_of_subject("fix: null check"), Some(Muda));
        assert_eq!(intent_of_subject("ref(ui): tidy"), Some(Muda));
        assert_eq!(
            intent_of_subject("Revert \"feat: add a filter\""),
            Some(Muda)
        );
        assert_eq!(intent_of_subject("build(deps): bump foo"), Some(Muda));
        assert_eq!(intent_of_subject("Add a filter"), None);
        assert_eq!(intent_of_subject("wip: something"), None);
        assert_eq!(intent_of_subject("feat(unclosed: x"), None);
        assert_eq!(intent_of_subject(""), None);
    }

    #[test]
    fn exclusions() {
        for p in [
            "pnpm-lock.yaml",
            "Cargo.lock",
            "src/sentry/locale/de/LC_MESSAGES/django.po",
            "src/sentry/migrations/0001_initial.py",
            "static/app/__snapshots__/foo.tsx.snap",
            "tests/sentry/grouping/snapshots/test_variants/python.pysnap",
            "static/dist/app.min.js",
            "api/generated/client.ts",
        ] {
            assert!(is_excluded(p), "{p} should be excluded");
        }
        for p in [
            "src/sentry/api/endpoints/issues.py",
            "static/app/views/issues.tsx",
            "tests/sentry/test_migrations_helper.py",
            "generated",
        ] {
            assert!(!is_excluded(p), "{p} should count");
        }
    }

    #[test]
    fn measurable_needs_most_commits_labelled() {
        let t = |commits, labelled| Tally {
            commits,
            labelled,
            value_add: 0.0,
        };
        assert!(t(10.0, 5.0).measurable());
        assert!(!t(10.0, 4.0).measurable());
        assert!(!t(0.0, 0.0).measurable());
    }
}
