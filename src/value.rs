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
        || [".po", ".pot", ".mo", ".snap", ".min.js", ".min.css", ".map"]
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

    /// Value-add when most of the churn is growth. The comparison is strict so that
    /// moving code between files, which is exactly as much deletion as growth, reads
    /// as the rework it is. A commit that changed no countable lines — a lockfile
    /// bump, a binary swap — is muda.
    pub fn intent(&self) -> Intent {
        if 2 * self.new > self.churn() {
            Intent::ValueAdd
        } else {
            Intent::Muda
        }
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
    value_add: f64,
    labelled: f64,
    labelled_value_add: f64,
    /// Labelled commits the diff shape also calls value-add, split by their label.
    /// Where the two disagree says more than how often they agree.
    feat_grew: f64,
    other_grew: f64,
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
            let by_shape = shape.intent();
            let by_label = intent_of_subject(r.s(c.subject));
            for t in [tally.entry(k).or_default(), &mut whole] {
                t.commits += 1.0;
                if by_shape == Intent::ValueAdd {
                    t.value_add += 1.0;
                }
                if let Some(label) = by_label {
                    t.labelled += 1.0;
                    if label == Intent::ValueAdd {
                        t.labelled_value_add += 1.0;
                    }
                    if by_shape == Intent::ValueAdd {
                        match label {
                            Intent::ValueAdd => t.feat_grew += 1.0,
                            Intent::Muda => t.other_grew += 1.0,
                        }
                    }
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
    let pct = |num: fn(&Tally) -> f64, den: fn(&Tally) -> f64| -> Vec<f64> {
        ax.iter()
            .map(|(k, _)| match tally.get(k) {
                Some(t) if den(t) > 0.0 => 100.0 * num(t) / den(t),
                _ => 0.0,
            })
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

    // A prefix line drawn from a handful of labelled commits would look like a
    // measurement of the whole repo, so it only appears when most commits carry one.
    let labelled_enough = whole.labelled * 2.0 >= whole.commits && whole.labelled > 0.0;
    let overlay_extra = if labelled_enough {
        vec![Series {
            name: "value-add % (feat: prefix)".into(),
            points: pct(|t| t.labelled_value_add, |t| t.labelled),
        }]
    } else {
        Vec::new()
    };

    let window = shown.values().fold(Shape::default(), |mut acc, s| {
        acc.add(*s);
        acc
    });
    let share = |n: f64, d: f64| if d > 0.0 { 100.0 * n / d } else { 0.0 };
    let mut note = format!(
        "{:.0}% of {} commits are value-add by diff shape",
        share(whole.value_add, whole.commits),
        group(whole.commits as i64),
    );
    if labelled_enough {
        note.push_str(&format!(
            ", {:.0}% by feat: prefix. Diff shape reads {:.0}% of feat: commits as \
             value-add, and {:.0}% of the other labelled commits",
            share(whole.labelled_value_add, whole.labelled),
            share(whole.feat_grew, whole.labelled_value_add),
            share(whole.other_grew, whole.labelled - whole.labelled_value_add),
        ));
    } else {
        note.push_str(&format!(
            "; too few commits carry a conventional prefix to cross-check ({:.0}%)",
            share(whole.labelled, whole.commits),
        ));
    }
    note.push_str(&format!(
        ". Net-new lines are {:.0}% of churn. Expected maintenance assumes each new line \
         costs {year1:?} lines of rework or deletion in its first year and {after:?} a \
         year after that. Lockfiles, migrations, snapshots and generated files are left \
         out.",
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
        overlay: Some(Series {
            name: "value-add % (diff shape)".into(),
            points: pct(|t| t.value_add, |t| t.commits),
        }),
        overlay_label: Some("% of commits".into()),
        overlay_extra,
        overlay_rate: true,
        reference: vec![Series {
            name: "expected maintenance".into(),
            points: expected,
        }],
        note: Some(note),
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
    fn shape_intent() {
        let s = |new, rework, deleted| Shape {
            new,
            rework,
            deleted,
        };
        assert_eq!(s(100, 0, 0).intent(), Intent::ValueAdd);
        assert_eq!(s(100, 40, 20).intent(), Intent::ValueAdd);
        // A move between files: as much deleted as grown.
        assert_eq!(s(100, 0, 100).intent(), Intent::Muda);
        assert_eq!(s(10, 200, 0).intent(), Intent::Muda);
        assert_eq!(s(0, 0, 0).intent(), Intent::Muda);
    }
}
