import type { CheckPreset, CheckRun, CoreEvent, IntegrationPreview, Session, VerificationRevision, Workspace } from "../lib/api";

/** Browser-only deterministic scenarios: ?verification=dirty|conflict|running.
 * First tests run fails, retry passes; other checks pass. A simulated edit (including
 * an untracked file) six seconds after the first failure makes earlier results stale.
 * Commands are inert strings: the mock never executes user input.
 */
export function verificationMock({ workspaces, sessions, emit, scenario = "", isRunning = () => false }: {
  workspaces: Workspace[];
  sessions: Record<string, Session[]>;
  emit: (...events: CoreEvent[]) => void;
  scenario?: string;
  isRunning?: (workspaceId: string) => boolean;
}) {
  const presets = new Map<string, CheckPreset[]>();
  const runs = new Map<string, CheckRun[]>();
  const attempts = new Map<string, number>();
  const heads = new Map<string, string>();
  const content = new Map<string, number>();
  const dirty = new Set<string>();
  const histories = new Map<string, CoreEvent[]>();
  let sequence = 0;
  const copy = <T,>(value: T): T => structuredClone(value);
  const workspace = (id: string) => {
    const ws = workspaces.find((w) => w.id === id && !w.archivedAt);
    if (!ws) throw new Error("Workspace not found");
    return ws;
  };
  const revision = (id: string): VerificationRevision => ({
    head: heads.get(id) ?? (id === "w2" ? "b222222" : "a111111"),
    fingerprint: `mock-content-${id}-${content.get(id) ?? 0}`,
  });
  const publish = (run: CheckRun) => emit({ type: "verificationChanged", workspaceId: run.workspaceId, run: copy(run) });
  const updateStaleness = (id: string) => {
    const current = revision(id);
    for (const run of runs.get(id) ?? []) {
      const stale = run.revision.head !== current.head || run.revision.fingerprint !== current.fingerprint;
      if (run.stale !== stale) { run.stale = stale; publish(run); }
    }
  };
  const assertIdle = (id: string) => {
    const activeChat = (sessions[id] ?? []).some((session) => {
      const last = histories.get(session.id)?.at(-1);
      return last?.type === "sessionState" && last.state === "running";
    });
    if (scenario === "running" || isRunning(id) || activeChat || (runs.get(id) ?? []).some((r) => r.status === "running")) throw new Error("Workspace has a running check or agent");
  };
  const pair = (id: string, sourceId: string) => {
    const target = workspace(id);
    const source = workspace(sourceId);
    if (id === sourceId || target.repoId !== source.repoId) throw new Error("Choose another workspace in the same repository");
    return { target, source };
  };
  const canApply = (id: string, sourceId: string) => {
    try { assertIdle(id); assertIdle(sourceId); } catch { return false; }
    const configured = presets.get(workspace(sourceId).repoId) ?? [];
    const current = revision(sourceId);
    const sourceChecksPass = configured.length > 0 && configured.every((preset) => {
      const latest = (runs.get(sourceId) ?? []).find((run) => run.presetId === preset.id);
      return latest?.status === "passed" && latest.command === preset.command && !latest.stale
        && latest.revision.head === current.head && latest.revision.fingerprint === current.fingerprint;
    });
    return !["dirty", "conflict"].includes(scenario) && !dirty.has(id) && !dirty.has(sourceId) && sourceChecksPass;
  };
  const createSession = (workspaceId: string, title: string, agentId: string, text: string, review: boolean): Session => {
    workspace(workspaceId);
    const session: Session = { id: `verification-session-${++sequence}`, workspaceId, agentId, acpSessionId: null, title, createdAt: Date.now(), model: null, effort: null };
    sessions[workspaceId] = [...(sessions[workspaceId] ?? []), session];
    const history: CoreEvent[] = [
      { type: "userMessage", sessionId: session.id, text, ts: Date.now() },
      { type: "sessionMode", sessionId: session.id, plan: review },
      { type: "sessionState", sessionId: session.id, state: "running", error: null },
    ];
    histories.set(session.id, history);
    emit(...copy(history));
    setTimeout(() => {
      const response: CoreEvent[] = [
        { type: "sessionUpdate", sessionId: session.id, update: { sessionUpdate: "agent_message_chunk", messageId: `${session.id}-reply`, content: { type: "text", text: review ? "Selected-file review findings:\n\n- `src/server/limiter.ts`: rate-limit buckets are never evicted. Add expiration to bound memory.\n- Add a test covering the Retry-After response.\n\nProtection: mutation permission requests denied; not an OS sandbox. No files were changed." : "I'll address the selected findings and failures in this task." } } },
        { type: "turnEnd", sessionId: session.id, stopReason: "end_turn", ts: Date.now() },
        { type: "sessionState", sessionId: session.id, state: "idle", error: null },
      ];
      history.push(...response);
      emit(...copy(response));
    }, 800);
    return copy(session);
  };
  const handlers: Record<string, (args: any) => any> = {
    verification_presets: ({ repoId }) => copy(presets.get(repoId) ?? []),
    save_verification_presets: ({ repoId, presets: next }: { repoId: string; presets: CheckPreset[] }) => {
      if (next.some((p) => !["tests", "lint", "typecheck", "build"].includes(p.id)) || new Set(next.map((p) => p.id)).size !== next.length) throw new Error("Invalid check presets");
      if (next.some((p) => !p.command.trim())) throw new Error("Only configured commands can be saved");
      presets.set(repoId, copy(next));
    },
    verification_status: ({ workspaceId }) => {
      workspace(workspaceId); updateStaleness(workspaceId);
      return copy(runs.get(workspaceId) ?? []);
    },
    run_check: ({ workspaceId, presetId }) => {
      const ws = workspace(workspaceId);
      const preset = presets.get(ws.repoId)?.find((p) => p.id === presetId);
      if (!preset?.command.trim()) throw new Error("Configure and save this command first");
      assertIdle(workspaceId);
      const key = `${workspaceId}:${presetId}`;
      const attempt = (attempts.get(key) ?? 0) + 1;
      attempts.set(key, attempt);
      const run: CheckRun = { id: `check-${++sequence}`, workspaceId, presetId, command: preset.command, cwd: ws.path, revision: revision(workspaceId), status: "running", log: `$ ${preset.command}\nRunning…\n`, exitCode: null, stale: false };
      runs.set(workspaceId, [run, ...(runs.get(workspaceId) ?? [])]); publish(run);
      setTimeout(() => {
        if (run.status !== "running") return;
        const failed = presetId === "tests" && attempt === 1;
        run.status = failed ? "failed" : "passed"; run.exitCode = failed ? 1 : 0;
        run.log += failed ? "FAIL limiter.test.ts\nExpected Retry-After header, received undefined.\n1 failed, 8 passed.\n" : `${presetId}: all checks passed.\n`;
        updateStaleness(workspaceId); publish(run);
        if (failed) setTimeout(() => {
          content.set(workspaceId, (content.get(workspaceId) ?? 0) + 1); dirty.add(workspaceId);
          updateStaleness(workspaceId);
          emit({ type: "workspaceStatus", workspaceId, status: "dirty" });
        }, 6000);
      }, 1000);
      return copy(run);
    },
    cancel_check: ({ workspaceId, runId }) => {
      const run = runs.get(workspaceId)?.find((r) => r.id === runId);
      if (!run) throw new Error("Check not found in this workspace");
      if (run.status !== "running") return;
      run.status = "cancelled"; run.log += "Cancelled by user.\n"; run.exitCode = null; publish(run);
    },
    start_review: ({ workspaceId, agentId, files }: { workspaceId: string; agentId: string; files: string[] }) => {
      if (!files.length) throw new Error("Select diff files to review");
      if (!["claude", "codex"].includes(agentId)) throw new Error("Agent does not support protected review");
      const protection = agentId === "codex" ? "agent-enforced read-only" : "plan mode; permissions denied; not sandboxed";
      return { session: createSession(workspaceId, "Review selected changes", agentId, `Review only these selected files:\n${files.join("\n")}\nReport findings. All mutation permission requests are denied. Protection: ${protection}.`, true), protection };
    },
    verification_task: ({ workspaceId, text }) => {
      if (!text.trim()) throw new Error("Select findings or failures for the task");
      return createSession(workspaceId, "Verification follow-up", "claude", text, false);
    },
    integration_preview: ({ workspaceId, sourceWorkspaceId }): IntegrationPreview => {
      pair(workspaceId, sourceWorkspaceId);
      return { sourceWorkspaceId, target: revision(workspaceId), source: revision(sourceWorkspaceId), conflicts: scenario === "conflict" ? ["src/server/limiter.ts"] : [], diff: "diff --git a/src/server/limiter.ts b/src/server/limiter.ts\n--- a/src/server/limiter.ts\n+++ b/src/server/limiter.ts\n@@ -1 +1 @@\n-export const rate = 60;\n+export const rate = 120;\n", canApply: canApply(workspaceId, sourceWorkspaceId) };
    },
    integrate_workspace: ({ workspaceId, preview }: { workspaceId: string; preview: IntegrationPreview }) => {
      pair(workspaceId, preview.sourceWorkspaceId);
      if (JSON.stringify(revision(workspaceId)) !== JSON.stringify(preview.target) || JSON.stringify(revision(preview.sourceWorkspaceId)) !== JSON.stringify(preview.source)) throw new Error("Revisions changed. Preview again before applying.");
      if (!canApply(workspaceId, preview.sourceWorkspaceId) || !preview.canApply || preview.conflicts.length) throw new Error("Merge blocked: workspaces must be clean and idle, conflicts empty, and every configured source check passed on current content");
      heads.set(workspaceId, `merge-${++sequence}`); content.set(workspaceId, (content.get(workspaceId) ?? 0) + 1);
      updateStaleness(workspaceId);
      emit({ type: "workspaceStatus", workspaceId, status: "dirty" });
    },
  };
  return { handlers, histories };
}
