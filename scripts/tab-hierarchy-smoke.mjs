import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const browser = await chromium.launch({ headless: true });
const output = process.env.SUNEIRO_SCREENSHOTS ?? "/tmp/suneiro-tab-hierarchy";
const url = new URL(process.env.SUNEIRO_UI_URL ?? "http://127.0.0.1:5145");
url.searchParams.set("hierarchy", "");
await mkdir(output, { recursive: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: "light" });
await page.addInitScript(() => {
  window.__hierarchyTestStore = () => {
    const entry = performance.getEntriesByType("resource").findLast((x) => /\/src\/lib\/store\.ts(?:\?|$)/.test(x.name));
    return import(entry?.name ?? "/src/lib/store.ts");
  };
});
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
try {
  await page.goto(url.href);
  await page.locator('[role="treeitem"][aria-level="3"]').waitFor();
  assert.equal(await page.getByRole("treeitem").count(), 4);
  const root = page.locator('[data-session-id="s1"]');
  await root.click();
  await page.waitForFunction(() => document.querySelector('[data-session-id="s1"][aria-selected="true"]'));
  assert.equal(
    await page.evaluate(() => document.activeElement?.getAttribute("data-session-id")),
    "s1",
    "chat composer must not steal tree focus",
  );
  await root.press("ArrowRight");
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("data-session-id")), "s3");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.querySelector('[data-session-id="s3"][aria-selected="true"]'));
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("data-session-id")), "s3");
  await page.keyboard.press("ArrowLeft");
  await page.waitForFunction(() => document.querySelector('[data-session-id="s3"]')?.getAttribute("aria-expanded") === "false");
  assert.equal(await page.getByRole("treeitem").count(), 3);
  await page.keyboard.press("ArrowRight");
  await page.locator('[data-session-id="s4"]').waitFor();
  await page.getByRole("button", { name: "New related chat", exact: true }).click();
  await page.getByRole("button", { name: "Child chat", exact: true }).click();
  await page.waitForFunction(() => document.querySelectorAll('[role="treeitem"]').length === 5);
  await chooseCodex(page);
  await page.waitForFunction(async () => {
    const { useStore } = await window.__hierarchyTestStore();
    const s = useStore.getState();
    return s.sessions.w1.some((x) => x.id === s.selectedSession.w1 && x.agentId === "codex" && x.parentSessionId === "s3");
  });
  const result = await page.evaluate(async () => {
    const { actions, useStore } = await window.__hierarchyTestStore();
    const { api } = await import("/src/lib/api.ts");
    const check = (ok, message) => {
      if (!ok) throw new Error(message);
    };
    const initial = useStore.getState();
    const created = initial.sessions.w1.find((x) => !["s1", "s3", "s4"].includes(x.id));
    check(created?.parentSessionId === "s3", "child menu must use selected parent");
    const grandchild = await actions.newRelatedChat(created.id, "child");
    check(grandchild?.parentSessionId === created.id, "grandchild parent");
    const sibling = await actions.newRelatedChat(created.id, "sibling");
    check(sibling?.parentSessionId === "s3", "sibling parent");
    const isolated = await actions.newRelatedChat("s1", "child", "w2");
    check(isolated?.workspaceId === "w2" && isolated?.parentSessionId === "s1", "cross-workspace relationship");
    await actions.closeSession("w1", created.id);
    const surviving = useStore.getState().sessions.w1.find((x) => x.id === grandchild.id);
    check(surviving?.parentSessionId === null, "close must detach surviving direct children");
    check(
      useStore.getState().sessions.w1.some((x) => x.id === sibling.id),
      "close must not delete siblings",
    );
    await actions.detachSession(isolated.id);
    check(useStore.getState().sessions.w2.find((x) => x.id === isolated.id)?.parentSessionId === null, "detach");
    await actions.saveSettings({ maxTabDepth: 2 });
    let rejected = false;
    try {
      await api.createChildSession("s4", "w1", "claude");
    } catch {
      rejected = true;
    }
    check(rejected, "depth limit");
    await actions.saveSettings({ maxTabDepth: 4 });
    let missingRejected = false;
    try {
      await api.createChildSession("missing", "w1", "claude");
    } catch {
      missingRejected = true;
    }
    check(missingRejected, "missing parent");
    await actions.closeSession("w2", isolated.id);
    await actions.closeSession("w2", "s5");
    let lastProtected = false;
    try {
      await api.deleteSession("s2");
    } catch {
      lastProtected = true;
    }
    check(lastProtected, "last chat protection");
    actions.selectWorkspace("w1");
    actions.selectSession("w1", "s4");
    return [
      "child menu",
      "grandchildren",
      "siblings",
      "isolated workspace",
      "non-cascading close",
      "detach",
      "depth limit",
      "missing parent",
      "last chat protection",
    ];
  });
  const parentWithChildren = await page.evaluate(async () => {
    const { actions } = await window.__hierarchyTestStore();
    const parent = await actions.newRelatedChat("s1", "child");
    const child = await actions.newRelatedChat(parent.id, "child");
    actions.selectSession("w1", parent.id);
    return { parent: parent.id, child: child.id };
  });
  await chooseCodex(page);
  await page.waitForFunction(async () => {
    const { useStore } = await window.__hierarchyTestStore();
    const s = useStore.getState();
    return s.sessions.w1.some((x) => x.id === s.selectedSession.w1 && x.agentId === "codex" && x.parentSessionId === "s1");
  });
  assert.equal(await page.evaluate(async ({ parent, child }) => {
    const { useStore } = await window.__hierarchyTestStore();
    const list = useStore.getState().sessions.w1;
    return list.some((x) => x.id === parent) && list.find((x) => x.id === child)?.parentSessionId === parent;
  }, parentWithChildren), true, "model switch must not close empty parents with children");
  result.push("model switch preserves hierarchy", "empty parent retention");
  await page.reload();
  await page.locator('[data-session-id="s5"]').waitFor();
  await page.evaluate(async () => {
    const { api } = await import("/src/lib/api.ts");
    await api.sendPrompt("s5", "Verify integration");
  });
  await page.waitForFunction(() => document.querySelectorAll('[aria-label="needs input"]').length === 2);
  await page.evaluate(async () => {
    const { api } = await import("/src/lib/api.ts");
    await api.respondPermission("s5", "p1", "allow");
  });
  await page.waitForFunction(() => document.querySelectorAll('[aria-label="unread"]').length === 2);
  await page.locator('[data-session-id="s5"]').click();
  await page.waitForFunction(() => document.querySelector('[data-session-id="s5"][aria-selected="true"]'));
  const workspace = await page.evaluate(async () => {
    const { useStore } = await window.__hierarchyTestStore();
    return useStore.getState().selectedWorkspace;
  });
  assert.equal(workspace, "w2");
  result.push("keyboard focus", "collapse/expand", "descendant input/unread rollup", "cross-workspace navigation");
  await page.reload();
  await page.locator('[role="treeitem"][aria-level="3"]').waitFor();
  await page.screenshot({ path: `${output}/light.png` });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({ path: `${output}/dark.png` });
  for (const width of [1280, 1024]) {
    await page.setViewportSize({ width, height: 900 });
    const overflow = await page.evaluate(() => {
      const tree = document.querySelector('[role="tree"]');
      const rect = tree.getBoundingClientRect();
      return rect.right > innerWidth || rect.left < 0;
    });
    assert.equal(overflow, false, `tree overflow at ${width}px`);
  }
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ passed: result, screenshots: output, pageErrors: errors }, null, 2));
} finally {
  await browser.close();
}
async function chooseCodex(page) {
  await page.getByTitle("Model and effort", { exact: true }).click();
  await page.getByText("GPT-6 Astra", { exact: true }).click();
}
