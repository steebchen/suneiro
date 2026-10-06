import { memo, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import {
  Archive,
  ChartColumn,
  ChevronRight,
  CircleDashed,
  CircleX,
  Download,
  FolderPlus,
  GitBranch,
  GitMerge,
  GitPullRequest,
  GitPullRequestClosed,
  House,
  Loader2,
  Mail,
  MailOpen,
  Pencil,
  Plus,
  Settings as SettingsIcon,
  Trash2,
} from "lucide-react";
import { checkSummary, prAppearance } from "../lib/pr";
import { useResizable } from "../lib/resize";
import { ResizeHandle } from "./ResizeHandle";
import { actions, formatCost, useStore, workspaceActivity } from "../lib/store";
import { type PrStatus, type Repo, type Workspace } from "../lib/api";
import { BrandMark } from "./BrandMark";
import { AddRepoMenu } from "./AddRepoMenu";
import { openRepoSettings } from "./RepoSettings";

/** Collapsed repo groups (a per-device convenience). */
function useCollapsed() {
  const key = "suneiro.collapsedRepos";
  const [collapsed, setCollapsed] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem(key) ?? "[]");
    } catch {
      return [];
    }
  });
  const toggle = (id: string) => {
    setCollapsed((c) => {
      const next = c.includes(id) ? c.filter((x) => x !== id) : [...c, id];
      localStorage.setItem(key, JSON.stringify(next));
      return next;
    });
  };
  return { collapsed, toggle };
}

/**
 * Reorder repo groups by dragging their header. Uses pointer events rather
 * than HTML5 drag and drop, which webviews handle inconsistently. `drop` is
 * the index in the list the dragged repo would be inserted before.
 */
