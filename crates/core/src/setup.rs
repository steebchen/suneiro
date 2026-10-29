//! Agent detection and user settings. We never store agent credentials: each
//! agent keeps using its own CLI login (Claude subscription, ChatGPT, ...), and
//! we only help run the official install/login commands.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::env::tokio_command;

/// A featured model in the picker: which agent, which model, which effort.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoadoutEntry {
    pub agent: String,
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Featured models, in order. The first one is the default for new chats.
    pub loadout: Vec<LoadoutEntry>,
    /// OpenCode models offered in the picker (it has hundreds).
    pub opencode_models: Vec<String>,
    pub plan_by_default: bool,
    pub enabled_agents: Vec<String>,
    pub default_agent: String,
    pub branch_prefix: String,
    pub workspaces_root: String,
    /// App name passed to `open -a`, or empty for Finder.
    pub editor: String,
    /// "system" | "light" | "dark"
    pub theme: String,
    /// Rename a workspace's branch after its task (until it's pushed).
    pub rename_branches: bool,
    /// Maximum parent links for a chat; roots have depth zero (1..=8).
    pub max_tab_depth: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            loadout: Vec::new(),
            opencode_models: Vec::new(),
            plan_by_default: false,
            enabled_agents: vec!["claude".into(), "codex".into(), "opencode".into()],
            default_agent: "claude".into(),
            branch_prefix: "suneiro/".into(),
            workspaces_root: dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join("suneiro").join("workspaces").to_string_lossy().into_owned(),
            editor: "Visual Studio Code".into(),
            theme: "system".into(),
            rename_branches: true,
            max_tab_depth: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
    pub logged_in: bool,
    /// Human readable account info, e.g. "you@example.com · max".
    pub account: Option<String>,
    pub install_command: String,
    pub login_command: String,
    pub logout_command: String,
}

async fn run(program: &str, args: &[&str]) -> Option<(bool, String)> {
    let out = tokio::time::timeout(std::time::Duration::from_secs(15), tokio_command(program).args(args).stdin(std::process::Stdio::null()).output())
        .await
        .ok()?
        .ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some((out.status.success(), text))
}

fn first_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()) && w.contains('.'))
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.').to_string())
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub async fn detect(id: &str) -> Option<AgentStatus> {
    let mut status = match id {
        "claude" => AgentStatus {
            id: id.into(),
            name: "Claude Code".into(),
            installed: false,
            version: None,
            logged_in: false,
            account: None,
            install_command: "curl -fsSL https://claude.ai/install.sh | bash".into(),
            login_command: "claude auth login".into(),
            logout_command: "claude auth logout".into(),
        },
        "codex" => AgentStatus {
            id: id.into(),
            name: "Codex".into(),
            installed: false,
            version: None,
            logged_in: false,
            account: None,
            install_command: "npm install -g @openai/codex".into(),
            login_command: "codex login".into(),
            logout_command: "codex logout".into(),
        },
        "opencode" => AgentStatus {
            id: id.into(),
            name: "OpenCode".into(),
            installed: false,
            version: None,
            logged_in: false,
            account: None,
            install_command: "curl -fsSL https://opencode.ai/install | bash".into(),
            login_command: "opencode auth login".into(),
            logout_command: "opencode auth logout".into(),
        },
        _ => return None,
    };

    let Some((true, version)) = run(id, &["--version"]).await else {
        return Some(status);
    };
    status.installed = true;
    status.version = first_version(&version);

    match id {
        "claude" => {
            if let Some((_, out)) = run("claude", &["auth", "status"]).await {
                if let Ok(v) = serde_json::from_str::<Value>(&out) {
                    status.logged_in = v["loggedIn"].as_bool().unwrap_or(false);
                    let email = v["email"].as_str();
                    let plan = v["subscriptionType"].as_str().or(v["authMethod"].as_str());
                    status.account = match (email, plan) {
                        (Some(e), Some(p)) => Some(format!("{e} · {p}")),
                        (Some(e), None) => Some(e.into()),
                        (None, p) => p.map(str::to_string),
                    };
                }
            }
        }
        "codex" => {
            if let Some((ok, out)) = run("codex", &["login", "status"]).await {
                let line = out.lines().find(|l| l.contains("Logged in")).map(str::trim);
                status.logged_in = ok && line.is_some();
                status.account = line.map(|l| l.replace("Logged in using ", "")).filter(|_| status.logged_in);
            }
        }
        "opencode" => {
            if let Some((_, out)) = run("opencode", &["auth", "list"]).await {
                let out = strip_ansi(&out);
                let providers: Vec<String> = out
                    .split("Environment")
                    .next()
                    .unwrap_or("")
                    .lines()
                    .filter_map(|l| l.trim_start().strip_prefix('●'))
                    .map(|l| l.split_whitespace().take_while(|w| *w != "api" && *w != "oauth").collect::<Vec<_>>().join(" "))
                    .collect();
                status.logged_in = !providers.is_empty();
                status.account = (!providers.is_empty()).then(|| providers.join(", "));
            }
        }
        _ => {}
    }
    Some(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(first_version("2.1.284 (Claude Code)").as_deref(), Some("2.1.284"));
        assert_eq!(first_version("codex-cli 0.140.0").as_deref(), Some("0.140.0"));
    }

    #[test]
    fn settings_fill_defaults() {
        let s: Settings = serde_json::from_str(r#"{"defaultAgent":"codex"}"#).unwrap();
        assert_eq!(s.default_agent, "codex");
        assert_eq!(s.branch_prefix, "suneiro/");
    }
}
