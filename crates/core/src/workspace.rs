//! Per-repo configuration (`suneiro.json`, `runner.json`, `conductor.json` or app settings),
//! the environment scripts and terminals get, and workspace naming.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Scripts {
    /// Runs once in a new worktree (install deps, etc.).
    pub setup: Option<String>,
    /// Started from the "Run" button in a terminal (dev server, etc.).
    pub run: Option<String>,
    /// Runs before a worktree is removed.
    pub archive: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoConfig {
    pub scripts: Scripts,
    /// Gitignored files to copy from the main checkout into new worktrees (e.g. `.env`).
    pub copy: Vec<String>,
}

/// Config files looked for in a checkout, in order. `conductor.json` has the
/// same `scripts` shape, so repos set up for Conductor work as they are.
pub const CONFIG_FILES: &[&str] = &["suneiro.json", "runner.json", "conductor.json"];

/// Newer Conductor versions keep the scripts in this TOML file instead of
/// `conductor.json`. Read last, after the JSON files.
pub const CONDUCTOR_SETTINGS: &str = ".conductor/settings.toml";

/// The config file committed in a checkout, if any, and its name.
pub fn load_file_config(dir: &Path) -> Option<(&'static str, RepoConfig)> {
    CONFIG_FILES
        .iter()
        .find_map(|name| {
            let text = std::fs::read_to_string(dir.join(name)).ok()?;
            serde_json::from_str(&text).ok().map(|c| (*name, c))
        })
        .or_else(|| {
            let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().and_then(|t| conductor_toml_scripts(&t));
            let shared = read(CONDUCTOR_SETTINGS);
            // The personal, uncommitted file overrides the shared one per script.
            let local = read(CONDUCTOR_LOCAL_SETTINGS);
            let name = if shared.is_some() { CONDUCTOR_SETTINGS } else { CONDUCTOR_LOCAL_SETTINGS };
            let (shared, local) = (shared.unwrap_or_default(), local.unwrap_or_default());
            let scripts = Scripts {
                setup: local.setup.or(shared.setup),
                run: local.run.or(shared.run),
                archive: local.archive.or(shared.archive),
            };
            (scripts.setup.is_some() || scripts.run.is_some() || scripts.archive.is_some())
                .then(|| (name, RepoConfig { scripts, copy: Vec::new() }))
        })
}

/// Personal Conductor settings next to `settings.toml` (usually not committed).
pub const CONDUCTOR_LOCAL_SETTINGS: &str = ".conductor/settings.local.toml";

/// `[scripts]` setup/run/archive of a Conductor `settings.toml`; `None` when
/// the file doesn't parse or sets none of them.
pub fn conductor_toml_scripts(text: &str) -> Option<Scripts> {
    #[derive(Deserialize)]
    struct File {
        scripts: Option<Scripts>,
    }
    let scripts = toml::from_str::<File>(text).ok()?.scripts?;
    let set = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
    let scripts = Scripts { setup: set(scripts.setup), run: set(scripts.run), archive: set(scripts.archive) };
    (scripts.setup.is_some() || scripts.run.is_some() || scripts.archive.is_some()).then_some(scripts)
}

/// The committed file wins per script; settings made in the app fill the
/// gaps. Files to copy are combined.
pub fn merge_config(file: Option<RepoConfig>, app: RepoConfig) -> RepoConfig {
    let Some(file) = file else { return app };
    let pick = |a: Option<String>, b: Option<String>| a.filter(|s| !s.trim().is_empty()).or(b.filter(|s| !s.trim().is_empty()));
    let mut copy = file.copy;
    for c in app.copy {
        if !copy.contains(&c) {
            copy.push(c);
        }
    }
    RepoConfig {
        scripts: Scripts {
            setup: pick(file.scripts.setup, app.scripts.setup),
            run: pick(file.scripts.run, app.scripts.run),
            archive: pick(file.scripts.archive, app.scripts.archive),
        },
        copy,
    }
}

