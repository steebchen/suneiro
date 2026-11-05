import { useEffect, useMemo, useState } from "react";
import clsx from "clsx";
import { api, type Usage, type Workspace } from "../lib/api";
import { actions, formatCost, formatTokens, totals, useStore, type Totals } from "../lib/store";
import { AGENT_NAMES, AgentIcon, findModel, modelName } from "../lib/models";

type Range = "7" | "30" | "all";
const DAY = 86_400_000;

function startOfDay(ts: number) {
  const d = new Date(ts);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

function groupBy(rows: Usage[], key: (u: Usage) => string) {
  const map = new Map<string, Usage[]>();
  for (const u of rows) {
    const k = key(u);
    if (!map.has(k)) map.set(k, []);
    map.get(k)!.push(u);
  }
  return [...map.entries()].map(([k, list]) => ({ key: k, rows: list, ...totals(list) })).sort((a, b) => b.cost - a.cost || b.tokens - a.tokens);
}

/** Totals over time: cost (reported by agents or estimated from tokens) and tokens. */
export function Insights() {
  const usage = useStore((s) => s.usage);
  const catalogs = useStore((s) => s.catalogs);
  const repos = useStore((s) => s.repos);
  const [range, setRange] = useState<Range>("30");
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);

  useEffect(() => {
    void actions.loadUsage();
    api.listAllWorkspaces().then(setWorkspaces).catch(() => {});
  }, []);

  const since = range === "all" ? 0 : startOfDay(Date.now()) - (Number(range) - 1) * DAY;
  const rows = useMemo(() => usage.filter((u) => u.ts >= since), [usage, since]);
  const all = totals(rows);
  const today = totals(usage.filter((u) => u.ts >= startOfDay(Date.now())));
  const sessions = new Set(rows.map((u) => u.sessionId)).size;

  const days = useMemo(() => {
    const first = range === "all" ? startOfDay(rows[0]?.ts ?? Date.now()) : since;
    const count = Math.max(1, Math.round((startOfDay(Date.now()) - first) / DAY) + 1);
    const buckets = Array.from({ length: Math.min(count, 120) }, (_, i) => ({ day: first + i * DAY, rows: [] as Usage[] }));
    for (const u of rows) {
      const i = Math.round((startOfDay(u.ts) - buckets[0].day) / DAY);
      buckets[i]?.rows.push(u);
    }
    return buckets.map((b) => ({ day: b.day, ...totals(b.rows) }));
  }, [rows, range, since]);

  const sessions_ = useStore((s) => s.sessions);
  const wsName = (id: string) => {
    const w = workspaces.find((x) => x.id === id);
    const chatTitle = (sessions_[id] ?? []).find((x) => x.title)?.title;
    return w ? w.title || chatTitle || w.name : "Deleted workspace";
  };
  const unpricedModels = [...new Set(rows.filter((u) => u.cost === null).map((u) => `${u.agent}:${u.model.split("[")[0]}`))];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-13 shrink-0 items-center gap-3 border-b border-border px-5" data-tauri-drag-region>
        <span className="font-medium">Insights</span>
        <div className="flex-1" data-tauri-drag-region />
        <div className="inline-flex rounded-md border border-border p-0.5">
          {(
            [
              ["7", "7 days"],
              ["30", "30 days"],
              ["all", "All time"],
            ] as [Range, string][]
          ).map(([r, label]) => (
            <button
              key={r}
              onClick={() => setRange(r)}
              className={clsx("rounded px-2.5 py-0.5 text-xs", range === r ? "bg-hover font-medium" : "text-muted hover:text-fg")}
            >
              {label}
            </button>
          ))}
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-4xl space-y-6 px-6 py-6">
          <div className="grid grid-cols-4 gap-3">
            <Card label="Total cost" value={formatCost(all.cost, all.estimated)} hint={costHint(all)} />
            <Card label="Today" value={formatCost(today.cost, today.estimated)} hint={`${formatTokens(today.tokens)} tokens`} />
            <Card label="Tokens" value={formatTokens(all.tokens)} hint={`${rows.length} turns`} />
            <Card label="Chats" value={String(sessions)} hint={sessions ? `${formatCost(all.cost / sessions)} per chat` : undefined} />
          </div>

          <Section title="Cost per day">
            <DayChart days={days} />
          </Section>

          <div className="grid grid-cols-2 gap-6">
            <Section title="By agent">
              <Table
                rows={groupBy(rows, (u) => u.agent).map((g) => ({
                  ...g,
                  label: (
                    <span className="flex items-center gap-1.5">
                      <AgentIcon agent={g.key} /> {AGENT_NAMES[g.key] ?? g.key}
                    </span>
                  ),
                }))}
              />
            </Section>
            <Section title="By model">
              <Table
                rows={groupBy(rows, (u) => `${u.agent}:${u.model.split("[")[0]}`).map((g) => {
                  const [agent, model] = g.key.split(/:(.*)/s);
                  return {
                    ...g,
                    label: (
                      <span className="flex items-center gap-1.5">
                        <AgentIcon agent={agent} /> {modelName(agent, findModel(catalogs, agent, model), model)}
                      </span>
                    ),
                  };
                })}
              />
            </Section>
          </div>

          <div className="grid grid-cols-2 gap-6">
            <Section title="By repository">
              <Table rows={groupBy(rows, (u) => u.repoId).map((g) => ({ ...g, label: repos.find((r) => r.id === g.key)?.name ?? "Removed repository" }))} />
            </Section>
            <Section title="Top workspaces">
              <Table rows={groupBy(rows, (u) => u.workspaceId).slice(0, 8).map((g) => ({ ...g, label: wsName(g.key) }))} />
            </Section>
          </div>

          {unpricedModels.length > 0 && (
            <div className="rounded-lg border border-border bg-elevated px-4 py-3 text-xs text-muted">
              Some models don't report cost (
              {unpricedModels
                .map((k) => {
                  const [agent, model] = k.split(/:(.*)/s);
                  return modelName(agent, findModel(catalogs, agent, model), model);
                })
                .join(", ")}
              ) and have no catalog price. Their tokens are counted but not included in costs.
            </div>
          )}
          <p className="text-[11px] text-faint">
            Costs are at API prices. Subscription plans (Claude Pro/Max, ChatGPT) aren't billed per token, so for them this measures how much
            work was done.
          </p>
        </div>
      </div>
    </div>
  );
}

