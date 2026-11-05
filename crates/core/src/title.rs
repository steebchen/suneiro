//! Short texts written for the user: titles for sessions and workspaces ("Add
//! rate limiting"), commit messages and PR descriptions. A heuristic title is
//! shown instantly; a Haiku summary via the user's Claude login replaces it a
//! few seconds later when available.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use crate::env::tokio_command;

const FILLER: &[&str] = &[
    "please", "can", "could", "would", "you", "we", "i", "want", "to", "need", "let's", "lets",
    "hey", "hi", "so", "ok", "okay", "the", "a", "an",
];

/// Instant fallback: first few meaningful words of the first line.
pub fn quick_title(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let words: Vec<&str> = line.split_whitespace().collect();
    let start = words
        .iter()
        .position(|w| !FILLER.contains(&w.to_lowercase().trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')))
        .unwrap_or(0);
    let mut picked: Vec<&str> = words[start..].iter().take(4).copied().collect();
    const DANGLING: &[&str] = &["to", "the", "a", "an", "and", "of", "for", "in", "on", "with", "our", "my", "that"];
    while picked.len() > 1 && picked.last().is_some_and(|w| DANGLING.contains(&w.to_lowercase().as_str())) {
        picked.pop();
    }
    while picked.len() > 1 && picked.join(" ").chars().count() > 40 {
        picked.pop();
    }
    let mut title: String = picked.join(" ").chars().take(40).collect();
    title = title.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string();
    capitalize(&title)
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Replies that are commentary rather than a title.
fn looks_like_commentary(line: &str) -> bool {
    let l = line.to_lowercase();
    ["this isn't", "this is not", "i can't", "i cannot", "i'm ", "i am ", "sorry", "not a coding", "as an ai", "here is", "here's"]
        .iter()
        .any(|p| l.starts_with(p) || l.contains("coding task"))
        || line.split_whitespace().count() > 6
        || line.contains('—')
}

fn clean(summary: &str) -> Option<String> {
    let line = summary.lines().map(str::trim).find(|l| !l.is_empty())?;
    if looks_like_commentary(line) {
        return None;
    }
    let line = line.trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c == '*' || c == '.');
    let line = line.strip_prefix("Title:").unwrap_or(line).trim();
    let words: Vec<&str> = line.split_whitespace().take(4).collect();
    let title = capitalize(&words.join(" "));
    (!title.is_empty() && title.chars().count() <= 40).then_some(title)
}

/// Ask Claude Haiku for a 2–4 word title. `None` if Claude isn't installed,
/// isn't signed in, or takes too long.
pub async fn summarize(text: &str) -> Option<String> {
    if std::env::var_os("SUNEIRO_NO_AI_TITLES").is_some() || std::env::var_os("RUNNER_NO_AI_TITLES").is_some() {
        return None;
    }
    let excerpt: String = text.chars().take(2000).collect();
    let prompt = format!(
        "Write a 2-4 word title, under 40 characters, for a compact task tab.\n\
         - A coding task: imperative form, like \"Add dark mode\" or \"Fix login bug\".\n\
         - A question or anything else: name its topic, like \"Question about billing\" or \"Explain auth flow\".\n\
         - If there's nothing to summarize: \"Question\".\n\
         Never comment on or refuse the message. No punctuation. Reply with the title only.\n\nMessage:\n{excerpt}"
    );
    clean(&ask_haiku(&prompt, Duration::from_secs(25)).await?)
}

/// Run a one-off prompt through Claude Haiku with the user's Claude login,
/// with no tools, settings or session history. `None` if Claude isn't
/// installed, isn't signed in, fails or takes longer than `timeout`.
pub async fn ask_haiku(prompt: &str, timeout: Duration) -> Option<String> {
    let mut child = tokio_command("claude")
        .args([
            "-p",
            "--model",
            "haiku",
            "--no-session-persistence",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--tools",
            "",
        ])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    stdin.write_all(prompt.as_bytes()).await.ok()?;
    drop(stdin);
    let out = tokio::time::timeout(timeout, child.wait_with_output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Drop a surrounding code fence, if the model wrapped its answer in one.
fn unfence(text: &str) -> &str {
    let t = text.trim();
    match t.strip_prefix("```") {
        Some(rest) => rest.split_once('\n').map(|(_, r)| r).unwrap_or(rest).trim_end().trim_end_matches("```").trim(),
        None => t,
    }
}

/// A commit message for uncommitted changes (`context` = stat + diff).
pub async fn commit_message(context: &str) -> Option<String> {
    let prompt = format!(
        "Write a git commit message for the changes below. First line: a concise summary in the imperative mood, \
         at most 72 characters, no trailing period. Add a blank line and a short body only if the change needs \
         explaining. Reply with the commit message only.\n\n{context}"
    );
    let reply = ask_haiku(&prompt, Duration::from_secs(45)).await?;
    let msg = unfence(&reply).trim();
    (!msg.is_empty()).then(|| msg.to_string())
}

/// A pull request title and Markdown description for a branch's changes.
pub async fn pr_text(context: &str) -> Option<(String, String)> {
    let prompt = format!(
        "Write a GitHub pull request title and description for the branch below. Reply with the title on the \
         first line (imperative mood, under 70 characters, no prefix), then a blank line, then the description \
         in Markdown: one or two sentences on what changed and why, then a short bullet list of the notable \
         changes. No headings, no preamble.\n\n{context}"
    );
    parse_pr_text(&ask_haiku(&prompt, Duration::from_secs(60)).await?)
}

fn parse_pr_text(reply: &str) -> Option<(String, String)> {
    let text = unfence(reply);
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    let title = first.trim().trim_start_matches('#').trim();
    let title = title.strip_prefix("Title:").unwrap_or(title).trim().trim_matches('*').trim();
    let body = rest.trim();
    let body = body.strip_prefix("Description:").unwrap_or(body).trim();
    (!title.is_empty()).then(|| (title.to_string(), body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_titles() {
        assert_eq!(quick_title("can you please add rate limiting to the api endpoints"), "Add rate limiting");
        assert_eq!(quick_title("add dark mode support to the settings page"), "Add dark mode support");
        assert_eq!(quick_title("\n\nfix the bug."), "Fix the bug");
        assert_eq!(quick_title(""), "");
        assert_eq!(quick_title("Investigate authentication middleware authorization configuration"), "Investigate authentication middleware");
        assert_eq!(quick_title(&"界".repeat(80)).chars().count(), 40);
    }

    #[test]
    fn parses_pr_text() {
        let (t, b) = parse_pr_text("```\nTitle: Add rate limiting\n\nAdds a limiter.\n\n- one\n```").unwrap();
        assert_eq!(t, "Add rate limiting");
        assert_eq!(b, "Adds a limiter.\n\n- one");
        assert_eq!(parse_pr_text("# Fix bug").unwrap(), ("Fix bug".into(), String::new()));
        assert_eq!(unfence("```text\nFix it\n```"), "Fix it");
    }

    #[test]
    fn cleans_summaries() {
        assert_eq!(clean("\"Add API rate limiting.\"\n").as_deref(), Some("Add API rate limiting"));
        assert_eq!(clean("Title: fix flaky test").as_deref(), Some("Fix flaky test"));
        assert_eq!(clean("   "), None);
        assert_eq!(clean("Add comprehensive API rate limiting").as_deref(), Some("Add comprehensive API rate"));
        assert_eq!(clean(&"界".repeat(41)), None);
        assert_eq!(clean("This isn't a coding task — it's a question"), None);
        assert_eq!(clean("Sorry, I can't title that"), None);
        assert_eq!(clean("Question about billing").as_deref(), Some("Question about billing"));
    }
}
