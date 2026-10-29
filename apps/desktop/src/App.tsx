import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { FolderPlus } from "lucide-react";
import { actions, startBadgeSync, toast, useStore } from "./lib/store";
import { CommandPalette } from "./components/CommandPalette";
import { AddRepoMenu } from "./components/AddRepoMenu";
import { PR_TAB } from "./components/PrView";
import { api } from "./lib/api";
import { BrandMark } from "./components/BrandMark";
import { Sidebar } from "./components/Sidebar";
import { WorkspaceView } from "./components/WorkspaceView";
import { Toast } from "./components/Toast";
import { UpdateToast } from "./components/UpdateToast";
import type { UpdateStatus } from "./lib/api";
import { AgentSetup, Settings } from "./components/Settings";
import { Home } from "./components/Home";
import { Insights } from "./components/Insights";
import { NewWorkspaceDialog } from "./components/NewWorkspaceDialog";
import { Shortcuts } from "./components/Shortcuts";

export async function pickRepo() {
  const path = await open({ directory: true, title: "Choose a git repository" });
  if (typeof path === "string") await actions.addRepo(path);
}

/** Handle a command from the native menu or a keyboard shortcut. */
type Toggle = (open: boolean | ((o: boolean) => boolean)) => void;

function runCommand(id: string, setPalette: Toggle, setShortcuts?: Toggle) {
  const s = useStore.getState();
  const ws = s.workspaces.find((w) => w.id === s.selectedWorkspace);
  switch (id) {
    case "settings":
      return actions.openSettings(true);
    case "check-updates":
      toast("Checking for updates…", "info");
      return void api
        .checkForUpdates()
        .then((update) => {
          // Asking again brings back a dismissed update.
          useStore.setState({ update, updateDismissed: null });
          if (!update.version) toast("Suneiro is up to date", "info");
        })
        .catch((e) => toast(`Update check failed: ${e}`));
    case "palette":
      return setPalette((o) => !o);
    case "shortcuts":
      return setShortcuts?.((o) => !o);
    case "new-workspace": {
      const repoId = ws?.repoId ?? s.repos[0]?.id;
      if (repoId) void actions.createWorkspace(repoId);
      return;
    }
    case "new-workspace-from": {
      const repoId = ws?.repoId ?? s.repos[0]?.id;
      if (repoId) actions.openNewFrom(repoId);
      return;
    }
    case "new-chat": {
      if (!ws || s.page !== "workspace") return;
      void actions.newChat(ws.id);
      return;
    }
    case "close-chat": {
      const sid = ws && s.selectedSession[ws.id];
      if (ws && sid && sid !== PR_TAB && s.page === "workspace") void actions.closeSession(ws.id, sid);
      return;
    }
  }
}

/** ⌘⇧[ / ⌘⇧]: previous / next chat in the current workspace. */
function cycleChat(dir: number) {
  const s = useStore.getState();
  const ws = s.selectedWorkspace;
  const list = ws ? (s.sessions[ws] ?? []) : [];
  if (!ws || list.length < 2) return;
  const i = list.findIndex((x) => x.id === s.selectedSession[ws]);
  const next = list[(i + dir + list.length) % list.length];
  actions.selectSession(ws, next.id);
}

