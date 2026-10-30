import { memo, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import clsx from "clsx";
import { ChevronDown, ChevronRight, GitPullRequest, Plus, Unlink, X } from "lucide-react";
import type { Session } from "../lib/api";
import { actions, useStore } from "../lib/store";
import { AgentIcon } from "../lib/models";
import { Menu } from "./Menu";
import { PR_TAB } from "./PrView";
import { prAppearance } from "../lib/pr";

type Node = { session: Session; depth: number; children: string[]; descendants: string[] };
const EMPTY: Record<string, Session[]> = {};
const COLLAPSED_KEY = "runner.tabChildrenCollapsed";

function ancestors(session: Session | undefined, byId: Map<string, Session>): Session[] {
  const path: Session[] = [];
  const seen = new Set<string>();
  while (session && !seen.has(session.id)) {
    seen.add(session.id);
    path.unshift(session);
    session = session.parentSessionId ? byId.get(session.parentSessionId) : undefined;
  }
  return path;
}

function buildTree(lists: Record<string, Session[]>, workspaceId: string) {
  const all = Object.values(lists).flat();
  const byId = new Map(all.map((s) => [s.id, s]));
  const children = new Map<string, string[]>();
  for (const s of all) {
    if (!s.parentSessionId) continue;
    const list = children.get(s.parentSessionId) ?? [];
    list.push(s.id);
    children.set(s.parentSessionId, list);
  }
  const included = new Set<string>();
  const includeDescendants = (id: string) => {
    if (included.has(id)) return;
    included.add(id);
    for (const child of children.get(id) ?? []) includeDescendants(child);
  };
  for (const s of lists[workspaceId] ?? []) {
    const root = ancestors(s, byId)[0] ?? s;
    includeDescendants(root.id);
  }
  const nodes = new Map<string, Node>();
  const roots: string[] = [];
  for (const s of all) {
    if (!included.has(s.id)) continue;
    const descendants: string[] = [];
    const collect = (id: string) => {
      if (descendants.includes(id)) return;
      descendants.push(id);
      for (const child of children.get(id) ?? []) collect(child);
    };
    collect(s.id);
    nodes.set(s.id, {
      session: s,
      depth: ancestors(s, byId).length - 1,
      children: (children.get(s.id) ?? []).filter((id) => included.has(id)),
      descendants,
    });
    if (!s.parentSessionId || !included.has(s.parentSessionId)) roots.push(s.id);
  }
  return { byId, nodes, roots };
}

function readCollapsed(): Set<string> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]");
    return new Set(Array.isArray(value) ? value.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function navigate(session: Session) {
  actions.selectWorkspace(session.workspaceId);
  actions.selectSession(session.workspaceId, session.id);
}

export const TabHierarchy = memo(function TabHierarchy({ workspaceId, activeId }: { workspaceId: string; activeId?: string }) {
  const lists = useStore((s) => s.sessions) ?? EMPTY;
  const workspaces = useStore((s) => s.workspaces);
  const maxDepth = useStore((s) => s.settings?.maxTabDepth ?? 4);
  const pr = useStore((s) => s.prs[workspaceId]);
  const tree = useMemo(() => buildTree(lists, workspaceId), [lists, workspaceId]);
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const ref = useRef<HTMLDivElement>(null);
  const breadcrumbRef = useRef<HTMLElement>(null);
  const active = activeId ? tree.byId.get(activeId) : undefined;
  const path = ancestors(active, tree.byId);
  const canCreateChild = !!active && path.length <= maxDepth;
  const currentWorkspace = workspaces.find((w) => w.id === workspaceId);
  const targets = workspaces.filter((w) => w.repoId === currentWorkspace?.repoId && w.status !== "creating");
  useEffect(() => {
    if (!activeId) return;
    breadcrumbRef.current?.querySelector('[aria-current="page"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });
    const path = ancestors(tree.byId.get(activeId), tree.byId).slice(0, -1);
    setCollapsed((prev) => {
      if (!path.some((s) => prev.has(s.id))) return prev;
      const next = new Set(prev);
      for (const s of path) next.delete(s.id);
      return next;
    });
  }, [activeId, tree]);
  useEffect(() => {
    try {
      localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...collapsed]));
    } catch {
      /* Per-device convenience. */
    }
  }, [collapsed]);
  const toggle = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const visible: Node[] = [];
  const visit = (id: string) => {
    const node = tree.nodes.get(id);
    if (!node) return;
    visible.push(node);
    if (!collapsed.has(id)) for (const child of node.children) visit(child);
  };
  for (const id of tree.roots) visit(id);
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    if (target.getAttribute("role") !== "treeitem") return;
    const buttons = [...(ref.current?.querySelectorAll<HTMLButtonElement>('[role="treeitem"]') ?? [])];
    const index = buttons.indexOf(target as HTMLButtonElement);
    const node = visible[index];
    if (!node) return;
    let next = index;
    switch (e.key) {
      case "ArrowDown":
        next = Math.min(index + 1, buttons.length - 1);
        break;
      case "ArrowUp":
        next = Math.max(index - 1, 0);
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = buttons.length - 1;
        break;
      case "ArrowRight":
        if (node.children.length && collapsed.has(node.session.id)) toggle(node.session.id);
        else if (node.children.length) next = index + 1;
        break;
      case "ArrowLeft":
        if (node.children.length && !collapsed.has(node.session.id)) toggle(node.session.id);
        else
          next = Math.max(
            0,
            visible.findIndex((n) => n.session.id === node.session.parentSessionId),
          );
        break;
      default:
        return;
    }
    e.preventDefault();
    buttons[next]?.focus();
  };
  return (
    <div className="shrink-0 border-b border-border bg-panel">
      <div className="flex min-h-9 items-center gap-1 px-2">
        {pr && (
          <button
            onClick={() => actions.selectSession(workspaceId, PR_TAB)}
            className={clsx(
              "flex shrink-0 items-center gap-1 rounded px-2 py-1 text-xs",
              activeId === PR_TAB ? "bg-hover text-fg" : "text-muted hover:text-fg",
            )}
          >
            <GitPullRequest size={12} className={prAppearance(pr).color} /> PR #{pr.number}
          </button>
        )}
        <nav
          ref={breadcrumbRef}
          data-chat-navigation
          aria-label="Chat breadcrumb"
          className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto text-xs text-muted"
        >
          {path.map((s, i) => (
            <span key={s.id} className="flex min-w-0 shrink-0 items-center gap-1">
              {i > 0 && <ChevronRight size={11} />}
              <button
                onClick={() => navigate(s)}
                aria-current={s.id === activeId ? "page" : undefined}
                className="max-w-40 truncate rounded px-1 py-1 hover:bg-hover hover:text-fg"
              >
                {s.title || "New chat"}
              </button>
            </span>
          ))}
        </nav>
        <button
          aria-label="New root chat"
          title="New root chat (⌘T)"
          onClick={() => void actions.newChat(workspaceId)}
          className="rounded p-1 text-muted hover:bg-hover hover:text-fg"
        >
          <Plus size={14} />
        </button>
        {active && (
          <Menu
            label="New related chat"
            items={[
              ...(canCreateChild
                ? [
                    {
                      label: "Child chat",
                      onSelect: () => {
                        void actions.newRelatedChat(active.id, "child");
                      },
                    },
                  ]
                : []),
              {
                label: "Sibling chat",
                onSelect: () => {
                  void actions.newRelatedChat(active.id, "sibling");
                },
              },
              ...(canCreateChild
                ? targets
                    .filter((w) => w.id !== workspaceId)
                    .map((w) => ({
                      label: `Child in ${w.title || w.name}`,
                      onSelect: () => {
                        void actions.newRelatedChat(active.id, "child", w.id);
                      },
                    }))
                : []),
            ]}
          />
        )}
      </div>
      <div
        ref={ref}
        data-chat-navigation
        role="tree"
        aria-label="Chat hierarchy"
        onKeyDown={onKeyDown}
        className="max-h-44 overflow-y-auto px-2 pb-1"
      >
        {visible.map((node) => (
          <HierarchyRow
            key={node.session.id}
            node={node}
            active={node.session.id === activeId}
            collapsed={collapsed.has(node.session.id)}
            onToggle={toggle}
            workspaceLabel={
              node.session.workspaceId !== workspaceId ? workspaces.find((w) => w.id === node.session.workspaceId)?.name : undefined
            }
            closable={(lists[node.session.workspaceId]?.length ?? 0) > 1}
          />
        ))}
      </div>
      {active && !canCreateChild && (
        <div className="px-3 pb-1 text-[11px] text-muted">
          Maximum child depth ({maxDepth}) reached. Create a sibling or detach this chat.
        </div>
      )}
    </div>
  );
});

