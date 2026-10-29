//! "Recent projects" for the Add repository menu, learned from where the user
//! has been running coding agents: Claude Code (`~/.claude.json` plus the
//! modification time of its per-project history folders) and Codex (the `cwd`
//! of recent session logs). Worktrees are mapped back to their main repository.

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    /// Milliseconds since the epoch of the last agent activity we found.
    pub last_used: i64,
}

fn millis(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// How Claude Code names a project's history folder.
fn claude_dir_name(path: &str) -> String {
    path.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn claude_candidates(home: &Path) -> Vec<(String, i64)> {
    let Ok(text) = std::fs::read_to_string(home.join(".claude.json")) else { return vec![] };
    let Ok(json) = serde_json::from_str::<Value>(&text) else { return vec![] };
    let history = home.join(".claude").join("projects");
    json["projects"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(path, _)| {
            let modified = std::fs::metadata(history.join(claude_dir_name(path))).and_then(|m| m.modified()).ok()?;
            Some((path.clone(), millis(modified)))
        })
        .collect()
}

fn codex_candidates(home: &Path, limit: usize) -> Vec<(String, i64)> {
    // sessions/YYYY/MM/DD/rollout-*.jsonl: walk newest first.
    let mut files = Vec::new();
    let sorted_dirs = |dir: &Path| -> Vec<PathBuf> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        entries.sort();
        entries.reverse();
        entries
    };
    'outer: for year in sorted_dirs(&home.join(".codex").join("sessions")) {
        for month in sorted_dirs(&year) {
            for day in sorted_dirs(&month) {
                for file in sorted_dirs(&day) {
                    files.push(file);
                    if files.len() >= limit {
                        break 'outer;
                    }
                }
            }
        }
    }
    files
        .into_iter()
        .filter_map(|file| {
            let modified = std::fs::metadata(&file).and_then(|m| m.modified()).ok()?;
            let mut first = String::new();
            BufReader::new(std::fs::File::open(&file).ok()?).read_line(&mut first).ok()?;
            let cwd = serde_json::from_str::<Value>(&first).ok()?["payload"]["cwd"].as_str()?.to_string();
            Some((cwd, millis(modified)))
        })
        .collect()
}

/// The main repository a path belongs to (resolving worktrees), if any.
/// Reads `.git` directly instead of running git: this runs for many paths.
fn main_repo(path: &Path) -> Option<PathBuf> {
    let mut dir = path;
    loop {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Some(dir.to_path_buf());
        }
        if dot_git.is_file() {
            // Worktree: "gitdir: <main>/.git/worktrees/<name>".
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
            let common = gitdir.ancestors().find(|p| p.file_name().is_some_and(|n| n == ".git"))?;
            return common.parent().map(Path::to_path_buf).filter(|p| p.is_dir());
        }
        dir = dir.parent()?;
    }
}

/// Most recently used repositories, newest first, excluding `exclude`.
pub fn recent_projects(exclude: &[String], limit: usize) -> Vec<RecentProject> {
    let Some(home) = dirs::home_dir() else { return vec![] };
    let temp = std::env::temp_dir();
    let mut candidates = claude_candidates(&home);
    candidates.extend(codex_candidates(&home, 400));
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.1));

    let exclude: HashSet<PathBuf> = exclude.iter().map(PathBuf::from).collect();
    let mut seen_paths = HashSet::new();
    let mut seen_repos = HashSet::new();
    let mut out = Vec::new();
    for (path, last_used) in candidates {
        if out.len() >= limit {
            break;
        }
        let p = PathBuf::from(&path);
        if !seen_paths.insert(p.clone()) || p == home || p.starts_with(&temp) || p.starts_with("/private/tmp") || p.starts_with("/tmp") {
            continue;
        }
        let Some(repo) = main_repo(&p) else { continue };
        if exclude.contains(&repo) || !seen_repos.insert(repo.clone()) {
            continue;
        }
        out.push(RecentProject {
            name: repo.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            path: repo.to_string_lossy().to_string(),
            last_used,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_repos_and_worktrees() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git/worktrees/wt")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}/.git/worktrees/wt\n", repo.display())).unwrap();
        assert_eq!(main_repo(&repo.join("src/deep")), Some(repo.clone()));
        assert_eq!(main_repo(&wt), Some(repo));
        assert_eq!(main_repo(&tmp.path().join("nope")), None);
    }

    #[test]
    fn encodes_claude_dir_names() {
        assert_eq!(claude_dir_name("/Users/me/projects/my.app"), "-Users-me-projects-my-app");
    }
}
