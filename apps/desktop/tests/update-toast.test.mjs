import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { runInNewContext } from "node:vm";
import ts from "typescript";

const require = createRequire(import.meta.url);

function fixture(phase = "waiting", dismissed = null) {
  const state = {
    update: { version: "0.8.0", phase, automatic: false, activity: { agents: 1, workspaces: 1 } },
    updateDismissed: dismissed,
    workspaces: [],
  };
  const calls = [];
  const useStore = (select) => select(state);
  useStore.getState = () => state;
  useStore.setState = (patch) => Object.assign(state, patch);
  const mocks = {
    "../lib/store": { useStore, toast() {}, workspaceActivity: () => "idle" },
    "../lib/api": { api: {
      cancelUpdate: async () => { calls.push("cancel"); },
      installUpdate: async (idle) => { calls.push(idle ? "idle" : "restart"); },
    } },
    "@tauri-apps/plugin-dialog": { ask: async () => true },
    "lucide-react": { Download: () => null, Loader2: () => null },
  };
  function load(file, name) {
    let source = readFileSync(new URL("../src/components/" + file + ".tsx", import.meta.url), "utf8");
    if (name === "UpdateButton") {
      const parsed = ts.createSourceFile(file + ".tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
      const declaration = parsed.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === name);
      source = 'import { Download } from "lucide-react"; import { useStore } from "../lib/store"; export ' + declaration.getText(parsed);
    }
    const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX } });
    const exports = {};
    runInNewContext(outputText, { exports, require: (id) => mocks[id] ?? require(id) });
    return exports[name];
  }
  return { state, calls, toast: load("UpdateToast", "UpdateToast"), indicator: load("Sidebar", "UpdateButton") };
}

function text(node) {
  if (node == null || typeof node === "boolean") return "";
  if (typeof node !== "object") return String(node);
  return [node.props?.children].flat(Infinity).map(text).join("");
}

function buttons(node) {
  if (!node || typeof node !== "object") return [];
  return [...(node.type === "button" ? [node] : []), ...[node.props?.children].flat(Infinity).flatMap(buttons)];
}

test("queued update offers Hide immediately before Restart now without canceling", () => {
  const f = fixture();
  const controls = buttons(f.toast());
  assert.deepEqual(controls.map(text), ["Cancel", "Hide", "Restart now"]);
  const update = f.state.update;
  controls[1].props.onClick();
  assert.equal(f.state.updateDismissed, "0.8.0");
  assert.equal(f.state.update, update);
  assert.deepEqual(f.calls, []);
  assert.equal(f.toast(), null);
  f.state.update = { ...update, activity: { agents: 2, workspaces: 2 } };
  assert.equal(f.toast(), null);
});

test("hidden queued update stays in the sidebar and can be reopened", () => {
  const f = fixture("waiting", "0.8.0");
  assert.equal(f.toast(), null);
  const indicator = f.indicator();
  assert.match(text(indicator), /Update queued/);
  assert.ok(!indicator.props.disabled);
  indicator.props.onClick();
  assert.equal(f.state.updateDismissed, null);
  assert.notEqual(f.toast(), null);
  assert.deepEqual(f.calls, []);
});

test("Cancel and Restart now retain their updater actions", async () => {
  const f = fixture();
  const controls = buttons(f.toast());
  await controls.find((button) => text(button) === "Cancel").props.onClick();
  await controls.find((button) => text(button) === "Restart now").props.onClick();
  assert.deepEqual(f.calls, ["cancel", "restart"]);
});

test("dismissal is version-specific and available updates still reopen", () => {
  const f = fixture("available");
  assert.equal(f.indicator(), null);
  buttons(f.toast()).find((button) => text(button) === "Dismiss").props.onClick();
  assert.equal(f.toast(), null);
  f.indicator().props.onClick();
  assert.notEqual(f.toast(), null);
  f.state.updateDismissed = "0.7.0";
  assert.notEqual(f.toast(), null);
});

test("no update renders neither toast nor sidebar indicator", () => {
  for (const update of [null, { version: null, phase: "none" }]) {
    const f = fixture();
    f.state.update = update;
    assert.equal(f.toast(), null);
    assert.equal(f.indicator(), null);
  }
});
