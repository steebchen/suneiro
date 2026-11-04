import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import { ArrowDownToLine, Folder, FolderOpen, Globe, Loader2 } from "lucide-react";
import { actions, useStore } from "../lib/store";
import { pickRepo } from "../App";
import { useConductorPending } from "./ConductorImport";

const tilde = (p: string) => p.replace(/^\/Users\/[^/]+/, "~");

/**
 * "Add repository": open a folder, clone from GitHub, or pick one of the
 * repos you recently used with Claude Code / Codex.
 */
export function AddRepoMenu({ trigger, align = "left" }: { trigger: (open: () => void) => ReactNode; align?: "left" | "right" | "center" }) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);
  return (
    <>
      <span ref={anchor} className="inline-flex">
        {trigger(() => {
          setOpen(true);
          void actions.loadRecents();
        })}
      </span>
      {open && anchor.current && <Popover anchor={anchor.current} align={align} onClose={() => setOpen(false)} />}
    </>
  );
}

function Popover({ anchor, align, onClose }: { anchor: HTMLElement; align: "left" | "right" | "center"; onClose: () => void }) {
  const recents = useStore((s) => s.recents);
  const conductor = useStore((s) => !!s.conductor?.detected);
  const pending = useConductorPending();
  const [cloning, setCloning] = useState(false);
  const [spec, setSpec] = useState("");
  const [busy, setBusy] = useState(false);
  const [active, setActive] = useState(-1);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: 0, top: 0 });

  useLayoutEffect(() => {
    const r = anchor.getBoundingClientRect();
    const width = 340;
    const left = align === "right" ? r.right - width : align === "center" ? r.left + r.width / 2 - width / 2 : r.left;
    setPos({ left: Math.max(8, Math.min(left, window.innerWidth - width - 8)), top: r.bottom + 6 });
  }, [anchor, align]);

  useEffect(() => {
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && onClose();
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", key);
    };
  }, [onClose]);

  const add = (path: string) => {
    onClose();
    void actions.addRepo(path);
  };

  const clone = async () => {
    if (!spec.trim()) return;
    setBusy(true);
    const ok = await actions.cloneRepo(spec.trim());
    setBusy(false);
    if (ok) onClose();
  };

  const row = "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13px] hover:bg-hover";

  return createPortal(
    <div
      ref={ref}
      style={pos}
      className="fixed z-50 w-[340px] rounded-xl border border-border bg-elevated p-1.5 shadow-2xl"
      onKeyDown={(e) => {
        if (cloning) return;
        if (e.key === "ArrowDown") setActive((a) => Math.min(recents.length - 1, a + 1));
        else if (e.key === "ArrowUp") setActive((a) => Math.max(0, a - 1));
        else if (e.key === "Enter" && recents[active]) add(recents[active].path);
      }}
      tabIndex={-1}
    >
      <button
        className={row}
        onClick={() => {
          onClose();
          void pickRepo();
        }}
      >
        <FolderOpen size={15} className="text-muted" /> Open project…
      </button>
      {cloning ? (
        <div className="flex items-center gap-2 px-2.5 py-1.5">
          <Globe size={15} className="shrink-0 text-muted" />
          <input
            autoFocus
            value={spec}
            onChange={(e) => setSpec(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void clone();
              if (e.key === "Escape") setCloning(false);
            }}
            placeholder="owner/repo or URL"
            disabled={busy}
            className="selectable min-w-0 flex-1 rounded-md border border-border bg-bg px-2 py-1 text-[13px] outline-none focus:border-accent"
          />
          <button
            onClick={() => void clone()}
            disabled={busy || !spec.trim()}
            className="flex items-center gap-1 rounded-md bg-accent px-2.5 py-1 text-xs font-medium text-accent-fg disabled:opacity-50"
          >
            {busy && <Loader2 size={11} className="animate-spin" />}
            {busy ? "Cloning" : "Clone"}
          </button>
        </div>
      ) : (
        <button className={row} onClick={() => setCloning(true)}>
          <Globe size={15} className="text-muted" /> Clone from GitHub…
        </button>
      )}
      {conductor && (
        <button
          className={row}
          onClick={() => {
            onClose();
            actions.openConductorImport();
          }}
        >
          <ArrowDownToLine size={15} className="text-muted" /> Import from Conductor…
          {pending > 0 && <span className="ml-auto text-[11px] text-muted">{pending}</span>}
        </button>
      )}
      {recents.length > 0 && (
        <>
          <div className="mx-1 my-1.5 h-px bg-border" />
          <div className="px-2.5 pt-0.5 pb-1 text-[11px] text-muted" title="Repositories you recently used with Claude Code or Codex">
            Recents
          </div>
          {recents.map((r, i) => (
            <button
              key={r.path}
              onClick={() => add(r.path)}
              onMouseEnter={() => setActive(i)}
              className={clsx(row, "py-1.5 text-xs", i === active && "bg-hover")}
              title={r.path}
            >
              <Folder size={14} className="shrink-0 text-faint" />
              <span className="truncate">{tilde(r.path)}</span>
            </button>
          ))}
        </>
      )}
    </div>,
    document.body,
  );
}