function useRepoDrag(listRef: React.RefObject<HTMLDivElement | null>) {
  const [drag, setDrag] = useState<{ id: string; drop: number } | null>(null);
  // Set once a press turns into a drag, so the click that follows doesn't toggle the group.
  const dragged = useRef(false);

  const startDrag = (id: string, e: React.PointerEvent) => {
    dragged.current = false;
    if (e.button !== 0 || (e.target as HTMLElement).closest("[data-no-drag]")) return;
    const startY = e.clientY;
    let drop = -1;
    const dropAt = (y: number) => {
      const groups = [...(listRef.current?.querySelectorAll<HTMLElement>("[data-repo-id]") ?? [])];
      const index = groups.findIndex((g) => {
        const r = g.getBoundingClientRect();
        return y < r.top + r.height / 2;
      });
      return index < 0 ? groups.length : index;
    };
    const move = (ev: PointerEvent) => {
      if (!dragged.current && Math.abs(ev.clientY - startY) < 5) return;
      if (!dragged.current) {
        dragged.current = true;
        document.body.style.cursor = "grabbing";
      }
      const next = dropAt(ev.clientY);
      if (next !== drop) setDrag({ id, drop: (drop = next) });
    };
    const end = (ev: Event) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      window.removeEventListener("blur", end);
      document.body.style.cursor = "";
      setDrag(null);
      if (dragged.current && ev.type === "pointerup" && drop >= 0) void actions.reorderRepo(id, drop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
    window.addEventListener("blur", end);
  };

  return { drag, dragged, startDrag };
}

export function Sidebar() {
  const repos = useStore((s) => s.repos);
  const { collapsed, toggle } = useCollapsed();
  const listRef = useRef<HTMLDivElement>(null);
  const { drag, dragged, startDrag } = useRepoDrag(listRef);
  const from = drag ? repos.findIndex((r) => r.id === drag.id) : -1;
  // Dropping right before or after itself leaves the order unchanged: no indicator.
  const drop = drag && drag.drop !== from && drag.drop !== from + 1 ? drag.drop : -1;
  const { width, ref, onPointerDown } = useResizable("runner.sidebarWidth", 280, 200, 480, "right");
  return (
    <aside
      ref={ref as React.RefObject<HTMLElement>}
      style={{ width }}
      className="relative flex shrink-0 flex-col border-r border-border bg-panel"
    >
      <ResizeHandle edge="right" onPointerDown={onPointerDown} />
      <div className="flex h-13 shrink-0 items-center justify-end px-2" data-tauri-drag-region>
        <AddRepoMenu
          align="right"
          trigger={(open) => (
            <button onClick={open} title="Add repository" className="rounded p-1.5 text-muted hover:bg-hover hover:text-fg">
              <FolderPlus size={15} />
            </button>
          )}
        />
      </div>
      <div className="mb-3 flex items-center gap-2 px-4 text-fg" data-tauri-drag-region>
        <BrandMark size={26} />
        <span className="suneiro-wordmark text-base">Suneiro</span>
      </div>
      <div className="space-y-0.5 px-2 pb-2">
        <HomeButton />
        <InsightsButton />
      </div>
      <div ref={listRef} className="flex-1 overflow-y-auto px-2 pb-3">
        {repos.map((r, i) => (
          <RepoGroup
            key={r.id}
            repo={r}
            collapsed={collapsed.includes(r.id)}
            dragging={drag?.id === r.id}
            dropBefore={drop === i}
            dropAfter={drop === repos.length && i === repos.length - 1}
            onToggle={() => {
              if (!dragged.current) toggle(r.id);
            }}
            onPointerDown={(e) => startDrag(r.id, e)}
          />
        ))}
      </div>
      <div className="shrink-0 border-t border-border p-2">
        <UpdateButton />
        <button
          onClick={() => actions.openSettings(useStore.getState().page !== "settings")}
          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-muted hover:bg-hover hover:text-fg"
        >
          <SettingsIcon size={14} /> Settings
          <span className="ml-auto text-[10px] text-faint">⌘,</span>
        </button>
      </div>
    </aside>
  );
}

/** Brings back a dismissed update toast, or shows that one is queued. */
function UpdateButton() {
  const update = useStore((s) => s.update);
  const dismissed = useStore((s) => s.updateDismissed);
  if (!update?.version || update.phase === "none") return null;
  // The toast is showing; no need for a second prompt.
  if (update.phase === "available" && dismissed !== update.version) return null;
  const queued = update.phase !== "available";
  return (
    <button
      onClick={() => useStore.setState({ updateDismissed: null })}
      disabled={queued}
      title={queued ? `Suneiro ${update.version} installs once agents finish` : `Suneiro ${update.version} is available`}
      className="mb-1 flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-accent hover:bg-hover disabled:hover:bg-transparent"
    >
      <Download size={14} /> {queued ? "Update queued" : "Update available"}
      <span className="ml-auto text-[10px] opacity-80">{update.version}</span>
    </button>
  );
}

function HomeButton() {
  const active = useStore((s) => s.page === "home");
  const count = useStore((s) => s.workspaces.length);
  return (
    <button
      onClick={() => actions.openHome()}
      className={clsx(
        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left",
        active ? "bg-hover font-medium text-fg" : "text-fg/90 hover:bg-hover/60",
      )}
    >
      <House size={14} className="shrink-0 text-muted" />
      <span className="flex-1">Home</span>
      <span className="text-[11px] text-faint">{count}</span>
    </button>
  );
}

function InsightsButton() {
  const active = useStore((s) => s.page === "insights");
  const today = useStore((s) => {
    const start = new Date().setHours(0, 0, 0, 0);
    return s.usage.reduce((sum, u) => (u.ts >= start && u.cost ? sum + u.cost : sum), 0);
  });
  return (
    <button
      onClick={() => actions.openInsights()}
      className={clsx(
        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left",
        active ? "bg-hover font-medium text-fg" : "text-fg/90 hover:bg-hover/60",
      )}
    >
      <ChartColumn size={14} className="shrink-0 text-muted" />
      <span className="flex-1">Insights</span>
      {today > 0 && (
        <span className="text-[11px] text-faint" title="Cost today">
          {formatCost(today)}
        </span>
      )}
    </button>
  );
}

function RepoGroup({
  repo,
  collapsed,
  dragging,
  dropBefore,
  dropAfter,
  onToggle,
  onPointerDown,
}: {
  repo: Repo;
  collapsed: boolean;
  dragging: boolean;
  dropBefore: boolean;
  dropAfter: boolean;
  onToggle: () => void;
  onPointerDown: (e: React.PointerEvent) => void;
}) {
  const workspaces = useStore((s) => s.workspaces);
  const list = workspaces.filter((w) => w.repoId === repo.id);
  return (
    <div data-repo-id={repo.id} className={clsx("relative pb-3", dragging && "opacity-50")}>
      {(dropBefore || dropAfter) && (
        <div className={clsx("pointer-events-none absolute inset-x-1 h-0.5 rounded bg-accent", dropBefore ? "-top-px" : "bottom-1")} />
      )}
      <div
        onPointerDown={onPointerDown}
        className="group flex h-7 items-center gap-1 pr-2 pl-1 text-[11px] font-semibold tracking-wide text-muted uppercase"
      >
        <button
          onClick={onToggle}
          aria-expanded={!collapsed}
          title={repo.path}
          className="flex min-w-0 flex-1 items-center gap-1 rounded py-1 text-left tracking-wide uppercase hover:text-fg"
        >
          <ChevronRight size={12} className={clsx("shrink-0 text-faint transition-transform", !collapsed && "rotate-90")} />
          <span className="truncate">{repo.name}</span>
          {collapsed && list.length > 0 && <span className="font-normal text-faint">{list.length}</span>}
        </button>
        {list.length === 0 && (
          <button
            data-no-drag
            onClick={() => actions.removeRepo(repo.id)}
            title="Remove repository"
            className="hidden rounded p-1 hover:bg-hover hover:text-fg group-hover:block"
          >
            <Trash2 size={12} />
          </button>
        )}
        <button
          data-no-drag
          onClick={() => actions.openNewFrom(repo.id)}
          title="New workspace from a branch or pull request (⌘⇧N)"
          className="hidden rounded p-1 hover:bg-hover hover:text-fg group-hover:block"
        >
          <GitBranch size={12} />
        </button>
        <button
          data-no-drag
          onClick={() => openRepoSettings(repo.id)}
          title="Repository settings (scripts, files to copy)"
          className="hidden rounded p-1 hover:bg-hover hover:text-fg group-hover:block"
        >
          <SettingsIcon size={12} />
        </button>
        <button
          data-no-drag
          onClick={() => actions.createWorkspace(repo.id)}
          title="New workspace (⌘N)"
          className="rounded p-1 hover:bg-hover hover:text-fg"
        >
          <Plus size={13} />
        </button>
      </div>
      {!collapsed && list.map((w) => <WorkspaceRow key={w.id} ws={w} index={workspaces.indexOf(w)} />)}
    </div>
  );
}

const WorkspaceRow = memo(function WorkspaceRow({ ws, index }: { ws: Workspace; index: number }) {
  const selected = useStore((s) => s.page === "workspace" && s.selectedWorkspace === ws.id);
  const activity = useStore((s) => workspaceActivity(s, ws.id));
  const sessionTitle = useStore((s) => (s.sessions[ws.id] ?? []).find((x) => x.title)?.title ?? "");
  const pr = useStore((s) => s.prs[ws.id]);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState(false);
  const title = ws.title || sessionTitle;
  const display = title || (ws.status === "creating" ? "New workspace" : ws.name);
  const showArchive = !renaming && ws.status !== "creating";

  return (
    <>
      <div
        role="button"
        onClick={() => !renaming && actions.selectWorkspace(ws.id)}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ x: e.clientX, y: e.clientY });
        }}
        className={clsx(
          "group relative flex w-full flex-col rounded-md px-2 py-1.5 text-left",
          selected ? "bg-hover" : "hover:bg-hover/60",
          menu && !selected && "bg-hover/60",
        )}
      >
        <div className="flex w-full items-center gap-2">
          <StatusDot activity={activity} setup={ws.status} />
          {renaming ? (
            <RenameInput
              initial={display}
              onDone={(value) => {
                setRenaming(false);
                if (value !== null && value.trim() && value.trim() !== display) void actions.renameWorkspace(ws.id, value);
              }}
            />
          ) : (
            <span
              className={clsx(
                "flex-1 truncate",
                ws.unread ? "font-semibold text-fg" : selected ? "font-medium text-fg" : "text-fg/90",
                !title && !ws.unread && "text-muted",
              )}
            >
              {display}
            </span>
          )}
          <span className={clsx("shrink-0", showArchive && "group-hover:invisible")}>
            {pr ? <PrBadge pr={pr} /> : index < 9 && <span className="text-[10px] text-faint">⌘{index + 1}</span>}
          </span>
        </div>
        <div className="flex items-center gap-1 pl-4 text-[11px] text-muted">
          <GitBranch size={10} className="shrink-0" />
          <span className="min-w-0 flex-1 truncate">{ws.status === "creating" ? "Creating worktree…" : ws.branch}</span>
          <span className={clsx("shrink-0", showArchive && "group-hover:invisible")}>
            <DiffStat workspaceId={ws.id} />
          </span>
        </div>
        {showArchive && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              void actions.archiveWorkspace(ws.id);
            }}
            title="Archive"
            aria-label="Archive workspace"
            className="absolute top-1/2 right-1.5 hidden -translate-y-1/2 rounded p-1 text-muted group-hover:block hover:bg-hover hover:text-fg"
          >
            <Archive size={14} />
          </button>
        )}
      </div>
      {menu && (
        <ContextMenu
          at={menu}
          onClose={() => setMenu(null)}
          items={[
            ws.unread
              ? { label: "Mark as read", icon: MailOpen, onSelect: () => actions.markUnread(ws.id, false) }
              : { label: "Mark as unread", icon: Mail, onSelect: () => actions.markUnread(ws.id, true) },
            { label: "Rename…", icon: Pencil, onSelect: () => setRenaming(true) },
            { label: "Archive", icon: Archive, onSelect: () => actions.archiveWorkspace(ws.id) },
          ]}
        />
      )}
    </>
  );
});

