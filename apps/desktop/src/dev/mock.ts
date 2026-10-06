// Dev-only fake backend so the UI can run in a plain browser (`pnpm dev`,
// then open http://localhost:1420). Never bundled into the Tauri build.
/* eslint-disable @typescript-eslint/no-explicit-any */

type Channel = { id: number };
const callbacks = new Map<number, (data: any) => void>();
const channelIndex = new Map<number, number>();
let nextCb = 1;

function send(ch: Channel, message: any) {
  const index = channelIndex.get(ch.id) ?? 0;
  channelIndex.set(ch.id, index + 1);
  callbacks.get(ch.id)?.({ index, message });
}

let events: Channel | null = null;
const emit = (...batch: any[]) => events && send(events, batch);
// `?update` in the URL pretends a new version is available.
let updateStatus: any = new URLSearchParams(location.search).has("update")
  ? { version: "0.2.0", phase: "available", activity: { agents: 0, workspaces: 0 } }
  : { version: null, phase: "none", activity: { agents: 0, workspaces: 0 } };
let updateListener = 0;
const mockUpdateEvent = () => callbacks.get(updateListener)?.({ event: "update", id: 0, payload: updateStatus });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const repos = [
  { id: "r1", name: "acme-web", path: "/Users/dev/acme-web", defaultBranch: "main" },
  { id: "r2", name: "acme-api", path: "/Users/dev/acme-api", defaultBranch: "main" },
  { id: "r3", name: "docs", path: "/Users/dev/docs", defaultBranch: "main" },
];
const workspaces = [
  { id: "w1", repoId: "r1", name: "tokyo", branch: "suneiro/tokyo", baseBranch: "main", path: "/Users/dev/suneiro/workspaces/acme-web/tokyo", status: "ready", createdAt: Date.now() - 3 * 3600e3, title: "Add API rate limiting", archivedAt: null, unread: false },
  { id: "w2", repoId: "r1", name: "lisbon", branch: "suneiro/lisbon", baseBranch: "main", path: "/Users/dev/suneiro/workspaces/acme-web/lisbon", status: "ready", createdAt: Date.now() - 26 * 3600e3, title: "", archivedAt: null, unread: false },
];
const archived: any[] = [
  { id: "w9", repoId: "r1", name: "oslo", branch: "suneiro/oslo", baseBranch: "main", path: "/tmp/oslo", status: "archived", createdAt: Date.now() - 9 * 86400e3, title: "Migrate to Postgres 17", archivedAt: Date.now() - 2 * 86400e3 },
];
const sessions: Record<string, any[]> = {
  w1: [{ id: "s1", workspaceId: "w1", agentId: "claude", acpSessionId: "a1", title: "Add rate limiting to the API", createdAt: 1 }],
  w2: [{ id: "s2", workspaceId: "w2", agentId: "codex", acpSessionId: "a2", title: "Fix flaky checkout test", createdAt: 1 }],
};
const efforts = ["low", "medium", "high", "xhigh", "max"].map((v) => ({ value: v, name: v === "xhigh" ? "Extra high" : v[0].toUpperCase() + v.slice(1) }));
const catalogs: Record<string, any> = {
  claude: { models: [{ value: "opus", name: "Opus 5.5" }, { value: "claude-fable-5-1", name: "Fable 5.1" }, { value: "sonnet", name: "Sonnet 5" }, { value: "haiku", name: "Haiku 4.5" }], efforts, hasFast: true, updatedAt: 0 },
  codex: { models: [{ value: "gpt-6-astra", name: "6 Astra" }, { value: "gpt-6-sol", name: "6 Sol" }, { value: "gpt-6-luna", name: "6 Luna" }], efforts, hasFast: true, updatedAt: 0 },
  opencode: { models: [{ value: "zai/glm-5.3", name: "Z.ai/GLM-5.3" }, { value: "deepseek/deepseek-v4", name: "DeepSeek/DeepSeek V4" }, { value: "openrouter/aion-3.5", name: "OpenRouter/Aion 3.5" }], efforts: [], hasFast: false, updatedAt: 0 },
};
let settings = {
  loadout: [
    { agent: "claude", model: "opus", effort: "high" },
    { agent: "claude", model: "claude-fable-5-1", effort: "high" },
    { agent: "codex", model: "gpt-6-astra", effort: "high" },
    { agent: "codex", model: "gpt-6-sol", effort: "xhigh" },
    { agent: "opencode", model: "zai/glm-5.3", effort: null },
  ],
  opencodeModels: ["zai/glm-5.3", "deepseek/deepseek-v4"],
  planByDefault: false,
  enabledAgents: ["claude", "codex", "opencode"],
  defaultAgent: "claude",
  branchPrefix: "suneiro/",
  workspacesRoot: "/Users/dev/suneiro/workspaces",
  editor: "Visual Studio Code",
  theme: "system",
  renameBranches: true,
};
const config = [
  { id: "mode", name: "Mode", category: "mode", type: "select", currentValue: "bypassPermissions", options: [{ value: "default", name: "Manual" }, { value: "plan", name: "Plan" }, { value: "bypassPermissions", name: "Bypass permissions" }] },
  { id: "model", name: "Model", description: "AI model to use", category: "model", type: "select", currentValue: "opus", options: [{ value: "opus", name: "Opus 5.5" }, { value: "sonnet", name: "Sonnet 5" }] },
  { id: "effort", name: "Effort", description: "Available effort levels for this model", category: "thought_level", type: "select", currentValue: "high", options: efforts },
  { id: "fast", name: "Fast mode", description: "Faster responses on supported models", category: "model_config", type: "select", currentValue: "off", options: [{ value: "on", name: "On" }, { value: "off", name: "Off" }] },
];

