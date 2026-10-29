import { useEffect, useState } from "react";
import { Inbox as InboxIcon, ArrowUpRight, Gauge } from "lucide-react";
import {
  api,
  type InboxItem,
  type RepoControls,
  type ResourceUsage,
} from "../lib/api";
import { actions, toast, useStore, formatTokens } from "../lib/store";
import { dismissInbox, useSupervision } from "../lib/supervision";

export function InboxButton() {
  const count = useSupervision((s) => s.inbox.length);
  const selected = useStore((s) => s.page === "inbox");
  return (
    <button
      onClick={() => useStore.setState({ page: "inbox" })}
      className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 ${selected ? "bg-hover text-fg" : "text-muted hover:bg-hover hover:text-fg"}`}
    >
      <InboxIcon size={14} /> Inbox{" "}
      <span className="ml-auto text-xs tabular-nums">{count || ""}</span>
    </button>
  );
}

async function openConversation(item: InboxItem) {
  const s = useStore.getState();
  if (!s.workspaces.some((w) => w.id === item.workspaceId)) {
    await actions.restoreWorkspace(item.workspaceId);
    if (!useStore.getState().workspaces.some((w) => w.id === item.workspaceId))
      return;
  }
  actions.selectWorkspace(item.workspaceId);
  actions.selectSession(item.workspaceId, item.sessionId);
  await actions.loadHistory(item.sessionId);
}

function InboxRow({ item }: { item: InboxItem }) {
  const title = useStore(
    (s) =>
      s.sessions[item.workspaceId]?.find((x) => x.id === item.sessionId)?.title,
  );
  const workspace = useStore(
    (s) => s.workspaces.find((x) => x.id === item.workspaceId)?.name,
  );
  const archived = useStore(
    (s) => !s.workspaces.some((w) => w.id === item.workspaceId),
  );
  const actionable = item.kind === "question" || item.kind === "permission";
  return (
    <li className="flex items-start gap-3 border-b border-border py-3">
      <button
        className="min-w-0 flex-1 text-left hover:text-accent"
        onClick={() => void openConversation(item)}
      >
        <span className="flex items-center gap-2 text-xs text-muted">
          <span className="capitalize">{item.kind}</span>
          <span>·</span>
          <span>{title || workspace || "Conversation"}</span>
          <ArrowUpRight size={12} />
        </span>
        <span className="mt-1 block break-words">{item.message}</span>
        <span className="mt-1 block text-xs text-muted">
          {archived
            ? "Restore & open conversation"
            : actionable
              ? "Open conversation to respond"
              : "Open conversation"}{" "}
          · {new Date(item.createdAt).toLocaleString()}
        </span>
      </button>
      {!actionable && (
        <button
          className="shrink-0 rounded px-2 py-1 text-xs text-muted hover:bg-hover hover:text-fg"
          onClick={() =>
            void dismissInbox(item.id).catch((e) => toast(String(e)))
          }
        >
          Dismiss
        </button>
      )}
    </li>
  );
}

export function Inbox() {
  const items = useSupervision((s) => s.inbox);
  const repos = useStore((s) => s.repos);
  const focus = useSupervision((s) => s.resourceFocus);
  const [repoId, setRepoId] = useState(focus?.repoId ?? repos[0]?.id ?? "");
  const [filter, setFilter] = useState("all");
  const visible = items.filter((i) => filter === "all" || i.repoId === filter);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header
        className="flex h-13 shrink-0 items-center gap-2 border-b border-border px-5"
        data-tauri-drag-region
      >
        <InboxIcon size={16} />
        <h1 className="font-medium">Inbox & resources</h1>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-5">
        <div className="mx-auto max-w-4xl space-y-7">
          <section aria-label="Attention inbox">
            <div className="flex items-center justify-between gap-3">
              <h2 className="font-medium">
                Needs attention{" "}
                <span className="text-muted">{items.length}</span>
              </h2>
              <select
                aria-label="Filter inbox repository"
                className="rounded border border-border bg-panel px-2 py-1"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
              >
                <option value="all">All repositories</option>
                {repos.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.name}
                  </option>
                ))}
              </select>
            </div>
            {visible.length ? (
              <ul>
                {visible.map((item) => (
                  <InboxRow key={item.id} item={item} />
                ))}
              </ul>
            ) : (
              <p className="py-5 text-muted">
                No questions, permissions, failures or delegation reports to
                review.
              </p>
            )}
          </section>
          <section
            aria-label="Repository resource controls"
            className="border-t border-border pt-5"
          >
            <div className="mb-4 flex items-center justify-between gap-3">
              <h2 className="font-medium">Resources & scheduling</h2>
              <select
                aria-label="Resource repository"
                className="rounded border border-border bg-panel px-2 py-1"
                value={repoId}
                onChange={(e) => setRepoId(e.target.value)}
              >
                {repos.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.name}
                  </option>
                ))}
              </select>
            </div>
            {repoId && <ResourceControls key={repoId} repoId={repoId} />}
          </section>
        </div>
      </div>
    </div>
  );
}

function ResourceControls({ repoId }: { repoId: string }) {
  const [controls, setControls] = useState<RepoControls | null>(null);
  const current = useSupervision((s) => s.controls[repoId]);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    let live = true;
    void api
      .repoControls(repoId)
      .then((c) => {
        if (live) setControls(c);
      })
      .catch((e) => toast(String(e)));
    return () => {
      live = false;
    };
  }, [repoId]);
  useEffect(() => {
    if (current) setControls(current);
  }, [current]);
  const save = async () => {
    if (!controls || saving) return;
    setSaving(true);
    try {
      await api.saveRepoControls(repoId, controls);
      toast("Repository scheduling controls saved", "info");
    } catch (e) {
      toast(String(e));
    } finally {
      setSaving(false);
    }
  };
  if (!controls) return <p className="text-muted">Loading controls…</p>;
  return (
    <>
      <ResourceSummary repoId={repoId} />
      <form
        className="mt-4 space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={controls.paused}
            onChange={(e) =>
              setControls({ ...controls, paused: e.target.checked })
            }
          />{" "}
          Pause new turns in this repository
        </label>
        <div className="flex flex-wrap gap-4">
          <label className="flex flex-col gap-1 text-xs text-muted">
            Concurrent turns
            <input
              aria-label="Concurrent turns"
              className="w-32 rounded border border-border bg-panel px-2 py-1.5 text-fg"
              type="number"
              min="1"
              max="64"
              required
              value={controls.maxConcurrent}
              onChange={(e) =>
                setControls({
                  ...controls,
                  maxConcurrent: Number(e.target.value),
                })
              }
            />
          </label>
          <label className="flex flex-col gap-1 text-xs text-muted">
            Warn at observed USD
            <input
              aria-label="Spend warning USD"
              className="w-40 rounded border border-border bg-panel px-2 py-1.5 text-fg"
              type="number"
              min="0.01"
              step="0.01"
              placeholder="Off"
              value={controls.warningUsd ?? ""}
              onChange={(e) =>
                setControls({
                  ...controls,
                  warningUsd: e.target.value ? Number(e.target.value) : null,
                })
              }
            />
          </label>
          <label className="flex flex-col gap-1 text-xs text-muted">
            Stop new turns at USD
            <input
              aria-label="Spend stop USD"
              className="w-40 rounded border border-border bg-panel px-2 py-1.5 text-fg"
              type="number"
              min="0.01"
              step="0.01"
              placeholder="Off"
              value={controls.stopUsd ?? ""}
              onChange={(e) =>
                setControls({
                  ...controls,
                  stopUsd: e.target.value ? Number(e.target.value) : null,
                })
              }
            />
          </label>
        </div>
        <div className="flex flex-wrap items-center gap-3 text-xs text-muted">
          <span>
            Spend window:{" "}
            {controls.spendSince
              ? new Date(controls.spendSince).toLocaleString()
              : "all recorded turns"}
          </span>
          <button
            type="button"
            className="underline hover:text-fg"
            onClick={() => setControls({ ...controls, spendSince: Date.now() })}
          >
            Start window now
          </button>
          <button
            type="button"
            className="underline hover:text-fg"
            onClick={() => setControls({ ...controls, spendSince: 0 })}
          >
            All history
          </button>
        </div>
        <p className="max-w-2xl text-xs text-muted">
          Limits are enforced in core before a turn starts. Rejected starts are
          not queued. Pausing or reaching a spend threshold does not cancel
          running turns. Reported usage can arrive late; estimates and unknown
          costs mean these controls are not an exact dollar cap.
        </p>
        <button
          disabled={saving}
          className="rounded border border-border px-3 py-1.5 hover:bg-hover disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save controls"}
        </button>
      </form>
    </>
  );
}

function ResourceSummary({ repoId }: { repoId: string }) {
  const sessions = useStore((s) => s.sessions);
  const workspaces = useStore((s) => s.workspaces);
  const revision = useSupervision((s) => s.revision);
  const focus = useSupervision((s) => s.resourceFocus);
  const [root, setRoot] = useState(
    focus?.repoId === repoId ? focus.sessionId : "",
  );
  const [usage, setUsage] = useState<ResourceUsage | null>(null);
  const [error, setError] = useState("");
  const chats = workspaces
    .filter((w) => w.repoId === repoId)
    .flatMap((w) =>
      (sessions[w.id] ?? []).map((s) => ({
        ...s,
        label: `${w.name} · ${s.title || "Untitled chat"}`,
      })),
    );
  useEffect(() => {
    let live = true;
    const load = () =>
      (root ? api.subtreeResources(root) : api.repoResources(repoId))
        .then((u) => {
          if (live) {
            setUsage(u);
            setError("");
          }
        })
        .catch((e) => {
          if (live) setError(String(e));
        });
    void load();
    return () => {
      live = false;
    };
  }, [repoId, root, revision, sessions]);
  return (
    <div className="space-y-3">
      <label className="flex items-center gap-2 text-xs text-muted">
        Usage scope
        <select
          aria-label="Usage scope"
          className="max-w-full rounded border border-border bg-panel px-2 py-1 text-fg"
          value={root}
          onChange={(e) => {
            setUsage(null);
            setRoot(e.target.value);
          }}
        >
          <option value="">Repository · spend window</option>
          {chats.map((s) => (
            <option key={s.id} value={s.id}>
              {s.label} · subtree, all history
            </option>
          ))}
        </select>
      </label>
      {error && (
        <p role="alert" className="text-red-500">
          {error}
        </p>
      )}
      {usage && (
        <div className="flex flex-wrap gap-x-7 gap-y-2 text-xs">
          <span>
            {usage.running} running / {usage.sessions} chats · {usage.turns}{" "}
            turns
          </span>
          <span>
            {formatTokens(
              usage.inputTokens + usage.cachedTokens + usage.outputTokens,
            )}{" "}
            tokens
          </span>
          <span>Reported ${usage.reportedUsd.toFixed(2)}</span>
          <span>Estimated ~${usage.estimatedUsd.toFixed(2)}</span>
          <span>{usage.unknownTurns} turns with unknown cost</span>
        </div>
      )}
    </div>
  );
}
export function TaskResourcesButton({
  repoId,
  sessionId,
}: {
  repoId: string;
  sessionId: string;
}) {
  return (
    <button
      title="Show this chat's subtree usage and repository controls"
      className="shrink-0 rounded p-1.5 text-muted hover:bg-hover hover:text-fg"
      onClick={() => {
        useSupervision.setState({ resourceFocus: { repoId, sessionId } });
        useStore.setState({ page: "inbox" });
      }}
    >
      <Gauge size={14} />
    </button>
  );
}