function DiffStat({ workspaceId }: { workspaceId: string }) {
  const stats = useStore((s) => s.diffStats[workspaceId]);
  if (!stats?.files) return null;
  return (
    <span className="shrink-0 font-mono text-[10px]" title={`${stats.files} changed file${stats.files === 1 ? "" : "s"}`}>
      <span className="text-add-fg">+{stats.add}</span> <span className="text-del-fg">−{stats.del}</span>
    </span>
  );
}

/** Failed/pending checks icon, then the PR icon + number in its state color. */
function PrBadge({ pr }: { pr: PrStatus }) {
  const { color, label } = prAppearance(pr);
  const checks = checkSummary(pr);
  const Icon = pr.state === "MERGED" ? GitMerge : pr.state === "CLOSED" ? GitPullRequestClosed : GitPullRequest;
  const open = pr.state === "OPEN";
  return (
    <span
      className="flex shrink-0 items-center gap-1 text-[10.5px]"
      title={`PR #${pr.number} · ${label}${open && checks.failed.length ? ` · failing: ${checks.failed.join(", ")}` : open && checks.pending ? " · checks running" : ""}`}
    >
      {open && checks.state === "failure" && <CircleX size={11} className="text-del-fg" />}
      {open && checks.state === "pending" && <CircleDashed size={11} className="animate-spin text-warn [animation-duration:3s]" />}
      <Icon size={11} className={color} />
      <span className={color}>#{pr.number}</span>
    </span>
  );
}

