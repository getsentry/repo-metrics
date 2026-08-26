use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssistKind {
    Human,
    AgentAssisted,
    Bot,
}

impl AssistKind {
    pub fn label(&self) -> &'static str {
        match self {
            AssistKind::Human => "human",
            AssistKind::AgentAssisted => "agent_assisted",
            AssistKind::Bot => "bot",
        }
    }
    pub fn all() -> [AssistKind; 3] {
        [AssistKind::Human, AssistKind::AgentAssisted, AssistKind::Bot]
    }
}

/// `agent` writes code; `infra` opens dependency PRs and reverts. Both are bots as
/// far as authorship goes, but rolling them together would report Dependabot as an
/// AI coding assistant.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    Agent,
    Infra,
}

#[derive(Clone, Deserialize)]
pub struct Rule {
    /// github_user_id | email | email_domain | name_prefix
    pub match_kind: String,
    pub match_value: String,
    pub tool: String,
    pub kind: ToolKind,
    #[serde(default)]
    pub priority: i64,
}

#[derive(Deserialize, Default)]
struct RuleFile {
    #[serde(default)]
    identity: Vec<Rule>,
}

pub struct Identities {
    rules: Vec<Rule>,
}

fn default_priority(kind: &str) -> i64 {
    match kind {
        "github_user_id" => 100,
        "email" => 80,
        "email_domain" => 60,
        "name_prefix" => 40,
        _ => 10,
    }
}

/// Longer prefixes are more specific and must win: without this, `devin` (the
/// coding agent) captures `devinfra-flakiness[bot]`, which is CI infrastructure.
fn rule_priority(kind: &str, value: &str) -> i64 {
    let base = default_priority(kind);
    if kind == "name_prefix" {
        base + value.chars().count() as i64
    } else {
        base
    }
}

/// Shipped defaults. Every one of these is a real identity observed in getsentry
/// history; the file at ~/.config/repo-metrics/identities.toml adds to or overrides
/// them without touching the cache.
const DEFAULTS: &[(&str, &str, &str, ToolKind)] = &[
    // Keyed on the numeric GitHub id where one exists, because bots get renamed:
    // 157164994 appears as sentry-autofix[bot], seer-by-sentry[bot],
    // sentry-autofix-experimental[bot] and sentry-ai-autofix-experimental[bot].
    ("github_user_id", "157164994", "seer", ToolKind::Agent),
    ("email", "noreply@anthropic.com", "claude", ToolKind::Agent),
    ("email", "noreply@openai.com", "codex", ToolKind::Agent),
    ("email", "cursoragent@cursor.com", "cursor", ToolKind::Agent),
    ("email_domain", "anthropic.com", "claude", ToolKind::Agent),
    ("email_domain", "openai.com", "codex", ToolKind::Agent),
    ("email_domain", "cursor.com", "cursor", ToolKind::Agent),
    ("email_domain", "devin.ai", "devin", ToolKind::Agent),
    // Fallback for identities carrying a wrong address, e.g. the 72 commits from
    // "Claude Opus 4.6 <noreply@example.com>".
    ("name_prefix", "claude", "claude", ToolKind::Agent),
    ("name_prefix", "codex", "codex", ToolKind::Agent),
    ("name_prefix", "cursor", "cursor", ToolKind::Agent),
    ("name_prefix", "devin", "devin", ToolKind::Agent),
    ("name_prefix", "seer", "seer", ToolKind::Agent),
    ("name_prefix", "sentry-autofix", "seer", ToolKind::Agent),
    ("name_prefix", "sentry-junior", "jr", ToolKind::Agent),
    ("name_prefix", "sentry-ai", "seer", ToolKind::Agent),
    // Infrastructure: matches no [bot] rule by address alone.
    ("email", "bot@sentry.io", "getsantry", ToolKind::Infra),
    ("name_prefix", "getsentry-bot", "getsentry", ToolKind::Infra),
    ("name_prefix", "getsantry", "getsantry", ToolKind::Infra),
    ("name_prefix", "dependabot", "dependabot", ToolKind::Infra),
    ("name_prefix", "devinfra", "devinfra", ToolKind::Infra),
    ("name_prefix", "sentry-io", "getsentry", ToolKind::Infra),
    ("name_prefix", "renovate", "renovate", ToolKind::Infra),
    ("name_prefix", "github-actions", "github-actions", ToolKind::Infra),
];

pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("REPO_METRICS_IDENTITIES") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".config")
        .join("repo-metrics")
        .join("identities.toml")
}

impl Identities {
    pub fn load() -> Self {
        let mut rules: Vec<Rule> = DEFAULTS
            .iter()
            .map(|(k, v, t, kind)| Rule {
                match_kind: k.to_string(),
                match_value: v.to_lowercase(),
                tool: t.to_string(),
                kind: *kind,
                priority: rule_priority(k, v),
            })
            .collect();

        let p = config_path();
        if let Ok(text) = std::fs::read_to_string(&p) {
            match toml::from_str::<RuleFile>(&text) {
                Ok(f) => {
                    for mut r in f.identity {
                        if r.priority == 0 {
                            r.priority = rule_priority(&r.match_kind, &r.match_value);
                        }
                        r.match_value = r.match_value.to_lowercase();
                        // User rules win ties against the shipped defaults.
                        r.priority += 1;
                        rules.push(r);
                    }
                }
                Err(e) => eprintln!("warning: ignoring {}: {e}", p.display()),
            }
        }
        rules.sort_by_key(|r| -r.priority);
        Self { rules }
    }

    /// Modern GitHub noreply addresses embed the numeric user id: `1234+login@...`.
    /// That id is the only stable handle on a bot that has been renamed.
    fn github_id(email: &str) -> Option<&str> {
        let local = email.split('@').next()?;
        let (id, _) = local.split_once('+')?;
        if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
            Some(id)
        } else {
            None
        }
    }

    pub fn match_identity(&self, name: &str, email: &str) -> Option<&Rule> {
        let name_l = name.to_lowercase();
        let email_l = email.to_lowercase();
        let domain = email_l.split('@').nth(1).unwrap_or("");
        let gid = Self::github_id(&email_l);

        for r in &self.rules {
            let hit = match r.match_kind.as_str() {
                "github_user_id" => gid == Some(r.match_value.as_str()),
                "email" => email_l == r.match_value,
                "email_domain" => domain == r.match_value,
                "name_prefix" => name_l.starts_with(&r.match_value),
                _ => false,
            };
            if hit {
                return Some(r);
            }
        }
        None
    }

    /// True for identities that are bots but match no specific rule. A `[bot]`
    /// suffix is a real signal — GitHub App identities always render that way.
    ///
    /// `@users.noreply.github.com` deliberately is NOT a signal here: it is the
    /// default privacy address for ordinary accounts, and treating it as one files
    /// most of the engineering team as robots.
    fn looks_like_bot(name: &str) -> bool {
        let n = name.trim().to_lowercase();
        n.ends_with("[bot]") || n.ends_with("-bot") || n == "bot"
    }

    /// Returns the author's classification and every agent tool credited on the
    /// commit, derived fresh on each read from the raw strings in the cache.
    pub fn classify(
        &self,
        author_name: &str,
        author_email: &str,
        coauthors: &[(String, String)],
    ) -> (AssistKind, Vec<String>) {
        let mut tools: Vec<String> = Vec::new();

        let author_rule = self.match_identity(author_name, author_email);
        let author_is_bot = author_rule.is_some() || Self::looks_like_bot(author_name);

        // The coding agents show up as co-authors, not authors: Claude, Codex and
        // Cursor sit in Co-authored-by trailers while the commit's author stays the
        // human who ran them. Classifying on the author field alone reports almost
        // no agent activity at all.
        for (n, e) in coauthors {
            if let Some(r) = self.match_identity(n, e) {
                if r.kind == ToolKind::Agent && !tools.iter().any(|t| t == &r.tool) {
                    tools.push(r.tool.clone());
                }
            }
        }

        if author_is_bot {
            if let Some(r) = author_rule {
                if !tools.iter().any(|t| t == &r.tool) {
                    tools.push(r.tool.clone());
                }
            }
            return (AssistKind::Bot, tools);
        }
        if !tools.is_empty() {
            return (AssistKind::AgentAssisted, tools);
        }
        (AssistKind::Human, tools)
    }
}
