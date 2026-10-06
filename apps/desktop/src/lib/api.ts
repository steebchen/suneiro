import { Channel, invoke } from "@tauri-apps/api/core";

export type Repo = { id: string; name: string; path: string; defaultBranch: string };

export type Workspace = {
  id: string;
  repoId: string;
  name: string;
  branch: string;
  baseBranch: string;
  path: string;
  /** "creating" | "setting_up" | "ready" | "setup_failed" | "failed" */
  status: string;
  createdAt: number;
  title: string;
  archivedAt: number | null;
  unread: boolean;
};

export type Session = {
  id: string;
  workspaceId: string;
  agentId: string;
  acpSessionId: string | null;
  title: string;
  createdAt: number;
  model: string | null;
  effort: string | null;
};

export type AgentDef = { id: string; name: string; command: string; args: string[] };

export type RepoConfig = { scripts: { setup?: string | null; run?: string | null; archive?: string | null }; copy: string[] };

export type Choice = { value: string; name: string; description?: string | null };
export type Catalog = { models: Choice[]; efforts: Choice[]; hasFast: boolean; updatedAt: number };
export type LoadoutEntry = { agent: string; model: string; effort?: string | null };

export type Usage = {
  sessionId: string;
  workspaceId: string;
  repoId: string;
  agent: string;
  model: string;
  ts: number;
  inputTokens: number;
  cachedTokens: number;
  outputTokens: number;
  costUsd: number | null;
  /** reported or estimated; null when unknown (no price set) */
  cost: number | null;
  estimated: boolean;
};
export type ModelPrice = { input: number; cachedInput: number; output: number };

export type Settings = {
  loadout: LoadoutEntry[];
  opencodeModels: string[];
  planByDefault: boolean;
  enabledAgents: string[];
  defaultAgent: string;
  branchPrefix: string;
  workspacesRoot: string;
  editor: string;
  theme: "system" | "light" | "dark";
  renameBranches: boolean;
};

export type AgentStatus = {
  id: string;
  name: string;
  installed: boolean;
  version: string | null;
  loggedIn: boolean;
  account: string | null;
  installCommand: string;
  loginCommand: string;
  logoutCommand: string;
};

export type ChangedFile = { path: string; status: string; additions: number | null; deletions: number | null };

export type SelectOption = { value: string; name: string; description?: string | null };
export type ConfigOption = {
  id: string;
  name: string;
  description?: string | null;
  category?: string | null;
  type: "select" | "boolean";
  currentValue: string | boolean;
  options?: SelectOption[] | { group: string; name: string; options: SelectOption[] }[];
};

/** Update state from the Tauri layer ("update" event). */
export type UpdateStatus = {
  version: string | null;
  phase: "none" | "available" | "downloading" | "waiting" | "installing";
  /** what "restart when idle" is waiting for */
  activity: { agents: number; workspaces: number };
  /** installing on its own right after launch, without being asked */
  automatic: boolean;
};

export type Branch = { name: string; remote: boolean; updatedAt: number; subject: string };
export type OpenPr = {
  number: number;
  title: string;
  headRefName: string;
  baseRefName: string;
  author: { login: string } | null;
  isCrossRepository: boolean;
  isDraft: boolean;
  updatedAt: string;
};

export type SyncStatus = { ahead: number; behind: number; merging: boolean; conflicts: string[] };

export type PermissionOption = { optionId: string; name: string; kind: string };

// Raw ACP session/update payload; the transcript reducer interprets it.
export type AcpUpdate = { sessionUpdate: string; [key: string]: any };

export type CoreEvent =
  | { type: "sessionUpdate"; sessionId: string; update: AcpUpdate }
  | { type: "userMessage"; sessionId: string; text: string; ts: number; images?: string[] }
  | { type: "checkpoint"; sessionId: string; commit: string }
  | { type: "checkpointRestored"; sessionId: string; commit: string; undo: string | null; ts: number }
  | { type: "turnEnd"; sessionId: string; stopReason: string; ts: number }
  | { type: "sessionState"; sessionId: string; state: string; error: string | null }
  | { type: "sessionConfig"; sessionId: string; configOptions: ConfigOption[] }
  | { type: "sessionTitle"; sessionId: string; title: string }
  | { type: "sessionMode"; sessionId: string; plan: boolean }
  | { type: "usage"; sessionId: string; usage: Usage }
  | { type: "permissionRequest"; sessionId: string; requestId: string; toolCall: any; options: PermissionOption[] }
  | { type: "permissionResolved"; sessionId: string; requestId: string }
  | {
      type: "question";
      sessionId: string;
      requestId: string;
      message: string;
      schema: any;
      toolCallId: string | null;
      autoResolveMs: number | null;
    }
  | { type: "questionResolved"; sessionId: string; requestId: string }
  | { type: "workspaceStatus"; workspaceId: string; status: string }
  | { type: "workspaceTitle"; workspaceId: string; title: string }
  | { type: "workspaceBranch"; workspaceId: string; branch: string }
  | { type: "workspacePr"; workspaceId: string; pr: PrStatus | null }
  | { type: "scriptOutput"; workspaceId: string; data: string };