function RenameInput({ initial, onDone }: { initial: string; onDone: (value: string | null) => void }) {
  const [value, setValue] = useState(initial);
  return (
    <input
      autoFocus
      value={value}
      onFocus={(e) => e.target.select()}
      onChange={(e) => setValue(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onBlur={() => onDone(value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") onDone(value);
        else if (e.key === "Escape") onDone(null);
      }}
      className="selectable min-w-0 flex-1 rounded border border-accent bg-bg px-1 text-[13px] outline-none"
    />
  );
}

type MenuItem = { label: string; icon: typeof Archive; onSelect: () => void };

function ContextMenu({ at, items, onClose }: { at: { x: number; y: number }; items: MenuItem[]; onClose: () => void }) {
  useEffect(() => {
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      onClose();
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
      window.removeEventListener("blur", close);
    };
  }, [onClose]);
  return createPortal(
    <div
      className="fixed z-50 min-w-44 rounded-md border border-border bg-elevated p-1 shadow-xl"
      style={{ left: Math.min(at.x, window.innerWidth - 190), top: Math.min(at.y, window.innerHeight - 40 * items.length) }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      {items.map((item) => (
        <button
          key={item.label}
          onClick={() => {
            onClose();
            item.onSelect();
          }}
          className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[13px] hover:bg-hover"
        >
          <item.icon size={13} className="text-muted" />
          {item.label}
        </button>
      ))}
    </div>,
    document.body,
  );
}

function StatusDot({ activity, setup }: { activity: ReturnType<typeof workspaceActivity>; setup: string }) {
  if (setup === "creating" || (setup === "setting_up" && activity === "idle")) {
    return <Loader2 size={10} className="shrink-0 animate-spin text-muted" />;
  }
  const cls =
    setup === "setup_failed" || setup === "failed"
      ? "bg-del-fg"
      : {
          "needs-input": "bg-warn",
          running: "bg-accent pulse",
          error: "bg-del-fg",
          unread: "bg-add-fg",
          idle: "bg-faint/50",
        }[activity];
  return <span className={clsx("h-2 w-2 shrink-0 rounded-full", cls)} />;
}
