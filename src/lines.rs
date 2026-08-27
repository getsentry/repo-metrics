//! Splitting text lines into blank, comment and code.
//!
//! Git counts lines, not source lines: a diff that adds forty blank lines and a
//! licence header reads exactly like one that adds forty lines of logic. Keeping
//! the three apart is what lets "lines" mean something specific, and it is the only
//! way to watch comment volume move on its own — which is worth watching when a
//! growing share of the code is written by agents.

/// What a single line of a file is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Blank,
    Comment,
    Code,
}

/// Which lines a metric counts. Applies to every line measure — churn, added,
/// removed, modified, and folder sizes — so one control answers "lines of what?".
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Debug, Default)]
pub enum Lines {
    /// Every line, exactly as git counts it. The default, so no existing number
    /// moves when the toggle is added.
    #[default]
    All,
    /// Everything except blank lines.
    NonBlank,
    /// Code only: no blank lines, no comments.
    Code,
    /// Comment lines only.
    Comments,
}

impl Lines {
    pub fn label(&self) -> &'static str {
        match self {
            Lines::All => "lines",
            Lines::NonBlank => "non-blank lines",
            Lines::Code => "code lines",
            Lines::Comments => "comment lines",
        }
    }
    /// Suffix for a chart title. `All` adds nothing, since it is what "lines" has
    /// always meant here.
    pub fn suffix(&self) -> &'static str {
        match self {
            Lines::All => "",
            Lines::NonBlank => " (non-blank)",
            Lines::Code => " (code only)",
            Lines::Comments => " (comments only)",
        }
    }
    /// Given a total and its blank/comment parts, how many lines this mode counts.
    /// Code is the remainder rather than its own counter, so the three always sum
    /// back to the total git reported.
    pub fn of(&self, total: i64, comment: i64, blank: i64) -> i64 {
        match self {
            Lines::All => total,
            Lines::NonBlank => (total - blank).max(0),
            Lines::Code => (total - blank - comment).max(0),
            Lines::Comments => comment,
        }
    }
}

/// How a language marks comments. This is a lexical approximation, not a parser:
/// enough to classify a line from its own text, which is all a diff hunk gives us.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// No comment syntax worth recognising — JSON, lockfiles, binary assets.
    None,
    /// `#` to end of line.
    Hash,
    /// `#`, plus `"""` and `''' `docstrings, which count as comments the way most
    /// line counters treat them.
    Python,
    /// `//` to end of line, and `/* */` blocks.
    Slash,
    /// `/* */` blocks only.
    Block,
    /// `--` to end of line, and `/* */` blocks.
    Sql,
    /// `<!-- -->`.
    Html,
}

/// Comment syntax by file extension.
///
/// Deliberately separate from `language_of`, which names a language for display.
/// These two answer different questions — `.h` is "C" to a reader but shares its
/// comment syntax with C++ — and tying them together would make one wrong to keep
/// the other right.
pub fn style_of(path: &str) -> Style {
    let ext = match path.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => return Style::None,
    };
    match ext.as_str() {
        "py" | "pyi" | "pyx" => Style::Python,
        // `po`/`pot` are gettext catalogues. Worth naming explicitly: in sentry they
        // are the single largest extension in the tree, and leaving them unmapped
        // filed 1.8M lines of translation catalogue — most of it `#` metadata — as
        // source code.
        "rb" | "sh" | "bash" | "zsh" | "fish" | "yml" | "yaml" | "toml" | "cfg" | "ini"
        | "conf" | "pl" | "pm" | "r" | "tf" | "dockerfile" | "mk" | "gemfile" | "po" | "pot"
        | "env" | "properties" => Style::Hash,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "rs" | "go" | "java"
        | "kt" | "kts" | "swift" | "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "cs" | "php"
        | "scala" | "proto" | "graphql" | "gql" | "dart" | "groovy" | "m" | "mm" => Style::Slash,
        // SCSS and LESS also take `//`, but `/* */` is the shared case and treating
        // them as block-only never misreads a line of code as a comment.
        "css" => Style::Block,
        "scss" | "sass" | "less" => Style::Slash,
        "sql" => Style::Sql,
        "html" | "htm" | "xml" | "vue" | "svelte" | "md" | "mdx" => Style::Html,
        _ => Style::None,
    }
}

/// Open block-comment state: the delimiter we are waiting to see. `None` means we
/// are not inside a block. Carrying the terminator rather than a bare flag is what
/// keeps a Python docstring from being closed by a stray `*/`.
pub type Block = Option<&'static str>;

/// Classify one line.
///
/// `block` carries block-comment state between lines. Reading a whole file it is
/// threaded through and the answer is exact. Reading a diff there is no reliable
/// state to carry — a hunk shows changed lines, not the file around them — so
/// callers pass a fresh `None` each line and accept a lexical approximation. The
/// approximation is deliberately conservative: it credits a line as a comment only
/// on direct evidence in the line itself, so it under-counts comments rather than
/// mistaking code for one.
pub fn classify(style: Style, line: &str, block: &mut Block) -> Kind {
    let t = line.trim();
    if t.is_empty() {
        // Blank wins over block state, even in the middle of a docstring. It keeps
        // "blank" meaning exactly "no visible characters", which is both what a
        // reader expects and what cloc reports, so the two can be compared.
        return Kind::Blank;
    }
    if let Some(end) = *block {
        return match t.split_once(end) {
            Some((_, after)) => {
                *block = None;
                // Code after the terminator makes this a code line.
                if after.trim().is_empty() {
                    Kind::Comment
                } else {
                    Kind::Code
                }
            }
            None => Kind::Comment,
        };
    }
    match style {
        Style::None => Kind::Code,
        Style::Hash => starts_with_any(t, &["#"]),
        Style::Python => {
            if t.starts_with('#') {
                return Kind::Comment;
            }
            for q in ["\"\"\"", "'''"] {
                if t.starts_with(q) {
                    return open_block(t, q, q, block);
                }
            }
            Kind::Code
        }
        Style::Slash => block_or_line(t, &["//"], "/*", "*/", block),
        Style::Block => block_or_line(t, &[], "/*", "*/", block),
        Style::Sql => block_or_line(t, &["--"], "/*", "*/", block),
        Style::Html => block_or_line(t, &[], "<!--", "-->", block),
    }
}

