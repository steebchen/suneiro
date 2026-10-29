import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import clsx from "clsx";
import { open } from "@tauri-apps/plugin-dialog";
import { Check, CircleAlert, Loader2, RefreshCw, X } from "lucide-react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api, type AgentStatus, type Settings as SettingsT } from "../lib/api";
import { actions, useStore } from "../lib/store";
import { followTheme, xtermTheme } from "../lib/xtermTheme";
import { AgentIcon, findModel, modelName } from "../lib/models";
import { ModelSettings } from "./ModelSettings";
import { RepoSettings } from "./RepoSettings";

export function Settings() {
  const settings = useStore((s) => s.settings);
  useEffect(() => {
    void actions.detectAgents();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && actions.openSettings(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  if (!settings) return null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-13 shrink-0 items-center border-b border-border px-5" data-tauri-drag-region>
        <span className="font-medium">Settings</span>
        <div className="flex-1" data-tauri-drag-region />
        <button onClick={() => actions.openSettings(false)} className="rounded p-1.5 text-muted hover:bg-hover hover:text-fg" title="Close (Esc)">
          <X size={14} />
        </button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-6 py-8">
          <Section title="Agents" description="Suneiro drives the agent CLIs you already use, with your existing subscriptions. Nothing is proxied or stored by Suneiro.">
            <AgentSetup />
          </Section>
          <Section title="Default models">
            <ModelSettings />
            <Field label="Default to plan mode in new chats" hint="New chats start in plan mode instead of auto-accepting changes. Shift+Tab switches any time.">
              <Toggle checked={settings.planByDefault} onChange={(planByDefault) => actions.saveSettings({ planByDefault })} />
            </Field>
          </Section>
          <Section
            title="Pricing"
            description="Claude Code and OpenCode report their own costs. For models that only report tokens (Codex), enter prices to estimate costs in Insights. USD per million tokens."
          >
            <PricingTable />
          </Section>
          <Section title="Related chats">
            <Field label="Related chat levels" hint="Choose how many levels of related chats you can add. Lowering this keeps existing chats. Closing a chat keeps its related chats.">
              <select aria-label="Related chat levels" value={settings.maxTabDepth} onChange={(e) => void actions.saveSettings({ maxTabDepth: Number(e.target.value) })} className="rounded-md border border-border bg-bg px-2 py-1.5 text-xs">
                {Array.from({ length: 8 }, (_, i) => i + 1).map((depth) => <option key={depth} value={depth}>{depth}</option>)}
              </select>
            </Field>
          </Section>
          <Section title="Workspaces">
            <Field label="Workspaces folder" hint="New git worktrees are created here, grouped by repository.">
              <div className="flex gap-2">
                <input readOnly value={settings.workspacesRoot} className="selectable min-w-0 flex-1 rounded-md border border-border bg-bg px-2.5 py-1.5 font-mono text-xs" />
                <button
                  onClick={async () => {
                    const dir = await open({ directory: true, defaultPath: settings.workspacesRoot });
                    if (typeof dir === "string") void actions.saveSettings({ workspacesRoot: dir });
                  }}
                  className="rounded-md border border-border px-2.5 text-xs hover:bg-hover"
                >
                  Change…
                </button>
              </div>
            </Field>
            <Field label="Branch prefix" hint={`New branches are named like ${settings.branchPrefix}tokyo.`}>
              <TextInput value={settings.branchPrefix} onCommit={(v) => actions.saveSettings({ branchPrefix: v })} />
            </Field>
            <Field
              label="Name branches after their task"
              hint={`Once a task has a title, ${settings.branchPrefix}tokyo becomes e.g. ${settings.branchPrefix}fix-login-bug. Branches that were pushed are never renamed.`}
            >
              <Toggle checked={settings.renameBranches} onChange={(renameBranches) => actions.saveSettings({ renameBranches })} />
            </Field>
          </Section>
          <Section
            title="Repositories"
            description="Scripts that set up, run and clean up workspaces, and files to copy into them. A suneiro.json (or conductor.json) committed to the repository takes precedence."
          >
            <RepoSettings />
          </Section>
          <Section title="Appearance & tools">
            <Field label="Theme">
              <Segmented<SettingsT["theme"]>
                value={settings.theme}
                options={[
                  ["system", "System"],
                  ["light", "Light"],
                  ["dark", "Dark"],
                ]}
                onChange={(theme) => actions.saveSettings({ theme })}
              />
            </Field>
            <Field label="Editor" hint="Used by “Open in editor” (⌘O) in a workspace.">
              <select
                value={settings.editor}
                onChange={(e) => actions.saveSettings({ editor: e.target.value })}
                className="rounded-md border border-border bg-bg px-2 py-1.5 text-xs"
              >
                {["Visual Studio Code", "Cursor", "Zed", "Xcode", "Sublime Text", "WebStorm"].map((e) => (
                  <option key={e}>{e}</option>
                ))}
              </select>
            </Field>
          </Section>
        </div>
      </div>
    </div>
  );
}

/** Agent cards: detection, enable/default, and install/login flows. Also used on the welcome screen. */
export function AgentSetup({ compact = false }: { compact?: boolean }) {
  const statuses = useStore((s) => s.agentStatus);
  const settings = useStore((s) => s.settings);
  const [running, setRunning] = useState<{ agentId: string; title: string; command: string } | null>(null);
  const [checking, setChecking] = useState(false);

  const recheck = async () => {
    setChecking(true);
    await actions.detectAgents();
    setChecking(false);
  };

  if (!statuses || !settings) {
    return (
      <div className="flex items-center gap-2 py-4 text-muted">
        <Loader2 size={13} className="animate-spin" /> Detecting installed agents…
      </div>
    );
  }

  const toggle = (id: string, on: boolean) => {
    const enabled = on ? [...new Set([...settings.enabledAgents, id])] : settings.enabledAgents.filter((a) => a !== id);
    void actions.saveSettings({ enabledAgents: enabled });
    if (on) void actions.ensureCatalogs();
  };

  return (
    <div className="space-y-2">
      {statuses.map((a) => (
        <div key={a.id} className="rounded-lg border border-border bg-elevated px-3.5 py-3">
          <div className="flex items-center gap-3">
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <AgentIcon agent={a.id} size={14} />
                <span className="font-medium">{a.name}</span>
                {a.version && <span className="text-xs text-faint">v{a.version}</span>}
              </div>
              <StatusLine status={a} />
            </div>
            <AgentActions status={a} onRun={(title, command) => setRunning({ agentId: a.id, title, command })} />
            {!compact && (
              <>
                <Toggle checked={settings.enabledAgents.includes(a.id)} onChange={(on) => toggle(a.id, on)} />
              </>
            )}
          </div>
          {running?.agentId === a.id && (
            <SetupTerminal
              key={running.command}
              title={running.title}
              command={running.command}
              onClose={() => {
                setRunning(null);
                void recheck();
              }}
            />
          )}
        </div>
      ))}
      <button onClick={recheck} className="flex items-center gap-1.5 rounded px-1 py-1 text-xs text-muted hover:text-fg">
        <RefreshCw size={11} className={clsx(checking && "animate-spin")} /> Re-check
      </button>
    </div>
  );
}

function StatusLine({ status: a }: { status: AgentStatus }) {
  if (!a.installed)
    return (
      <div className="mt-0.5 flex items-center gap-1 text-xs text-muted">
        <CircleAlert size={11} className="text-faint" /> Not installed
      </div>
    );
  if (!a.loggedIn)
    return (
      <div className="mt-0.5 flex items-center gap-1 text-xs text-warn">
        <CircleAlert size={11} /> Installed · not signed in
      </div>
    );
  return (
    <div className="mt-0.5 flex items-center gap-1 text-xs text-muted">
      <Check size={11} className="text-add-fg" /> Signed in{a.account ? ` · ${a.account}` : ""}
    </div>
  );
}

function AgentActions({ status: a, onRun }: { status: AgentStatus; onRun: (title: string, command: string) => void }) {
  const btn = "rounded-md px-2.5 py-1 text-xs font-medium";
  if (!a.installed)
    return (
      <button onClick={() => onRun(`Installing ${a.name}`, a.installCommand)} className={clsx(btn, "bg-accent text-accent-fg")} title={a.installCommand}>
        Install
      </button>
    );
  if (!a.loggedIn)
    return (
      <button onClick={() => onRun(`Signing in to ${a.name}`, a.loginCommand)} className={clsx(btn, "bg-accent text-accent-fg")}>
        Sign in
      </button>
    );
  return (
    <button onClick={() => onRun(`Signing out of ${a.name}`, a.logoutCommand)} className={clsx(btn, "text-muted hover:bg-hover hover:text-fg")}>
      Sign out
    </button>
  );
}

/** Runs an install/login command in a real pty so interactive prompts and browser OAuth work. */
function SetupTerminal({ title, command, onClose }: { title: string; command: string; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [done, setDone] = useState(false);

  useEffect(() => {
    const term = new Terminal({
      fontFamily: '"SF Mono", ui-monospace, Menlo, monospace',
      fontSize: 11.5,
      rows: 14,
      convertEol: false,
      theme: xtermTheme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    const unfollow = followTheme(term);
    term.open(ref.current!);
    fit.fit();
    const id = `setup:${crypto.randomUUID()}`;
    term.onData((d) => void api.terminalWrite(id, d).catch(() => {}));
    term.write(`\x1b[2m$ ${command}\x1b[0m\r\n`);
    api
      .setupTerminalOpen(id, term.cols, term.rows, command, (d) => term.write(d), () => {
        term.write("\r\n\x1b[2m[finished]\x1b[0m\r\n");
        setDone(true);
      })
      .catch((e) => term.write(`\x1b[31m${e}\x1b[0m\r\n`));
    term.focus();
    return () => {
      void api.terminalKill(id).catch(() => {});
      unfollow();
      term.dispose();
    };
  }, [command]);

  return (
    <div className="mt-3 overflow-hidden rounded-md border border-border">
      <div className="flex items-center gap-2 border-b border-border bg-panel px-2.5 py-1.5 text-xs">
        {!done && <Loader2 size={11} className="animate-spin text-accent" />}
        <span className="flex-1">{title}</span>
        <button onClick={onClose} className="rounded px-2 py-0.5 hover:bg-hover">
          {done ? "Done" : "Cancel"}
        </button>
      </div>
      <div ref={ref} className="h-56 bg-bg" />
    </div>
  );
}

/** Per-model prices for agents that report tokens but not cost. */
function PricingTable() {
  const pricing = useStore((s) => s.pricing);
  const catalogs = useStore((s) => s.catalogs);
  const usage = useStore((s) => s.usage);
  const models = useMemo(() => {
    const set = new Map<string, string>();
    for (const m of catalogs.codex?.models ?? []) set.set(m.value.split("[")[0], "codex");
    for (const u of usage) if (u.costUsd === null) set.set(u.model.split("[")[0], u.agent);
    for (const k of Object.keys(pricing)) if (!set.has(k)) set.set(k, "codex");
    return [...set.entries()];
  }, [catalogs, usage, pricing]);
  if (!models.length) return <div className="text-xs text-muted">No models need prices yet.</div>;
  const update = (model: string, field: "input" | "cachedInput" | "output", value: string) => {
    const cur = pricing[model] ?? { input: 0, cachedInput: 0, output: 0 };
    const next = { ...pricing, [model]: { ...cur, [field]: Number(value) || 0 } };
    if (!next[model].input && !next[model].cachedInput && !next[model].output) delete next[model];
    void actions.savePricing(next);
  };
  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <div className="grid grid-cols-[1fr_repeat(3,90px)] gap-2 border-b border-border bg-panel px-3 py-1.5 text-[11px] text-muted">
        <span>Model</span>
        <span>Input</span>
        <span>Cached input</span>
        <span>Output</span>
      </div>
      {models.map(([model, agent]) => (
        <div key={model} className="grid grid-cols-[1fr_repeat(3,90px)] items-center gap-2 border-b border-border px-3 py-1.5 last:border-b-0">
          <span className="flex min-w-0 items-center gap-1.5 truncate text-xs">
            <AgentIcon agent={agent} size={12} />
            {modelName(agent, findModel(catalogs, agent, model), model)}
          </span>
          {(["input", "cachedInput", "output"] as const).map((field) => (
            <PriceInput key={field} value={pricing[model]?.[field]} onCommit={(v) => update(model, field, v)} />
          ))}
        </div>
      ))}
    </div>
  );
}

function PriceInput({ value, onCommit }: { value?: number; onCommit: (v: string) => void }) {
  const [v, setV] = useState(value ? String(value) : "");
  useEffect(() => setV(value ? String(value) : ""), [value]);
  return (
    <input
      value={v}
      inputMode="decimal"
      placeholder="—"
      onChange={(e) => setV(e.target.value.replace(/[^0-9.]/g, ""))}
      onBlur={() => v !== (value ? String(value) : "") && onCommit(v)}
      onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
      className="selectable w-full rounded border border-border bg-bg px-1.5 py-1 text-right text-xs tabular-nums outline-none focus:border-accent"
    />
  );
}

function Section({ title, description, children }: { title: string; description?: string; children: ReactNode }) {
  return (
    <section className="mb-10">
      <h2 className="text-[15px] font-semibold">{title}</h2>
      {description && <p className="mt-1 mb-4 text-xs text-muted">{description}</p>}
      <div className={clsx(!description && "mt-4", "space-y-5")}>{children}</div>
    </section>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div>
      <div className="mb-1.5 text-xs font-medium">{label}</div>
      {children}
      {hint && <div className="mt-1 text-[11px] text-muted">{hint}</div>}
    </div>
  );
}

function TextInput({ value, onCommit }: { value: string; onCommit: (v: string) => void }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <input
      value={v}
      onChange={(e) => setV(e.target.value)}
      onBlur={() => v !== value && onCommit(v)}
      onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
      className="selectable w-64 rounded-md border border-border bg-bg px-2.5 py-1.5 font-mono text-xs outline-none focus:border-accent"
    />
  );
}

function Segmented<T extends string>({ value, options, onChange }: { value: T; options: [T, string][]; onChange: (v: T) => void }) {
  return (
    <div className="inline-flex rounded-md border border-border p-0.5">
      {options.map(([v, label]) => (
        <button
          key={v}
          onClick={() => onChange(v)}
          className={clsx("rounded px-3 py-1 text-xs", value === v ? "bg-hover font-medium" : "text-muted hover:text-fg")}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={clsx("relative h-5 w-9 shrink-0 rounded-full transition-colors", checked ? "bg-accent" : "bg-border")}
    >
      <span className={clsx("absolute top-0.5 left-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform", checked && "translate-x-4")} />
    </button>
  );
}
