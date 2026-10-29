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


async fn run_output(program: &str, args: &[&str]) -> Option<std::process::Output> {
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tokio_command(program).args(args).stdin(std::process::Stdio::null()).kill_on_drop(true).output(),
    ).await.ok()?.ok()
}

async fn run(program: &str, args: &[&str]) -> Option<(bool, String)> {
    let out = run_output(program, args).await?;

    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some((out.status.success(), text))
}

fn claude_auth_status(out: &std::process::Output) -> (bool, Option<String>) {
    let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) else { return (false, None) };
    if !out.status.success() || v["loggedIn"].as_bool() != Some(true) {
        return (false, None);
    }
    let email = v["email"].as_str();
    let plan = v["subscriptionType"].as_str().or(v["authMethod"].as_str());
    let account = match (email, plan) {
        (Some(e), Some(p)) => Some(format!("{e} · {p}")),
        (Some(e), None) => Some(e.into()),
        (None, p) => p.map(str::to_string),
    };
    (true, account)
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
            if let Some(out) = run_output("claude", &["auth", "status", "--json"]).await {
                (status.logged_in, status.account) = claude_auth_status(&out);
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
            if let Some((true, out)) = run("opencode", &["auth", "list"]).await {
                let providers = crate::opencode::providers(&strip_ansi(&out));
                status.logged_in = !providers.is_empty();
                status.account = (!providers.is_empty()).then(|| providers.join(", "));
            }
            if !status.logged_in {
                if let Some((true, out)) = run("opencode", &["models"]).await {
                    status.logged_in = crate::opencode::has_models(&strip_ansi(&out));
                    status.account = Some(if status.logged_in {
                        "Models available; provider authentication is checked when used".into()
                    } else {
                        crate::opencode::NO_MODELS.into()
                    });
                }
            }
        }
        _ => {}
    }
    Some(status)
}

/// Read-only preflight for the pinned Claude adapter. Does not install anything,
/// authenticate, read credential files, or start a model turn.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDiagnostics {
    pub cli: AgentStatus,
    pub node_version: Option<String>,
    pub npx_version: Option<String>,
    pub issues: Vec<String>,
}

pub async fn claude_diagnostics() -> ClaudeDiagnostics {
    let (cli, node, npx) = tokio::join!(detect("claude"), run("node", &["--version"]), run("npx", &["--version"]));
    let cli = cli.expect("Claude is a builtin agent");
    let node_version = node.filter(|(ok, _)| *ok).and_then(|(_, s)| first_version(s.trim_start_matches('v')));
    let npx_version = npx.filter(|(ok, _)| *ok).and_then(|(_, s)| first_version(&s));
    let mut issues = Vec::new();
    if !cli.installed { issues.push("Claude Code is missing or cannot run. Install the official CLI, then run claude auth login.".into()); }
    else if !cli.logged_in { issues.push("Claude authentication could not be confirmed. Run claude auth status --json, then claude auth login if needed.".into()); }
    if node_version.as_deref().and_then(|v| v.split('.').next()?.parse::<u32>().ok()).is_none_or(|major| major < 22) {
        issues.push("The pinned Claude ACP adapter requires Node.js 22 or newer on the login-shell PATH.".into());
    }
    if npx_version.is_none() { issues.push("npx is missing or cannot run. Install npm alongside Node.js; the ACP adapter starts through npx.".into()); }
    ClaudeDiagnostics { cli, node_version, npx_version, issues }
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

    #[tokio::test]
    async fn claude_auth_ignores_stderr_diagnostics() {
        let out = run_output("node", &["-e", r#"process.stdout.write(JSON.stringify({loggedIn:true,email:'test@example.invalid',subscriptionType:'max'}));process.stderr.write('warning: runtime diagnostic\n')"#]).await.unwrap();
        assert_eq!(claude_auth_status(&out), (true, Some("test@example.invalid · max".into())));
    }

    #[tokio::test]
    async fn claude_auth_rejects_failed_malformed_and_logged_out_probes() {
        for script in [
            "console.log(JSON.stringify({loggedIn:true,email:'stale@example.invalid'}));process.exitCode=1",
            "console.log('not json')",
            "console.log(JSON.stringify({loggedIn:false,email:'stale@example.invalid'}))",
            "console.error(JSON.stringify({loggedIn:true}))",
        ] {
            let out = run_output("node", &["-e", script]).await.unwrap();
            assert_eq!(claude_auth_status(&out), (false, None));
        }
    }
}