const commands = {
  sessionUpdate: "available_commands_update",
  availableCommands: [
    { name: "review", description: "Review the current changes", input: null },
    { name: "compact", description: "Summarize the conversation to free up context", input: { hint: "focus" } },
    { name: "init", description: "Create an AGENTS.md for this repository", input: null },
    { name: "pr-comments", description: "Address the pull request's review comments", input: null },
  ],
};

const sessionHistory = (sid: string) => [
  { type: "userMessage", sessionId: sid, text: "Add rate limiting to the public API endpoints", ts: Date.now() - 134_000 },
  { type: "checkpoint", sessionId: sid, commit: "c0ffee1" },
  { type: "sessionUpdate", sessionId: sid, update: { sessionUpdate: "agent_message_chunk", messageId: "m1", content: { type: "text", text: "I'll look at how the API routes are set up first." } } },
  { type: "sessionUpdate", sessionId: sid, update: { sessionUpdate: "tool_call", toolCallId: "t1", title: "Read src/server/routes.ts", kind: "read", status: "completed" } },
  { type: "sessionUpdate", sessionId: sid, update: { sessionUpdate: "plan", entries: [{ content: "Add a token bucket limiter", priority: "high", status: "completed" }, { content: "Wire it into public routes", priority: "high", status: "completed" }, { content: "Add tests", priority: "medium", status: "in_progress" }] } },
  { type: "sessionUpdate", sessionId: sid, update: { sessionUpdate: "tool_call", toolCallId: "t2", title: "Edit src/server/limiter.ts", kind: "edit", status: "completed", content: [{ type: "diff", path: "src/server/limiter.ts", oldText: "export const limits = {};\n", newText: "export const limits = {\n  public: { rate: 60, burst: 20 },\n};\n" }] } },
  { type: "sessionUpdate", sessionId: sid, update: { sessionUpdate: "agent_message_chunk", messageId: "m2", content: { type: "text", text: "Done. Public routes now go through a **token bucket** limiter:\n\n- `60` requests/minute with a burst of `20`\n- returns `429` with a `Retry-After` header\n\n```ts\napp.use('/api/public', rateLimit(limits.public));\n```" } } },
  { type: "sessionUpdate", sessionId: sid, update: { sessionUpdate: "usage_update", used: 184_300, size: 1_000_000 } },
  { type: "turnEnd", sessionId: sid, stopReason: "end_turn", ts: Date.now() },
];

