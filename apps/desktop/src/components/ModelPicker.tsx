import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import { ArrowUpRight, Check, ChevronDown, ChevronRight, GripVertical, Search, Settings as SettingsIcon, Zap } from "lucide-react";
import { api, type ConfigOption, type LoadoutEntry } from "../lib/api";
import { actions, useStore } from "../lib/store";
import {
  AGENT_NAMES,
  AgentIcon,
  effortName,
  effortOption,
  fastOption,
  findModel,
  flatOptions,
  modelName,
  modelOption,
  sameEntry,
  searchableModels,
} from "../lib/models";
import { useReorder } from "../lib/reorder";

type Props = { sessionId: string; workspaceId: string; agentId: string; config: ConfigOption[] };

/** Apply a model (+effort) to this chat, or open a new chat when it's another agent. */
async function choose(props: Props, entry: LoadoutEntry) {
  const { sessionId, workspaceId, agentId, config } = props;
  if (entry.agent === agentId) {
    const m = modelOption(config);
    if (m && m.currentValue !== entry.model) await actions.setConfig(sessionId, m.id, entry.model);
    const e = effortOption(useStore.getState().views[sessionId]?.config ?? config);
    if (e && entry.effort && e.currentValue !== entry.effort) await actions.setConfig(sessionId, e.id, entry.effort);
    return;
  }
  const s = useStore.getState();
  const session = s.sessions[workspaceId]?.find((x) => x.id === sessionId);
  const view = s.views[sessionId];
  const hasChildren = Object.values(s.sessions).some((list) => list.some((x) => x.parentSessionId === sessionId));
  const empty = !!view?.loaded && !view.transcript.items.length && !hasChildren;
  const created = await actions.createSession(workspaceId, entry.agent, entry.model, entry.effort, session?.parentSessionId);
  // Keep parents with children even when their own transcript is empty.
  if (created && empty) await actions.closeSession(workspaceId, sessionId);
}

export function cycleEffort(sessionId: string, config: ConfigOption[]) {
  const e = effortOption(config);
  if (!e) return;
  const values = flatOptions(e).map((o) => o.value);
  const next = values[(values.indexOf(String(e.currentValue)) + 1) % values.length];
  if (next) void actions.setConfig(sessionId, e.id, next);
}

export function toggleFast(sessionId: string, config: ConfigOption[]) {
  const f = fastOption(config);
  if (!f) return;
  const on = f.type === "boolean" ? !!f.currentValue : f.currentValue === "on";
  void actions.setConfig(sessionId, f.id, f.type === "boolean" ? !on : on ? "off" : "on");
}

export function ModelPicker(props: Props) {
  const { agentId, config } = props;
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const catalogs = useStore((s) => s.catalogs);
  const session = useStore((s) => (s.sessions[props.workspaceId] ?? []).find((x) => x.id === props.sessionId));
  const m = modelOption(config);
  const e = effortOption(config);
  // Live config when the agent is running, otherwise the chat's last known model.
  const modelValue = m ? String(m.currentValue) : session?.model;
  const label = modelValue
    ? modelName(agentId, flatOptions(m).find((o) => o.value === modelValue) ?? findModel(catalogs, agentId, modelValue), modelValue)
    : (AGENT_NAMES[agentId] ?? agentId);
  const effortValue = e ? String(e.currentValue) : session?.effort;
  const effort = effortValue
    ? (flatOptions(e).find((o) => o.value === effortValue)?.name ?? effortName(catalogs, agentId, effortValue))
    : null;

  return (
    <>
      <button
        ref={triggerRef}
        onClick={() => {
          // Start a dormant chat's agent so effort/fast options show up.
          if (!open && !m) void api.connectSession(props.sessionId).catch(() => {});
          setOpen((o) => !o);
        }}
        className={clsx("flex shrink-0 items-center gap-1.5 rounded px-1.5 py-1 text-xs hover:bg-hover", open ? "bg-hover text-fg" : "text-fg/80")}
        title="Model and effort"
      >
        <AgentIcon agent={agentId} size={12} />
        <span className="font-medium">{label}</span>
        {effort && <span className="text-muted">{effort}</span>}
        <ChevronDown size={11} className="text-faint" />
      </button>
      {open && triggerRef.current && <Popover anchor={triggerRef.current} onClose={() => setOpen(false)} {...props} />}
    </>
  );
}

