import { useEffect, useMemo, useState } from "react";
import clsx from "clsx";
import { ArrowDownToLine, Check, CircleAlert, GitBranch, Loader2, MessageSquare, Search, X } from "lucide-react";
import type { ConductorWorkspace } from "../lib/api";
import { actions, useStore } from "../lib/store";
import { ago } from "../lib/time";

const tilde = (p: string) => p.replace(/^\/Users\/[^/]+/, "~");
const HINT_DISMISSED = "suneiro.conductorHintDismissed";

/** Conductor worktrees, active in Conductor, that aren't in Suneiro yet. */
export function useConductorPending() {
  return useStore((s) => s.conductor?.workspaces.filter((w) => !w.imported && !w.archivedInConductor).length ?? 0);
}

/** First-run hint shown when a Conductor install with importable workspaces is found. */
export function ConductorHint({ className }: { className?: string }) {
  const pending = useConductorPending();
  const [dismissed, setDismissed] = useState(() => localStorage.getItem(HINT_DISMISSED) === "1");
  if (!pending || dismissed) return null;
  return (
    <div className={clsx("flex items-center gap-3 rounded-lg border border-border bg-elevated px-3.5 py-2.5 text-[13px]", className)}>
      <ArrowDownToLine size={15} className="shrink-0 text-accent" />
      <div className="min-w-0 flex-1">
        <div className="font-medium">Coming from Conductor?</div>
        <div className="text-xs text-muted">
          {pending === 1 ? "1 workspace" : `${pending} workspaces`} can move over with {pending === 1 ? "its" : "their"} worktrees, branches and chats. Nothing in Conductor changes.
        </div>
      </div>
      <button onClick={() => actions.openConductorImport()} className="shrink-0 rounded-md bg-accent px-2.5 py-1 text-xs font-medium text-accent-fg">
        Import…
      </button>
      <button
        onClick={() => {
          localStorage.setItem(HINT_DISMISSED, "1");
          setDismissed(true);
        }}
        className="shrink-0 rounded p-1 text-muted hover:bg-hover hover:text-fg"
        title="Dismiss"
      >
        <X size={13} />
      </button>
    </div>
  );
}