fn starts_with_any(t: &str, markers: &[&str]) -> Kind {
    if markers.iter().any(|m| t.starts_with(m)) {
        Kind::Comment
    } else {
        Kind::Code
    }
}

fn block_or_line(
    t: &str,
    markers: &[&str],
    open: &'static str,
    close: &'static str,
    block: &mut Block,
) -> Kind {
    if markers.iter().any(|m| t.starts_with(m)) {
        return Kind::Comment;
    }
    // A continuation line of a `/** ... */` block, which is how most of these are
    // written and the shape a diff hunk shows most often.
    if close == "*/" && t.starts_with('*') {
        return Kind::Comment;
    }
    if t.starts_with(open) {
        return open_block(t, open, close, block);
    }
    Kind::Code
}

/// A line that opens a block. If the block also closes on this line the state stays
/// clear, and anything after the terminator makes the line code.
fn open_block(t: &str, open: &'static str, close: &'static str, block: &mut Block) -> Kind {
    let rest = &t[open.len()..];
    match rest.split_once(close) {
        Some((_, after)) if !after.trim().is_empty() => Kind::Code,
        Some(_) => Kind::Comment,
        None => {
            *block = Some(close);
            Kind::Comment
        }
    }
}

/// Count a whole file's lines by kind. Exact: block state is threaded through.
pub fn count_file(path: &str, text: &str) -> (u64, u64, u64) {
    let style = style_of(path);
    let (mut blank, mut comment, mut code) = (0u64, 0u64, 0u64);
    let mut block: Block = None;
    for l in text.lines() {
        match classify(style, l, &mut block) {
            Kind::Blank => blank += 1,
            Kind::Comment => comment += 1,
            Kind::Code => code += 1,
        }
    }
    (code, comment, blank)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(path: &str, text: &str) -> (u64, u64, u64) {
        count_file(path, text)
    }

    #[test]
    fn python_hash_and_docstrings() {
        let src = "import os\n\n# a comment\ndef f():\n    \"\"\"Doc.\n\n    More.\n    \"\"\"\n    return 1\n";
        // code: import, def, return = 3; comment: # plus 3 docstring lines = 4;
        // blank: the one before the comment and the one inside the docstring = 2
        assert_eq!(counts("a.py", src), (3, 4, 2));
    }

    #[test]
    fn one_line_docstring_does_not_open_a_block() {
        let src = "def f():\n    \"\"\"Doc.\"\"\"\n    return 1\n";
        assert_eq!(counts("a.py", src), (2, 1, 0));
    }

    #[test]
    fn slash_line_and_block() {
        let src = "const a=1;\n\n// line\n/* open\n * mid\n */\nconst b=2;\n";
        assert_eq!(counts("a.ts", src), (2, 4, 1));
    }

    #[test]
    fn block_opened_and_closed_on_one_line() {
        let mut b: Block = None;
        // Nothing after the terminator: a comment, and no block left open.
        assert_eq!(classify(Style::Slash, "/* c */", &mut b), Kind::Comment);
        assert_eq!(b, None);
        // Code after the terminator: the line is code.
        assert_eq!(
            classify(Style::Slash, "/* c */ const a=1;", &mut b),
            Kind::Code
        );
        assert_eq!(b, None);
    }

    #[test]
    fn a_docstring_is_not_closed_by_a_stray_block_terminator() {
        let src = "def f():\n    \"\"\"Doc\n    a */ b\n    \"\"\"\n    return 1\n";
        // The `*/` must not end a Python docstring.
        assert_eq!(counts("a.py", src), (2, 3, 0));
    }

    #[test]
    fn html_and_css() {
        assert_eq!(counts("a.html", "<p>x</p>\n<!-- c -->\n"), (1, 1, 0));
        assert_eq!(counts("a.css", ".a{}\n/* c */\n"), (1, 1, 0));
    }

    #[test]
    fn json_has_no_comments() {
        assert_eq!(counts("a.json", "{\n  \"a\": 1\n}\n"), (3, 0, 0));
    }

    #[test]
    fn a_blank_line_inside_a_block_is_still_blank() {
        let src = "/* a\n\n b */\ncode\n";
        assert_eq!(counts("a.rs", src), (1, 2, 1));
    }

    #[test]
    fn gettext_catalogues_are_hash_commented() {
        assert_eq!(
            style_of("src/sentry/locale/fr/LC_MESSAGES/django.po"),
            Style::Hash
        );
    }

    #[test]
    fn lines_modes_sum_back_to_the_total() {
        let (total, comment, blank) = (100i64, 20, 15);
        assert_eq!(Lines::All.of(total, comment, blank), 100);
        assert_eq!(Lines::NonBlank.of(total, comment, blank), 85);
        assert_eq!(Lines::Code.of(total, comment, blank), 65);
        assert_eq!(Lines::Comments.of(total, comment, blank), 20);
    }
}
