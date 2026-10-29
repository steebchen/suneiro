import { useEffect, useState, memo } from "react";
import { api, type DelegatedTask, type DelegationMessage } from "../lib/api";
import { actions, toast, useStore } from "../lib/store";
import { AgentIcon } from "../lib/models";
import { Markdown } from "./Markdown";
const button = "rounded px-1.5 py-0.5 text-[11px] text-muted hover:bg-hover hover:text-fg disabled:opacity-40";

export const DelegationPanel = memo(function DelegationPanel({ sessionId }: { sessionId: string }) {
  const revision = useStore((s) => s.delegationRevision[sessionId] ?? 0);
  const [tasks, setTasks] = useState<DelegatedTask[]>([]);
  const [messages, setMessages] = useState<DelegationMessage[]>([]);
  useEffect(() => {
    let current = true;
    void Promise.all([api.delegationTasks(sessionId), api.delegationMessages(sessionId)])
      .then(([next, mail]) => {
        if (!current) return;
        setTasks(next.filter((task) => task.callerSessionId === sessionId));
        setMessages(mail);
      })
      .catch(() => {});
    return () => { current = false; };
  }, [sessionId, revision]);
  if (!tasks.length && !messages.length) return null;
  const running = tasks.filter((task) => task.state === "running" || task.state === "launching").length;
  const results = tasks.filter((task) => task.report).length;
  return (
    <details className="mb-2 rounded-md border border-border text-xs" aria-label="Related chat updates">
      <summary className="cursor-pointer px-3 py-2 text-muted">
        Chat updates{running > 0 && ` · ${running} working`}{results > 0 && ` · ${results} ${results === 1 ? "result" : "results"}`}
      </summary>
      <div className="max-h-64 space-y-3 overflow-y-auto border-t border-border px-3 py-2">
        {tasks.map((task) => <DelegationTask key={task.id} task={task} sessionId={sessionId} />)}
        {messages.map((message) => <div key={message.id} className="border-t border-border pt-2"><Markdown text={message.text} /></div>)}
      </div>
    </details>
  );
});
function DelegationTask({ task, sessionId }: { task: DelegatedTask; sessionId: string }) {
  const title = useStore((s) => (s.sessions[task.workspaceId ?? ""] ?? []).find((session) => session.id === task.sessionId)?.title) || task.request.brief;
  const [busy, setBusy] = useState(false);
  const active = task.state === "running" || task.state === "launching";
  const status = ({ launching: "Starting", running: "Working", completed: "Done", cancelled: "Stopped", interrupted: "Stopped", failed: "Needs attention" } as Record<string, string>)[task.state] ?? task.state;
  const stop = async () => {
    setBusy(true);
    try { await api.delegationCancel(sessionId, task.id); }
    catch (error) { toast(String(error)); }
    finally { setBusy(false); }
  };
  return (
    <article>
      <div className="flex min-w-0 items-center gap-2">
        <AgentIcon agent={task.request.agentId} size={12} />
        <span className="min-w-0 flex-1 truncate font-medium">{title}</span>
        <span className="shrink-0 text-muted">{status}</span>
        {task.sessionId && task.workspaceId && <button className={button} onClick={() => { actions.selectWorkspace(task.workspaceId!); actions.selectSession(task.workspaceId!, task.sessionId!); }}>Open chat</button>}
        {active && <button className={button} disabled={busy} onClick={() => void stop()}>Stop</button>}
      </div>
      {task.report && <div className="mt-2"><Markdown text={task.report} /></div>}
    </article>
  );
}
