//! What "Import from Conductor" sees on this machine (read-only).
//!
//!   cargo run -p suneiro-core --example conductor
//!   cargo run -p suneiro-core --example conductor -- --import /tmp/suneiro-trial
//!
//! `--import <dir>` imports everything into a throwaway Suneiro data dir
//! (never your real one) to check timing and transcripts. Conductor's own
//! files are only read either way.

use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let import_dir = args.iter().position(|a| a == "--import").and_then(|i| args.get(i + 1)).cloned();
    let data = match &import_dir {
        Some(d) => std::path::PathBuf::from(d),
        None => std::env::temp_dir().join(format!("suneiro-conductor-scan-{}", std::process::id())),
    };
    let core = suneiro_core::Core::new(&data, Arc::new(|_| {}))?;
    let started = std::time::Instant::now();
    let scan = core.conductor_scan().await?;
    println!("detected: {} ({}), db error: {:?}, scan took {:?}", scan.detected, scan.root, scan.db_error, started.elapsed());
    for w in &scan.workspaces {
        println!(
            "{:<20} {:<28} {:<40} chats={:<3} {}{}",
            w.repo_name,
            w.name,
            w.branch,
            w.chats,
            w.title,
            if w.archived_in_conductor { "  [archived in Conductor]" } else { "" }
        );
    }
    println!("{} importable workspaces", scan.workspaces.len());
    for u in &scan.unavailable {
        println!("unavailable {}: {}", u.path, u.reason);
    }
    if import_dir.is_some() {
        let keys: Vec<String> = scan.workspaces.iter().map(|w| w.key.clone()).collect();
        let started = std::time::Instant::now();
        let report = core.conductor_import(&keys, true).await?;
        println!("imported {} ({} skipped) in {:?}; scripts for {:?}", report.imported.len(), report.skipped.len(), started.elapsed(), report.scripts_for);
        for s in &report.skipped {
            println!("  skipped {}: {}", s.key, s.reason);
        }
        let again = core.conductor_import(&keys, true).await?;
        println!("second run: imported {}, skipped {}", again.imported.len(), again.skipped.len());
    } else {
        let _ = std::fs::remove_dir_all(&data);
    }
    Ok(())
}