/// A block of 10 ports for the workspace's dev servers, so several
/// workspaces can run side by side. Stable for a workspace id.
pub fn port_base(workspace_id: &str) -> u16 {
    let hash = workspace_id.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    50_000 + (hash % 1_000) as u16 * 10
}

/// Environment for setup/run/archive scripts and workspace terminals. The
/// `RUNNER_*` and `CONDUCTOR_*` remain aliases for existing repository scripts.
pub fn script_env(root_path: &str, name: &str, path: &str, workspace_id: &str) -> Vec<(String, String)> {
    let port = port_base(workspace_id).to_string();
    let mut env = Vec::new();
    for prefix in ["SUNEIRO", "RUNNER", "CONDUCTOR"] {
        env.push((format!("{prefix}_ROOT_PATH"), root_path.to_string()));
        env.push((format!("{prefix}_WORKSPACE_NAME"), name.to_string()));
        env.push((format!("{prefix}_WORKSPACE_PATH"), path.to_string()));
        env.push((format!("{prefix}_PORT"), port.clone()));
    }
    env
}

const NAMES: &[&str] = &[
    "amsterdam", "athens", "austin", "bangkok", "barcelona", "berlin", "bogota", "boston",
    "cairo", "chicago", "copenhagen", "dakar", "denver", "dublin", "florence", "geneva",
    "hanoi", "havana", "helsinki", "istanbul", "jakarta", "kyoto", "lagos", "lima", "lisbon",
    "london", "madrid", "manila", "marseille", "melbourne", "miami", "milan", "montreal",
    "mumbai", "munich", "nairobi", "naples", "oslo", "paris", "porto", "prague", "quito",
    "reykjavik", "riga", "rome", "santiago", "seoul", "seville", "sofia", "stockholm",
    "sydney", "taipei", "tallinn", "tokyo", "toronto", "tunis", "valencia", "vienna",
    "vilnius", "warsaw", "zagreb", "zurich",
];

/// A branch-name-friendly slug of a task title: "Fix login bug" -> "fix-login-bug".
/// `None` for titles too vague to name a branch after (a single word).
pub fn branch_slug(title: &str) -> Option<String> {
    let words: Vec<String> = title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .collect();
    if words.len() < 2 {
        return None;
    }
    let mut slug = String::new();
    for w in words {
        if !slug.is_empty() && slug.len() + 1 + w.len() > 40 {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&w);
    }
    Some(slug)
}

