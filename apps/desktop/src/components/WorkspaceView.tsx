import { useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { Archive, ExternalLink, GitBranch, Loader2, PanelRight } from "lucide-react";
import { useResizable } from "../lib/resize";
import { ResizeHandle } from "./ResizeHandle";
import { actions, toast, useStore } from "../lib/store";
import { PR_TAB, PrView } from "./PrView";
import { api } from "../lib/api";
import { Chat } from "./Chat";
import { ChangesPanel } from "./ChangesPanel";
import { VerificationPanel } from "./VerificationPanel";
import { TerminalPanel } from "./TerminalPanel";
import { PrActions } from "./PrActions";
import { SyncBadge } from "./SyncBadge";
import { Menu } from "./Menu";


import { TabHierarchy } from "./TabHierarchy";
import { ProfileLauncher } from "./ProfileDialog";

import { TaskResourcesButton } from "./Inbox";



type Tab = "changes" | "checks" | "terminal" | "setup";


export function WorkspaceView({ workspaceId }: { workspaceId: string }) {
  const ws = useStore((s) => s.workspaces.find((w) => w.id === workspaceId));
  const sessionId = useStore((s) => s.selectedSession[workspaceId]);
  // The chat to target from the header (e.g. "Fix checks") even while the PR tab is open.
  const chatId = useStore((s) => {
    const sel = s.selectedSession[workspaceId];
    if (sel && sel !== PR_TAB) return sel;
    const list = s.sessions[workspaceId] ?? [];
    return list[list.length - 1]?.id;
  });
  const hasSetupLog = useStore((s) => !!s.scriptLog[workspaceId]);
  const failed = useStore((s) => s.workspaces.find((w) => w.id === workspaceId)?.status === "failed");
  const editor = useStore((s) => s.settings?.editor);
  const sessionTitle = useStore((s) => (s.sessions[workspaceId] ?? []).find((x) => x.title)?.title ?? "");
  const [panel, setPanel] = useState(true);
  const [tab, setTab] = useState<Tab>("changes");
  useEffect(() => {
    if (failed) setTab("setup");
  }, [failed]);
  const panelSize = useResizable("runner.panelWidth", 560, 320, 1100, "left");

  useEffect(() => {
    if (sessionId) actions.selectSession(workspaceId, sessionId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  if (!ws) return null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-13 shrink-0 items-center gap-2 border-b border-border px-3" data-tauri-drag-region>
        <div className="flex min-w-0 items-center gap-2" data-tauri-drag-region>
          <span className="font-medium">{ws.title || sessionTitle || ws.name}</span>
          <span className="flex items-center gap-1 truncate text-xs text-muted" data-tauri-drag-region>
            <GitBranch size={11} /> {ws.branch} → {ws.baseBranch}
          </span>
          {ws.status !== "creating" && ws.status !== "failed" && <SyncBadge workspace={ws} sessionId={chatId} />}
        </div>
        <div className="flex-1" data-tauri-drag-region />
        {chatId && <TaskResourcesButton repoId={ws.repoId} sessionId={chatId} />}
          {ws.status !== "creating" && ws.status !== "failed" && <PrActions workspace={ws} sessionId={chatId} />}
        <Menu
          label={
            <span className="flex items-center gap-1">
              <ExternalLink size={13} /> Open
            </span>
          }
          items={[
            ...(editor ? [{ label: `${editor}  ⌘O`, onSelect: () => api.openPath(ws.path, editor).catch((e) => toast(String(e))) }] : []),
            { label: "Finder", onSelect: () => api.openPath(ws.path) },
            { label: "Terminal", onSelect: () => api.openPath(ws.path, "Terminal") },
            { label: "Copy path", onSelect: () => void navigator.clipboard.writeText(ws.path).then(() => toast("Path copied", "info")) },
            { label: "Copy branch name", onSelect: () => void navigator.clipboard.writeText(ws.branch).then(() => toast("Branch name copied", "info")) },
          ]}
        />
        <button
          title="Archive workspace (removes the worktree, keeps the branch)"
          onClick={() => actions.archiveWorkspace(ws.id)}
          className="rounded p-1.5 text-muted hover:bg-hover hover:text-fg"
        >
          <Archive size={14} />
        </button>
        <button
          title="Toggle panel"
          onClick={() => setPanel((p) => !p)}
          className={clsx("rounded p-1.5 hover:bg-hover", panel ? "text-fg" : "text-muted")}
        >
          <PanelRight size={14} />
        </button>
      </header>

      <div className="flex min-h-0 flex-1">
        <section className="flex min-w-0 flex-1 flex-col">
          <div className="flex shrink-0 items-center justify-end border-b border-border px-2 py-1"><ProfileLauncher workspaceId={ws.id} /></div>
          <TabHierarchy workspaceId={ws.id} activeId={sessionId} />
          {sessionId === PR_TAB ? (
            <PrView workspaceId={ws.id} />
          ) : sessionId ? (
            <Chat key={sessionId} sessionId={sessionId} workspaceId={ws.id} />
          ) : (
            <div className="flex flex-1 items-center justify-center text-muted">Start a chat with an agent using +</div>
          )}
        </section>
        {panel && (
          <section
            ref={panelSize.ref as React.RefObject<HTMLElement>}
            style={{ width: panelSize.width }}
            className="relative flex max-w-[65%] shrink-0 flex-col border-l border-border bg-panel"
          >
            <ResizeHandle edge="left" onPointerDown={panelSize.onPointerDown} />
            <div className="flex h-9 shrink-0 items-center gap-1 border-b border-border px-2">
              {(["changes", "checks", "terminal", ...(hasSetupLog ? ["setup"] : [])] as Tab[]).map((t) => (
                <button
                  key={t}
                  onClick={() => setTab(t)}
                  className={clsx(
                    "rounded px-2 py-1 text-xs capitalize",
                    tab === t ? "bg-hover font-medium text-fg" : "text-muted hover:text-fg",
                  )}
                >
                  {t}
                </button>
              ))}
            </div>
            {ws.status === "creating" ? (
              <div className="flex flex-1 items-center justify-center gap-2 text-muted">
                <Loader2 size={13} className="animate-spin" /> Creating worktree…
              </div>
            ) : (
              <>
                <div className={clsx("min-h-0 flex-1", tab !== "changes" && "hidden")}>
                  <ChangesPanel key={ws.id} workspaceId={ws.id} sessionId={sessionId} />
                </div>
                <div className={clsx("min-h-0 flex-1", tab !== "checks" && "hidden")}>
                  <VerificationPanel key={ws.id} workspace={ws} visible={tab === "checks"} />
                </div>
                {/* Terminals stay mounted so switching tabs never loses scrollback. */}
                <div className={clsx("min-h-0 flex-1", tab !== "terminal" && "hidden")}>
                  <TerminalPanel workspaceId={ws.id} visible={tab === "terminal"} />
                </div>
              </>
            )}
            {tab === "setup" && <SetupLog workspaceId={ws.id} />}
          </section>
        )}
      </div>
    </div>
  );
}

function SetupLog({ workspaceId }: { workspaceId: string }) {
  const log = useStore((s) => s.scriptLog[workspaceId] ?? "");
  const ref = useRef<HTMLPreElement>(null);
  useEffect(() => {
    ref.current?.scrollTo(0, ref.current.scrollHeight);
  }, [log]);
  return (
    <pre ref={ref} className="selectable min-h-0 flex-1 overflow-auto p-3 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap">
      {log}
    </pre>
  );
}
