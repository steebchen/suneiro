import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const browser = await chromium.launch({ headless: true });
const output = process.env.SUNEIRO_SCREENSHOTS ?? ".empryo/artifacts/chat-layout";
const url = new URL(process.env.SUNEIRO_UI_URL ?? "http://127.0.0.1:5145");
url.searchParams.set("hierarchy", "");
await mkdir(output, { recursive: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: "light" });
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
async function store(fn, arg) {
  return page.evaluate(
    async ({ source, arg }) => {
      const entry = performance.getEntriesByType("resource").findLast((x) => /\/src\/lib\/store\.ts(?:\?|$)/.test(x.name));
      const module = await import(entry?.name ?? "/src/lib/store.ts");
      return new Function("module", "arg", `return (${source})(module, arg)`)(module, arg);
    },
    { source: fn.toString(), arg },
  );
}
try {
  await page.goto(url.href);
  await page.getByRole("button", { name: "New root chat", exact: true }).waitFor();
  assert.equal(await page.getByRole("button", { name: "changes", exact: true }).count(), 0, "review panel must be closed by default");
  await page.getByRole("tablist", { name: "Root chats", exact: true }).waitFor();
  const panelToggle = page.getByRole("button", { name: "Show changes and terminal", exact: true });
  assert.equal(await panelToggle.getAttribute("aria-expanded"), "false");
  await panelToggle.click();
  await page.getByRole("button", { name: "changes", exact: true }).waitFor();
  await page.getByRole("button", { name: "Hide changes and terminal", exact: true }).click();
  await page.locator('[role="tab"][data-session-id="s3"]').click();
  await page.getByRole("tablist", { name: "Chats under Review limiter edge cases", exact: true }).waitFor();
  await page.locator('[role="tab"][data-session-id="s4"]').click();
  assert.equal(await page.getByRole("tablist").count(), 3, "only the active ancestry gets sub-tab rows");
  await page.locator('[role="tab"][data-session-id="s3"]').click();
  await page.locator('[role="tab"][data-session-id="s3"]').press("ArrowRight");
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("data-session-id")), "s5");
  await page.keyboard.press("Enter");
  assert.equal(await store(({ useStore }) => useStore.getState().selectedWorkspace), "w2");
  await store(({ actions }) => {
    actions.selectWorkspace("w1");
    actions.selectSession("w1", "s4");
  });
  await page.screenshot({ path: `${output}/horizontal-light.png` });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({ path: `${output}/horizontal-dark.png` });
  await page.emulateMedia({ colorScheme: "light" });
  await store(({ useStore }) => {
    useStore.setState((s) => ({
      sessions: {
        ...s.sessions,
        w1: [
          ...s.sessions.w1,
          ...Array.from({ length: 18 }, (_, i) => ({
            id: `overflow-${i}`,
            workspaceId: "w1",
            agentId: "claude",
            parentSessionId: null,
            acpSessionId: null,
            title: `A very long task title that must never stretch the chat navigation ${i}`,
            createdAt: 10 + i,
          })),
        ],
      },
    }));
  });
  for (const width of [1024, 1280, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    const bounds = await page
      .getByRole("tablist", { name: "Root chats", exact: true })
      .evaluate((el) => ({ client: el.clientWidth, scroll: el.scrollWidth, body: document.body.scrollWidth, viewport: innerWidth }));
    assert.ok(bounds.scroll > bounds.client, "many tabs must scroll horizontally");
    assert.ok(bounds.body <= bounds.viewport, `tabs must not overflow the viewport at ${width}px`);
    const tab = page.locator('[role="tab"][data-session-id="overflow-17"]');
    await tab.focus();
    await tab.press("Enter");
    await tab.scrollIntoViewIfNeeded();
    assert.ok((await tab.boundingBox()).width <= 192, "long titles must be capped");
  }
  await page.screenshot({ path: `${output}/overflow.png` });
  await store(({ actions }) => actions.openSettings(true));
  await page.getByRole("button", { name: "Vertical tree", exact: true }).click();
  assert.equal(await store(({ useStore }) => useStore.getState().settings.chatTabLayout), "tree");
  await page.getByRole("button", { name: "Close (Esc)", exact: true }).click();
  await page.getByRole("tree", { name: "Chat hierarchy", exact: true }).waitFor();
  await page.screenshot({ path: `${output}/vertical-tree.png` });
  assert.deepEqual(errors, []);
  console.log(
    "PASS: collapsed review panel, horizontal roots/sub-tabs, keyboard and cross-workspace navigation, capped/scrolling titles, light/dark, vertical setting",
  );
} finally {
  await browser.close();
}