const patch = `diff --git a/src/server/limiter.ts b/src/server/limiter.ts
--- a/src/server/limiter.ts
+++ b/src/server/limiter.ts
@@ -1,3 +1,14 @@
 import type { Request } from "express";
-export const limits = {};
+export const limits = {
+  public: { rate: 60, burst: 20 },
+};
+
+export function rateLimit(opts: { rate: number; burst: number }) {
+  const buckets = new Map<string, number>();
+  return (req: Request, res: any, next: () => void) => {
+    const key = req.ip ?? "anon";
+    buckets.set(key, (buckets.get(key) ?? opts.burst) - 1);
+    next();
+  };
+}
 export default limits;`;

async function streamReply(sessionId: string, text: string, images?: string[]) {
  emit({ type: "userMessage", sessionId, text, ts: Date.now(), images }, { type: "sessionState", sessionId, state: "running", error: null }, { type: "checkpoint", sessionId, commit: `c${Date.now()}` });
  await sleep(300);
  emit({ type: "sessionUpdate", sessionId, update: { sessionUpdate: "tool_call", toolCallId: `x${Date.now()}`, title: "Run pnpm test", kind: "execute", status: "in_progress" } });
  const words = "Sure — I'll handle that. First I'll check the existing tests, then make the change and run the suite again to confirm everything passes.".split(" ");
  for (const w of words) {
    await sleep(40);
    emit({ type: "sessionUpdate", sessionId, update: { sessionUpdate: "agent_message_chunk", messageId: "live", content: { type: "text", text: w + " " } } });
  }
  emit({ type: "permissionRequest", sessionId, requestId: "p1", toolCall: { title: "Write src/server/limiter.test.ts" }, options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }, { optionId: "always", name: "Always allow", kind: "allow_always" }, { optionId: "reject", name: "Reject", kind: "reject_once" }] });
}

let pricing: Record<string, any> = {};
const attachments = new Map<string, string>();
const mergeState: Record<string, any> = {};
let repoSettings: any = { scripts: { run: "pnpm dev --port $SUNEIRO_PORT" }, copy: [".env"] };
function usageRows() {
  const rows: any[] = [];
  let seed = 7;
  const rand = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  const now = Date.now();
  for (let d = 29; d >= 0; d--) {
    const turns = Math.floor(rand() * 9);
    for (let t = 0; t < turns; t++) {
      const pick = rand();
      const [agent, model, session, ws] = pick < 0.6 ? ["claude", "opus", "s1", "w1"] : pick < 0.9 ? ["codex", "gpt-6-astra", "s2", "w2"] : ["opencode", "zai/glm-5.3", "s1", "w1"];
      const input = Math.floor(rand() * 4000);
      const cached = Math.floor(20000 + rand() * 180000);
      const output = Math.floor(200 + rand() * 6000);
      const p = pricing[model];
      const reported = agent === "codex" ? null : agent === "opencode" ? 0.01 * rand() : (input * 5 + cached * 0.5 + output * 25) / 1e6;
      const cost = reported ?? (p ? (input * p.input + cached * p.cachedInput + output * p.output) / 1e6 : null);
      rows.push({ sessionId: session, workspaceId: ws, repoId: "r1", agent, model, ts: now - d * 86400e3 - t * 600e3, inputTokens: input, cachedTokens: cached, outputTokens: output, costUsd: reported, cost, estimated: reported === null && cost !== null });
    }
  }
  return rows.sort((a, b) => a.ts - b.ts);
}