export type PrStatus = {
  number: number;
  url: string;
  state: "OPEN" | "CLOSED" | "MERGED";
  title: string;
  isDraft: boolean;
  mergeable: string;
  mergeStateStatus: string;
  headRefName?: string;
  statusCheckRollup: { name?: string; context?: string; status?: string; conclusion?: string; state?: string }[];
};

export const api = {
  subscribe: (onEvents: (events: CoreEvent[]) => void) => {
    const channel = new Channel<CoreEvent[]>();
    channel.onmessage = onEvents;
    return invoke<void>("subscribe", { channel });
  },
  listAgents: () => invoke<AgentDef[]>("list_agents"),
  listRepos: () => invoke<Repo[]>("list_repos"),
  addRepo: (path: string) => invoke<Repo>("add_repo", { path }),
  recentProjects: () => invoke<{ path: string; name: string; lastUsed: number }[]>("recent_projects"),
  cloneRepo: (spec: string) => invoke<Repo>("clone_repo", { spec }),
  removeRepo: (repoId: string) => invoke<void>("remove_repo", { repoId }),
  reorderRepos: (repoIds: string[]) => invoke<void>("reorder_repos", { repoIds }),
  listWorkspaces: () => invoke<Workspace[]>("list_workspaces"),
  createWorkspace: (repoId: string) => invoke<Workspace>("create_workspace", { repoId }),
  createWorkspaceFrom: (repoId: string, branch: string | null, pr: number | null) =>
    invoke<Workspace>("create_workspace_from", { repoId, branch, pr }),
  listBranches: (repoId: string) => invoke<Branch[]>("list_branches", { repoId }),
  openPrs: (repoId: string) => invoke<OpenPr[]>("open_prs", { repoId }),
  listAllWorkspaces: () => invoke<Workspace[]>("list_all_workspaces"),
  restoreWorkspace: (workspaceId: string) => invoke<Workspace>("restore_workspace", { workspaceId }),
  archiveWorkspace: (workspaceId: string) => invoke<void>("archive_workspace", { workspaceId }),
  repoConfig: (workspaceId: string) => invoke<RepoConfig>("repo_config", { workspaceId }),
  repoSettings: (repoId: string) =>
    invoke<{ config: RepoConfig; file: { name: string; config: RepoConfig } | null }>("repo_settings", { repoId }),
  saveRepoSettings: (repoId: string, config: RepoConfig) => invoke<void>("save_repo_settings", { repoId, config }),
  listSessions: (workspaceId: string) => invoke<Session[]>("list_sessions", { workspaceId }),
  createSession: (workspaceId: string, agentId: string, model?: string | null, effort?: string | null) =>
    invoke<Session>("create_session", { workspaceId, agentId, model: model ?? null, effort: effort ?? null }),
  usage: (since = 0) => invoke<Usage[]>("usage", { since }),
  getPricing: () => invoke<Record<string, ModelPrice>>("get_pricing"),
  savePricing: (pricing: Record<string, ModelPrice>) => invoke<void>("save_pricing", { pricing }),
  modelCatalogs: () => invoke<Record<string, Catalog>>("model_catalogs"),
  refreshCatalog: (agentId: string) => invoke<Catalog>("refresh_catalog", { agentId }),
  connectSession: (sessionId: string) => invoke<void>("connect_session", { sessionId }),
  deleteSession: (sessionId: string) => invoke<void>("delete_session", { sessionId }),
  sessionEvents: (sessionId: string) => invoke<CoreEvent[]>("session_events", { sessionId }),
  sendPrompt: (sessionId: string, text: string, images: string[] = []) => invoke<void>("send_prompt", { sessionId, text, images }),
  steer: (sessionId: string, text: string, images: string[] = []) =>
    invoke<"injected" | "interrupted" | "sent">("steer", { sessionId, text, images }),
  saveAttachment: (mimeType: string, data: string) => invoke<string>("save_attachment", { mimeType, data }),
  importAttachment: (path: string) => invoke<string>("import_attachment", { path }),
  attachmentDataUrl: (path: string) => invoke<string>("attachment_data_url", { path }),
  cancelPrompt: (sessionId: string) => invoke<void>("cancel_prompt", { sessionId }),
  respondPermission: (sessionId: string, requestId: string, optionId: string | null) =>
    invoke<void>("respond_permission", { sessionId, requestId, optionId }),
  setConfig: (sessionId: string, configId: string, value: string | boolean) =>
    invoke<void>("set_config", { sessionId, configId, value }),
  answerQuestion: (sessionId: string, requestId: string, response: { action: "accept"; content: Record<string, unknown> } | { action: "decline" | "cancel" }) =>
    invoke<void>("answer_question", { sessionId, requestId, response }),
  syncStatus: (workspaceId: string) => invoke<SyncStatus>("sync_status", { workspaceId }),
  mergeBaseBranch: (workspaceId: string) => invoke<string[]>("merge_base_branch", { workspaceId }),
  abortMerge: (workspaceId: string) => invoke<void>("abort_merge", { workspaceId }),
  restoreCheckpoint: (sessionId: string, commit: string) => invoke<string>("restore_checkpoint", { sessionId, commit }),
  setPlanMode: (sessionId: string, plan: boolean) => invoke<void>("set_plan_mode", { sessionId, plan }),
  changedFiles: (workspaceId: string) => invoke<ChangedFile[]>("changed_files", { workspaceId }),
  listFiles: (workspaceId: string) => invoke<string[]>("list_files", { workspaceId }),
  fileDiff: (workspaceId: string, path: string) => invoke<string>("file_diff", { workspaceId, path }),
  revertFile: (workspaceId: string, path: string) => invoke<void>("revert_file", { workspaceId, path }),
  commitAll: (workspaceId: string, message: string) => invoke<void>("commit_all", { workspaceId, message }),
  push: (workspaceId: string) => invoke<void>("push", { workspaceId }),
  commitAndPush: (workspaceId: string) => invoke<void>("commit_and_push", { workspaceId }),
  draftPr: (workspaceId: string) => invoke<[string, string]>("draft_pr", { workspaceId }),
  prStatus: (workspaceId: string) => invoke<PrStatus | null>("pr_status", { workspaceId }),
  createPr: (workspaceId: string, title: string, body: string) => invoke<string>("create_pr", { workspaceId, title, body }),
  cachedPrs: () => invoke<Record<string, PrStatus | null>>("cached_prs"),
  refreshPrs: () => invoke<void>("refresh_prs"),
  renameWorkspace: (workspaceId: string, title: string) => invoke<void>("rename_workspace", { workspaceId, title }),
  setWorkspaceUnread: (workspaceId: string, unread: boolean) => invoke<void>("set_workspace_unread", { workspaceId, unread }),
  prDetails: (workspaceId: string) => invoke<any>("pr_details", { workspaceId }),
  fetchImage: (url: string) => invoke<string>("fetch_image", { url }),
  mergePr: (workspaceId: string) => invoke<void>("merge_pr", { workspaceId }),
  terminalOpen: (
    workspaceId: string,
    terminalId: string,
    cols: number,
    rows: number,
    command: string | null,
    onData: (data: Uint8Array) => void,
    onExit: () => void,
  ) => {
    const dataChannel = new Channel<ArrayBuffer>();
    dataChannel.onmessage = (buf) => onData(new Uint8Array(buf));
    const exitChannel = new Channel<boolean>();
    exitChannel.onmessage = onExit;
    return invoke<void>("terminal_open", { workspaceId, terminalId, cols, rows, command, onData: dataChannel, onExit: exitChannel });
  },
  terminalWrite: (terminalId: string, data: string) => invoke<void>("terminal_write", { terminalId, data }),
  terminalResize: (terminalId: string, cols: number, rows: number) => invoke<void>("terminal_resize", { terminalId, cols, rows }),
  terminalKill: (terminalId: string) => invoke<void>("terminal_kill", { terminalId }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  /** Look for a newer version; nothing is installed until `installUpdate`. */
  checkForUpdates: () => invoke<UpdateStatus>("check_for_updates"),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  /** Download, install and restart: now, or once no agent or workspace is busy. */
  installUpdate: (whenIdle: boolean) => invoke<void>("install_update", { whenIdle }),
  /** Stop waiting for work to finish before updating. */
  cancelUpdate: () => invoke<void>("cancel_update"),
  detectAgents: () => invoke<AgentStatus[]>("detect_agents"),
  setupTerminalOpen: (terminalId: string, cols: number, rows: number, command: string, onData: (d: Uint8Array) => void, onExit: () => void) => {
    const dataChannel = new Channel<ArrayBuffer>();
    dataChannel.onmessage = (buf) => onData(new Uint8Array(buf));
    const exitChannel = new Channel<boolean>();
    exitChannel.onmessage = onExit;
    return invoke<void>("setup_terminal_open", { terminalId, cols, rows, command, onData: dataChannel, onExit: exitChannel });
  },
  openPath: (path: string, appName?: string) => invoke<void>("open_path", { path, appName: appName ?? null }),
};
