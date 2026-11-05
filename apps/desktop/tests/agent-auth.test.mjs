import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";
import { createRequire } from "node:module";
import { runInNewContext } from "node:vm";

// Use the existing TypeScript compiler so this test needs no agent or extra runner.
const source = readFileSync(new URL("../src/components/AgentError.tsx", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, esModuleInterop: true },
}).outputText;
const exports = {};
runInNewContext(compiled, { exports, require: createRequire(import.meta.url) });
const { AgentError, subscriptionAuthAgent } = exports;


const errors = [
  "ACP `session/prompt` failed: agent error -32000: Authentication required",
  "Failed to authenticate: OAuth session expired and could not be refreshed",
  "Authentication failed",
  "Your access token could not be refreshed because your refresh token was already used. Please log out and sign in again.",
  "Invalid refresh token",
  "Not logged in. Please sign in.",
  "401 Unauthorized",
];

test("Claude and Codex authentication failures offer subscription recovery", () => {
  for (const [agentId, name] of [["claude", "Claude Code"], ["codex", "Codex"]]) {
    for (const text of errors) assert.equal(subscriptionAuthAgent(agentId, false, text), name, text);
  }
});

test("unrelated errors, other agents and cloud tasks do not suggest local login", () => {
  for (const text of ["Rate limit exceeded", "Usage limit reached", "Connection timed out", "Permission denied", "HTTP 500", "Could not refresh model catalog"]) {
    assert.equal(subscriptionAuthAgent("claude", false, text), null);
    assert.equal(subscriptionAuthAgent("codex", false, text), null);
  }
  for (const agentId of ["opencode", "custom", undefined]) assert.equal(subscriptionAuthAgent(agentId, false, errors[0]), null);
  for (const agentId of ["claude", "codex"]) assert.equal(subscriptionAuthAgent(agentId, true, errors[0]), null);
});

test("auth error preserves details and opens settings only on explicit click", () => {
  let opened = 0;
  const props = { text: errors[0], agentId: "claude", cloud: false, onOpenSettings: () => opened++ };
  const markup = renderToStaticMarkup(createElement(AgentError, props));
  assert.ok(markup.includes(errors[0]));
  assert.match(markup, /Sign in to Claude Code again/);
  assert.match(markup, /Open Settings/);
  assert.equal(opened, 0);
  const tree = AgentError(props);
  tree.props.children[1].props.children[1].props.onClick();
  assert.equal(opened, 1);
  assert.doesNotMatch(renderToStaticMarkup(createElement(AgentError, { ...props, text: "Rate limit exceeded" })), /Open Settings/);
});

test("transcript error rows connect recovery to existing Settings without resending", () => {
  const chat = readFileSync(new URL("../src/components/Chat.tsx", import.meta.url), "utf8");
  const row = chat.slice(chat.indexOf('case "error":', chat.indexOf("const Row =")), chat.indexOf("/** After an agent error"));
  assert.match(row, /<SessionError sessionId=\{sessionId\} text=\{item.text\}/);
  assert.match(chat, /onOpenSettings=\{\(\) => actions.openSettings\(true\)\}/);
  const recovery = chat.slice(chat.indexOf("function SessionError("));
  assert.doesNotMatch(recovery, /actions\.(?:send|connectSession|reconnectCloud)\(/);
});