const HierarchyRow = memo(function HierarchyRow({
  node,
  active,
  collapsed,
  onToggle,
  workspaceLabel,
  closable,
}: {
  node: Node;
  active: boolean;
  collapsed: boolean;
  onToggle: (id: string) => void;
  workspaceLabel?: string;
  closable: boolean;
}) {
  const { session, children, descendants, depth } = node;
  const status = useStore((s) => {
    let running = false,
      error = false,
      unread = false;
    for (const id of descendants) {
      const v = s.views[id];
      if (v?.permissions.length || v?.questions.length) return "needs input";
      running ||= v?.state === "running" || v?.state === "connecting";
      error ||= v?.state === "error";
      unread ||= !!v?.unread;
    }
    return error ? "error" : running ? "running" : unread ? "unread" : "";
  });
  const rowRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (active) rowRef.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [active]);
  return (
    <div role="none" className="group flex items-center gap-1" style={{ paddingLeft: Math.min(depth, 8) * 14 }}>
      {children.length ? (
        <button
          aria-label={`${collapsed ? "Expand" : "Collapse"} children of ${session.title || "New chat"}`}
          onClick={() => onToggle(session.id)}
          className="shrink-0 rounded p-0.5 text-muted hover:bg-hover"
        >
          {collapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
        </button>
      ) : (
        <span className="w-4 shrink-0" />
      )}
      <button
        ref={rowRef}
        data-session-id={session.id}
        role="treeitem"
        aria-level={depth + 1}
        aria-expanded={children.length ? !collapsed : undefined}
        aria-selected={active}
        onClick={() => navigate(session)}
        className={clsx(
          "flex min-w-0 flex-1 items-center gap-1.5 rounded px-1.5 py-1 text-left text-xs focus-visible:outline-2 focus-visible:outline-accent",
          active ? "bg-hover font-medium text-fg" : "text-muted hover:bg-hover hover:text-fg",
        )}
      >
        <AgentIcon agent={session.agentId} size={12} />
        <span className="truncate">{session.title || "New chat"}</span>
        {workspaceLabel && <span className="shrink-0 text-[10px] text-muted">· {workspaceLabel}</span>}
        {collapsed && children.length > 0 && <span className="text-[10px] text-muted">{descendants.length - 1}</span>}
        {status && (
          <span
            aria-label={status}
            title={`${status}${descendants.length > 1 ? " in this chat or its children" : ""}`}
            className={clsx(
              "ml-auto h-1.5 w-1.5 shrink-0 rounded-full",
              status === "needs input"
                ? "bg-warn"
                : status === "error"
                  ? "bg-del-fg"
                  : status === "running"
                    ? "pulse bg-accent"
                    : "bg-add-fg",
            )}
          />
        )}
      </button>
      {session.parentSessionId && (
        <button
          title="Detach chat (keep this chat and its children)"
          aria-label={`Detach ${session.title || "New chat"}`}
          onClick={() => void actions.detachSession(session.id)}
          className="invisible rounded p-1 text-muted hover:bg-hover group-hover:visible group-focus-within:visible"
        >
          <Unlink size={11} />
        </button>
      )}
      {closable && (
        <button
          title="Close this chat only; children become root chats"
          aria-label={`Close ${session.title || "New chat"}`}
          onClick={() => void actions.closeSession(session.workspaceId, session.id)}
          className="invisible rounded p-1 text-muted hover:bg-hover group-hover:visible group-focus-within:visible"
        >
          <X size={11} />
        </button>
      )}
    </div>
  );
});
