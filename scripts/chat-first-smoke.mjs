import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.SUNEIRO_UI_URL ?? "http://127.0.0.1:1420";
const output = process.env.SUNEIRO_SCREENSHOTS ?? "/tmp/suneiro-chat-first";
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
const errors = [];
const passed = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.route("**/*", (route) => new URL(route.request().url()).hostname === new URL(base).hostname ? route.continue() : route.abort());
try {
  await mkdir(output, { recursive: true });
  await page.goto(`${base}/?hierarchy`);
  await page.getByPlaceholder(/Ask the agent/).waitFor();
  await page.waitForFunction(async () => (await import("/src/lib/store.ts")).useStore.getState().ready);
  const mock = await page.evaluate(async () => (await import("/src/lib/api.ts")).api.listAgents());
  assert.ok(mock.length && mock.every((agent) => agent.command === ""), "only run against the browser mock");
  for (const name of ["Profiles / handoff", "Inbox", "Checks", "Review selected files", "Resources"]) {
    assert.equal(await page.getByRole("button", { name, exact: true }).count(), 0, `${name} must not be a chat control`);
  }
  assert.equal(await page.getByLabel("Delegation brief", { exact: true }).count(), 0);
  assert.equal(await page.getByLabel("Related chat updates").count(), 0);
  passed.push("chat-first controls and empty chat updates");
  await page.getByTitle("Model and effort", { exact: true }).click();
  await page.getByPlaceholder(/Search models/).waitFor();
  await page.keyboard.press("Escape");
  passed.push("standard model picker");
  await page.screenshot({ path: `${output}/light.png` });
  await page.evaluate(async () => {
    const { useStore } = await import("/src/lib/store.ts");
    const state = useStore.getState();
    const id = state.selectedSession[state.selectedWorkspace];
    const original = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
    window.__chatAnswers = [];
    window.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "answer_question") window.__chatAnswers.push(structuredClone(args));
      return original(command, args);
    };
    useStore.setState({ views: { ...state.views, [id]: { ...state.views[id], questions: [{ requestId: "chat-question", message: "A couple of details before I start.", schema: { type: "object", required: ["scope", "details"], properties: { scope: { type: "string", title: "Approach", description: "How should I approach this task?", oneOf: [{ const: "focused", title: "Keep it focused", description: "Make the smallest useful change." }, { const: "explore", title: "Explore alternatives", description: "Compare approaches first." }] }, details: { type: "string", title: "Details", description: "Anything else I should keep in mind?" } } }, toolCallId: null, autoResolveMs: null, receivedAt: Date.now() }] } } });
  });
  await page.getByText("How should I approach this task?", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.__chatAnswers.length), 0, "questions must not auto-answer");
  await page.screenshot({ path: `${output}/question.png` });
  await page.getByRole("button", { name: /Keep it focused/ }).click();
  await page.getByPlaceholder("Your answer", { exact: true }).waitFor();
  assert.equal(await page.getByRole("button", { name: "Submit", exact: true }).isEnabled(), false);
  await page.getByPlaceholder("Your answer", { exact: true }).fill("Keep the existing behavior and run the relevant tests.");
  await page.getByRole("button", { name: "Submit", exact: true }).click();
  await page.waitForFunction(() => window.__chatAnswers.length === 1);
  const answer = await page.evaluate(() => window.__chatAnswers[0]);
  assert.deepEqual(answer.response, { action: "accept", content: { scope: "focused", details: "Keep the existing behavior and run the relevant tests." } });
  await page.getByText("The agent has questions", { exact: true }).waitFor({ state: "hidden" });
  passed.push("in-chat choice and typed answer, explicit submission and resolution");
  const target = await page.evaluate(async () => {
    const { useStore } = await import("/src/lib/store.ts");
    const state = useStore.getState();
    const caller = state.selectedSession[state.selectedWorkspace];
    const child = Object.values(state.sessions).flat().find((session) => session.id !== caller && session.workspaceId !== state.selectedWorkspace);
    if (!child) throw new Error("hierarchy fixture requires a related workspace");
    const task = { id: "chat-result", callerSessionId: caller, sessionId: child.id, workspaceId: child.workspaceId, state: "completed", report: "The callback tests cover the redirect edge cases. **Result: ready for your review.**", request: { requestKey: "chat-result", relation: "child", isolation: "isolated", base: "caller_head", agentId: child.agentId, model: null, effort: null, brief: "Review callback tests", acceptanceCriteria: "Check the behavior and report results", selectedContext: "" } };
    const original = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
    window.__TAURI_INTERNALS__.invoke = (command, args) => command === "delegation_tasks" && args.sessionId === caller ? Promise.resolve([task]) : original(command, args);
    useStore.setState({ delegationRevision: { ...state.delegationRevision, [caller]: (state.delegationRevision[caller] ?? 0) + 1 } });
    return { id: child.id, workspaceId: child.workspaceId };
  });
  const updates = page.getByLabel("Related chat updates");
  await updates.locator("summary").click();
  await updates.getByText("Result: ready for your review.", { exact: true }).waitFor();
  assert.equal(await updates.getByRole("textbox").count(), 0, "chat updates must not become a configuration form");
  await page.screenshot({ path: `${output}/result.png` });
  await updates.getByRole("button", { name: "Open chat", exact: true }).click();
  await page.waitForFunction(async (expected) => { const s = (await import("/src/lib/store.ts")).useStore.getState(); return s.selectedWorkspace === expected.workspaceId && s.selectedSession[expected.workspaceId] === expected.id; }, target);
  passed.push("compact related-chat result and direct chat navigation");
  await page.evaluate(() => { document.documentElement.dataset.theme = "dark"; });
  await page.screenshot({ path: `${output}/dark.png` });
  for (const width of [1440, 1280, 1024]) {
    await page.setViewportSize({ width, height: 1000 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `page overflow at ${width}`);
  }
  passed.push("light/dark and desktop widths");
  assert.deepEqual(errors, []);
  await writeFile(`${output}/validation.json`, JSON.stringify({ passed, pageErrors: errors }, null, 2));
  console.log(JSON.stringify({ passed, screenshots: output, pageErrors: errors }, null, 2));
} finally {
  await browser.close();
}
