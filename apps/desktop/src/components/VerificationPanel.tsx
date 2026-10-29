import { useEffect, useState } from "react";
import { api, type CheckPreset, type IntegrationPreview, type Workspace } from "../lib/api";
import { actions, toast, useStore } from "../lib/store";

const presetIds = ["tests", "lint", "typecheck", "build"];
const button = "rounded border border-border px-2 py-1 hover:bg-hover disabled:opacity-40";
const input = "min-w-0 rounded border border-border bg-bg px-2 py-1 text-fg";

export function VerificationPanel({ workspace, visible }: { workspace: Workspace; visible: boolean }) {
  const runs = useStore((s) => s.verification[workspace.id]);
  const [presets, setPresets] = useState<CheckPreset[] | null>(null);
  const [saved, setSaved] = useState<CheckPreset[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [text, setText] = useState("");

  useEffect(() => {
    let active = true;
    api.verificationPresets(workspace.repoId).then((items) => {
      if (!active) return;
      const next = presetIds.map((id) => ({ id, command: items.find((p) => p.id === id)?.command ?? "" }));
      setPresets(next);
      setSaved(next);
    }).catch((e) => toast(String(e)));
    return () => { active = false; };
  }, [workspace.repoId]);

  useEffect(() => {
    if (!visible) return;
    let active = true;
    let pending = false;
    const refresh = async () => {
      if (!active || pending || document.visibilityState !== "visible") return;
      pending = true;
      const before = useStore.getState().verification[workspace.id];
      try {
        const next = await api.verificationStatus(workspace.id);
        // A streamed event that arrived during this request is more recent.
        if (active && before === useStore.getState().verification[workspace.id]) actions.setVerificationRuns(workspace.id, next);
      } catch (e) {
        if (active) toast(String(e));
      } finally { pending = false; }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 2000);
    document.addEventListener("visibilitychange", refresh);
    return () => { active = false; clearInterval(timer); document.removeEventListener("visibilitychange", refresh); };
  }, [workspace.id, visible]);

  const perform = async (key: string, fn: () => Promise<void>) => {
    setBusy(key);
    try { await fn(); } catch (e) { toast(String(e)); } finally { setBusy(null); }
  };
  const dirty = JSON.stringify(presets) !== JSON.stringify(saved);
  const failures = (runs ?? []).filter((run) => selected.includes(run.id) && run.status === "failed");

  return (
    <div className="h-full overflow-auto text-xs">
      <section className="space-y-2 border-b border-border p-3">
        <h3 className="font-medium">Local checks</h3>
        <p className="text-muted">Commands you configure in app settings for this repository. Never populated from agent output.</p>
        <p className="break-all font-mono text-muted">Working directory: {workspace.path}</p>
        {presets === null ? <p className="text-muted">Loading commands…</p> : presets.map((preset) => (
          <div key={preset.id} className="flex items-center gap-2">
            <label className="w-16 shrink-0 capitalize" htmlFor={`check-${workspace.id}-${preset.id}`}>{preset.id}</label>
            <input id={`check-${workspace.id}-${preset.id}`} className={`${input} flex-1 font-mono`} placeholder="Not configured" value={preset.command}
              onChange={(e) => setPresets(presets.map((p) => p.id === preset.id ? { ...p, command: e.target.value } : p))} />
            <button className={button} disabled={!!busy || dirty || !preset.command.trim() || runs?.some((r) => r.status === "running")}
              onClick={() => void perform(preset.id, async () => {
                await api.runCheck(workspace.id, preset.id);
                // Events own live results; polling recovers if an event was missed.
              })}>Run</button>
          </div>
        ))}
        <button className={button} disabled={!!busy || !presets || !dirty} onClick={() => void perform("save", async () => {
          const next = presets!.map((p) => ({ ...p, command: p.command.trim() }));
          await api.saveVerificationPresets(workspace.repoId, next.filter((p) => p.command));
          setSaved(next); setPresets(next);
        })}>{busy === "save" ? "Saving…" : "Save commands"}</button>
      </section>
      <section className="divide-y divide-border border-b border-border">
        {!runs?.length && <p className="p-3 text-muted">No local check results yet.</p>}
        {(runs ?? []).map((run) => (
          <div key={run.id} className="space-y-1 p-3">
            <div className="flex items-center gap-2">
              {run.status === "failed" && <input type="checkbox" aria-label={`Include ${run.presetId} failure in task`} checked={selected.includes(run.id)} onChange={(e) => setSelected((ids) => e.target.checked ? [...ids, run.id] : ids.filter((id) => id !== run.id))} />}
              <strong className="capitalize">{run.presetId}</strong>
              <span className={run.status === "failed" ? "text-del-fg" : run.status === "passed" ? "text-add-fg" : "text-muted"}>{run.status}</span>
              {run.stale && <span className="text-warn" title="Workspace content changed, including untracked files. Run again to verify current content.">Stale — content changed</span>}
              {run.exitCode !== null && <span className="text-muted">exit {run.exitCode}</span>}
              {run.status === "running" && <button className={`${button} ml-auto`} disabled={!!busy} onClick={() => void perform(run.id, () => api.cancelCheck(workspace.id, run.id))}>Cancel</button>}
            </div>
            <p className="break-all font-mono">{run.command}</p>
            <p className="break-all text-muted">cwd: {run.cwd}</p>
            <p className="break-all font-mono text-muted">HEAD: {run.revision.head} · content: {run.revision.fingerprint}</p>
            <details open={run.status === "failed"}><summary className="cursor-pointer text-muted">Log</summary>
              <pre className="selectable max-h-60 overflow-auto whitespace-pre-wrap break-all bg-bg p-2 font-mono">{run.log || "Waiting for output…"}</pre>
            </details>
          </div>
        ))}
      </section>
      <section className="space-y-2 border-b border-border p-3">
        <h3 className="font-medium">Create a follow-up task</h3>
        <p className="text-muted">Select failures above and add only the findings you want addressed. Nothing is sent automatically.</p>
        <textarea aria-label="Selected findings and task instructions" className={`${input} min-h-24 w-full`} placeholder="Paste selected reviewer findings or write a task…" value={text} onChange={(e) => setText(e.target.value)} />
        <button className={button} disabled={!!busy || (!text.trim() && !failures.length)} onClick={() => void perform("task", async () => {
          const body = [text.trim(), ...failures.map((run) => `Check failure: ${run.presetId}${run.stale ? " (stale result)" : ""}\nCommand: ${run.command}\ncwd: ${run.cwd}\nHEAD: ${run.revision.head}\nContent: ${run.revision.fingerprint}\n${run.log}`)].filter(Boolean).join("\n\n");
          const session = await api.verificationTask(workspace.id, body);
          await actions.reloadSessions(workspace.id, session);
          setText(""); setSelected([]);
        })}>Send {failures.length ? `${failures.length} selected failure(s) and findings` : "findings"} as new task</button>
      </section>
      <IntegrationSection key={workspace.id} workspace={workspace} />
    </div>
  );
}

function IntegrationSection({ workspace }: { workspace: Workspace }) {
  const workspaces = useStore((s) => s.workspaces);
  const [sourceId, setSourceId] = useState("");
  const [preview, setPreview] = useState<IntegrationPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const sources = workspaces.filter((w) => w.repoId === workspace.repoId && w.id !== workspace.id && !w.archivedAt);
  const validSource = sources.some((w) => w.id === sourceId);
  return (
    <section className="space-y-2 p-3">
      <h3 className="font-medium">Integrate branch · merge commit</h3>
      <p className="text-muted">Merge another workspace into this one with a merge commit (--no-ff). Both workspaces must be clean and idle, revisions unchanged, and conflicts resolved. Configure checks and pass every current source check before applying.</p>
      <div className="flex gap-2">
        <select aria-label="Source workspace" className={`${input} flex-1`} value={sourceId} disabled={busy} onChange={(e) => { setSourceId(e.target.value); setPreview(null); }}>
          <option value="">Choose source workspace…</option>
          {sources.map((w) => <option key={w.id} value={w.id}>{w.title || w.name} · {w.branch}</option>)}
        </select>
        <button className={button} disabled={busy || !validSource} onClick={async () => {
          setBusy(true); setPreview(null);
          try { setPreview(await api.integrationPreview(workspace.id, sourceId)); } catch (e) { toast(String(e)); } finally { setBusy(false); }
        }}>Preview</button>
      </div>
      {!sources.length && <p className="text-muted">No other workspaces in this repository.</p>}
      {preview && <>
        <p className="break-all font-mono text-muted">Target HEAD: {preview.target.head} · content: {preview.target.fingerprint}</p>
        <p className="break-all font-mono text-muted">Source HEAD: {preview.source.head} · content: {preview.source.fingerprint}</p>
        {!!preview.conflicts.length && <p className="whitespace-pre-wrap text-del-fg">Conflicts: {preview.conflicts.join("\n")}</p>}
        {!preview.canApply && <p className="text-warn">Cannot integrate. Both workspaces must be clean and idle, with no conflicts. Configure at least one check and pass all configured checks on current source content, then preview again.</p>}
        <pre className="selectable max-h-80 overflow-auto whitespace-pre-wrap break-all bg-bg p-2 font-mono">{preview.diff || "No diff."}</pre>
        <button className={button} disabled={busy || !validSource || !preview.canApply || !!preview.conflicts.length} onClick={async () => {
          setBusy(true);
          try { await api.integrateWorkspace(workspace.id, preview); toast("Workspace integrated with a merge commit", "info"); }
          catch (e) { toast(String(e)); }
          finally { setPreview(null); setBusy(false); }
        }}>Apply merge commit</button>
      </>}
    </section>
  );
}
