import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { chunkIds, installErrorReporting, parseStack, toWebError, withChunkIds } from "../lib/errorReports.ts";

test("parses WebKit and V8 stacks oldest call first", () => {
  const webkit = "render@tauri://localhost/assets/index-abc.js:12:34\n@tauri://localhost/assets/index-abc.js:1:2\nglobal code";
  assert.deepEqual(parseStack(webkit), [
    { function: undefined, filename: "tauri://localhost/assets/index-abc.js", lineno: 1, colno: 2 },
    { function: "render", filename: "tauri://localhost/assets/index-abc.js", lineno: 12, colno: 34 },
  ]);
  const v8 = "TypeError: boom\n    at render (http://localhost:1420/src/App.tsx:5:6)\n    at http://localhost:1420/src/main.tsx:7:8";
  assert.deepEqual(parseStack(v8).map((f) => [f.function, f.lineno]), [[undefined, 7], ["render", 5]]);
  assert.deepEqual(parseStack(undefined), []);
});

test("normalizes thrown values", () => {
  const error = toWebError(new TypeError("bad"), "error");
  assert.equal(error.name, "TypeError");
  assert.equal(error.message, "bad");
  assert.deepEqual(toWebError({ code: 1 }, "unhandledrejection"), { mechanism: "unhandledrejection", name: "UnhandledRejection", message: '{"code":1}', frames: [] });
  assert.equal(toWebError(undefined, "unhandledrejection"), null);
});

test("forwards window errors and rejections once each, capped and never throwing", async () => {
  const listeners = {};
  const sent = [];
  installErrorReporting(async (e) => { sent.push(e); throw new Error("ipc down"); }, { addEventListener: (type, fn) => (listeners[type] = fn) });
  const err = new Error("same");
  listeners.error({ error: err, message: err.message });
  listeners.error({ error: err, message: err.message });
  listeners.error({ error: undefined, message: "" }); // resource load failure
  listeners.unhandledrejection({ reason: "nope" });
  for (let i = 0; i < 40; i++) listeners.error({ error: new Error(`e${i}`), message: "" });
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(sent[0].message, "same");
  assert.equal(sent[1].mechanism, "unhandledrejection");
  assert.equal(sent.length, 20);
});

test("frames carry the chunk IDs injected by posthog-cli", () => {
  const ids = chunkIds({
    "@tauri://localhost/assets/index-abc.js:1:120\nglobal code": "chunk-index",
    "Error\n    at http://tauri.localhost/assets/lazy-def.js:1:10": "chunk-lazy",
    "": "ignored",
    "@tauri://localhost/assets/bad.js:1:1": 42,
  });
  assert.deepEqual([...ids], [["tauri://localhost/assets/index-abc.js", "chunk-index"], ["http://tauri.localhost/assets/lazy-def.js", "chunk-lazy"]]);
  assert.equal(chunkIds(undefined).size, 0);
  const error = withChunkIds(toWebError(Object.assign(new Error("x"), { stack: "a@tauri://localhost/assets/index-abc.js:3:4\nb@tauri://localhost/other.js:5:6" }), "error"), ids);
  assert.deepEqual(error.frames.map((f) => f.chunk_id), [undefined, "chunk-index"]);
});

test("release builds upload source maps before Tauri embeds the bundle", () => {
  const pkg = JSON.parse(readFileSync(new URL("../../package.json", import.meta.url), "utf8"));
  assert.match(pkg.scripts["build:release"], /vite build --sourcemap && node \.\.\/\.\.\/scripts\/posthog-symbols\.mjs sourcemaps dist/);
  const release = JSON.parse(readFileSync(new URL("../../src-tauri/tauri.release.conf.json", import.meta.url), "utf8"));
  assert.equal(release.build.beforeBuildCommand, "pnpm build:release");
});

test("the desktop entry installs reporting only outside the browser mock", () => {
  const main = readFileSync(new URL("../main.tsx", import.meta.url), "utf8");
  assert.match(main, /installErrorReporting\(\(error\) => invoke\("report_error", \{ error \}\)\);\n  boot\(\);/);
  const settings = readFileSync(new URL("../components/Settings.tsx", import.meta.url), "utf8");
  assert.match(settings, /Send error reports/);
});
