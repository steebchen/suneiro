import { create } from "zustand";
import { api, type ChangedFile, type AgentDef, type AgentStatus, type Catalog, type LoadoutEntry, type ModelPrice, type PrStatus, type Settings, type Usage, type UpdateStatus, type ConfigOption, type CoreEvent, type PermissionOption, type Repo, type Session, type Workspace } from "./api";
import { applyEvents, emptyTranscript, type Transcript } from "./transcript";

export type Permission = { requestId: string; toolCall: any; options: PermissionOption[] };
export type PendingQuestion = {
  requestId: string;
  message: string;
  schema: any;
  toolCallId: string | null;
  autoResolveMs: number | null;
  receivedAt: number;
};

/** A follow-up waiting for the current turn to end. */
export type Queued = { text: string; images: string[] };

export type DiffStats = { files: number; add: number; del: number };

export type SlashCommand = { name: string; description?: string | null; input?: { hint?: string | null } | null };

export type SessionView = {
  transcript: Transcript;
  state: "disconnected" | "connecting" | "idle" | "running" | "error";
  error: string | null;
  config: ConfigOption[];
  permissions: Permission[];
  questions: PendingQuestion[];
  /** history loaded from disk (or session created in this run) */
  loaded: boolean;
  /** finished a turn while not being looked at */
  unread: boolean;
  /** plan mode (agent asks before acting); otherwise everything is auto-accepted */
  plan: boolean;
  /** follow-ups typed while the agent was working, sent when the turn ends */
  queued: Queued[];
  /** a queued message is being edited: hold the queue until it's saved */
  queueHeld: boolean;
  /** slash commands the agent advertises (ACP available_commands_update) */
  commands: SlashCommand[];
};

const newView = (loaded: boolean): SessionView => ({
  transcript: emptyTranscript(),
  state: "disconnected",
  error: null,
  config: [],
  permissions: [],
  questions: [],
  loaded,
  unread: false,
  plan: false,
  queued: [],
  queueHeld: false,
  commands: [],
});

type State = {
  ready: boolean;
  agents: AgentDef[];
  repos: Repo[];
  workspaces: Workspace[];
  sessions: Record<string, Session[]>;
  views: Record<string, SessionView>;
  selectedWorkspace: string | null;
  selectedSession: Record<string, string>;
  scriptLog: Record<string, string>;
  /** bumped whenever a workspace's files may have changed */
  changesTick: Record<string, number>;
  /** lines added/removed on each workspace's branch (incl. uncommitted) */
  diffStats: Record<string, DiffStats>;
  toast: { text: string; kind: "error" | "info" } | null;
  /** repo whose "New workspace from branch or PR" dialog is open */
  newFromRepo: string | null;
  /** unsent composer text per session */
  drafts: Record<string, string>;
  /** images attached in the composer (stored attachment paths) per session */
  attachments: Record<string, string[]>;
  settings: Settings | null;
  agentStatus: AgentStatus[] | null;
  page: "workspace" | "settings" | "home" | "insights";
  /** latest PR per workspace; missing = not fetched yet, null = no PR */
  prs: Record<string, PrStatus | null>;
  /** models/efforts per agent, discovered from the agents themselves */
  catalogs: Record<string, Catalog>;
  /** every recorded turn's tokens and cost, oldest first */
  usage: Usage[];
  pricing: Record<string, ModelPrice>;
  /** repos recently used with coding agents, for "Add repository" */
  recents: { path: string; name: string; lastUsed: number }[];
  /** newer app version and how far installing it got */
  update: UpdateStatus | null;
  /** version whose update toast the user dismissed (until the next launch) */
  updateDismissed: string | null;
};

export const useStore = create<State>(() => ({
  ready: false,
  agents: [],
  repos: [],
  workspaces: [],
  sessions: {},
  views: {},
  selectedWorkspace: null,
  selectedSession: {},
  scriptLog: {},
  changesTick: {},
  diffStats: {},
  toast: null,
  newFromRepo: null,
  drafts: {},
  attachments: {},
  settings: null,
  agentStatus: null,
  page: "workspace",
  prs: {},
  catalogs: {},
  recents: [],
  usage: [],
  pricing: {},
  update: null,
  updateDismissed: null,
}));

const set = useStore.setState;
const get = useStore.getState;

export function toast(text: string, kind: "error" | "info" = "error") {
  set({ toast: { text, kind } });
  setTimeout(() => {
    if (get().toast?.text === text) set({ toast: null });
  }, 5000);
}

