import { useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { Archive, ExternalLink, GitBranch, GitPullRequest, Loader2, PanelRight, Plus, X } from "lucide-react";
import { useResizable } from "../lib/resize";
import { ResizeHandle } from "./ResizeHandle";
import { actions, enabledAgents, toast, useStore } from "../lib/store";
import { useShallow } from "zustand/react/shallow";
import { AgentIcon, effortName, findModel, modelName } from "../lib/models";
import { PR_TAB, PrView } from "./PrView";
import { prAppearance } from "../lib/pr";
import { api, type LoadoutEntry, type Session } from "../lib/api";
import { Chat } from "./Chat";
import { ChangesPanel } from "./ChangesPanel";
import { TerminalPanel } from "./TerminalPanel";
import { PrActions } from "./PrActions";
import { SyncBadge } from "./SyncBadge";
import { Menu } from "./Menu";

type Tab = "changes" | "terminal" | "setup";

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
          <SessionTabs workspaceId={ws.id} activeId={sessionId} />
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
              {(["changes", "terminal", ...(hasSetupLog ? ["setup"] : [])] as Tab[]).map((t) => (
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
                  <ChangesPanel workspaceId={ws.id} sessionId={sessionId} />
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

const NO_LOADOUT: LoadoutEntry[] = [];

function SessionTabs({ workspaceId, activeId }: { workspaceId: string; activeId?: string }) {
  const sessions = useStore((s) => s.sessions[workspaceId]) ?? [];
  const agents = useStore((s) => s.agents);
  const menuAgents = useStore(useShallow(enabledAgents));
  const loadout = useStore((s) => s.settings?.loadout) ?? NO_LOADOUT;
  const catalogs = useStore((s) => s.catalogs);
  const featured = loadout.filter((l) => menuAgents.some((a) => a.id === l.agent));
  const pr = useStore((s) => s.prs[workspaceId]);
  return (
    <div className="flex h-9 shrink-0 items-center gap-1 overflow-x-auto overflow-y-hidden border-b border-border px-2">
      {pr && (
        <button
          onClick={() => actions.selectSession(workspaceId, PR_TAB)}
          className={clsx(
            "flex shrink-0 items-center gap-1.5 rounded px-2 py-1 text-xs",
            activeId === PR_TAB ? "bg-hover font-medium text-fg" : "text-muted hover:text-fg",
          )}
        >
          <GitPullRequest size={12} className={prAppearance(pr).color} /> PR #{pr.number}
        </button>
      )}
      {sessions.map((x) => (
        <SessionTab
          key={x.id}
          session={x}
          agentName={agents.find((a) => a.id === x.agentId)?.name ?? x.agentId}
          active={x.id === activeId}
        />
      ))}
      <Menu
        label={<Plus size={13} />}
        align="left"
        empty="No agents enabled"
        items={[
          ...featured.map((l) => ({
            label: `${modelName(l.agent, findModel(catalogs, l.agent, l.model), l.model)} · ${effortName(catalogs, l.agent, l.effort)}`,
            onSelect: () => actions.createSession(workspaceId, l.agent, l.model, l.effort),
          })),
          ...menuAgents
            .filter((a) => !featured.some((l) => l.agent === a.id))
            .map((a) => ({ label: a.name, onSelect: () => actions.createSession(workspaceId, a.id) })),
        ]}
      />
    </div>
  );
}

function SessionTab({ session, agentName, active }: { session: Session; agentName: string; active: boolean }) {
  // Only non-idle states get a dot; idle chats stay calm.
  const dot = useStore((s) => {
    const v = s.views[session.id];
    if (v?.permissions.length || v?.questions.length) return "bg-warn";
    if (v?.state === "running") return "pulse bg-accent";
    if (v?.state === "error") return "bg-del-fg";
    return v?.unread ? "bg-add-fg" : null;
  });
  const model = useStore((s) => {
    const value = session.model;
    return value ? modelName(session.agentId, findModel(s.catalogs, session.agentId, value), value) : "";
  });
  return (
    <div
      onClick={() => actions.selectSession(session.workspaceId, session.id)}
      title={[agentName, model].filter(Boolean).join(" · ")}
      className={clsx(
        "group flex max-w-56 shrink-0 items-center gap-1.5 rounded px-2 py-1 text-xs",
        active ? "bg-hover text-fg" : "text-muted hover:text-fg",
      )}
    >
      <AgentIcon agent={session.agentId} size={12} />
      <span className={clsx("truncate", active && "font-medium", !session.title && "text-muted")}>{session.title || "New chat"}</span>
      {dot && <span className={clsx("h-1.5 w-1.5 shrink-0 rounded-full", dot)} />}
      <button
        onClick={(e) => {
          e.stopPropagation();
          void actions.closeSession(session.workspaceId, session.id);
        }}
        className="invisible shrink-0 rounded p-0.5 hover:bg-bg group-hover:visible"
      >
        <X size={11} />
      </button>
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
