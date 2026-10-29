import { useEffect, useRef, useState } from "react";
import { Plus, X } from "lucide-react";
import { actions, useStore } from "../lib/store";
import { profilesApi, type AgentProfile, type ContextMessage, type ContextPackage } from "../lib/profiles";

const input = "w-full rounded border border-border bg-bg px-2 py-1.5 text-sm text-fg focus:outline-2 focus:outline-accent";
const button = "rounded border border-border px-3 py-1.5 text-sm hover:bg-hover disabled:opacity-50";
const paths = (text: string) => [...new Set(text.split("\n").map((line) => line.trim()).filter(Boolean))];

export function ProfileLauncher({ workspaceId, sessionId }: { workspaceId: string; sessionId?: string }) {
  const [open, setOpen] = useState(false);
  return <>
    <button className="shrink-0 rounded px-2 py-1 text-xs text-muted hover:bg-hover hover:text-fg" onClick={() => setOpen(true)}>Profiles / handoff</button>
    {open && <ProfileDialog workspaceId={workspaceId} sessionId={sessionId} onClose={() => setOpen(false)} />}
  </>;
}

export function ProfileDialog({ workspaceId, sessionId, onClose }: { workspaceId: string; sessionId?: string; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const sessions = useStore((s) => s.sessions[workspaceId]);
  const agents = useStore((s) => s.agents);
  const catalogs = useStore((s) => s.catalogs);
  const enabled = useStore((s) => s.settings?.enabledAgents);
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [profileId, setProfileId] = useState("");
  const [agentId, setAgentId] = useState(() => useStore.getState().settings?.defaultAgent ?? "claude");
  const [source, setSource] = useState(sessionId ?? "");
  const [messages, setMessages] = useState<ContextMessage[]>([]);
  const [selected, setSelected] = useState<{ sessionId: string; index: number }[]>([]);
  const [task, setTask] = useState("");
  const [criteria, setCriteria] = useState("");
  const [files, setFiles] = useState("");
  const [diffs, setDiffs] = useState("");
  const [dirty, setDirty] = useState(false);
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [preview, setPreview] = useState<ContextPackage | null>(null);
  const profile = profiles.find((p) => p.id === profileId);
  const preference = profile?.preferences[agentId] ?? { model: null, effort: null };
  const availableAgents = agents.filter((a) => !enabled?.length || enabled.includes(a.id));

  useEffect(() => {
    const node = dialog.current;
    const previous = document.activeElement as HTMLElement | null;
    node?.showModal();
    node?.querySelector("textarea")?.focus();
    let active = true;
    profilesApi.list().then((list) => {
      if (!active) return;
      setProfiles(list);
      setProfileId(list[0]?.id ?? "");
      useStore.setState({ profiles: list });
    }).catch((e) => active && setError(String(e)));
    return () => { active = false; node?.close(); previous?.focus(); };
  }, []);

  useEffect(() => {
    let active = true;
    setMessages([]);
    if (source) profilesApi.messages(workspaceId, source).then((list) => active && setMessages(list)).catch((e) => active && setError(String(e)));
    return () => { active = false; };
  }, [workspaceId, source]);

  function update(patch: Partial<AgentProfile>) {
    setProfiles((list) => list.map((p) => p.id === profileId ? { ...p, ...patch } : p));
    setDirty(true);
    setNotice("");
  }

  async function save() {
    setBusy(true);
    setError("");
    try {
      await profilesApi.save(profiles);
      setDirty(false);
      setNotice("Profiles saved on this device.");
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  async function prepare() {
    if (!profile || dirty) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      setPreview(await profilesApi.prepare(workspaceId, profile.id, agentId, { task, acceptanceCriteria: criteria, messages: selected, files: paths(files), diffs: paths(diffs) }));
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  async function launch() {
    if (!preview) return;
    setBusy(true);
    setError("");
    try {
      const session = await profilesApi.launch(workspaceId, preview.id);
      // Once launched, never offer the same preview for a second send.
      setPreview(null);
      await actions.profileLaunched(session);
      onClose();
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  return <dialog ref={dialog} onKeyDown={(e) => { e.stopPropagation(); if (e.key === "Escape" && !busy) { e.preventDefault(); onClose(); } }} aria-labelledby="profiles-title" onCancel={(e) => { if (busy) e.preventDefault(); else onClose(); }} className="m-auto max-h-[90vh] w-[min(760px,94vw)] overflow-auto rounded-lg border border-border bg-panel p-0 text-fg shadow-xl backdrop:bg-black/40">
    <header className="flex items-center justify-between border-b border-border px-5 py-3">
      <h2 id="profiles-title" className="font-medium">Profiles and context handoff</h2>
      <button aria-label="Close profiles" disabled={busy} onClick={onClose} className="rounded p-1 hover:bg-hover"><X size={16} /></button>
    </header>
    <div className="space-y-4 p-5">
      <p className="text-sm text-muted">Start a new conversation with reusable instructions. Only the items you select below are shared; no hidden transcript or native agent session is transferred. The agent still has normal access to this workspace.</p>
      {error && <p role="alert" className="selectable text-sm text-del-fg">{error}</p>}
      {notice && <p role="status" className="text-sm text-muted">{notice}</p>}
      {preview ? <>
        <div className="text-sm"><strong>{preview.profileName}</strong> · {preview.agentId} · {preview.model ?? "Provider default model"} · {preview.effort ?? "Provider default effort"}</div>
        {preview.warnings.map((warning) => <p key={warning} className="text-sm text-warn">{warning}</p>)}
        <label className="block text-sm font-medium" htmlFor="context-preview">Exact shared brief · {preview.bytes.toLocaleString()} / 65,536 bytes</label>
        <textarea id="context-preview" readOnly value={preview.text} rows={18} className={`${input} selectable font-mono text-xs`} />
        <p className="text-xs text-muted">This frozen text is sent unchanged, even if a source file or conversation changes. Repository instructions and provider system prompts remain separate.</p>
        <div className="flex justify-end gap-2">
          <button disabled={busy} className={button} onClick={() => setPreview(null)}>Edit selection</button>
          <button disabled={busy} className={`${button} bg-accent text-accent-fg`} onClick={() => void launch()}>{busy ? "Starting…" : "Start new conversation"}</button>
        </div>
      </> : <>
        <fieldset disabled={busy} className="space-y-4 disabled:opacity-60">
          <div className="grid grid-cols-2 gap-3">
            <label className="space-y-1 text-sm">Profile<select className={input} value={profileId} onChange={(e) => setProfileId(e.target.value)}><option value="" disabled>Select profile</option>{profiles.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}</select></label>
            <label className="space-y-1 text-sm">Provider<select className={input} value={agentId} onChange={(e) => setAgentId(e.target.value)}>{availableAgents.map((a) => <option key={a.id} value={a.id}>{a.name}</option>)}</select></label>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <button className={button} onClick={() => setEditing(!editing)}>{editing ? "Hide profile editor" : "Edit profiles"}</button>
            <button className={button} disabled={profiles.length >= 32} onClick={() => { const p = { id: crypto.randomUUID(), name: "New profile", instructions: "Follow the requested task and report verification.", preferences: {} }; setProfiles([...profiles, p]); setProfileId(p.id); setEditing(true); setDirty(true); }}><Plus size={12} className="mr-1 inline" />New profile</button>
            {dirty && <button className={button} onClick={() => void save()}>Save profiles</button>}
          </div>
          {editing && profile && <div className="space-y-3 border-l-2 border-border pl-3">
            <label className="block space-y-1 text-sm">Profile name<input className={input} maxLength={80} value={profile.name} onChange={(e) => update({ name: e.target.value })} /></label>
            <label className="block space-y-1 text-sm">Instructions<textarea className={input} rows={5} maxLength={8192} value={profile.instructions} onChange={(e) => update({ instructions: e.target.value })} /></label>
            <div className="grid grid-cols-2 gap-3">
              <label className="space-y-1 text-sm">{agentId} model preference<select className={input} value={preference.model ?? ""} onChange={(e) => update({ preferences: { ...profile.preferences, [agentId]: { ...preference, model: e.target.value || null } } })}><option value="">Provider default</option>{preference.model && !catalogs[agentId]?.models.some((m) => m.value === preference.model) && <option value={preference.model}>{preference.model} (not advertised)</option>}{catalogs[agentId]?.models.map((m) => <option key={m.value} value={m.value}>{m.name}</option>)}</select></label>
              <label className="space-y-1 text-sm">{agentId} effort preference<select className={input} value={preference.effort ?? ""} onChange={(e) => update({ preferences: { ...profile.preferences, [agentId]: { ...preference, effort: e.target.value || null } } })}><option value="">Provider default</option>{preference.effort && !catalogs[agentId]?.efforts.some((m) => m.value === preference.effort) && <option value={preference.effort}>{preference.effort} (not advertised)</option>}{catalogs[agentId]?.efforts.map((m) => <option key={m.value} value={m.value}>{m.name}</option>)}</select></label>
            </div>
            <p className="text-xs text-muted">Preferences are saved separately for each provider and applied only when its discovered capabilities support them. Unsupported preferences are explained in preview.</p>
            <button className={`${button} text-del-fg`} onClick={() => { setProfiles(profiles.filter((p) => p.id !== profileId)); setProfileId(profiles.find((p) => p.id !== profileId)?.id ?? ""); setDirty(true); }}>Delete this profile</button>
          </div>}
          <label className="block space-y-1 text-sm">Task<textarea className={input} rows={3} maxLength={16384} placeholder="What should this new conversation do?" value={task} onChange={(e) => setTask(e.target.value)} /></label>
          <label className="block space-y-1 text-sm">Acceptance criteria (optional)<textarea className={input} rows={2} maxLength={16384} value={criteria} onChange={(e) => setCriteria(e.target.value)} /></label>
          <details className="space-y-2">
            <summary className="cursor-pointer text-sm">Select messages · {selected.length} selected</summary>
            <label className="block space-y-1 text-sm">Source conversation<select className={input} value={source} onChange={(e) => setSource(e.target.value)}><option value="">None</option>{sessions?.map((s) => <option key={s.id} value={s.id}>{s.title || "New chat"} · {s.agentId}</option>)}</select></label>
            <div className="max-h-48 space-y-2 overflow-auto">
              {messages.map((m) => <label key={m.index} className="flex items-start gap-2 text-xs">
                <input type="checkbox" checked={selected.some((r) => r.sessionId === source && r.index === m.index)} onChange={(e) => setSelected(e.target.checked ? [...selected, { sessionId: source, index: m.index }] : selected.filter((r) => r.sessionId !== source || r.index !== m.index))} />
                <span className="min-w-0"><strong>{m.role}</strong><span className="line-clamp-3 whitespace-pre-wrap">{m.text}</span></span>
              </label>)}
            </div>
            <p className="text-xs text-muted">Text messages up to 16 KiB only. Tool output, thoughts and images are not swept into handoffs.</p>
            {!!selected.length && <button className={button} onClick={() => setSelected([])}>Clear selected messages</button>}
          </details>
          <div className="grid grid-cols-2 gap-3">
            <label className="space-y-1 text-sm">Files (one relative path per line)<textarea className={`${input} font-mono text-xs`} rows={3} placeholder="src/example.ts" value={files} onChange={(e) => setFiles(e.target.value)} /></label>
            <label className="space-y-1 text-sm">Diffs (one relative path per line)<textarea className={`${input} font-mono text-xs`} rows={3} placeholder="src/example.ts" value={diffs} onChange={(e) => setDiffs(e.target.value)} /></label>
          </div>
          <p className="text-xs text-muted">Explicit files only from this workspace. No folders, traversal, outside symlinks or secret/configuration files (.env, keys, credentials). Maximum 32 items, 16 KiB each, 64 KiB total. Oversized content is rejected, never silently truncated. Review selected text for secrets before sharing.</p>
          <div className="flex justify-end"><button disabled={!profile || dirty || !task.trim()} className={`${button} bg-accent text-accent-fg`} onClick={() => void prepare()}>{busy ? "Preparing…" : dirty ? "Save profiles before preview" : "Preview shared context"}</button></div>
        </fieldset>
      </>}
    </div>
  </dialog>;
}