/// Pick a city name not used by this repo yet, adding a suffix once all are taken.
pub fn pick_name(taken: &[String], seed: u64) -> String {
    let start = (seed as usize) % NAMES.len();
    for round in 0.. {
        for i in 0..NAMES.len() {
            let base = NAMES[(start + i) % NAMES.len()];
            let name = if round == 0 { base.to_string() } else { format!("{base}-{}", round + 1) };
            if !taken.contains(&name) {
                return name;
            }
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_unused_names() {
        let taken: Vec<String> = NAMES.iter().map(|s| s.to_string()).collect();
        assert!(pick_name(&taken, 0).ends_with("-2"));
        assert_eq!(pick_name(&[], 0), NAMES[0]);
    }

    #[test]
    fn slugs_titles_for_branches() {
        assert_eq!(branch_slug("Fix login bug").as_deref(), Some("fix-login-bug"));
        assert_eq!(branch_slug("Add café/dark-mode (v2)!").as_deref(), Some("add-caf-dark-mode-v2"));
        assert_eq!(branch_slug("Question"), None);
        let long = branch_slug("Refactor the authentication middleware to support multiple providers").unwrap();
        assert!(long.len() <= 40 && !long.ends_with('-'), "{long}");
    }

    #[test]
    fn file_config_wins_and_app_config_fills_gaps() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_file_config(dir.path()).is_none());
        std::fs::write(dir.path().join("conductor.json"), r#"{"scripts":{"setup":"npm ci","run":""}}"#).unwrap();
        let (name, file) = load_file_config(dir.path()).unwrap();
        assert_eq!(name, "conductor.json");
        let app = RepoConfig {
            scripts: Scripts { setup: Some("pnpm i".into()), run: Some("pnpm dev".into()), archive: None },
            copy: vec![".env".into()],
        };
        let c = merge_config(Some(file), app);
        assert_eq!(c.scripts.setup.as_deref(), Some("npm ci"));
        assert_eq!(c.scripts.run.as_deref(), Some("pnpm dev"));
        assert_eq!(c.copy, vec![".env"]);
        std::fs::write(dir.path().join("runner.json"), r#"{"copy":[".env.local"]}"#).unwrap();
        assert_eq!(load_file_config(dir.path()).unwrap().0, "runner.json");
        std::fs::write(dir.path().join("suneiro.json"), r#"{"scripts":{"run":"pnpm start"}}"#).unwrap();
        let (name, config) = load_file_config(dir.path()).unwrap();
        assert_eq!(name, "suneiro.json");
        assert_eq!(config.scripts.run.as_deref(), Some("pnpm start"));
    }

    #[test]
    fn reads_conductor_settings_toml_last() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".conductor")).unwrap();
        std::fs::write(
            dir.path().join(CONDUCTOR_SETTINGS),
            "\"$schema\" = \"https://conductor.build/schemas/settings.repo.schema.json\"\n\n[scripts]\nsetup = \"cp \\\"$CONDUCTOR_ROOT_PATH/.env\\\" .\\n\\npnpm install\"\nrun_mode = \"concurrent\"\n",
        )
        .unwrap();
        let (name, config) = load_file_config(dir.path()).unwrap();
        assert_eq!(name, CONDUCTOR_SETTINGS);
        assert_eq!(config.scripts.setup.as_deref(), Some("cp \"$CONDUCTOR_ROOT_PATH/.env\" .\n\npnpm install"));
        assert!(config.scripts.run.is_none());
        std::fs::write(dir.path().join(CONDUCTOR_LOCAL_SETTINGS), "[scripts]\nrun = \"pnpm dev\"\nsetup = \"\"\n").unwrap();
        let (_, config) = load_file_config(dir.path()).unwrap();
        assert_eq!(config.scripts.run.as_deref(), Some("pnpm dev"), "local file adds scripts");
        assert!(config.scripts.setup.as_deref().is_some_and(|s| s.contains("pnpm install")), "empty local script doesn't hide the shared one");
        // A file without scripts (e.g. settings.local.toml-style) doesn't count.
        assert!(conductor_toml_scripts("spotlight_testing = true\n[scripts]\nrun_mode = \"concurrent\"\n").is_none());
        assert!(conductor_toml_scripts("not toml [").is_none());
        // conductor.json still wins over the TOML file.
        std::fs::write(dir.path().join("conductor.json"), r#"{"scripts":{"setup":"npm ci"}}"#).unwrap();
        assert_eq!(load_file_config(dir.path()).unwrap().0, "conductor.json");
    }

    #[test]
    fn script_environment_keeps_legacy_aliases() {
        let env: std::collections::HashMap<_, _> = script_env("/repo", "athens", "/workspace", "abc").into_iter().collect();
        for prefix in ["SUNEIRO", "RUNNER", "CONDUCTOR"] {
            assert_eq!(env[&format!("{prefix}_ROOT_PATH")], "/repo");
            assert_eq!(env[&format!("{prefix}_WORKSPACE_NAME")], "athens");
            assert_eq!(env[&format!("{prefix}_WORKSPACE_PATH")], "/workspace");
            assert_eq!(env[&format!("{prefix}_PORT")], port_base("abc").to_string());
        }
    }

    #[test]
    fn ports_are_stable_blocks() {
        let p = port_base("abc");
        assert_eq!(p, port_base("abc"));
        assert!((50_000..60_000).contains(&p) && p.is_multiple_of(10));
    }

    #[test]
    fn parses_config() {
        let c: RepoConfig =
            serde_json::from_str(r#"{"scripts":{"setup":"pnpm i"},"copy":[".env"]}"#).unwrap();
        assert_eq!(c.scripts.setup.as_deref(), Some("pnpm i"));
        assert_eq!(c.copy, vec![".env"]);
    }
}