async function guard<T>(p: Promise<T>): Promise<T | undefined> {
  try {
    return await p;
  } catch (e) {
    toast(String(e));
    return undefined;
  }
}

function handleEvents(events: CoreEvent[]) {
  const s = get();
  const attention: Attention[] = [];
  const views = { ...s.views };
  let scriptLog = s.scriptLog;
  let changesTick = s.changesTick;
  let sessions = s.sessions;
  const bySession = new Map<string, CoreEvent[]>();

  for (const e of events) {
    if (e.type === "scriptOutput") {
      if (scriptLog === s.scriptLog) scriptLog = { ...scriptLog };
      scriptLog[e.workspaceId] = (scriptLog[e.workspaceId] ?? "") + e.data;
      continue;
    }
    if (e.type === "usage") {
      set({ usage: [...get().usage, e.usage] });
      continue;
    }
    if (e.type === "workspacePr") {
      set({ prs: { ...get().prs, [e.workspaceId]: e.pr } });
      continue;
    }
    if (e.type === "workspaceTitle") {
      set({ workspaces: get().workspaces.map((w) => (w.id === e.workspaceId ? { ...w, title: e.title } : w)) });
      continue;
    }
    if (e.type === "workspaceBranch") {
      set({ workspaces: get().workspaces.map((w) => (w.id === e.workspaceId ? { ...w, branch: e.branch } : w)) });
      continue;
    }
    if (e.type === "workspaceStatus") {
      if (changesTick === s.changesTick) changesTick = { ...changesTick };
      changesTick[e.workspaceId] = (changesTick[e.workspaceId] ?? 0) + 1;
      if (e.status !== "dirty") {
        set({ workspaces: get().workspaces.map((w) => (w.id === e.workspaceId ? { ...w, status: e.status } : w)) });
      }
      continue;
    }
    let list = bySession.get(e.sessionId);
    if (!list) bySession.set(e.sessionId, (list = []));
    list.push(e);
  }

  for (const [sessionId, list] of bySession) {
    let v = { ...(views[sessionId] ?? newView(true)) };
    for (const e of list) {
      switch (e.type) {
        case "sessionState":
          v.state = e.state as SessionView["state"];
          v.error = e.error;
          break;
        case "sessionConfig":
          v.config = e.configOptions;
          break;
        case "sessionMode":
          v.plan = e.plan;
          break;
        case "sessionTitle":
          sessions = Object.fromEntries(
            Object.entries(sessions).map(([ws, l]) => [ws, l.map((x) => (x.id === sessionId ? { ...x, title: e.title } : x))]),
          );
          break;
        case "permissionRequest":
          v.permissions = [...v.permissions, { requestId: e.requestId, toolCall: e.toolCall, options: e.options }];
          break;
        case "permissionResolved":
          v.permissions = v.permissions.filter((p) => p.requestId !== e.requestId);
          break;
        case "question":
          v.questions = [
            ...v.questions,
            { requestId: e.requestId, message: e.message, schema: e.schema, toolCallId: e.toolCallId, autoResolveMs: e.autoResolveMs, receivedAt: Date.now() },
          ];
          break;
        case "questionResolved":
          v.questions = v.questions.filter((q) => q.requestId !== e.requestId);
          break;
        case "sessionUpdate":
          if (e.update.sessionUpdate === "available_commands_update") {
            v.commands = e.update.availableCommands ?? [];
          } else if (e.update.sessionUpdate === "current_mode_update") {
            v.config = v.config.map((c) => (c.id === "mode" || c.category === "mode" ? { ...c, currentValue: e.update.currentModeId } : c));
          }
          break;
      }
    }
    // Transcript events, including the ones handled above that also render.
    v.transcript = applyEvents(v.transcript, list);
    const finished = list.find((e) => e.type === "turnEnd");
    const asks = list.find((e) => e.type === "question" || e.type === "permissionRequest");
    if ((finished || asks) && !isVisible(sessionId)) {
      if (finished) v.unread = true;
      attention.push({ sessionId, kind: asks ? "input" : "done", detail: asks?.type === "question" ? asks.message : undefined });
    }
    // Send the next queued follow-up once a turn ends normally.
    if (finished && finished.type === "turnEnd" && finished.stopReason === "end_turn" && v.queued.length && !v.queueHeld) {
      const [next, ...rest] = v.queued;
      v.queued = rest;
      setTimeout(() => void actions.send(sessionId, next.text, next.images), 0);
    }
    views[sessionId] = v;
  }
  set({ views, scriptLog, changesTick, sessions });
  if (changesTick !== s.changesTick) {
    for (const id of Object.keys(changesTick)) if (changesTick[id] !== s.changesTick[id]) scheduleDiffStats(id);
  }
  for (const a of attention) notifyAttention(a);
}