export function App() {
  const ready = useStore((s) => s.ready);
  const selected = useStore((s) => s.selectedWorkspace);
  const hasRepos = useStore((s) => s.repos.length > 0);
  const page = useStore((s) => s.page);
  const [palette, setPalette] = useState(false);
  const [shortcuts, setShortcuts] = useState(false);
  const newFromRepo = useStore((s) => s.newFromRepo);

  useEffect(() => {
    void actions.init();
    startBadgeSync();
    // Native menu items (Settings…, New Chat, Close Chat, …).
    let unlisten: (() => void) | undefined;
    listen<string>("menu", (e) => runCommand(e.payload, setPalette, setShortcuts))
      .then((u) => (unlisten = u))
      .catch(() => {});
    let unlistenUpdate: (() => void) | undefined;
    listen<UpdateStatus>("update", (e) => useStore.setState({ update: e.payload }))
      .then((u) => (unlistenUpdate = u))
      .catch(() => {});
    return () => {
      unlisten?.();
      unlistenUpdate?.();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey) return;
      const s = useStore.getState();
      const shortcut: Record<string, string> = { ",": "settings", k: "palette", t: "new-chat", w: "close-chat", "/": "shortcuts" };
      if (!e.shiftKey && shortcut[e.key]) {
        e.preventDefault();
        runCommand(shortcut[e.key], setPalette, setShortcuts);
      } else if (e.shiftKey && (e.key === "[" || e.key === "]" || e.key === "{" || e.key === "}")) {
        e.preventDefault();
        cycleChat(e.key === "[" || e.key === "{" ? -1 : 1);
      } else if (e.key === "o" && !e.shiftKey) {
        const ws = s.workspaces.find((w) => w.id === s.selectedWorkspace);
        if (ws && s.page === "workspace") {
          e.preventDefault();
          void api.openPath(ws.path, s.settings?.editor || undefined).catch((err) => toast(String(err)));
        }
      } else if (e.key >= "1" && e.key <= "9") {
        const ws = s.workspaces[Number(e.key) - 1];
        if (ws) {
          e.preventDefault();
          actions.selectWorkspace(ws.id);
        }
      } else if (e.key === "n" && !e.shiftKey) {
        const ws = s.workspaces.find((w) => w.id === s.selectedWorkspace);
        const repoId = ws?.repoId ?? s.repos[0]?.id;
        if (repoId) {
          e.preventDefault();
          void actions.createWorkspace(repoId);
        }
      } else if (e.key.toLowerCase() === "n" && e.shiftKey) {
        e.preventDefault();
        runCommand("new-workspace-from", setPalette);
      } else if (e.key === "o" && e.shiftKey) {
        e.preventDefault();
        void pickRepo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!ready) return <div className="h-full" data-tauri-drag-region />;

  return (
    <div className="flex h-full">
      <Sidebar />
      <main className="flex min-w-0 flex-1 flex-col">
        {page === "settings" ? (
          <Settings />
        ) : page === "home" ? (
          <Home />
        ) : page === "insights" ? (
          <Insights />
        ) : selected ? (
          <WorkspaceView key={selected} workspaceId={selected} />
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-3" data-tauri-drag-region>
            <BrandMark size={64} className="mb-3 text-fg" />
            <div className="text-lg font-medium">{hasRepos ? "No workspace selected" : "Welcome to Suneiro"}</div>
            <div className="max-w-sm text-center text-muted">
              Run Claude Code, Codex and OpenCode in parallel, each in its own git worktree.
            </div>
            <AddRepoMenu
              align="center"
              trigger={(open) => (
                <button onClick={open} className="mt-2 flex items-center gap-2 rounded-md bg-accent px-3 py-1.5 font-medium text-accent-fg">
                  <FolderPlus size={14} /> Add repository
                </button>
              )}
            />
            {!hasRepos && (
              <div className="mt-8 w-full max-w-lg">
                <div className="mb-2 text-xs font-medium text-muted">Your agents</div>
                <AgentSetup compact />
              </div>
            )}
          </div>
        )}
      </main>
      <div className="fixed right-4 bottom-4 z-50 flex flex-col items-end gap-2">
        <UpdateToast />
        <Toast />
      </div>
      {palette && <CommandPalette onClose={() => setPalette(false)} onShortcuts={() => setShortcuts(true)} />}
      {shortcuts && <Shortcuts onClose={() => setShortcuts(false)} />}
      {newFromRepo && <NewWorkspaceDialog repoId={newFromRepo} onClose={() => actions.openNewFrom(null)} />}
    </div>
  );
}
