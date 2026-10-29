import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";
import { allowRepliesControls, replyLimitsActive } from "../lib/replyLimits.ts";

const source = (name) => readFileSync(new URL(name, import.meta.url), "utf8");
const component = (name) => source(`../components/${name}.tsx`);

test("workspace opens directly into chat and related tabs with changes and terminal", () => {
  const workspace = component("WorkspaceView");
  for (const name of ["TabHierarchy", "Chat", "ChangesPanel", "TerminalPanel", "PrView", "PrActions", "SyncBadge"]) {
    assert.match(workspace, new RegExp(`<${name}\\b`));
  }
  assert.match(workspace, /type Tab = "changes" \| "terminal" \| "setup"/);
  assert.doesNotMatch(workspace, /ProfileLauncher|ProfileDialog|VerificationPanel|TaskResourcesButton|"checks"/);
});

test("human diff review keeps viewed marks, line comments, chat drafts and virtualized diffs", () => {
  const changes = component("ChangesPanel");
  for (const text of ["useViewed", "useVirtualizer", "Comment on this line", "Mark as viewed (V)", "actions.appendDraft", "onClick={sendComments}", "api.revertFile", "onComment="]) {
    assert.ok(changes.includes(text), `missing human review affordance: ${text}`);
  }
  assert.doesNotMatch(changes, /Reviewer agent|Review selected files|startReview|verificationTask|reviewFiles|Send findings as new task/);
});

test("sidebar retains workspace status and unread indicators without an inbox control center", () => {
  const sidebar = component("Sidebar");
  assert.match(sidebar, /<HomeButton \/>/);
  assert.match(sidebar, /<InsightsButton \/>/);
  assert.match(sidebar, /workspaceActivity/);
  assert.match(sidebar, /ws\.unread/);
  assert.match(sidebar, /<UpdateButton \/>/);
  assert.doesNotMatch(sidebar, /InboxButton|\.\/Inbox|TaskResources/);
});

test("routing remains chat-first and does not expose a separate inbox page", () => {
  const app = source("../App.tsx");
  for (const name of ["Sidebar", "Settings", "Home", "Insights", "WorkspaceView", "UpdateToast"]) {
    assert.match(app, new RegExp(`<${name}\\b`));
  }
  assert.doesNotMatch(app, /page === "inbox"|components\/Inbox|<Inbox/);
});

test("related chat settings use plain language without changing the saved depth preference", () => {
  const settings = component("Settings");
  assert.match(settings, /title="Related chats"/);
  assert.match(settings, /label="Related chat levels"/);
  assert.match(settings, /settings\.maxTabDepth/);
  assert.doesNotMatch(settings, /Roots have depth 0|Maximum child depth/);
});

test("advanced check and resource console components are removed", () => {
  assert.equal(existsSync(new URL("../components/VerificationPanel.tsx", import.meta.url)), false);
  assert.equal(existsSync(new URL("../components/Inbox.tsx", import.meta.url)), false);
});
test("saved reply limits stay hidden for defaults and appear only for saved restrictions", () => {
  const defaults = { maxConcurrent: 4, paused: false, stopUsd: null, warningUsd: null, spendSince: 42 };
  assert.equal(replyLimitsActive(undefined), false);
  assert.equal(replyLimitsActive(defaults), false);
  assert.equal(replyLimitsActive({ ...defaults, paused: true }), true);
  assert.equal(replyLimitsActive({ ...defaults, stopUsd: 0 }), true);
  assert.equal(replyLimitsActive({ ...defaults, warningUsd: 10 }), true);
});
test("explicit allow replies clears only pause and spend limits, retaining other saved controls", () => {
  const saved = { maxConcurrent: 3, paused: true, stopUsd: 5, warningUsd: 2, spendSince: 123456 };
  const next = allowRepliesControls(saved);
  assert.deepEqual(next, { maxConcurrent: 3, paused: false, stopUsd: null, warningUsd: null, spendSince: 123456 });
  assert.equal(saved.paused, true);
  assert.equal(saved.stopUsd, 5);
  const notice = component("SavedReplyLimits");
  assert.match(notice, /if \(!replyLimitsActive\(controls\)\) return null/);
  assert.match(notice, /allowRepliesControls\(await api\.repoControls\(repoId\)\)/);
  assert.match(notice, /onClick=\{\(\) => void allowReplies\(\)\}/);
  assert.doesNotMatch(notice.slice(notice.indexOf("useEffect(()"), notice.indexOf("if (!replyLimitsActive")), /saveRepoControls/);
});