const statsTimers = new Map<string, ReturnType<typeof setTimeout>>();

/** Refresh a workspace's diff stats soon (coalescing bursts of changes). */
function scheduleDiffStats(workspaceId: string, delay = 1500) {
  clearTimeout(statsTimers.get(workspaceId));
  statsTimers.set(
    workspaceId,
    setTimeout(() => {
      statsTimers.delete(workspaceId);
      void actions.refreshDiffStats(workspaceId);
    }, delay),
  );
}

type Attention = { sessionId: string; kind: "done" | "input"; detail?: string };

/** Bold the workspace in the sidebar and post a macOS notification. */
function notifyAttention(a: Attention) {
  const s = get();
  const ws = s.workspaces.find((w) => (s.sessions[w.id] ?? []).some((x) => x.id === a.sessionId));
  if (!ws) return;
  if (!ws.unread) void actions.markUnread(ws.id, true);
  if (document.hasFocus() && s.selectedWorkspace === ws.id) return;
  const title = ws.title || (s.sessions[ws.id] ?? []).find((x) => x.title)?.title || ws.name;
  void notify(title, a.kind === "input" ? (a.detail ? `Question: ${a.detail}` : "Needs your input") : "Finished");
}

let notifyPermission: boolean | null = null;
async function notify(title: string, body: string) {
  if (!("__TAURI_INTERNALS__" in window)) return;
  try {
    const n = await import("@tauri-apps/plugin-notification");
    if (notifyPermission === null) {
      notifyPermission = (await n.isPermissionGranted()) || (await n.requestPermission()) === "granted";
    }
    if (notifyPermission) n.sendNotification({ title, body });
  } catch {
    // Notifications are best-effort.
  }
}

function isVisible(sessionId: string) {
  const s = get();
  const ws = s.selectedWorkspace;
  return !!ws && s.selectedSession[ws] === sessionId && document.hasFocus();
}

