//! Error reports to PostHog Error Tracking: Rust panics (posthog-rs panic
//! hook), uncaught webview errors and unhandled rejections (forwarded by
//! `lib/errorReports.ts` through `report_error`) and webview process crashes.
//!
//! `Settings.error_reports` (on by default) is separate from any product
//! analytics consent. Reports are personless, GeoIP is off, the home folder is
//! masked as `~`, and no prompts, transcripts, files or credentials are
//! attached. Only release builds report; set `SUNEIRO_ERROR_REPORTS=1` to try
//! it in a debug build. Turning the setting off stops sending immediately;
//! turning it on applies at the next launch (the panic hook is installed at
//! startup).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{json, Value};

/// Public, write-only project token (safe to embed, like any client SDK key).
const PROJECT_TOKEN: &str = "phc_wpwS9QMJiXAqxhKMXTkVknYU232bVamubn35NPNr5tpr";
const HOST: &str = "https://us.i.posthog.com";
const MAX_MESSAGE: usize = 2_000;
const MAX_FRAMES: usize = 60;

/// The client was initialized at launch.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The user's current choice; checked for every event before it is sent.
static ENABLED: AtomicBool = AtomicBool::new(false);
static HOME: OnceLock<Option<String>> = OnceLock::new();

/// Start reporting if this is a release build and the user hasn't turned it
/// off. Runs before Tauri starts, so it reads the saved settings directly.
pub fn init(identifier: &str) {
    if cfg!(debug_assertions) && std::env::var_os("SUNEIRO_ERROR_REPORTS").is_none() {
        return;
    }
    let Some(data) = dirs::data_dir() else { return };
    if !suneiro_core::setup::saved_settings(&data.join(identifier)).error_reports {
        return;
    }
    let _ = HOME.set(
        dirs::home_dir()
            .map(|h| h.to_string_lossy().trim_end_matches(['/', '\\']).to_string())
            .filter(|h| h.len() > 1),
    );
    let tracking = posthog_rs::ErrorTrackingOptionsBuilder::default()
        .capture_panics(true)
        .in_app_include_paths(vec!["suneiro_core::".into(), "suneiro_desktop".into()])
        .build();
    let options = tracking.map_err(|e| e.to_string()).and_then(|tracking| {
        posthog_rs::ClientOptionsBuilder::default()
            .api_key(PROJECT_TOKEN.to_string())
            .host(HOST)
            .disable_geoip(true)
            .shutdown_timeout_ms(2_000)
            .error_tracking(tracking)
            .before_send(prepare)
            .build()
            .map_err(|e| e.to_string())
    });
    let options = match options {
        Ok(options) => options,
        Err(error) => return log::warn!("error reports disabled: {error}"),
    };
    ENABLED.store(true, Ordering::SeqCst);
    match tauri::async_runtime::block_on(posthog_rs::init_global(options)) {
        Ok(()) => ACTIVE.store(true, Ordering::SeqCst),
        Err(error) => {
            ENABLED.store(false, Ordering::SeqCst);
            log::warn!("error reports disabled: {error}");
        }
    }
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// Follow the setting while the app runs. Off takes effect for the next event.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

/// Send what is still queued; bounded by `shutdown_timeout_ms`.
pub fn shutdown() {
    if active() {
        tauri::async_runtime::block_on(posthog_rs::shutdown());
    }
}

/// Gate and scrub every event, panics included, right before it is sent.
fn prepare(mut event: posthog_rs::Event) -> Option<posthog_rs::Event> {
    if !ENABLED.load(Ordering::SeqCst) {
        return None;
    }
    let home = HOME.get().cloned().flatten();
    let keys: Vec<String> = event.properties().keys().cloned().collect();
    for key in keys {
        if let Some(mut value) = event.remove_prop(&key) {
            scrub(&mut value, home.as_deref());
            let _ = event.insert_prop(key, value);
        }
    }
    if !event.properties().contains_key("process") {
        let _ = event.insert_prop("process", "rust");
    }
    let _ = event.insert_prop("$app_version", env!("CARGO_PKG_VERSION"));
    let _ = event.insert_prop("$os", os_name());
    let _ = event.insert_prop("arch", std::env::consts::ARCH);
    let _ = event.insert_prop("environment", if cfg!(debug_assertions) { "development" } else { "production" });
    Some(event)
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "Mac OS X",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

/// Mask the home folder (user names, project paths) in every string.
fn scrub(value: &mut Value, home: Option<&str>) {
    match value {
        Value::String(s) => {
            if let Some(home) = home.filter(|home| s.contains(home)) {
                *s = mask(s, home);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| scrub(v, home)),
        Value::Object(map) => map.values_mut().for_each(|v| scrub(v, home)),
        _ => {}
    }
}

/// Replace `home` with `~` where it is a whole path (not `/Users/adam` for `/Users/ada`).
fn mask(s: &str, home: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(home) {
        let after = &rest[i + home.len()..];
        let whole = after.chars().next().is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.')));
        out.push_str(&rest[..i]);
        out.push_str(if whole { "~" } else { home });
        rest = after;
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebError {
    /// "error" (window.onerror) or "unhandledrejection".
    pub mechanism: String,
    pub name: String,
    pub message: String,
    /// Oldest call first, as PostHog expects.
    #[serde(default)]
    pub frames: Vec<WebFrame>,
}

#[derive(Debug, Deserialize)]
pub struct WebFrame {
    pub function: Option<String>,
    pub filename: Option<String>,
    pub lineno: Option<u32>,
    pub colno: Option<u32>,
}

#[tauri::command]
pub fn report_error(error: WebError) {
    if active() {
        posthog_rs::capture(web_event(error));
    }
}

/// macOS gives no reason when WebKit's content process dies.
pub fn webview_terminated(label: &str) {
    if !active() {
        return;
    }
    let mut event = exception(
        "WebContentProcessTerminated",
        "The webview process exited unexpectedly",
        "webview-crash",
        None,
    );
    let _ = event.insert_prop("$exception_level", "fatal");
    let _ = event.insert_prop("webview", label);
    posthog_rs::capture(event);
}

fn web_event(error: WebError) -> posthog_rs::Event {
    let start = error.frames.len().saturating_sub(MAX_FRAMES);
    let frames: Vec<Value> = error.frames[start..]
        .iter()
        .map(|f| {
            let in_app = f.filename.as_deref().is_some_and(|name| {
                name.starts_with("tauri://") || name.starts_with("http://tauri.localhost") || name.starts_with("http://localhost:1420")
            });
            json!({
                "platform": "web:javascript",
                "function": f.function.as_deref().filter(|s| !s.is_empty()).unwrap_or("?"),
                "filename": f.filename,
                "abs_path": f.filename,
                "lineno": f.lineno,
                "colno": f.colno,
                "in_app": in_app,
            })
        })
        .collect();
    let stack = (!frames.is_empty()).then(|| json!({ "type": "raw", "frames": frames }));
    let name = if error.name.trim().is_empty() { "Error" } else { error.name.trim() };
    exception(&truncate(name, 200), &truncate(&error.message, MAX_MESSAGE), &error.mechanism, stack)
}

fn exception(kind: &str, message: &str, mechanism: &str, stacktrace: Option<Value>) -> posthog_rs::Event {
    let mut entry = json!({
        "type": kind,
        "value": message,
        "mechanism": { "type": mechanism, "handled": false, "synthetic": false },
    });
    if let Some(stacktrace) = stacktrace {
        entry["stacktrace"] = stacktrace;
    }
    let mut event = posthog_rs::Event::new_anon("$exception");
    let _ = event.insert_prop("$exception_list", [entry]);
    let _ = event.insert_prop("$exception_level", "error");
    let _ = event.insert_prop("process", "webview");
    event
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_masks_home_everywhere() {
        let mut v = json!({
            "msg": "failed to open /Users/ada/code/app/file.rs",
            "list": [{ "path": "/Users/ada/.config" }, 3],
            "other": "/Users/adam/x and /Users/ada",
        });
        scrub(&mut v, Some("/Users/ada"));
        assert_eq!(v["msg"], "failed to open ~/code/app/file.rs");
        assert_eq!(v["list"][0]["path"], "~/.config");
        assert_eq!(v["other"], "/Users/adam/x and ~");
    }

    #[test]
    fn web_errors_become_personless_exceptions_with_frames() {
        let event = web_event(WebError {
            mechanism: "error".into(),
            name: "TypeError".into(),
            message: "x".repeat(MAX_MESSAGE + 10),
            frames: vec![
                WebFrame { function: Some("boot".into()), filename: Some("tauri://localhost/assets/index.js".into()), lineno: Some(1), colno: Some(2) },
                WebFrame { function: None, filename: Some("https://example.com/x.js".into()), lineno: None, colno: None },
            ],
        });
        let props = event.properties();
        assert_eq!(props["$process_person_profile"], false);
        assert_eq!(props["process"], "webview");
        let entry = &props["$exception_list"][0];
        assert_eq!(entry["type"], "TypeError");
        assert_eq!(entry["value"].as_str().unwrap().chars().count(), MAX_MESSAGE + 1);
        let frames = entry["stacktrace"]["frames"].as_array().unwrap();
        assert_eq!(frames[0]["function"], "boot");
        assert_eq!(frames[0]["in_app"], true);
        assert_eq!(frames[1]["function"], "?");
        assert_eq!(frames[1]["in_app"], false);
    }

    #[test]
    fn disabled_reports_are_dropped_before_sending() {
        set_enabled(false);
        assert!(prepare(posthog_rs::Event::new_anon("$exception")).is_none());
        set_enabled(true);
        let event = prepare(posthog_rs::Event::new_anon("$exception")).unwrap();
        assert_eq!(event.properties()["process"], "rust");
        set_enabled(false);
    }
}
