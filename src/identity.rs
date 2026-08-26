use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssistKind {
    Human,
    /// Human author, agent credited as a co-author.
    AgentAssisted,
    /// No human wrote it: an agent is the author, or a bot landed an agent's work.
    Agent,
    /// Automation that does not write code — dependency bumps, releases, reverts, CI.
    Bot,
}

impl AssistKind {
    pub fn label(&self) -> &'static str {
        match self {
            AssistKind::Human => "human",
            AssistKind::AgentAssisted => "agent_assisted",
            AssistKind::Agent => "agent",
            AssistKind::Bot => "bot",
        }
    }
    pub fn all() -> [AssistKind; 4] {
        [
            AssistKind::Human,
            AssistKind::AgentAssisted,
            AssistKind::Agent,
            AssistKind::Bot,
        ]
    }
    /// Did an agent write this code, alone or alongside someone?
    pub fn is_agent(&self) -> bool {
        matches!(self, AssistKind::Agent | AssistKind::AgentAssisted)
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
    // --- coding agents -------------------------------------------------------
    // Keyed on the numeric GitHub id wherever one exists, because bots get
    // renamed: 157164994 appears as sentry-autofix[bot], seer-by-sentry[bot],
    // sentry-autofix-experimental[bot] and sentry-ai-autofix-experimental[bot].
    ("github_user_id", "157164994", "seer", ToolKind::Agent),
    ("github_user_id", "264270552", "jr", ToolKind::Agent),
    ("github_user_id", "206951365", "cursor", ToolKind::Agent),
    ("email", "noreply@anthropic.com", "claude", ToolKind::Agent),
    ("email", "noreply@openai.com", "codex", ToolKind::Agent),
    ("email", "cursoragent@cursor.com", "cursor", ToolKind::Agent),
    ("email_domain", "anthropic.com", "claude", ToolKind::Agent),
    ("email_domain", "openai.com", "codex", ToolKind::Agent),
    ("email_domain", "cursor.com", "cursor", ToolKind::Agent),
    ("email_domain", "devin.ai", "devin", ToolKind::Agent),
    // Fallbacks for identities carrying an unhelpful address, e.g. the commits
    // from "Claude Opus 4.6 <noreply@example.com>".
    ("name_prefix", "claude", "claude", ToolKind::Agent),
    ("name_prefix", "codex", "codex", ToolKind::Agent),
    ("name_prefix", "cursor", "cursor", ToolKind::Agent),
    ("name_prefix", "devin", "devin", ToolKind::Agent),
    ("name_prefix", "seer", "seer", ToolKind::Agent),
    ("name_prefix", "sentry-autofix", "seer", ToolKind::Agent),
    ("name_prefix", "sentry-ai-autofix", "seer", ToolKind::Agent),
    ("name_prefix", "sentry-junior", "jr", ToolKind::Agent),

    // --- automation that does not write code ---------------------------------
    ("github_user_id", "49699333", "dependabot", ToolKind::Infra),
    ("github_user_id", "66042841", "getsantry", ToolKind::Infra),
    ("github_user_id", "212413796", "devinfra", ToolKind::Infra),
    ("github_user_id", "180476844", "sentry-release", ToolKind::Infra),
    ("github_user_id", "57668832", "license-bump", ToolKind::Infra),
    ("github_user_id", "41898282", "github-actions", ToolKind::Infra),
    ("github_user_id", "200264868", "semgrep", ToolKind::Infra),
    // Reverting is not authoring, despite the name.
    ("github_user_id", "2129822", "seer-revert", ToolKind::Infra),
    ("name_prefix", "sentry-seer-fast-revert", "seer-revert", ToolKind::Infra),
    // A deterministic fixer rather than an LLM.
    ("github_user_id", "260785270", "fix-it-felix", ToolKind::Infra),
    ("name_prefix", "fix-it-felix", "fix-it-felix", ToolKind::Infra),
    // getsentry's own automation commits under two addresses and two spellings.
    ("email", "bot@sentry.io", "getsentry-bot", ToolKind::Infra),
    ("email", "bot@getsentry.com", "getsentry-bot", ToolKind::Infra),
    ("name_prefix", "getsentry-bot", "getsentry-bot", ToolKind::Infra),
    ("name_prefix", "sentry bot", "getsentry-bot", ToolKind::Infra),
    ("name_prefix", "getsantry", "getsantry", ToolKind::Infra),
    ("name_prefix", "dependabot", "dependabot", ToolKind::Infra),
    ("name_prefix", "devinfra", "devinfra", ToolKind::Infra),
    ("name_prefix", "renovate", "renovate", ToolKind::Infra),
    ("name_prefix", "github-actions", "github-actions", ToolKind::Infra),
    ("name_prefix", "semgrep", "semgrep", ToolKind::Infra),
    // Security scanners: "Snyk bot" has no [bot] suffix and would otherwise read
    // as a person.
    ("email_domain", "snyk.io", "snyk", ToolKind::Infra),
    ("name_prefix", "snyk", "snyk", ToolKind::Infra),
    ("name_prefix", "sentry-release-bot", "sentry-release", ToolKind::Infra),
    ("name_prefix", "sentry-update-license", "license-bump", ToolKind::Infra),
    // The Sentry GitHub App identity, confirmed as an AI agent. It opens fix PRs
    // under its own name with the affected code owners credited as co-authors, so
    // the humans on those commits are reviewers rather than the ones who wrote it.
    ("github_user_id", "39604003", "sentry-agent", ToolKind::Agent),
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

    /// A real person: matches no agent or infra rule, and doesn't look like a bot.
    /// Used to count how many humans were active in a period, which has to consider
    /// co-authors too — pairing and agent-assisted work both put people there.
    pub fn is_human(&self, name: &str, email: &str) -> bool {
        self.match_identity(name, email).is_none() && !Self::looks_like_bot(name)
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
        n.ends_with("[bot]") || n.ends_with("-bot") || n.ends_with(" bot") || n == "bot"
    }

    /// Returns the author's classification and every agent tool credited on the
    /// commit, derived fresh on each read from the raw strings in the cache.
    /// Returns the commit's classification and every agent tool credited on it.
    ///
    /// Order matters. An agent can be the sole author — Seer, Junior and Cursor all
    /// open PRs under their own identity — so checking co-authors first, or treating
    /// every `[bot]` as automation, files real agent work alongside Dependabot.
    pub fn classify(
        &self,
        author_name: &str,
        author_email: &str,
        coauthors: &[(String, String)],
    ) -> (AssistKind, Vec<String>) {
        let mut tools: Vec<String> = Vec::new();
        let push = |t: &str, tools: &mut Vec<String>| {
            if !tools.iter().any(|x| x == t) {
                tools.push(t.to_string());
            }
        };

        // Coding agents usually appear as co-authors: Claude, Codex and Cursor sit in
        // Co-authored-by trailers while the author stays the person who ran them.
        let mut agent_coauthor = false;
        for (n, e) in coauthors {
            if let Some(r) = self.match_identity(n, e) {
                if r.kind == ToolKind::Agent {
                    agent_coauthor = true;
                    push(&r.tool, &mut tools);
                }
            }
        }

        let author_rule = self.match_identity(author_name, author_email);
        if let Some(r) = author_rule {
            push(&r.tool, &mut tools);
            return match r.kind {
                // The agent wrote it under its own name, with no human author.
                ToolKind::Agent => (AssistKind::Agent, tools),
                // Automation authored it. If an agent is credited, the code still
                // came from the agent and the bot only landed it.
                ToolKind::Infra if agent_coauthor => (AssistKind::Agent, tools),
                ToolKind::Infra => (AssistKind::Bot, tools),
            };
        }

        // No rule matched. A `[bot]` suffix is still a real signal, because GitHub App
        // identities always render that way.
        if Self::looks_like_bot(author_name) {
            return if agent_coauthor {
                (AssistKind::Agent, tools)
            } else {
                (AssistKind::Bot, tools)
            };
        }

        if agent_coauthor {
            return (AssistKind::AgentAssisted, tools);
        }
        (AssistKind::Human, tools)
    }
}