const handlers: Record<string, (a: any) => any> = {
  subscribe: (a) => {
    events = a.channel;
    setTimeout(() => emit({ type: "sessionConfig", sessionId: "s1", configOptions: config }, { type: "sessionUpdate", sessionId: "s1", update: commands }, { type: "sessionState", sessionId: "s1", state: "idle", error: null }), 50);
  },
  list_agents: () => [
    { id: "claude", name: "Claude Code", command: "", args: [] },
    { id: "codex", name: "Codex", command: "", args: [] },
    { id: "opencode", name: "OpenCode", command: "", args: [] },
  ],
  list_repos: () => repos.map((r) => ({ ...r })),
  reorder_repos: (a) => void repos.sort((x, y) => a.repoIds.indexOf(x.id) - a.repoIds.indexOf(y.id)),
  list_workspaces: () => workspaces.map((w) => ({ ...w })),
  list_all_workspaces: () => [...workspaces, ...archived].map((w) => ({ ...w })),
  archive_workspace: async (a) => {
    await sleep(500);
    const i = workspaces.findIndex((w) => w.id === a.workspaceId);
    if (i >= 0) archived.unshift({ ...workspaces.splice(i, 1)[0], status: "archived", archivedAt: Date.now() });
  },
  restore_workspace: (a) => {
    const i = archived.findIndex((w) => w.id === a.workspaceId);
    const ws = { ...archived.splice(i, 1)[0], status: "creating", archivedAt: null };
    workspaces.unshift(ws);
    sessions[ws.id] ??= [];
    setTimeout(() => {
      ws.status = "ready";
      emit({ type: "workspaceStatus", workspaceId: ws.id, status: "ready" });
    }, 1500);
    return { ...ws };
  },
  list_sessions: (a) => sessions[a.workspaceId] ?? [],
  session_events: (a) => sessionHistory(a.sessionId),
  get_settings: () => settings,
  save_settings: (a) => void (settings = a.settings),
  detect_agents: async () => {
    await sleep(400);
    return [
      { id: "claude", name: "Claude Code", installed: true, version: "2.1.284", loggedIn: true, account: "dev@example.com · max", installCommand: "curl -fsSL https://claude.ai/install.sh | bash", loginCommand: "claude auth login", logoutCommand: "claude auth logout" },
      { id: "codex", name: "Codex", installed: true, version: "0.140.0", loggedIn: false, account: null, installCommand: "npm install -g @openai/codex", loginCommand: "codex login", logoutCommand: "codex logout" },
      { id: "opencode", name: "OpenCode", installed: false, version: null, loggedIn: false, account: null, installCommand: "curl -fsSL https://opencode.ai/install | bash", loginCommand: "opencode auth login", logoutCommand: "opencode auth logout" },
    ];
  },
  answer_question: (a) => {
    emit(
      { type: "questionResolved", sessionId: a.sessionId, requestId: a.requestId },
      { type: "sessionUpdate", sessionId: a.sessionId, update: { sessionUpdate: "agent_message_chunk", messageId: `ans${Date.now()}`, content: { type: "text", text: `Got it: \`${JSON.stringify(a.response)}\`` } } },
      { type: "turnEnd", sessionId: a.sessionId, stopReason: "end_turn", ts: 0 },
      { type: "sessionState", sessionId: a.sessionId, state: "idle", error: null },
    );
  },
  send_prompt: (a) => {
    if (a.text.startsWith("ask")) {
      const codex = a.text.includes("codex");
      emit({ type: "userMessage", sessionId: a.sessionId, text: a.text, ts: Date.now() }, { type: "sessionState", sessionId: a.sessionId, state: "running", error: null });
      const schema = codex
        ? { type: "object", required: ["preferred_color", "include_tests"], properties: {
            include_tests: { type: "string", title: "Should tests be included?", description: "Tests", _meta: { codex: { isOther: true } }, oneOf: [{ const: "Yes", title: "Yes", description: "Include tests." }, { const: "No", title: "No", description: "Do not include tests." }, { const: "None of the above", title: "None of the above", description: "Provide a different answer in the note field." }] },
            include_tests_note: { type: "string", title: "Additional answer or note", _meta: { codex: { role: "user_note", questionId: "include_tests" } } },
            preferred_color: { type: "string", title: "What is your preferred color?", description: "Color", _meta: { codex: { isOther: true } }, oneOf: [{ const: "Red", title: "Red", description: "Use red." }, { const: "Blue", title: "Blue", description: "Use blue." }] },
            preferred_color_note: { type: "string", title: "Additional answer or note", _meta: { codex: { role: "user_note", questionId: "preferred_color" } } },
          } }
        : { type: "object", properties: {
            question_0: { type: "string", title: "Storage", description: "Where should rate-limit counters live?", oneOf: [{ const: "Redis", title: "Redis", description: "Shared across instances; needs a Redis server.", _meta: { "_claude/askUserQuestionOption": { preview: "const store = new RedisStore(redis);" } } }, { const: "In memory", title: "In memory", description: "Simplest; per-instance limits." }] },
            question_0_custom: { type: "string", title: "Other", description: "Type your own answer, or add a note to the option you chose above (optional)." },
            question_1: { type: "array", title: "Routes", description: "Which routes should be limited?", items: { anyOf: [{ const: "/api/public", title: "/api/public" }, { const: "/api/auth", title: "/api/auth" }, { const: "/api/admin", title: "/api/admin" }] } },
            question_1_custom: { type: "string", title: "Other", description: "Type your own answer to add to your selection above (optional)." },
          } };
      setTimeout(() => emit({ type: "question", sessionId: a.sessionId, requestId: `q${Date.now()}`, message: codex ? "Codex needs your input to continue." : "Please answer the following questions.", schema, toolCallId: null, autoResolveMs: codex ? 60000 : null }), 300);
      return;
    }
    const ws = workspaces.find((w) => sessions[w.id]?.some((x) => x.id === a.sessionId));
    if (ws && !ws.title) {
      const quick = a.text.split(/\s+/).slice(0, 5).join(" ");
      emit({ type: "workspaceTitle", workspaceId: ws.id, title: quick }, { type: "sessionTitle", sessionId: a.sessionId, title: quick });
      setTimeout(() => {
        ws.title = "Summarized title";
        emit({ type: "workspaceTitle", workspaceId: ws.id, title: ws.title }, { type: "sessionTitle", sessionId: a.sessionId, title: ws.title });
        if (settings.renameBranches && ws.branch.endsWith(ws.name)) {
          ws.branch = "suneiro/summarized-title";
          emit({ type: "workspaceBranch", workspaceId: ws.id, branch: ws.branch });
        }
      }, 1500);
    }
    void streamReply(a.sessionId, a.text, a.images);
  },
  create_workspace: (a) => {
    const n = workspaces.length + 1;
    const ws = { id: `w${n}`, repoId: a.repoId, name: `city${n}`, branch: `suneiro/city${n}`, baseBranch: "main", path: `/tmp/city${n}`, status: "creating", createdAt: Date.now(), title: "", archivedAt: null, unread: false };
    workspaces.unshift(ws);
    sessions[ws.id] = [];
    setTimeout(() => {
      ws.status = "ready";
      emit({ type: "workspaceStatus", workspaceId: ws.id, status: "ready" });
    }, 2000);
    return { ...ws };
  },
  list_branches: () => [
    { name: "feature/dark-mode", remote: false, updatedAt: Date.now() - 2 * 3600e3, subject: "Add theme toggle" },
    { name: "fix/checkout-race", remote: true, updatedAt: Date.now() - 3 * 86400e3, subject: "Serialize cart updates" },
    { name: "main", remote: false, updatedAt: Date.now() - 600e3, subject: "Merge #41" },
  ],
  open_prs: async () => {
    await sleep(300);
    return [
      { number: 44, title: "Upgrade to React 19.3", headRefName: "deps/react-19", baseRefName: "main", author: { login: "octocat" }, isCrossRepository: false, isDraft: false, updatedAt: new Date(Date.now() - 5 * 3600e3).toISOString() },
      { number: 43, title: "Docs: rate limits", headRefName: "docs-limits", baseRefName: "main", author: { login: "contrib" }, isCrossRepository: true, isDraft: true, updatedAt: new Date(Date.now() - 86400e3).toISOString() },
    ];
  },
  create_workspace_from: async (a) => {
    await sleep(300);
    const ws = handlers.create_workspace(a);
    const target = workspaces.find((w) => w.id === ws.id)!;
    target.branch = ws.branch = a.branch ?? `pr-${a.pr}`;
    target.title = ws.title = a.pr ? `PR #${a.pr}` : "";
    return ws;
  },
  create_session: (a) => {
    const x = { id: `s${Date.now()}`, workspaceId: a.workspaceId, agentId: a.agentId, acpSessionId: null, title: "", createdAt: Date.now() };
    sessions[a.workspaceId] = [...(sessions[a.workspaceId] ?? []), x];
    setTimeout(() => emit({ type: "sessionConfig", sessionId: x.id, configOptions: config }, { type: "sessionUpdate", sessionId: x.id, update: commands }, { type: "sessionState", sessionId: x.id, state: "idle", error: null }), 2200);
    return x;
  },
  respond_permission: (a) => {
    emit({ type: "permissionResolved", sessionId: a.sessionId, requestId: a.requestId }, { type: "turnEnd", sessionId: a.sessionId, stopReason: "end_turn", ts: Date.now() }, { type: "sessionState", sessionId: a.sessionId, state: "idle", error: null });
  },
  cancel_prompt: () => {},
  commit_and_push: async () => sleep(800),
  draft_pr: async () => {
    await sleep(1200);
    return ["Add rate limiting to the public API", "Public endpoints now go through a token bucket limiter, so a single client can't exhaust the API.\n\n- `rateLimit()` middleware with per-IP buckets\n- 60 requests/minute, bursts of 20\n- Tests for the limiter"];
  },
  sync_status: (a) => mergeState[a.workspaceId] ?? { ahead: 3, behind: 2, merging: false, conflicts: [] },
  merge_base_branch: async (a) => {
    await sleep(600);
    mergeState[a.workspaceId] = { ahead: 4, behind: 0, merging: true, conflicts: ["src/server/routes.ts"] };
    return ["src/server/routes.ts"];
  },
  abort_merge: (a) => void delete mergeState[a.workspaceId],
  save_attachment: (a) => {
    const path = `/mock/attachments/${Date.now()}.png`;
    attachments.set(path, `data:${a.mimeType};base64,${a.data}`);
    return path;
  },
  import_attachment: (a) => a.path,
  attachment_data_url: (a) => attachments.get(a.path) ?? "data:image/svg+xml;base64," + btoa('<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><rect width="80" height="60" fill="#8ab"/></svg>'),
  restore_checkpoint: (a) => {
    const undo = `u${Date.now()}`;
    emit({ type: "checkpointRestored", sessionId: a.sessionId, commit: a.commit, undo, ts: Date.now() });
    return undo;
  },
  steer: (a) => {
    emit(
      { type: "userMessage", sessionId: a.sessionId, text: a.text, ts: Date.now() },
      { type: "sessionUpdate", sessionId: a.sessionId, update: { sessionUpdate: "agent_message_chunk", messageId: `st${Date.now()}`, content: { type: "text", text: `On it — also handling: ${a.text}` } } },
    );
    return "injected";
  },
  cached_prs: () => ({
    w1: { number: 42, url: "https://github.com/acme/web/pull/42", state: "OPEN", title: "Add API rate limiting", isDraft: false, mergeable: "MERGEABLE", mergeStateStatus: "BLOCKED", statusCheckRollup: [{ name: "ci", conclusion: "FAILURE" }, { name: "lint", conclusion: "SUCCESS" }] },
    w2: { number: 38, url: "https://github.com/acme/web/pull/38", state: "MERGED", title: "Fix flaky checkout test", isDraft: false, mergeable: "UNKNOWN", mergeStateStatus: "UNKNOWN", statusCheckRollup: [{ name: "ci", conclusion: "SUCCESS" }] },
  }),
  refresh_prs: () => {
    for (const w of workspaces) if (w.id !== "w1" && w.id !== "w2") emit({ type: "workspacePr", workspaceId: w.id, pr: null });
  },
  fetch_image: (a) => a.url,
  pr_details: async () => {
    await sleep(300);
    const img = "https://raw.githubusercontent.com/theopenco/llmgateway/019a437c68afa72f1ba04a6f8c7f9bbdb98a2d9d/agents-light.png";
    const user = { login: "steebchen", avatar_url: "https://avatars.githubusercontent.com/u/6099000?v=4" };
    return {
      repo: "acme/web",
      pull: {
        number: 42, title: "Add API rate limiting", html_url: "https://github.com/acme/web/pull/42", state: "open", draft: false, merged: false,
        additions: 46, deletions: 1, changed_files: 2, user, base: { ref: "main" }, head: { ref: "suneiro/tokyo" },
        body_html: `<h2>Problem</h2><p>Public endpoints have no rate limiting, so a single client can exhaust the API.</p><h2>Approach</h2><ul><li><strong>Limiter</strong>: token bucket per IP in <code>src/server/limiter.ts</code> (60/min, burst 20)</li><li><strong>Routes</strong>: applied to <code>/api/public</code>, returns <code>429</code> with <code>Retry-After</code></li></ul><ul class="contains-task-list"><li class="task-list-item"><input type="checkbox" checked disabled> Unit tests</li><li class="task-list-item"><input type="checkbox" disabled> Load test in staging</li></ul><h2>Screenshots</h2><p><img src="${img}" alt="agents view"></p><details><summary>Benchmark</summary><pre><code>p50 1.2ms  p99 3.4ms</code></pre></details>`,
      },
      comments: [{ id: 1, user: { login: "reviewer-bot", avatar_url: "" }, created_at: new Date(Date.now() - 3600e3).toISOString(), html_url: "https://github.com/acme/web/pull/42#issuecomment-1", body_html: "<p>Bundle size unchanged ✅</p>" }],
      reviews: [{ id: 10, user, state: "CHANGES_REQUESTED", submitted_at: new Date(Date.now() - 1800e3).toISOString(), html_url: "https://github.com/acme/web/pull/42#pullrequestreview-10", body_html: "<p>Looks good overall, one concern about memory growth.</p>" }],
      reviewComments: [{ id: 100, pull_request_review_id: 10, path: "src/server/limiter.ts", line: 7, diff_hunk: "@@ -1,3 +1,14 @@\n+export function rateLimit(opts) {\n+  const buckets = new Map<string, number>();", body_html: "<p>This map never shrinks. Evict idle keys?</p>" }],
      checks: [{ name: "ci", workflowName: "CI", conclusion: "FAILURE", detailsUrl: "https://github.com" }, { name: "lint", workflowName: "CI", conclusion: "SUCCESS", detailsUrl: "https://github.com" }],
    };
  },
  usage: () => usageRows(),
  get_pricing: () => pricing,
  save_pricing: (a) => void (pricing = a.pricing),
  check_for_updates: () => updateStatus,
  update_status: () => updateStatus,
  install_update: (a) => {
    updateStatus = { ...updateStatus, phase: a.whenIdle ? "waiting" : "installing", activity: { agents: 1, workspaces: 1 } };
    mockUpdateEvent();
    if (!a.whenIdle) setTimeout(() => location.reload(), 1500);
  },
  cancel_update: () => {
    updateStatus = { ...updateStatus, phase: "available", activity: { agents: 0, workspaces: 0 } };
    mockUpdateEvent();
  },
  "plugin:event|listen": (a) => {
    if (a.event === "update") updateListener = a.handler;
    return a.handler;
  },
  "plugin:event|unlisten": () => {},
  connect_session: () => {},
  recent_projects: () => [
    { path: "/Users/dev/projects/obsidian", name: "obsidian", lastUsed: Date.now() },
    { path: "/Users/dev/projects/example-project", name: "example-project", lastUsed: Date.now() },
    { path: "/Users/dev/projects/contracts", name: "contracts", lastUsed: Date.now() },
  ],
  clone_repo: async (a) => {
    await sleep(800);
    return { id: `r${Date.now()}`, name: a.spec.split("/").pop(), path: `/Users/dev/projects/${a.spec.split("/").pop()}`, defaultBranch: "main" };
  },
  list_files: () => ["README.md", "package.json", "src/server/limiter.ts", "src/server/limiter.test.ts", "src/server/routes.ts", "src/app.tsx", "docs/rate-limits.md"],
  model_catalogs: () => catalogs,
  refresh_catalog: async (a) => {
    await sleep(300);
    return catalogs[a.agentId];
  },
  rename_workspace: (a) => {
    const ws = workspaces.find((w) => w.id === a.workspaceId);
    if (ws) ws.title = a.title;
    emit({ type: "workspaceTitle", workspaceId: a.workspaceId, title: a.title });
  },
  set_workspace_unread: (a) => {
    const ws: any = workspaces.find((w) => w.id === a.workspaceId);
    if (ws) ws.unread = a.unread;
  },
  set_config: (a) => {
    const o: any = config.find((c) => c.id === a.configId);
    if (o) o.currentValue = a.value;
    emit({ type: "sessionConfig", sessionId: a.sessionId, configOptions: config.map((c) => ({ ...c })) });
  },
  set_plan_mode: (a) => emit({ type: "sessionMode", sessionId: a.sessionId, plan: a.plan }),
  changed_files: () => [
    { path: "src/server/limiter.ts", status: "M", additions: 12, deletions: 1 },
    { path: "src/server/limiter.test.ts", status: "A", additions: 34, deletions: 0 },
  ],
  file_diff: () => patch,
  pr_status: () => null,
  repo_config: () => ({ scripts: { run: "pnpm dev" }, copy: [] }),
  repo_settings: () => ({ config: repoSettings, file: { name: "conductor.json", config: { scripts: { setup: "pnpm install" }, copy: [] } } }),
  save_repo_settings: (a) => void (repoSettings = a.config),
  terminal_open: (a) => {
    const out = a.command
      ? `$ ${a.command}\r\n\r\n  \x1b[32mVITE\x1b[0m v8.3.1  ready in 212 ms\r\n\r\n  ➜  Local:   \x1b[36mhttp://localhost:\x1b[1m50120\x1b[22m/\x1b[0m\r\n`
      : "\x1b[32m➜\x1b[0m tokyo git:(suneiro/tokyo) ";
    setTimeout(() => send(a.onData, new TextEncoder().encode(out).buffer), 50);
  },
  setup_terminal_open: (a) => {
    setTimeout(() => send(a.onData, new TextEncoder().encode("Opening browser to sign in…\r\n").buffer), 100);
  },
  terminal_write: () => {},
  terminal_resize: () => {},
  terminal_kill: () => {},
  "plugin:window|set_badge_count": () => {},
};

(window as any).__TAURI_INTERNALS__ = {
  transformCallback(cb: (d: any) => void) {
    const id = nextCb++;
    callbacks.set(id, cb);
    return id;
  },
  unregisterCallback(id: number) {
    callbacks.delete(id);
  },
  async invoke(cmd: string, args: any) {
    const h = handlers[cmd];
    if (!h) {
      console.warn("[mock] unhandled", cmd, args);
      return null;
    }
    return h(args ?? {});
  },
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
};
(window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
export {};
