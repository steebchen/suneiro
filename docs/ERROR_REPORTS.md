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

1. **Discard client IP data** is on (Project settings → General).
2. Alerts: PostHog has no email destination for error alerts, so an assignment rule (Error tracking → Configuration → Assignment rules) assigns every new issue to the maintainer, and their "Issue assigned" email notification plus the "Error tracking weekly digest" are on. Slack/Discord/webhook alerts for spikes and reopened issues can be added under Configuration → Alerting.

## Readable stack traces

Release builds upload symbols with `posthog-cli` (`scripts/posthog-symbols.mjs`, pinned to `@posthog/cli@0.19.0`, release `suneiro-desktop@<version>`):

- **UI:** `tauri.release.conf.json` runs `pnpm build:release` = `vite build --sourcemap` then `posthog-symbols.mjs sourcemaps dist`. It injects a chunk ID into every bundle (`_posthogChunkIds`), uploads the maps, then deletes `.map` files and `sourceMappingURL` comments so no source ships inside the app. `lib/errorReports.ts` attaches each frame's `chunk_id`, which PostHog uses to find the map.
- **Rust:** the release step builds with `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` and `SPLIT_DEBUGINFO=packed`, so line tables go into a separate `suneiro-desktop.dSYM` per architecture (the shipped binary does not grow), and `posthog-symbols.mjs dsyms` uploads both after the release is published.
- **Credentials:** repository secret `POSTHOG_CLI_API_KEY`, a personal API key named "Suneiro release symbols (CI)" scoped to `error_tracking:write` on project 701028 only. Without the secret, uploads are skipped and the release still builds; a failed source-map upload is a warning, a failed dSYM upload fails the (already published) job so it is visible.
- Locally `pnpm build` produces no source maps; `pnpm --filter desktop build:release` without the env vars builds and strips maps without uploading.

## Try it locally

```sh
SUNEIRO_ERROR_REPORTS=1 pnpm dev
```

Then run `setTimeout(() => { throw new Error("test") })` in the webview devtools console; the event shows up under Error Tracking within a few seconds with `environment = development`.

Deterministic coverage: `cargo test -p suneiro-desktop error_reports`, `cargo test -p suneiro-core --lib setup`, `src/dev/error-reports.test.mjs`.

## Pricing

PostHog Error Tracking: first 100,000 exceptions per month free, then from $0.00037 per exception, decreasing with volume (https://posthog.com/pricing).
