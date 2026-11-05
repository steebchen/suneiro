#!/usr/bin/env node
// Upload release symbols to PostHog Error Tracking so stack traces are readable
// (docs/ERROR_REPORTS.md). Needs POSTHOG_CLI_API_KEY (personal key with
// error_tracking:write) and POSTHOG_CLI_PROJECT_ID; without them it only cleans up.
//
//   node scripts/posthog-symbols.mjs sourcemaps apps/desktop/dist
//     Inject chunk IDs into the bundles, upload their source maps, then delete
//     the maps so they never ship inside the app. Runs before Tauri embeds dist.
//     A failed upload is a warning: the release still builds.
//   node scripts/posthog-symbols.mjs dsyms <file.dSYM>...
//     Upload the Rust binary's dSYMs (one per architecture) for panic frames.
import { cpSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";

const CLI = "@posthog/cli@0.19.0";
const RELEASE_NAME = "suneiro-desktop";
const version = JSON.parse(readFileSync(new URL("../apps/desktop/package.json", import.meta.url), "utf8")).version;
const [command, ...paths] = process.argv.slice(2);
const configured = Boolean(process.env.POSTHOG_CLI_API_KEY && process.env.POSTHOG_CLI_PROJECT_ID);

function posthog(...args) {
  const result = spawnSync("npx", ["--yes", CLI, ...args, "--release-name", RELEASE_NAME, "--release-version", version], {
    stdio: "inherit",
    env: { POSTHOG_CLI_HOST: "https://us.posthog.com", ...process.env },
  });
  return result.status === 0;
}

function files(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : [path];
  });
}

/** Delete source maps and their `sourceMappingURL` comments. */
function dropSourceMaps(dir) {
  for (const path of files(dir)) {
    if (path.endsWith(".map")) rmSync(path);
    else if (/\.(m?js|css)$/.test(path)) {
      const text = readFileSync(path, "utf8");
      const stripped = text.replace(/\n?\/[/*][#@] sourceMappingURL=\S+(?: \*\/)?\s*$/, "\n");
      if (stripped !== text) writeFileSync(path, stripped);
    }
  }
}

if (command === "sourcemaps" && paths.length === 1) {
  const [dir] = paths;
  if (!configured) console.log("posthog-symbols: POSTHOG_CLI_API_KEY/POSTHOG_CLI_PROJECT_ID not set, skipping source map upload");
  else if (!posthog("sourcemap", "process", "--directory", dir)) {
    console.log("::warning::Source map upload to PostHog failed; UI stack traces of this release stay minified");
  }
  dropSourceMaps(dir);
} else if (command === "dsyms" && paths.length) {
  if (!configured) {
    console.log("posthog-symbols: POSTHOG_CLI_API_KEY/POSTHOG_CLI_PROJECT_ID not set, skipping dSYM upload");
    process.exit(0);
  }
  let ok = true;
  for (const dsym of paths) {
    if (!existsSync(dsym)) {
      console.error(`posthog-symbols: ${dsym} does not exist`);
      ok = false;
      continue;
    }
    // One dSYM per upload directory, so build-script dSYMs next to it are skipped.
    const dir = mkdtempSync(join(tmpdir(), "dsym-"));
    cpSync(dsym, join(dir, basename(dsym)), { recursive: true });
    ok = posthog("dsym", "upload", "--directory", dir) && ok;
    rmSync(dir, { recursive: true, force: true });
  }
  process.exit(ok ? 0 : 1);
} else {
  console.error("usage: posthog-symbols.mjs sourcemaps <dist> | dsyms <file.dSYM>...");
  process.exit(1);
}
