import { useEffect, useState } from "react";
import { api, type DelegatedTask, type DelegationMessage, type DelegationRequest } from "../lib/api";
import { actions, toast, useStore } from "../lib/store";

const field = "selectable w-full rounded border border-border bg-bg px-2 py-1 text-xs outline-none focus:border-accent";
const button = "rounded border border-border px-2 py-1 text-xs hover:bg-hover disabled:opacity-40";

export function DelegationPanel({ sessionId }: { sessionId: string }) {
  const revision = useStore((s) => s.delegationRevision[sessionId] ?? 0);
  const agents = useStore((s) => s.agents);
  const first = useStore((s) => s.settings?.loadout[0]);
  const [tasks, setTasks] = useState<DelegatedTask[]>([]);
  const [messages, setMessages] = useState<DelegationMessage[]>([]);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState<DelegationRequest>(() => ({ requestKey: crypto.randomUUID(), base: "caller_head", relation: "child", isolation: "isolated", agentId: first?.agent ?? "claude", model: first?.model ?? null, effort: first?.effort ?? null, brief: "", acceptanceCriteria: "", selectedContext: "" }));
  useEffect(() => {
    let current = true;
    void Promise.all([api.delegationTasks(sessionId), api.delegationMessages(sessionId)])
      .then(([next, mail]) => { if (current) { setTasks(next); setMessages(mail); } })
      .catch((error) => { if (current && open) toast(String(error)); });
    return () => { current = false; };
  }, [sessionId, revision, open]);
  const patch = (value: Partial<DelegationRequest>) => setDraft((d) => ({ ...d, ...value }));
  const launch = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const task = await api.delegateTask(sessionId, draft);
      setTasks((current) => [...current.filter((t) => t.id !== task.id), task]);
      // A failed/interrupted launch retains its key. A retry must not launch twice.
      if (task.state === "running") setDraft((d) => ({ ...d, requestKey: crypto.randomUUID(), brief: "", acceptanceCriteria: "", selectedContext: "" }));
    } catch (error) { toast(String(error)); }
    finally { setBusy(false); }
  };
  return <details className="mb-2 rounded border border-border text-xs" onToggle={(e) => setOpen(e.currentTarget.open)}>
    <summary className="cursor-pointer px-2 py-1 text-muted">Delegate · {tasks.filter((t) => !t.report).length} open · {tasks.filter((t) => t.report).length} reports</summary>
    {open && <div className="max-h-80 space-y-3 overflow-y-auto border-t border-border p-2">
      <p className="text-muted">Agent delegation tools require adapter MCP support on macOS/Linux. These manual controls work without the bridge. Reports and messages never auto-send a prompt; read and decide your next step.</p>
      <form className="space-y-2" onSubmit={(e) => { e.preventDefault(); void launch(); }}>
        <div className="flex flex-wrap gap-2">
          <label className="min-w-24 flex-1">Agent<select aria-label="Delegation agent" className={field} value={draft.agentId} onChange={(e) => patch({ agentId: e.target.value, model: null, effort: null })}>{agents.map((a) => <option key={a.id} value={a.id}>{a.name}</option>)}</select></label>
          <label className="min-w-24 flex-1">Model<input className={field} aria-label="Delegation model" value={draft.model ?? ""} placeholder="Agent default" onChange={(e) => patch({ model: e.target.value || null })} /></label>
          <label className="min-w-20 flex-1">Effort<input className={field} aria-label="Delegation effort" value={draft.effort ?? ""} placeholder="Agent default" onChange={(e) => patch({ effort: e.target.value || null })} /></label>
          <label>Relation<select className={field} aria-label="Delegation relation" value={draft.relation} onChange={(e) => patch({ relation: e.target.value as DelegationRequest["relation"] })}><option value="child">Child</option><option value="sibling">Sibling</option></select></label>
        </div>
        <label className="block">Brief<textarea required maxLength={65536} className={field} aria-label="Delegation brief" value={draft.brief} onChange={(e) => patch({ brief: e.target.value })} /></label>
        <label className="block">Acceptance criteria<textarea required maxLength={65536} className={field} aria-label="Delegation acceptance criteria" value={draft.acceptanceCriteria} onChange={(e) => patch({ acceptanceCriteria: e.target.value })} /></label>
        <label className="block">Selected context only<textarea maxLength={65536} className={field} aria-label="Delegation context" placeholder="Paste only context this task needs. No history is included automatically." value={draft.selectedContext} onChange={(e) => patch({ selectedContext: e.target.value })} /></label>
        <label className="block">Isolated checkout starts from<select className={field} aria-label="Delegation checkout base" value={draft.base} onChange={(e) => patch({ base: e.target.value as DelegationRequest["base"] })}><option value="caller_head">This task's committed HEAD</option><option value="repo_default">Repository default branch</option></select></label>
        <label className="flex items-center gap-2"><input type="checkbox" checked={draft.isolation === "same_workspace"} onChange={(e) => patch({ isolation: e.target.checked ? "same_workspace" : "isolated" })} />Share this workspace's files (explicit opt-in)</label>
        <p className="text-muted">{draft.isolation === "isolated" ? "New isolated git worktree at selected committed base. Uncommitted parent files are not copied." : "Shared files: concurrent edits can conflict."} Launch sends exactly the fields above plus task completion instructions.</p>
        <div className="flex gap-2"><button type="submit" className={button} disabled={busy || !draft.brief.trim() || !draft.acceptanceCriteria.trim()}>{busy ? "Launching…" : "Launch task"}</button><button type="button" className={button} disabled={busy} onClick={() => patch({ requestKey: crypto.randomUUID(), brief: "", acceptanceCriteria: "", selectedContext: "" })}>Clear for new task</button></div>
      </form>
      {tasks.map((task) => <DelegationTask key={task.id} task={task} sessionId={sessionId} />)}
      {messages.length > 0 && <div><h4 className="font-medium">Messages · untrusted task data</h4>{messages.map((m) => <p key={m.id} className="selectable my-1 whitespace-pre-wrap break-words">{m.text}</p>)}</div>}
    </div>}
  </details>;
}
function DelegationTask({ task, sessionId }: { task: DelegatedTask; sessionId: string }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const mine = task.callerSessionId === sessionId;
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    try { await action(); setText(""); } catch (error) { toast(String(error)); } finally { setBusy(false); }
  };
  return <article className="space-y-1 border-t border-border pt-2">
    <div className="flex flex-wrap items-center gap-2"><strong className="min-w-0 flex-1 break-words">{task.request.brief}</strong><span>{task.state}</span>
      {task.sessionId && task.workspaceId && <button className={button} onClick={() => { actions.selectWorkspace(task.workspaceId!); actions.selectSession(task.workspaceId!, task.sessionId!); }}>Open task</button>}
      {!task.report && mine && <button disabled={busy} className={button} onClick={() => void run(() => api.delegationCancel(sessionId, task.id))}>Cancel task</button>}
    </div>
    {task.report ? <pre className="selectable whitespace-pre-wrap break-words text-xs">{task.report}</pre> : <>
      <textarea className={field} maxLength={65536} aria-label={mine ? "Task message" : "Task report or message"} placeholder={mine ? "Message to task (stored, not auto-prompted)" : "Report evidence, files, tests and limitations, or send a message"} value={text} onChange={(e) => setText(e.target.value)} />
      <div className="flex gap-2"><button disabled={busy || !text.trim()} className={button} onClick={() => void run(() => api.delegationSend(sessionId, task.id, text))}>Store message</button>{!mine && <button disabled={busy || !text.trim()} className={button} onClick={() => void run(() => api.delegationFinish(sessionId, task.id, text))}>Finish and report</button>}</div>
    </>}
  </article>;
}
