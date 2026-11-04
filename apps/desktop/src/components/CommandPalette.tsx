import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import { ArrowDownToLine, ArrowRight, ChartColumn, FolderPlus, GitBranch, House, Keyboard, ListChecks, Plus, Search, Settings as SettingsIcon } from "lucide-react";
import { actions, useStore } from "../lib/store";
import { AgentIcon, effortName, findModel, modelName } from "../lib/models";
import { pickRepo } from "../App";

type Command = { id: string; label: string; hint?: string; icon: React.ReactNode; group: string; run: () => void };

function score(label: string, q: string) {
  const l = label.toLowerCase();
  if (l.startsWith(q)) return 0;
  const words = l.split(/[\s/·-]+/);
  if (words.some((w) => w.startsWith(q))) return 1;
  if (l.includes(q)) return 2;
  let i = 0;
  for (const c of l) if (c === q[i]) i++;
  return i === q.length ? 3 : -1;
}

/** ⌘K: jump to a workspace or run an action by typing. */
export function CommandPalette({ onClose, onShortcuts }: { onClose: () => void; onShortcuts: () => void }) {
  // Snapshot on open: the palette is short-lived and shouldn't re-render on
  // every streamed token.
  const [s] = useState(() => useStore.getState());
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  const commands = useMemo<Command[]>(() => {
    const out: Command[] = [];
    const ws = s.workspaces.find((w) => w.id === s.selectedWorkspace);
    const sessionId = ws ? s.selectedSession[ws.id] : undefined;
    for (const w of s.workspaces) {
      const title = w.title || (s.sessions[w.id] ?? []).find((x) => x.title)?.title || w.name;
      const repo = s.repos.find((r) => r.id === w.repoId)?.name;
      out.push({
        id: `ws:${w.id}`,
        label: title,
        hint: `${repo} · ${w.branch}`,
        icon: <GitBranch size={13} />,
        group: "Workspaces",
        run: () => actions.selectWorkspace(w.id),
      });
    }
    if (ws) {
      for (const l of s.settings?.loadout ?? []) {
        out.push({
          id: `chat:${l.agent}:${l.model}`,
          label: `New chat with ${modelName(l.agent, findModel(s.catalogs, l.agent, l.model), l.model)}`,
          hint: effortName(s.catalogs, l.agent, l.effort),
          icon: <AgentIcon agent={l.agent} />,
          group: "Actions",
          run: () => actions.createSession(ws.id, l.agent, l.model, l.effort),
        });
      }
      if (sessionId) {
        out.push({
          id: "plan",
          label: s.views[sessionId]?.plan ? "Switch to Auto mode" : "Switch to Plan mode",
          hint: "⇧⇥",
          icon: <ListChecks size={13} />,
          group: "Actions",
          run: () => actions.togglePlan(sessionId),
        });
      }
    }
    for (const r of s.repos) {
      out.push({
        id: `new:${r.id}`,
        label: `New workspace in ${r.name}`,
        hint: r.id === ws?.repoId ? "⌘N" : undefined,
        icon: <Plus size={13} />,
        group: "Actions",
        run: () => actions.createWorkspace(r.id),
      });
      out.push({
        id: `new-from:${r.id}`,
        label: `New workspace in ${r.name} from a branch or PR`,
        hint: r.id === ws?.repoId ? "⌘⇧N" : undefined,
        icon: <GitBranch size={13} />,
        group: "Actions",
        run: () => actions.openNewFrom(r.id),
      });
    }
    out.push(
      { id: "home", label: "Home: all workspaces", icon: <House size={13} />, group: "Go to", run: () => actions.openHome() },
      { id: "insights", label: "Insights: cost and usage", icon: <ChartColumn size={13} />, group: "Go to", run: () => actions.openInsights() },
      { id: "settings", label: "Settings", hint: "⌘,", icon: <SettingsIcon size={13} />, group: "Go to", run: () => actions.openSettings(true) },
      { id: "shortcuts", label: "Keyboard shortcuts", hint: "⌘/", icon: <Keyboard size={13} />, group: "Go to", run: onShortcuts },
      { id: "add-repo", label: "Add repository…", hint: "⇧⌘O", icon: <FolderPlus size={13} />, group: "Actions", run: () => void pickRepo() },
    );
    if (s.conductor?.detected) {
      out.push({ id: "import-conductor", label: "Import from Conductor…", icon: <ArrowDownToLine size={13} />, group: "Actions", run: () => actions.openConductorImport() });
    }
    for (const r of s.recents) {
      out.push({
        id: `recent:${r.path}`,
        label: `Add ${r.name}`,
        hint: r.path.replace(/^\/Users\/[^/]+/, "~"),
        icon: <FolderPlus size={13} />,
        group: "Recent repositories",
        run: () => void actions.addRepo(r.path),
      });
    }
    return out;
  }, [s, onShortcuts]);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return commands;
    return commands
      .map((c) => ({ c, score: Math.min(...[c.label, c.hint ?? ""].map((t) => score(t, q)).map((x) => (x < 0 ? 99 : x))) }))
      .filter((x) => x.score < 99)
      .sort((a, b) => a.score - b.score)
      .map((x) => x.c);
  }, [commands, query]);

  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const run = (c: Command | undefined) => {
    if (!c) return;
    onClose();
    c.run();
  };

  let lastGroup = "";
  return createPortal(
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/25 pt-[14vh]" onMouseDown={onClose}>
      <div
        className="w-[560px] max-w-[92vw] overflow-hidden rounded-xl border border-border bg-elevated shadow-2xl"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          else if (e.key === "ArrowDown") {
            e.preventDefault();
            setActive((a) => Math.min(results.length - 1, a + 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setActive((a) => Math.max(0, a - 1));
          } else if (e.key === "Enter") {
            e.preventDefault();
            run(results[active]);
          }
        }}
      >
        <div className="flex items-center gap-2 border-b border-border px-3.5 py-3">
          <Search size={15} className="text-faint" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Jump to a workspace or run a command"
            className="selectable flex-1 bg-transparent text-[14px] outline-none placeholder:text-faint"
          />
          <kbd className="font-sans text-[11px] text-faint">esc</kbd>
        </div>
        <div ref={listRef} className="max-h-[50vh] overflow-y-auto p-1.5">
          {results.length === 0 && <div className="px-3 py-6 text-center text-xs text-muted">Nothing matches.</div>}
          {results.map((c, i) => {
            const header = !query && c.group !== lastGroup ? c.group : null;
            lastGroup = c.group;
            return (
              <div key={c.id}>
                {header && <div className="px-2.5 pt-2 pb-1 text-[11px] text-muted">{header}</div>}
                <button
                  data-index={i}
                  onClick={() => run(c)}
                  onMouseMove={() => setActive(i)}
                  className={clsx("flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13px]", i === active && "bg-hover")}
                >
                  <span className="text-muted">{c.icon}</span>
                  <span className="truncate">{c.label}</span>
                  {c.hint && <span className="shrink-0 truncate text-xs text-faint">{c.hint}</span>}
                  <span className="flex-1" />
                  {i === active && <ArrowRight size={12} className="text-faint" />}
                </button>
              </div>
            );
          })}
        </div>
      </div>
    </div>,
    document.body,
  );
}

