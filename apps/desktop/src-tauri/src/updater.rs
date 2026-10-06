//! Updates that never interrupt work. Release builds check the endpoint in
//! `tauri.conf.json` at launch and every few hours, and report the state as an
//! "update" event. An update found right at launch installs and restarts on
//! its own, as long as it's ready within `STARTUP_WINDOW` and nothing has
//! started yet (the UI offers "Not now"). Otherwise nothing is installed until
//! the user picks "Restart now" or "Restart when idle"; the latter downloads
//! the update, waits until no agent is mid-turn and no workspace is being set
//! up or archived, and only then installs it and restarts.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use suneiro_core::Activity;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// How long after launch an update may still install without asking. Short,
/// because composer drafts don't survive a restart.
const STARTUP_WINDOW: Duration = Duration::from_secs(20);
const INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);
const IDLE_POLL: Duration = Duration::from_millis(1500);
/// Consecutive idle polls before restarting, so a message queued right after
/// a turn ends gets to start instead of being cut off.
const IDLE_POLLS: u32 = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Newer version that can be installed, if any.
    version: Option<String>,
    /// "none", "available", "downloading", "waiting" (for work to finish) or "installing".
    phase: &'static str,
    /// What the update is waiting for while "waiting".
    activity: Activity,
    /// Installing on its own at launch (the user didn't ask for it).
    automatic: bool,
    /// Bumped whenever the user confirms or cancels, so a stale install
    /// (e.g. one still waiting for agents) knows to stop.
    #[serde(skip)]
    generation: u64,
}

impl Default for Status {
    fn default() -> Self {
        Self { version: None, phase: "none", activity: Activity::default(), automatic: false, generation: 0 }
    }
}

#[derive(Default)]
pub struct Updates {
    update: Mutex<Option<Update>>,
    /// Downloaded but not installed yet: (version, bytes).
    downloaded: Mutex<Option<(String, Arc<Vec<u8>>)>>,
    status: Mutex<Status>,
    /// Held while checking, so the timer and the menu item never race.
    busy: tokio::sync::Mutex<()>,
}

/// Update the status and tell the UI. With `generation`, only if no newer
/// confirm/cancel happened since; returns whether it applied.
fn set_status(app: &AppHandle, generation: Option<u64>, f: impl FnOnce(&mut Status)) -> bool {
    let updates = app.state::<Updates>();
    let status = {
        let mut s = updates.status.lock();
        if generation.is_some_and(|g| g != s.generation) {
            return false;
        }
        f(&mut s);
        s.clone()
    };
    let _ = app.emit("update", &status);
    true
}

/// Start the background checks. Development builds never update themselves.
pub fn start(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    let launched = Instant::now();
    tauri::async_runtime::spawn(async move {
        match check(&app).await {
            Ok(status) if status.version.is_some() => {
                if let Err(e) = run(&app, Mode::Startup(launched + STARTUP_WINDOW)).await {
                    log::warn!("update at launch failed: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => log::warn!("update check failed: {e}"),
        }
        loop {
            tokio::time::sleep(INTERVAL).await;
            if let Err(e) = check(&app).await {
                log::warn!("update check failed: {e}");
            }
        }
    });
}

#[derive(Clone, Copy)]
enum Mode {
    /// Restart right away.
    Now,
    /// Restart once nothing is running.
    WhenIdle,
    /// Unasked, at launch: restart only if ready before the deadline and
    /// nothing has started; otherwise leave it to the user.
    Startup(Instant),
}

/// Look for a newer version. Never downloads or installs anything.
async fn check(app: &AppHandle) -> Result<Status, String> {
    let updates = app.state::<Updates>();
    let _busy = updates.busy.lock().await;
    {
        let s = updates.status.lock();
        if !matches!(s.phase, "none" | "available") {
            // The user already chose to update; don't swap the version under them.
            return Ok(s.clone());
        }
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    let version = update.as_ref().map(|u| u.version.clone());
    *updates.update.lock() = update;
    set_status(app, None, |s| {
        if matches!(s.phase, "none" | "available") {
            s.phase = if version.is_some() { "available" } else { "none" };
            s.version = version;
        }
    });
    let status = updates.status.lock().clone();
    Ok(status)
}

/// "Check for Updates…": the current update status after a fresh check.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<Status, String> {
    if cfg!(debug_assertions) {
        return Err("Updates are disabled in development builds".into());
    }
    check(&app).await
}

/// Current status, for a UI that missed the event.
#[tauri::command]
pub fn update_status(updates: tauri::State<'_, Updates>) -> Status {
    updates.status.lock().clone()
}

/// The user confirmed: download the update, then install it and restart,
/// either right away or once nothing is running anymore.
#[tauri::command]
pub async fn install_update(app: AppHandle, when_idle: bool) -> Result<(), String> {
    run(&app, if when_idle { Mode::WhenIdle } else { Mode::Now }).await
}

async fn run(app: &AppHandle, mode: Mode) -> Result<(), String> {
    let mut generation = 0;
    set_status(app, None, |s| {
        s.generation += 1;
        generation = s.generation;
        s.automatic = matches!(mode, Mode::Startup(_));
    });
    let result = install(app, mode, generation).await;
    if !matches!(result, Ok(true)) {
        // Failed, or the launch window passed: back to asking the user.
        set_status(app, Some(generation), |s| {
            s.phase = if s.version.is_some() { "available" } else { "none" };
            s.activity = Activity::default();
            s.automatic = false;
        });
    }
    result.map(|_| ())
}

/// Returns `false` when the user should be offered the update again (the
/// launch window passed); cancelling already resets the status itself.
async fn install(app: &AppHandle, mode: Mode, generation: u64) -> Result<bool, String> {
    let updates = app.state::<Updates>();
    let update = updates.update.lock().clone().ok_or("No update available")?;

    let cached = updates.downloaded.lock().clone().filter(|(v, _)| *v == update.version);
    let bytes = match cached {
        Some((_, bytes)) => bytes,
        None => {
            set_status(app, Some(generation), |s| s.phase = "downloading");
            let bytes = Arc::new(update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?);
            *updates.downloaded.lock() = Some((update.version.clone(), bytes.clone()));
            bytes
        }
    };

    let core = app.state::<crate::App>().core.clone();
    if let Mode::Startup(deadline) = mode {
        if Instant::now() > deadline || !core.activity().is_idle() {
            return Ok(false);
        }
    }
    if let Mode::WhenIdle = mode {
        let mut idle_polls = 0;
        loop {
            let activity = core.activity();
            idle_polls = if activity.is_idle() { idle_polls + 1 } else { 0 };
            if idle_polls >= IDLE_POLLS {
                break;
            }
            let current = set_status(app, Some(generation), |s| {
                s.phase = "waiting";
                s.activity = activity;
            });
            if !current {
                return Ok(true); // cancelled, or "Restart now" took over
            }
            tokio::time::sleep(IDLE_POLL).await;
        }
    }

    if !set_status(app, Some(generation), |s| s.phase = "installing") {
        return Ok(true); // cancelled
    }
    update.install(bytes.as_slice()).map_err(|e| e.to_string())?;
    app.request_restart();
    Ok(true)
}

/// Stop waiting for work to finish; the update stays available.
#[tauri::command]
pub fn cancel_update(app: AppHandle) {
    set_status(&app, None, |s| {
        if s.phase == "installing" {
            return;
        }
        s.generation += 1;
        s.phase = if s.version.is_some() { "available" } else { "none" };
        s.activity = Activity::default();
        s.automatic = false;
    });
}
