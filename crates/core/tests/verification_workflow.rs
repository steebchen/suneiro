use std::{path::Path, sync::Arc, time::Duration};
use suneiro_core::{
    git,
    verification::{revision, CheckPreset, CheckRun},
    AgentDef, Core, Workspace,
};

async fn fixture() -> (tempfile::TempDir, Arc<Core>, Workspace, Workspace) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "test@example.com"],
        &["config", "user.name", "Test"],
    ] {
        git::git(&repo, args).await.unwrap();
    }
    std::fs::write(repo.join("file.txt"), "initial\n").unwrap();
    git::commit_all(&repo, "initial").await.unwrap();
    let core = Core::new(&tmp.path().join("data"), Arc::new(|_| {})).unwrap();
    let r = core.add_repo(repo.to_str().unwrap()).await.unwrap();
    let mut ws = Vec::new();
    for id in ["target", "source"] {
        let path = tmp.path().join(id);
        git::git(
            &repo,
            &["worktree", "add", "-q", "-b", id, path.to_str().unwrap()],
        )
        .await
        .unwrap();
        let w = Workspace {
            id: id.into(),
            repo_id: r.id.clone(),
            name: id.into(),
            branch: id.into(),
            base_branch: "main".into(),
            path: path.to_str().unwrap().into(),
            status: "ready".into(),
            created_at: 0,
            title: String::new(),
            archived_at: None,
            unread: false,
        };
        core.store.add_workspace(&w).unwrap();
        ws.push(w);
    }
    (tmp, core, ws.remove(0), ws.remove(0))
}
fn preset(core: &Core, ws: &Workspace, command: &str) {
    core.save_verification_presets(
        &ws.repo_id,
        vec![CheckPreset {
            id: "tests".into(),
            command: command.into(),
        }],
    )
    .unwrap();
}
async fn finished(core: &Core, ws: &Workspace) -> CheckRun {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let runs = core.verification_status(&ws.id).await.unwrap();
            if let Some(r) = runs.last().filter(|r| r.status != "running") {
                return r.clone();
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("check did not finish")
}
#[tokio::test]
async fn checks_bind_revision_dirty_content_untracked_and_config() {
    let (_tmp, core, target, _) = fixture().await;
    preset(&core, &target, "printf verified");
    let run = core.run_check(&target.id, "tests").await.unwrap();
    assert_eq!(run.cwd, target.path);
    assert_eq!(finished(&core, &target).await.status, "passed");
    assert!(!core.verification_status(&target.id).await.unwrap()[0].stale);
    std::fs::write(Path::new(&target.path).join("draft.txt"), "one").unwrap();
    assert!(core.verification_status(&target.id).await.unwrap()[0].stale);
    core.run_check(&target.id, "tests").await.unwrap();
    finished(&core, &target).await;
    let before = revision(Path::new(&target.path)).await.unwrap();
    std::fs::write(Path::new(&target.path).join("draft.txt"), "two").unwrap();
    assert_ne!(before, revision(Path::new(&target.path)).await.unwrap());
    assert!(
        core.verification_status(&target.id)
            .await
            .unwrap()
            .last()
            .unwrap()
            .stale
    );
    core.run_check(&target.id, "tests").await.unwrap();
    finished(&core, &target).await;
    preset(&core, &target, "printf changed");
    assert!(
        core.verification_status(&target.id)
            .await
            .unwrap()
            .last()
            .unwrap()
            .stale
    );
    core.run_check(&target.id, "tests").await.unwrap();
    finished(&core, &target).await;
    git::commit_all(Path::new(&target.path), "commit")
        .await
        .unwrap();
    assert!(
        core.verification_status(&target.id)
            .await
            .unwrap()
            .last()
            .unwrap()
            .stale
    );
}
#[tokio::test]
async fn failed_checks_are_bounded_and_block_integration() {
    let (_tmp, core, target, source) = fixture().await;
    preset(&core, &source, "printf '%70000s' failure-line; exit 7");
    core.run_check(&source.id, "tests").await.unwrap();
    let result = finished(&core, &source).await;
    assert_eq!(result.status, "failed");
    assert_eq!(result.exit_code, Some(7));
    assert!(result.log.len() <= 65536);
    assert!(result.log.contains("failure-line"));
    let preview = core
        .integration_preview(&target.id, &source.id)
        .await
        .unwrap();
    assert!(!preview.can_apply);
    assert!(core.integrate_workspace(&target.id, preview).await.is_err());
}
#[tokio::test]
async fn running_check_guards_restore_archive_agents_and_other_checks_then_cancels() {
    let (_tmp, core, target, source) = fixture().await;
    preset(&core, &target, "sleep 60 & wait");
    core.agents.register(AgentDef {
        id: "fake".into(),
        name: "Fake".into(),
        command: "node".into(),
        args: vec![format!(
            "{}/tests/fake_agent.mjs",
            env!("CARGO_MANIFEST_DIR")
        )],
    });
    let session = core.create_session(&target.id, "fake", None, None).unwrap();
    let run = core.run_check(&target.id, "tests").await.unwrap();
    assert_eq!(core.activity().workspaces, 1);
    assert!(core.run_check(&target.id, "tests").await.is_err());
    assert!(core.agents.prompt(&session.id, "do work".into()).is_err());
    assert!(core
        .restore_checkpoint(&session.id, &run.revision.head)
        .await
        .is_err());
    assert!(core.archive_workspace(&target.id).await.is_err());
    assert!(core.commit_all(&target.id, "unsafe").await.is_err());
    assert!(
        !core
            .integration_preview(&source.id, &target.id)
            .await
            .unwrap()
            .can_apply
    );
    core.cancel_check(&target.id, &run.id).unwrap();
    assert_eq!(finished(&core, &target).await.status, "cancelled");
    core.shutdown();
}
#[tokio::test]
async fn edits_during_check_are_stale_even_when_restored() {
    let (_tmp, core, target, _) = fixture().await;
    preset(&core, &target, "sleep 1");
    core.run_check(&target.id, "tests").await.unwrap();
    std::fs::write(Path::new(&target.path).join("file.txt"), "changed\n").unwrap();
    assert!(core.verification_status(&target.id).await.unwrap()[0].stale);
    std::fs::write(Path::new(&target.path).join("file.txt"), "initial\n").unwrap();
    assert!(finished(&core, &target).await.stale);
}
#[tokio::test]
async fn integration_previews_conflicts_rejects_stale_and_merges_only_checked_source() {
    let (_tmp, core, target, source) = fixture().await;
    std::fs::write(Path::new(&source.path).join("child.txt"), "child\n").unwrap();
    git::commit_all(Path::new(&source.path), "child")
        .await
        .unwrap();
    preset(&core, &source, "true");
    core.run_check(&source.id, "tests").await.unwrap();
    finished(&core, &source).await;
    let preview = core
        .integration_preview(&target.id, &source.id)
        .await
        .unwrap();
    assert!(preview.can_apply);
    assert!(preview.diff.contains("child.txt"));
    std::fs::write(Path::new(&target.path).join("keep.txt"), "keep\n").unwrap();
    assert!(core
        .integrate_workspace(&target.id, preview)
        .await
        .unwrap_err()
        .to_string()
        .contains("stale"));
    assert_eq!(
        std::fs::read_to_string(Path::new(&target.path).join("keep.txt")).unwrap(),
        "keep\n"
    );
    git::commit_all(Path::new(&target.path), "keep")
        .await
        .unwrap();
    let preview = core
        .integration_preview(&target.id, &source.id)
        .await
        .unwrap();
    assert!(preview.can_apply);
    core.integrate_workspace(&target.id, preview).await.unwrap();
    assert!(Path::new(&target.path).join("child.txt").exists());
    assert!(git::git(Path::new(&target.path), &["rev-parse", "HEAD^2"])
        .await
        .is_ok());
    for (ws, content) in [(&target, "target\n"), (&source, "source\n")] {
        std::fs::write(Path::new(&ws.path).join("file.txt"), content).unwrap();
        git::commit_all(Path::new(&ws.path), "conflict")
            .await
            .unwrap();
    }
    core.run_check(&source.id, "tests").await.unwrap();
    finished(&core, &source).await;
    let preview = core
        .integration_preview(&target.id, &source.id)
        .await
        .unwrap();
    assert!(!preview.can_apply);
    assert!(preview.conflicts.iter().any(|p| p == "file.txt"));
    assert!(core.integrate_workspace(&target.id, preview).await.is_err());
    assert_eq!(
        std::fs::read_to_string(Path::new(&target.path).join("file.txt")).unwrap(),
        "target\n"
    );
}
#[tokio::test]
async fn integration_never_overwrites_ignored_local_files() {
    let (_tmp, core, target, source) = fixture().await;
    std::fs::write(Path::new(&target.path).join(".gitignore"), ".env\n").unwrap();
    git::commit_all(Path::new(&target.path), "ignore local environment")
        .await
        .unwrap();
    std::fs::write(Path::new(&target.path).join(".env"), "LOCAL_SECRET=keep\n").unwrap();
    std::fs::write(Path::new(&source.path).join(".env"), "source content\n").unwrap();
    git::commit_all(Path::new(&source.path), "source file")
        .await
        .unwrap();
    preset(&core, &source, "true");
    core.run_check(&source.id, "tests").await.unwrap();
    finished(&core, &source).await;
    let preview = core
        .integration_preview(&target.id, &source.id)
        .await
        .unwrap();
    assert!(preview.conflicts.iter().any(|file| file == ".env"));
    assert!(!preview.can_apply);
    assert!(
        core.integrate_workspace(&target.id, preview).await.is_err(),
        "ignored local content must not be overwritten"
    );
    assert_eq!(
        std::fs::read_to_string(Path::new(&target.path).join(".env")).unwrap(),
        "LOCAL_SECRET=keep\n"
    );
}

#[tokio::test]
async fn reviewer_uses_advertised_restriction_denies_permissions_and_never_elevates() {
    let (_tmp, core, target, _) = fixture().await;
    core.agents.register(AgentDef {
        id: "review".into(),
        name: "Review".into(),
        command: "node".into(),
        args: vec![format!(
            "{}/tests/review_agent.mjs",
            env!("CARGO_MANIFEST_DIR")
        )],
    });
    std::fs::write(Path::new(&target.path).join("file.txt"), "review me\n").unwrap();
    let review = core
        .start_review(&target.id, "review", vec!["file.txt".into()])
        .await
        .unwrap();
    assert_eq!(review.protection, "agent-enforced read-only");
    assert!(core
        .agents
        .set_plan_mode(&review.session.id, false)
        .await
        .is_err());
    assert!(core
        .agents
        .set_config(
            &review.session.id,
            "mode",
            serde_json::json!("agent-full-access")
        )
        .await
        .is_err());
    tokio::time::timeout(Duration::from_secs(5), async {
        while core.agents.is_running(&review.session.id) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(Path::new(&target.path).join("file.txt")).unwrap(),
        "review me\n"
    );
    core.agents.register(AgentDef {
        id: "plan-review".into(),
        name: "Plan review".into(),
        command: "node".into(),
        args: vec![
            format!("{}/tests/review_agent.mjs", env!("CARGO_MANIFEST_DIR")),
            "plan".into(),
        ],
    });
    let plan_review = core
        .start_review(&target.id, "plan-review", vec!["file.txt".into()])
        .await
        .unwrap();
    assert_eq!(
        plan_review.protection,
        "plan mode; permissions denied; not sandboxed"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while core.agents.is_running(&plan_review.session.id) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(Path::new(&target.path).join("file.txt")).unwrap(),
        "review me\n"
    );
    core.agents.register(AgentDef {
        id: "unsupported".into(),
        name: "Unsupported".into(),
        command: "node".into(),
        args: vec![format!(
            "{}/tests/fake_agent.mjs",
            env!("CARGO_MANIFEST_DIR")
        )],
    });
    assert!(core
        .start_review(&target.id, "unsupported", vec!["file.txt".into()])
        .await
        .is_err());
    core.shutdown();
}