export const actions = {
  setDraft(sessionId: string, text: string) {
    set({ drafts: { ...get().drafts, [sessionId]: text } });
  },

  appendDraft(sessionId: string, text: string) {
    const cur = get().drafts[sessionId]?.trimEnd();
    actions.setDraft(sessionId, cur ? `${cur}\n\n${text}` : text);
  },

  async init() {
    await api.subscribe(handleEvents);
    const [agents, repos, workspaces, settings, prs, catalogs] = await Promise.all([
      api.listAgents(),
      api.listRepos(),
      api.listWorkspaces(),
      api.getSettings(),
      api.cachedPrs().catch(() => ({})),
      api.modelCatalogs().catch(() => ({})),
    ]);
    applyTheme(settings.theme);
    void actions.detectAgents().then(() => actions.ensureCatalogs());
    void actions.loadRecents();
    void actions.loadUsage();
    void api.updateStatus().then((update) => set({ update })).catch(() => {});
    const lists = await Promise.all(workspaces.map((w) => api.listSessions(w.id)));
    const sessions: Record<string, Session[]> = {};
    const selectedSession: Record<string, string> = {};
    workspaces.forEach((w, i) => {
      sessions[w.id] = lists[i];
      if (lists[i].length) selectedSession[w.id] = lists[i][lists[i].length - 1].id;
    });
    const views: Record<string, SessionView> = {};
    for (const l of lists) for (const x of l) views[x.id] = newView(false);
    workspaces.forEach((w, i) => scheduleDiffStats(w.id, 500 + i * 150));
    set({ ready: true, prs: prs ?? {}, catalogs: catalogs ?? {}, settings, agents, repos, workspaces, sessions, selectedSession, views, selectedWorkspace: workspaces[0]?.id ?? null });
  },

  async refreshDiffStats(workspaceId: string) {
    const ws = get().workspaces.find((w) => w.id === workspaceId);
    if (!ws || ws.status === "creating" || ws.status === "failed") return;
    const files = await api.changedFiles(workspaceId).catch(() => null);
    if (files) actions.setDiffStats(workspaceId, files);
  },

  setDiffStats(workspaceId: string, files: ChangedFile[]) {
    const next = files.reduce((t, f) => ({ files: t.files + 1, add: t.add + (f.additions ?? 0), del: t.del + (f.deletions ?? 0) }), { files: 0, add: 0, del: 0 });
    const cur = get().diffStats[workspaceId];
    if (cur && cur.files === next.files && cur.add === next.add && cur.del === next.del) return;
    set({ diffStats: { ...get().diffStats, [workspaceId]: next } });
  },

  async loadUsage() {
    const [usage, pricing] = await Promise.all([api.usage(0).catch(() => null), api.getPricing().catch(() => null)]);
    set({ ...(usage ? { usage } : {}), ...(pricing ? { pricing } : {}) });
  },

  async savePricing(pricing: Record<string, ModelPrice>) {
    set({ pricing });
    await guard(api.savePricing(pricing));
    // Estimates are computed by the core; reload so they reflect new prices.
    const usage = await api.usage(0).catch(() => null);
    if (usage) set({ usage });
  },

  openNewFrom(repoId: string | null) {
    set({ newFromRepo: repoId });
  },

  openInsights() {
    set({ page: "insights" });
  },

  async loadRecents() {
    const recents = await api.recentProjects().catch(() => null);
    if (recents) set({ recents });
  },

  async cloneRepo(spec: string) {
    const repo = await guard(api.cloneRepo(spec));
    if (repo) await actions.repoAdded(repo);
    return !!repo;
  },

  async addRepo(path: string) {
    const repo = await guard(api.addRepo(path));
    if (repo) await actions.repoAdded(repo);
  },

  async repoAdded(repo: Repo) {
    set({ recents: get().recents.filter((r) => r.path !== repo.path) });
    if (!get().repos.some((r) => r.id === repo.id)) set({ repos: [...get().repos, repo] });
    await actions.createWorkspace(repo.id);
  },

  async removeRepo(repoId: string) {
    if (get().workspaces.some((w) => w.repoId === repoId)) {
      toast("Archive this repository's workspaces first");
      return;
    }
    await guard(api.removeRepo(repoId));
    set({ repos: get().repos.filter((r) => r.id !== repoId) });
  },

  /** Move a repo in the sidebar so it sits at `index` of the current list. */
  async reorderRepo(repoId: string, index: number) {
    const repos = get().repos;
    const from = repos.findIndex((r) => r.id === repoId);
    if (from < 0) return;
    const next = repos.filter((r) => r.id !== repoId);
    next.splice(index > from ? index - 1 : index, 0, repos[from]);
    if (next.every((r, i) => r === repos[i])) return;
    set({ repos: next });
    await guard(api.reorderRepos(next.map((r) => r.id)));
  },

  async createWorkspace(repoId: string) {
    const ws = await guard(api.createWorkspace(repoId));
    if (ws) await actions.workspaceCreated(ws);
  },

  /** Workspace on an existing branch or a PR's branch. */
  async createWorkspaceFrom(repoId: string, branch: string | null, pr: number | null) {
    const ws = await guard(api.createWorkspaceFrom(repoId, branch, pr));
    if (ws) await actions.workspaceCreated(ws);
    return !!ws;
  },

  /** Show a new workspace and open its first chat. */
  async workspaceCreated(ws: Workspace) {
    set({ workspaces: [ws, ...get().workspaces], sessions: { ...get().sessions, [ws.id]: [] }, selectedWorkspace: ws.id, page: "workspace" });
    await actions.newChat(ws.id);
  },

  /** Open a chat with the default model (first loadout entry); the composer's picker changes it later. */
  async newChat(workspaceId: string) {
    const first = get().settings?.loadout[0];
    if (first) await actions.createSession(workspaceId, first.agent, first.model, first.effort);
    else await actions.createSession(workspaceId, get().settings?.defaultAgent ?? "claude");
  },

  async archiveWorkspace(workspaceId: string) {
    // Optimistic: disappear right away, come back if archiving fails.
    const before = get();
    const ws = before.workspaces.find((w) => w.id === workspaceId);
    const workspaces = before.workspaces.filter((w) => w.id !== workspaceId);
    set({
      workspaces,
      selectedWorkspace: before.selectedWorkspace === workspaceId ? (workspaces[0]?.id ?? null) : before.selectedWorkspace,
    });
    const ok = await guard(api.archiveWorkspace(workspaceId).then(() => true));
    if (!ok && ws) {
      set({ workspaces: [ws, ...get().workspaces.filter((w) => w.id !== ws.id)].sort((a, b) => b.createdAt - a.createdAt) });
      return false;
    }
    return true;
  },

  async saveSettings(patch: Partial<Settings>) {
    const cur = get().settings;
    if (!cur) return;
    const settings = { ...cur, ...patch };
    set({ settings });
    applyTheme(settings.theme);
    await guard(api.saveSettings(settings));
  },

  /** Discover models for installed agents we have no catalog for yet. */
  async ensureCatalogs(force = false) {
    const s = get();
    const installed = (s.agentStatus ?? []).filter((a) => a.installed && s.settings?.enabledAgents.includes(a.id));
    for (const a of installed) {
      if (!force && get().catalogs[a.id]) continue;
      const c = await api.refreshCatalog(a.id).catch(() => null);
      if (c) set({ catalogs: { ...get().catalogs, [a.id]: c } });
    }
    actions.seedLoadout();
  },

  /** First run: feature each agent's current flagship at high effort. */
  seedLoadout() {
    const { settings, catalogs } = get();
    if (!settings || settings.loadout?.length) return;
    const loadout: LoadoutEntry[] = [];
    for (const agent of ["claude", "codex"]) {
      const c = catalogs[agent];
      if (!c?.models.length) continue;
      const effort = c.efforts.find((e) => e.value === "high")?.value ?? null;
      loadout.push({ agent, model: c.models[0].value, effort });
    }
    if (loadout.length) void actions.saveSettings({ loadout });
  },

  async detectAgents() {
    const agentStatus = await guard(api.detectAgents());
    if (agentStatus) set({ agentStatus });
  },

  openSettings(open = true) {
    set({ page: open ? "settings" : "workspace" });
  },

  openHome() {
    set({ page: "home" });
  },

  async restoreWorkspace(workspaceId: string) {
    const ws = await guard(api.restoreWorkspace(workspaceId));
    if (!ws) return;
    const sessions = (await guard(api.listSessions(ws.id))) ?? [];
    const s = get();
    const views = { ...s.views };
    for (const x of sessions) views[x.id] ??= newView(false);
    set({
      workspaces: [ws, ...s.workspaces.filter((w) => w.id !== ws.id)],
      sessions: { ...s.sessions, [ws.id]: sessions },
      views,
      selectedSession: sessions.length ? { ...s.selectedSession, [ws.id]: sessions[sessions.length - 1].id } : s.selectedSession,
    });
    return ws;
  },

  async markUnread(workspaceId: string, unread = true) {
    set({ workspaces: get().workspaces.map((w) => (w.id === workspaceId ? { ...w, unread } : w)) });
    await guard(api.setWorkspaceUnread(workspaceId, unread));
  },

  async renameWorkspace(workspaceId: string, title: string) {
    set({ workspaces: get().workspaces.map((w) => (w.id === workspaceId ? { ...w, title: title.trim() } : w)) });
    await guard(api.renameWorkspace(workspaceId, title));
  },

  selectWorkspace(workspaceId: string) {
    set({ selectedWorkspace: workspaceId, page: "workspace" });
    if (get().workspaces.find((w) => w.id === workspaceId)?.unread) void actions.markUnread(workspaceId, false);
    const sid = get().selectedSession[workspaceId];
    if (sid) actions.selectSession(workspaceId, sid);
  },

  async createSession(workspaceId: string, agentId: string, model?: string | null, effort?: string | null) {
    const session = await guard(api.createSession(workspaceId, agentId, model, effort));
    if (!session) return;
    const s = get();
    set({
      sessions: { ...s.sessions, [workspaceId]: [...(s.sessions[workspaceId] ?? []), session] },
      views: { ...s.views, [session.id]: { ...newView(true), ...s.views[session.id], state: "connecting" } },
      selectedSession: { ...s.selectedSession, [workspaceId]: session.id },
    });
  },

  async closeSession(workspaceId: string, sessionId: string) {
    // A workspace always keeps at least one chat, so the view never goes blank.
    if ((get().sessions[workspaceId] ?? []).length <= 1) return;
    await guard(api.deleteSession(sessionId));
    const s = get();
    const list = (s.sessions[workspaceId] ?? []).filter((x) => x.id !== sessionId);
    const selectedSession = { ...s.selectedSession };
    if (selectedSession[workspaceId] === sessionId) {
      if (list.length) selectedSession[workspaceId] = list[list.length - 1].id;
      else delete selectedSession[workspaceId];
    }
    set({ sessions: { ...s.sessions, [workspaceId]: list }, selectedSession });
  },

  selectSession(workspaceId: string, sessionId: string) {
    const s = get();
    const v = s.views[sessionId];
    set({
      selectedSession: { ...s.selectedSession, [workspaceId]: sessionId },
      views: v?.unread ? { ...s.views, [sessionId]: { ...v, unread: false } } : s.views,
    });
    if (v && !v.loaded) void actions.loadHistory(sessionId);
  },

  async loadHistory(sessionId: string) {
    const events = await guard(api.sessionEvents(sessionId));
    if (!events) return;
    const v = get().views[sessionId] ?? newView(false);
    // Sessions from earlier runs only receive live events after the user
    // prompts them, which requires the history to be shown first.
    const transcript = applyEvents(emptyTranscript(), events);
    set({ views: { ...get().views, [sessionId]: { ...v, transcript, loaded: true } } });
  },

  /** Put the workspace's files back to a checkpoint (or undo a restore). */
  async restoreCheckpoint(sessionId: string, commit: string) {
    await guard(api.restoreCheckpoint(sessionId, commit));
  },

  /** Queue a follow-up while the agent works; it's sent when the turn ends. */
  queue(sessionId: string, text: string, images: string[] = []) {
    const v = get().views[sessionId];
    if (v) set({ views: { ...get().views, [sessionId]: { ...v, queued: [...v.queued, { text, images }] } } });
  },

  unqueue(sessionId: string, index: number) {
    const v = get().views[sessionId];
    if (v) set({ views: { ...get().views, [sessionId]: { ...v, queued: v.queued.filter((_, i) => i !== index) } } });
  },

  /** Send a queued message now: injected into the running turn (or, for agents
   * without steering, the turn is stopped first). Put back on failure. */
  async steerQueued(sessionId: string, index: number) {
    const v = get().views[sessionId];
    const item = v?.queued[index];
    if (!v || item === undefined) return;
    actions.unqueue(sessionId, index);
    try {
      await api.steer(sessionId, item.text, item.images);
    } catch (e) {
      const cur = get().views[sessionId];
      if (cur) {
        const queued = cur.queued.slice();
        queued.splice(Math.min(index, queued.length), 0, item);
        set({ views: { ...get().views, [sessionId]: { ...cur, queued } } });
      }
      toast(String(e));
    }
  },

  /** Pause/resume the queue while a queued message is edited. */
  holdQueue(sessionId: string, held: boolean) {
    const v = get().views[sessionId];
    if (!v) return;
    set({ views: { ...get().views, [sessionId]: { ...v, queueHeld: held } } });
    if (!held) actions.flushQueue(sessionId);
  },

  editQueued(sessionId: string, index: number, text: string) {
    const v = get().views[sessionId];
    if (!v) return;
    const keep = (q: Queued) => q.text || q.images.length;
    const queued = v.queued.map((q, i) => (i === index ? { ...q, text: text.trim() } : q)).filter(keep);
    set({ views: { ...get().views, [sessionId]: { ...v, queued } } });
  },

  /** Send the next queued message if the agent is idle (e.g. it finished while the queue was held). */
  flushQueue(sessionId: string) {
    const v = get().views[sessionId];
    if (!v || v.queueHeld || !v.queued.length || v.state === "running" || v.state === "connecting") return;
    const [next, ...rest] = v.queued;
    set({ views: { ...get().views, [sessionId]: { ...v, queued: rest } } });
    void actions.send(sessionId, next.text, next.images);
  },

  async send(sessionId: string, text: string, images: string[] = []) {
    await guard(api.sendPrompt(sessionId, text, images));
  },

  /** Attach image files (copied into the app's data folder). */
  async attachFiles(sessionId: string, paths: string[]) {
    for (const p of paths) {
      const stored = await guard(api.importAttachment(p));
      if (stored) actions.setAttachments(sessionId, [...(get().attachments[sessionId] ?? []), stored]);
    }
  },

  /** Attach pasted image data. */
  async attachBlob(sessionId: string, blob: Blob) {
    const data = await new Promise<string>((resolve, reject) => {
      const r = new FileReader();
      r.onload = () => resolve(String(r.result).split(",", 2)[1] ?? "");
      r.onerror = () => reject(r.error);
      r.readAsDataURL(blob);
    });
    const stored = await guard(api.saveAttachment(blob.type, data));
    if (stored) actions.setAttachments(sessionId, [...(get().attachments[sessionId] ?? []), stored]);
  },

  setAttachments(sessionId: string, images: string[]) {
    set({ attachments: { ...get().attachments, [sessionId]: images } });
  },

  async cancel(sessionId: string) {
    await guard(api.cancelPrompt(sessionId));
  },

  async answerQuestion(sessionId: string, requestId: string, response: Parameters<typeof api.answerQuestion>[2]) {
    // Hide immediately; the core confirms with questionResolved.
    const v = get().views[sessionId];
    if (v) set({ views: { ...get().views, [sessionId]: { ...v, questions: v.questions.filter((q) => q.requestId !== requestId) } } });
    await guard(api.answerQuestion(sessionId, requestId, response));
  },

  async respondPermission(sessionId: string, requestId: string, optionId: string | null) {
    await guard(api.respondPermission(sessionId, requestId, optionId));
  },

  async togglePlan(sessionId: string) {
    const s = get();
    const v = s.views[sessionId];
    if (!v) return;
    const plan = !v.plan;
    set({ views: { ...s.views, [sessionId]: { ...v, plan } } });
    await guard(api.setPlanMode(sessionId, plan));
  },

  async setConfig(sessionId: string, configId: string, value: string | boolean) {
    const s = get();
    const v = s.views[sessionId];
    if (v) {
      const config = v.config.map((c) => (c.id === configId ? { ...c, currentValue: value } : c));
      set({ views: { ...s.views, [sessionId]: { ...v, config } } });
    }
    await guard(api.setConfig(sessionId, configId, value));
  },
};

