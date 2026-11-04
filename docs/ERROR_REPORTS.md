# Error reports

Suneiro reports crashes and unexpected errors to [PostHog Error Tracking](https://posthog.com/docs/error-tracking), in the same PostHog project as product analytics (US cloud, project 701028). Error reports have their own setting and do not depend on analytics consent.

## What is reported

| Source | How |
|---|---|
| Rust panics (core and Tauri layer) | `posthog-rs` panic hook, installed at launch |
| Uncaught UI errors, unhandled promise rejections | `window` listeners in `lib/errorReports.ts` → `report_error` command → Rust client |
| WebKit content-process crashes (macOS) | `on_web_content_process_terminate` hook, then the webview reloads |

Each event is a personless `$exception` (random distinct ID, `$process_person_profile: false`, `$geoip_disable: true`) with the exception type, message, stack frames, `$app_version`, `$os`, `arch`, `environment` and `process` (`rust` or `webview`). The home folder is replaced by `~` in every property. No prompts, transcripts, file contents, settings, credentials, user or install IDs are attached. The UI sends at most 20 distinct errors per launch.

Not covered: native (non-panic) crashes such as segfaults in system libraries, which would need minidumps (Sentry supports those; PostHog does not).

## Consent

- `Settings.error_reports`, default **on**, toggle in Settings → Privacy.
- The setting is read before Tauri starts (`setup::saved_settings`, which never creates or migrates the database). When it is off, no client or panic hook is created.
- Every event passes `before_send`, which drops it while the setting is off, so turning it off applies immediately. Turning it on applies at the next launch.
- Debug builds never report unless `SUNEIRO_ERROR_REPORTS=1` is set.

## PostHog project setup (one time)

1. In PostHog → Project settings → General, enable **Discard client IP data**.
2. Open Error Tracking (https://us.posthog.com/project/701028/error_tracking) and set up alerts (e.g. new issue → Slack/email).
3. Optional, for readable stack traces in release builds: upload JavaScript source maps and Rust debug symbols with `posthog-cli` from the release workflow (`posthog-cli sourcemap inject/upload` for `apps/desktop/dist`; see https://posthog.com/docs/error-tracking/upload-source-maps). Until then, UI frames point into the minified bundle; Rust panic frames are resolved in-process as far as the release binary's symbols allow.

## Try it locally

```sh
SUNEIRO_ERROR_REPORTS=1 pnpm dev
```

Then run `setTimeout(() => { throw new Error("test") })` in the webview devtools console; the event shows up under Error Tracking within a few seconds with `environment = development`.

Deterministic coverage: `cargo test -p suneiro-desktop error_reports`, `cargo test -p suneiro-core --lib setup`, `src/dev/error-reports.test.mjs`.

## Pricing

PostHog Error Tracking: first 100,000 exceptions per month free, then from $0.00037 per exception, decreasing with volume (https://posthog.com/pricing).
