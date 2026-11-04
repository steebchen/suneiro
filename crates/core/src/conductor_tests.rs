use super::*;

#[test]
fn parses_both_timestamp_formats() {
    assert_eq!(parse_ts("1970-01-01 00:00:00"), Some(0));
    assert_eq!(parse_ts("2026-02-27 06:24:59"), Some(1_772_173_499_000));
    assert_eq!(parse_ts("2026-08-10T15:17:11.298Z"), Some(1_786_375_031_298));
    assert_eq!(parse_ts("2026-08-10T15:17:11.3Z"), Some(1_786_375_031_300));
    assert_eq!(parse_ts("2026-08-10"), Some(1_786_320_000_000));
    assert_eq!(parse_ts("garbage"), None);
    assert_eq!(parse_ts("2026-13-01 00:00:00"), None);
}

#[test]
fn encodes_paths_for_sqlite_uris() {
    let uri = sqlite_uri(Path::new("/Users/me/Library/Application Support/com.conductor.app/conductor.db"));
    assert_eq!(uri, "file:/Users/me/Library/Application%20Support/com.conductor.app/conductor.db?immutable=1");
    assert_eq!(sqlite_uri(Path::new("/a?b#c%d")), "file:/a%3Fb%23c%25d?immutable=1");
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn inspects_worktrees_from_files_only() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("repo");
    let git_dir = main.join(".git/worktrees/tokyo");
    write(&git_dir.join("HEAD"), "ref: refs/heads/feature/login\n");
    write(&git_dir.join("commondir"), "../..\n");
    let wt = tmp.path().join("conductor/workspaces/repo/tokyo");
    write(&wt.join(".git"), &format!("gitdir: {}\n", git_dir.display()));
    let found = inspect_worktree(&wt).unwrap();
    assert_eq!(found.branch, "feature/login");
    assert_eq!(found.main_checkout, main.canonicalize().unwrap());
    assert_eq!(found.path, wt.canonicalize().unwrap());

    // A rename alias (symlink) resolves to the same worktree.
    #[cfg(unix)]
    {
        let alias = tmp.path().join("conductor/workspaces/repo/fix-login");
        std::os::unix::fs::symlink(&wt, &alias).unwrap();
        assert_eq!(inspect_worktree(&alias).unwrap().path, found.path);
    }

    // Detached HEAD, stale pointer and plain clones aren't importable.
    write(&git_dir.join("HEAD"), "0123456789abcdef0123456789abcdef01234567\n");
    assert!(inspect_worktree(&wt).is_none());
    let stale = tmp.path().join("conductor/workspaces/repo/stale");
    write(&stale.join(".git"), "gitdir: /nonexistent/.git/worktrees/stale\n");
    assert!(inspect_worktree(&stale).is_none());
    let clone = tmp.path().join("conductor/workspaces/repo/clone");
    std::fs::create_dir_all(clone.join(".git")).unwrap();
    assert!(inspect_worktree(&clone).is_none());
}

fn rows(list: &[(&str, &str)]) -> Vec<(String, String, Option<i64>)> {
    list.iter().enumerate().map(|(i, (r, c))| (r.to_string(), c.to_string(), Some(1000 + i as i64))).collect()
}

#[test]
fn converts_claude_stream_messages() {
    let events = convert_messages(
        "s1",
        &rows(&[
            ("user", "Fix the login bug"),
            ("assistant", r#"{"type":"system","subtype":"init"}"#),
            ("assistant", r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"Looking."}]}}"#),
            ("assistant", r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la\nsecond line"}}]}}"#),
            ("assistant", r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"boom","is_error":true}]}}"#),
            ("assistant", r#"{"type":"assistant","message":{"id":"m3","content":[{"type":"text","text":"Done."}]}}"#),
            ("assistant", r#"{"type":"result","subtype":"success"}"#),
            ("assistant", "not json"),
            ("SUMMARY", "ignored"),
            ("user", "Thanks"),
        ]),
    );
    let kinds: Vec<String> = events
        .iter()
        .map(|e| match e["type"].as_str().unwrap() {
            "sessionUpdate" => e["update"]["sessionUpdate"].as_str().unwrap().to_string(),
            t => t.to_string(),
        })
        .collect();
    assert_eq!(
        kinds,
        ["agent_message_chunk", "userMessage", "agent_message_chunk", "tool_call", "tool_call_update", "agent_message_chunk", "turnEnd", "userMessage", "turnEnd"]
    );
    assert!(events[0]["update"]["content"]["text"].as_str().unwrap().contains("Imported from Conductor"));
    assert_eq!(events[1]["text"], "Fix the login bug");
    assert_eq!(events[1]["sessionId"], "s1");
    assert_eq!(events[1]["ts"], 1000);
    assert_eq!(events[2]["update"]["messageId"], "m1");
    assert_eq!(events[3]["update"]["title"], "Bash: ls -la");
    assert_eq!(events[3]["update"]["kind"], "execute");
    assert_eq!(events[3]["update"]["status"], "completed");
    assert_eq!(events[4]["update"]["status"], "failed");
    assert_eq!(events.last().unwrap()["ts"], 1009);
}

#[test]
fn long_chats_keep_the_latest_whole_turns() {
    let mut list = Vec::new();
    for _ in 0..(MAX_EVENTS) {
        list.push(("user", "again"));
        list.push(("assistant", r#"{"type":"assistant","message":{"content":[{"type":"text","text":"ok"}]}}"#));
    }
    let events = convert_messages("s", &rows(&list));
    assert!(events.len() <= MAX_EVENTS + 1);
    assert_eq!(events[1]["type"], "userMessage", "starts at a prompt after the note");
}

#[test]
fn trims_long_tool_input() {
    let long = "x".repeat(MAX_INPUT_CHARS + 10);
    let v = trim_input(&json!({"content": long, "n": 1}));
    assert_eq!(v["content"].as_str().unwrap().chars().count(), MAX_INPUT_CHARS + 1);
    assert_eq!(v["n"], 1);
    let many: Vec<String> = (0..40).map(|_| "y".repeat(MAX_INPUT_CHARS)).collect();
    let capped = capped_input(&json!({ "edits": many }));
    assert!(capped.to_string().len() < 5 * MAX_INPUT_CHARS * 2);
    assert!(capped["input"].is_string());
}

#[test]
fn maps_agents() {
    assert_eq!(agent_id(Some("claude"), "x", None), "claude");
    assert_eq!(agent_id(Some("codex"), "x", None), "codex");
    assert_eq!(agent_id(Some("acp"), "x", Some("conductor-opencode-v2:ses_1")), "opencode");
    assert_eq!(agent_id(None, "x", None), "claude");
}

#[test]
fn missing_install_is_not_detected() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths { root: tmp.path().join("conductor"), db: tmp.path().join("nope.db") };
    let scan = scan(&paths);
    assert!(!scan.detected);
    assert!(scan.workspaces.is_empty());
}