/** Aggregate status for sidebar dots. */
export function workspaceActivity(s: State, workspaceId: string): "needs-input" | "running" | "error" | "unread" | "idle" {
  const list = s.sessions[workspaceId] ?? [];
  let result: ReturnType<typeof workspaceActivity> = "idle";
  for (const x of list) {
    const v = s.views[x.id];
    if (!v) continue;
    if (v.permissions.length || v.questions.length) return "needs-input";
    if (v.state === "running") result = "running";
    else if (v.state === "error" && result === "idle") result = "error";
    else if (v.unread && result === "idle") result = "unread";
  }
  return result;
}

export function applyTheme(theme: Settings["theme"]) {
  if (theme === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
  window.dispatchEvent(new Event("suneiro-theme"));
}

/** Agents shown in "new chat" menus. */
export function enabledAgents(s: State) {
  const enabled = s.settings?.enabledAgents;
  return enabled ? s.agents.filter((a) => enabled.includes(a.id)) : s.agents;
}

/** Dock badge: how many workspaces want attention (waiting for input or unread). */
export function startBadgeSync() {
  if (!("__TAURI_INTERNALS__" in window)) return;
  let last = -1;
  const update = async () => {
    const s = get();
    const count = s.workspaces.filter((w) => w.unread || workspaceActivity(s, w.id) === "needs-input").length;
    if (count === last) return;
    last = count;
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().setBadgeCount(count || undefined);
    } catch {}
  };
  useStore.subscribe(() => void update());
  void update();
}

export type Totals = { cost: number; tokens: number; estimated: boolean; unpriced: number };

/** Sum usage rows: cost (reported + estimated), tokens, and how many turns have no price. */
export function totals(rows: Usage[]): Totals {
  const t: Totals = { cost: 0, tokens: 0, estimated: false, unpriced: 0 };
  for (const u of rows) {
    t.tokens += u.inputTokens + u.cachedTokens + u.outputTokens;
    if (u.cost === null) t.unpriced++;
    else {
      t.cost += u.cost;
      if (u.estimated) t.estimated = true;
    }
  }
  return t;
}

export function formatCost(cost: number, estimated = false) {
  const s = cost >= 100 ? cost.toFixed(0) : cost >= 1 ? cost.toFixed(2) : cost > 0 ? cost.toFixed(cost < 0.01 ? 3 : 2) : "0.00";
  return `${estimated ? "≈ " : ""}$${s}`;
}

export function formatTokens(n: number) {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(n >= 1e5 ? 0 : 1)}k`;
  return String(n);
}