/** Pick Conductor workspaces and import them, pointing at their existing worktrees. */
export function ConductorImportDialog({ onClose }: { onClose: () => void }) {
  const scan = useStore((s) => s.conductor);
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<Set<string> | null>(null);
  const [history, setHistory] = useState(true);
  const [busy, setBusy] = useState(false);
  const [skipped, setSkipped] = useState<{ key: string; reason: string }[]>([]);

  // Preselect what's active in Conductor and not imported yet.
  useEffect(() => {
    if (scan && picked === null) setPicked(new Set(scan.workspaces.filter((w) => !w.imported && !w.archivedInConductor).map((w) => w.key)));
  }, [scan, picked]);

  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && !busy && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [busy, onClose]);

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = (scan?.workspaces ?? []).filter((w) => !q || [w.title, w.name, w.branch, w.repoName].some((f) => f.toLowerCase().includes(q)));
    const by = new Map<string, ConductorWorkspace[]>();
    for (const w of list) by.set(w.repoPath, [...(by.get(w.repoPath) ?? []), w]);
    return [...by.entries()].sort((a, b) => a[1][0].repoName.localeCompare(b[1][0].repoName));
  }, [scan, query]);

  const sel = picked ?? new Set<string>();
  const toggle = (keys: string[], on: boolean) => {
    const next = new Set(sel);
    for (const k of keys) on ? next.add(k) : next.delete(k);
    setPicked(next);
  };
  const chosen = (scan?.workspaces ?? []).filter((w) => sel.has(w.key) && !w.imported);

  const run = async () => {
    if (!chosen.length || busy) return;
    setBusy(true);
    const report = await actions.importConductor(
      chosen.map((w) => w.key),
      history,
    );
    setBusy(false);
    if (!report) return;
    setSkipped(report.skipped);
    setPicked(new Set());
    if (!report.skipped.length) onClose();
  };

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/30 pt-16" onMouseDown={() => !busy && onClose()}>
      <div className="flex max-h-[80vh] w-[680px] flex-col overflow-hidden rounded-xl border border-border bg-elevated shadow-2xl" onMouseDown={(e) => e.stopPropagation()}>
        <div className="border-b border-border px-4 pt-3 pb-2.5">
          <div className="flex items-center gap-2">
            <span className="font-medium">Import from Conductor</span>
            {scan?.detected && <span className="text-xs text-muted">{tilde(scan.root)}</span>}
            <div className="flex-1" />
            <button onClick={onClose} disabled={busy} className="rounded p-1 text-muted hover:bg-hover hover:text-fg" title="Close (Esc)">
              <X size={14} />
            </button>
          </div>
          <div className="mt-1 text-xs text-muted">
            Each workspace keeps its existing worktree and branch; nothing is checked out again. Conductor's files and database are only read, never changed.
          </div>
        </div>
        {scan === null ? (
          <div className="flex items-center gap-2 px-4 py-6 text-muted">
            <Loader2 size={13} className="animate-spin" /> Looking for Conductor workspaces…
          </div>
        ) : !scan.detected ? (
          <div className="px-4 py-6 text-muted">Conductor wasn't found on this Mac (no ~/conductor folder or app data).</div>
        ) : (
          <>
            <div className="flex items-center gap-2 border-b border-border px-4 py-2">
              <Search size={13} className="text-muted" />
              <input
                autoFocus
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Filter by repository, title or branch"
                className="selectable flex-1 bg-transparent py-1 outline-none placeholder:text-faint"
              />
              <button
                onClick={() => toggle(scan.workspaces.filter((w) => !w.imported).map((w) => w.key), chosen.length === 0)}
                className="rounded px-2 py-0.5 text-xs text-muted hover:bg-hover hover:text-fg"
              >
                {chosen.length === 0 ? "Select all" : "Select none"}
              </button>
            </div>
            <div className="min-h-24 flex-1 overflow-y-auto p-1.5">
              {groups.length === 0 && <div className="px-3 py-3 text-muted">{query ? "Nothing matches." : "No Conductor workspaces with a worktree on disk."}</div>}
              {groups.map(([repoPath, list]) => {
                const open = list.filter((w) => !w.imported).map((w) => w.key);
                const all = open.length > 0 && open.every((k) => sel.has(k));
                return (
                  <section key={repoPath} className="mb-2">
                    <label className="flex items-center gap-2 px-2.5 pt-1.5 pb-1 text-[11px] font-semibold tracking-wide text-muted uppercase" title={repoPath}>
                      <input type="checkbox" checked={all} disabled={!open.length} onChange={(e) => toggle(open, e.target.checked)} />
                      {list[0].repoName}
                      <span className="font-normal tracking-normal normal-case text-faint">{tilde(repoPath)}</span>
                    </label>
                    {list.map((w) => (
                      <label
                        key={w.key}
                        title={w.path}
                        className={clsx("flex items-center gap-2.5 rounded-md px-2.5 py-1.5", w.imported ? "opacity-60" : "cursor-pointer hover:bg-hover")}
                      >
                        <input type="checkbox" checked={!!w.imported || sel.has(w.key)} disabled={!!w.imported} onChange={(e) => toggle([w.key], e.target.checked)} />
                        <div className="min-w-0 flex-1">
                          <div className="truncate text-[13px]">{w.title}</div>
                          <div className="flex items-center gap-1 truncate text-[11px] text-muted">
                            <GitBranch size={10} className="shrink-0" /> <span className="truncate">{w.branch}</span>
                            <span className="text-faint">· {w.name}</span>
                            {w.lastActivity && <span className="text-faint">· {ago(w.lastActivity)}</span>}
                          </div>
                        </div>
                        {w.chats > 0 && (
                          <span className="flex shrink-0 items-center gap-1 text-[11px] text-muted" title="Chats with history">
                            <MessageSquare size={11} /> {w.chats}
                          </span>
                        )}
                        {w.imported ? (
                          <span className="flex shrink-0 items-center gap-1 rounded border border-border px-1.5 text-[10px] text-muted">
                            <Check size={10} /> In Suneiro
                          </span>
                        ) : (
                          w.archivedInConductor && <span className="shrink-0 rounded border border-border px-1.5 text-[10px] text-muted">archived in Conductor</span>
                        )}
                      </label>
                    ))}
                  </section>
                );
              })}
              {scan.unavailable.length > 0 && (
                <details className="mx-2.5 mt-2 mb-1 text-[11px] text-muted">
                  <summary className="cursor-pointer">
                    {scan.unavailable.length} worktree{scan.unavailable.length === 1 ? "" : "s"} can't be imported
                  </summary>
                  {scan.unavailable.map((u) => (
                    <div key={u.path} className="selectable mt-1">
                      <span className="font-mono">{tilde(u.path)}</span>: {u.reason}
                    </div>
                  ))}
                </details>
              )}
            </div>
            {skipped.length > 0 && (
              <div className="max-h-24 overflow-y-auto border-t border-border px-4 py-2 text-xs text-muted">
                {skipped.map((s) => (
                  <div key={s.key} className="flex gap-1.5">
                    <CircleAlert size={12} className="mt-0.5 shrink-0 text-warn" />
                    <span className="selectable">
                      {tilde(s.key)}: {s.reason}
                    </span>
                  </div>
                ))}
              </div>
            )}
            <div className="flex items-center gap-3 border-t border-border px-4 py-2.5">
              <label className="flex items-center gap-2 text-xs" title="Read-only copies of the latest chats. Agents start a new conversation when you continue.">
                <input type="checkbox" checked={history} onChange={(e) => setHistory(e.target.checked)} /> Import chat history
              </label>
              {scan.dbError && <span className="truncate text-xs text-muted" title={scan.dbError}>Conductor's database couldn't be read; titles and chats are missing.</span>}
              <div className="flex-1" />
              <button
                onClick={() => void run()}
                disabled={busy || chosen.length === 0}
                className="flex items-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-xs font-medium text-accent-fg disabled:opacity-50"
              >
                {busy && <Loader2 size={12} className="animate-spin" />}
                {busy ? "Importing…" : `Import ${chosen.length || ""} workspace${chosen.length === 1 ? "" : "s"}`}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