function Popover({ anchor, onClose, ...props }: Props & { anchor: HTMLElement; onClose: () => void }) {
  const { sessionId, agentId, config } = props;
  const settings = useStore((s) => s.settings)!;
  const catalogs = useStore((s) => s.catalogs);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [effortOpen, setEffortOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; bottom: number }>({ left: 0, bottom: 0 });

  useLayoutEffect(() => {
    const r = anchor.getBoundingClientRect();
    setPos({ left: Math.max(8, Math.min(r.left - 8, window.innerWidth - 400)), bottom: window.innerHeight - r.top + 6 });
  }, [anchor]);

  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node) && !anchor.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [anchor, onClose]);

  const currentModel = String(modelOption(config)?.currentValue ?? "");
  const loadout = settings.loadout.filter((l) => settings.enabledAgents.includes(l.agent));
  const q = query.trim().toLowerCase();
  const rows: LoadoutEntry[] = useMemo(() => {
    if (!q) return loadout;
    return searchableModels(catalogs, settings)
      .filter(({ agent, model }) =>
        [modelName(agent, model), model.value, AGENT_NAMES[agent] ?? agent].some((f) => f.toLowerCase().includes(q)),
      )
      .slice(0, 50)
      .map(({ agent, model }) => loadout.find((l) => sameEntry(l, { agent, model: model.value })) ?? { agent, model: model.value });
  }, [q, catalogs, settings, loadout]);

  useEffect(() => setActive(0), [q]);

  const { itemProps, overIndex, dragIndex } = useReorder(settings.loadout, (loadout) => actions.saveSettings({ loadout }));

  const pick = (entry: LoadoutEntry) => {
    onClose();
    void choose(props, entry);
  };

  const e = effortOption(config);
  const f = fastOption(config);
  const fastOn = f ? (f.type === "boolean" ? !!f.currentValue : f.currentValue === "on") : false;

  return createPortal(
    <div
      ref={ref}
      style={{ left: pos.left, bottom: pos.bottom }}
      className="fixed z-50 w-[380px] overflow-hidden rounded-xl border border-border bg-elevated shadow-2xl"
      onKeyDown={(ev) => {
        if (ev.key === "Escape") onClose();
        else if (ev.key === "ArrowDown") {
          ev.preventDefault();
          setActive((a) => Math.min(rows.length - 1, a + 1));
        } else if (ev.key === "ArrowUp") {
          ev.preventDefault();
          setActive((a) => Math.max(0, a - 1));
        } else if (ev.key === "Enter" && rows[active]) {
          ev.preventDefault();
          pick(rows[active]);
        }
      }}
    >
      <div className="flex items-center gap-2 border-b border-border px-3 py-2.5">
        <Search size={14} className="text-faint" />
        <input
          autoFocus
          value={query}
          onChange={(ev) => setQuery(ev.target.value)}
          placeholder="Search models"
          className="selectable flex-1 bg-transparent text-[13px] outline-none placeholder:text-faint"
        />
      </div>

      <div className="max-h-80 overflow-y-auto p-1.5">
        {rows.length === 0 && (
          <div className="px-3 py-4 text-center text-xs text-muted">
            {q ? "No matching models." : "No featured models yet. Search, or add some under Edit."}
          </div>
        )}
        {rows.map((entry, i) => {
          const isCurrent = entry.agent === agentId && entry.model === currentModel;
          const otherAgent = entry.agent !== agentId;
          const index = settings.loadout.findIndex((l) => sameEntry(l, entry));
          const draggable = !q && index >= 0;
          return (
            <div
              key={`${entry.agent}:${entry.model}`}
              {...(draggable ? itemProps(index) : {})}
              onMouseEnter={() => setActive(i)}
              onClick={() => pick(entry)}
              className={clsx(
                "group flex items-center gap-2 rounded-lg px-2 py-2 text-[13px]",
                i === active && "bg-hover",
                draggable && overIndex === index && "ring-1 ring-accent",
                draggable && dragIndex === index && "opacity-50",
                draggable && "select-none",
              )}
            >
              {!q && <GripVertical size={13} className="shrink-0 cursor-grab text-faint" />}
              <AgentIcon agent={entry.agent} />
              <span className="truncate font-medium">{modelName(entry.agent, findModel(catalogs, entry.agent, entry.model), entry.model)}</span>
              {entry.effort ? (
                <span className="shrink-0 text-muted">{effortName(catalogs, entry.agent, entry.effort)}</span>
              ) : (
                q && <span className="shrink-0 text-[11px] text-faint">{AGENT_NAMES[entry.agent]}</span>
              )}
              <span className="flex-1" />
              {isCurrent ? (
                <Check size={14} className="shrink-0 text-fg" />
              ) : otherAgent ? (
                <span title="Opens a new chat with this agent">
                  <ArrowUpRight size={14} className="shrink-0 text-faint" />
                </span>
              ) : null}
            </div>
          );
        })}
      </div>

      {(e || f) && (
        <div className="border-t border-border p-1.5">
          {e && (
            <>
              <button onClick={() => setEffortOpen((o) => !o)} className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-[13px] hover:bg-hover">
                Effort
                <span className="flex-1" />
                <span className="text-muted">{flatOptions(e).find((o) => o.value === e.currentValue)?.name}</span>
                <ChevronRight size={13} className={clsx("text-faint transition-transform", effortOpen && "rotate-90")} />
              </button>
              {effortOpen && (
                <div className="mb-1 ml-3 flex flex-wrap gap-1 px-2 pb-1">
                  {flatOptions(e).map((o) => (
                    <button
                      key={o.value}
                      onClick={() => actions.setConfig(sessionId, e.id, o.value)}
                      className={clsx(
                        "rounded-md px-2 py-1 text-xs",
                        o.value === e.currentValue ? "bg-accent text-accent-fg" : "border border-border hover:bg-hover",
                      )}
                    >
                      {o.name}
                    </button>
                  ))}
                </div>
              )}
            </>
          )}
          {f && (
            <button onClick={() => toggleFast(sessionId, config)} className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-[13px] hover:bg-hover" title={f.description ?? undefined}>
              <Zap size={13} className={fastOn ? "text-warn" : "text-faint"} fill={fastOn ? "currentColor" : "none"} />
              Fast
              <span className="flex-1" />
              <kbd className="font-sans text-[11px] text-faint">⌘⇧E</kbd>
              <span className={clsx("relative h-5 w-9 rounded-full transition-colors", fastOn ? "bg-accent" : "bg-border")}>
                <span className={clsx("absolute top-0.5 left-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform", fastOn && "translate-x-4")} />
              </span>
            </button>
          )}
        </div>
      )}

      <div className="flex items-center gap-2 border-t border-border bg-panel px-3 py-2 text-xs text-muted">
        <button
          onClick={() => {
            onClose();
            actions.openSettings(true);
          }}
          className="flex items-center gap-1.5 hover:text-fg"
        >
          <SettingsIcon size={12} /> Edit
        </button>
        <span className="flex-1" />
        {e && (
          <>
            <kbd className="rounded border border-border px-1 font-sans text-[11px]">⌘⇧/</kbd> Cycle effort
          </>
        )}
      </div>
    </div>,
    document.body,
  );
}