function costHint(t: Totals) {
  if (t.unpriced && t.estimated) return "partly estimated · some unpriced";
  if (t.unpriced) return `${t.unpriced} turns unpriced`;
  if (t.estimated) return "partly estimated from tokens";
  return "as reported by agents";
}

function Card({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="rounded-lg border border-border bg-elevated px-4 py-3">
      <div className="text-xs text-muted">{label}</div>
      <div className="mt-1 text-xl font-semibold tabular-nums">{value}</div>
      {hint && <div className="mt-0.5 truncate text-[11px] text-faint">{hint}</div>}
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section>
      <h2 className="mb-2 text-xs font-semibold tracking-wide text-muted uppercase">{title}</h2>
      {children}
    </section>
  );
}

function Table({ rows }: { rows: ({ key: string; label: React.ReactNode } & Totals)[] }) {
  if (!rows.length) return <div className="rounded-lg border border-dashed border-border px-3 py-4 text-center text-xs text-muted">No usage yet.</div>;
  const max = Math.max(...rows.map((r) => r.cost), 1e-9);
  return (
    <div className="overflow-hidden rounded-lg border border-border">
      {rows.map((r) => (
        <div key={r.key} className="relative flex items-center gap-3 border-b border-border px-3 py-2 text-[13px] last:border-b-0">
          <div className="absolute inset-y-0 left-0 bg-accent/8" style={{ width: `${(r.cost / max) * 100}%` }} />
          <span className="relative min-w-0 flex-1 truncate">{r.label}</span>
          <span className="relative shrink-0 text-xs text-faint tabular-nums">{formatTokens(r.tokens)}</span>
          <span className="relative w-16 shrink-0 text-right tabular-nums">{r.cost || !r.unpriced ? formatCost(r.cost, r.estimated) : "—"}</span>
        </div>
      ))}
    </div>
  );
}

function DayChart({ days }: { days: ({ day: number } & Totals)[] }) {
  const max = Math.max(...days.map((d) => d.cost), 1e-9);
  const fmt = (ts: number) => new Date(ts).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return (
    <div className="rounded-lg border border-border bg-elevated px-3 pt-4 pb-2">
      <div className="flex h-36 items-end gap-[3px]">
        {days.map((d) => (
          <div key={d.day} className="group relative flex h-full flex-1 items-end" title={`${fmt(d.day)}: ${formatCost(d.cost, d.estimated)} · ${formatTokens(d.tokens)} tokens`}>
            <div
              className={clsx("w-full rounded-t-sm", d.cost ? "bg-accent/70 group-hover:bg-accent" : "bg-border")}
              style={{ height: d.cost ? `${Math.max(3, (d.cost / max) * 100)}%` : "2px" }}
            />
          </div>
        ))}
      </div>
      <div className="mt-1.5 flex justify-between text-[10px] text-faint">
        <span>{fmt(days[0]?.day ?? Date.now())}</span>
        <span>{fmt(days[days.length - 1]?.day ?? Date.now())}</span>
      </div>
    </div>
  );
}
