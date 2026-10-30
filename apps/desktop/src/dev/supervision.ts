import type {
  CoreEvent,
  InboxItem,
  RepoControls,
  ResourceUsage,
  Usage,
} from "../lib/api";

export function mockSupervision(
  sessions: Record<
    string,
    { id: string; workspaceId: string; parentSessionId?: string | null }[]
  >,
  workspaces: { id: string; repoId: string }[],
  usage: () => Usage[],
  emit: (...events: CoreEvent[]) => unknown,
) {
  const inbox = new Map<string, InboxItem>();
  const controls = new Map<string, RepoControls>();
  const notifications = new Set<string>();
  const running = new Set<string>();
  const defaults = (): RepoControls => ({
    maxConcurrent: 4,
    paused: false,
    warningUsd: null,
    stopUsd: null,
    spendSince: 0,
  });
  const repoFor = (id: string) => {
    const chat = Object.values(sessions)
      .flat()
      .find((s) => s.id === id);
    return workspaces.find((w) => w.id === chat?.workspaceId)?.repoId;
  };
  const summary = (repoId: string, root = ""): ResourceUsage => {
    const ids = new Set<string>();
    const all = Object.values(sessions).flat();
    if (root) {
      ids.add(root);
      let changed = true;
      while (changed) {
        changed = false;
        for (const s of all)
          if (
            s.parentSessionId &&
            ids.has(s.parentSessionId) &&
            !ids.has(s.id)
          ) {
            ids.add(s.id);
            changed = true;
          }
      }
    } else for (const s of all) if (repoFor(s.id) === repoId) ids.add(s.id);
    const c = controls.get(repoId) ?? defaults();
    const rows = usage().filter(
      (u) => ids.has(u.sessionId) && (root || u.ts >= c.spendSince),
    );
    return rows.reduce(
      (r, u) => ({
        ...r,
        turns: r.turns + 1,
        inputTokens: r.inputTokens + u.inputTokens,
        cachedTokens: r.cachedTokens + u.cachedTokens,
        outputTokens: r.outputTokens + u.outputTokens,
        reportedUsd: r.reportedUsd + (!u.estimated ? (u.cost ?? 0) : 0),
        estimatedUsd: r.estimatedUsd + (u.estimated ? (u.cost ?? 0) : 0),
        unknownTurns: r.unknownTurns + (u.cost === null ? 1 : 0),
      }),
      {
        sessions: ids.size,
        running: [...running].filter((id) => ids.has(id)).length,
        turns: 0,
        inputTokens: 0,
        cachedTokens: 0,
        outputTokens: 0,
        reportedUsd: 0,
        estimatedUsd: 0,
        unknownTurns: 0,
      },
    );
  };
  return {
    admit(sessionId: string) {
      const repo = repoFor(sessionId);
      if (!repo) throw new Error("Conversation not found");
      const c = controls.get(repo) ?? defaults();
      if (running.has(sessionId)) throw new Error("agent is still working");
      if (c.paused) throw new Error("repository scheduling is paused");
      if (
        [...running].filter((id) => repoFor(id) === repo).length >=
        c.maxConcurrent
      )
        throw new Error("repository concurrency limit reached");
      const spent = summary(repo);
      if (
        c.stopUsd !== null &&
        spent.reportedUsd + spent.estimatedUsd >= c.stopUsd
      )
        throw new Error("observed spend stop threshold reached");
      running.add(sessionId);
    },
    observe(events: CoreEvent[]) {
      const extra: CoreEvent[] = [];
      for (const e of events) {
        if (
          e.type === "turnEnd" ||
          (e.type === "sessionState" &&
            ["idle", "error", "disconnected"].includes(e.state))
        )
          running.delete(e.sessionId);
        if (e.type === "questionResolved" || e.type === "permissionResolved") {
          const id = `${e.type === "questionResolved" ? "question" : "permission"}:${e.sessionId}:${e.requestId}`;
          if (inbox.delete(id)) extra.push({ type: "inboxResolved", id });
        }
        if (
          e.type !== "question" &&
          e.type !== "permissionRequest" &&
          !(e.type === "sessionState" && e.state === "error")
        )
          continue;
        const kind =
          e.type === "question"
            ? "question"
            : e.type === "permissionRequest"
              ? "permission"
              : "failure";
        const id = `${kind}:${e.sessionId}:${"requestId" in e ? e.requestId : "error"}`;
        const chat = Object.values(sessions)
          .flat()
          .find((s) => s.id === e.sessionId);
        const repoId = repoFor(e.sessionId);
        if (!chat || !repoId || inbox.has(id)) continue;
        const item: InboxItem = {
          id,
          sessionId: chat.id,
          workspaceId: chat.workspaceId,
          repoId,
          kind,
          message:
            e.type === "question"
              ? e.message
              : e.type === "sessionState"
                ? (e.error ?? "Agent failed")
                : "Permission required",
          createdAt: Date.now(),
          resolved: false,
        };
        inbox.set(id, item);
        extra.push({ type: "inboxUpdated", item });
      }
      return extra;
    },
    handlers: {
      supervision_inbox: () => [...inbox.values()],
      dismiss_inbox: ({ id }: { id: string }) => {
        const item = inbox.get(id);
        if (item && ["question", "permission"].includes(item.kind))
          throw new Error("answer this request in its conversation");
        inbox.delete(id);
        emit({ type: "inboxResolved", id });
      },
      claim_notification: ({ id }: { id: string }) => {
        if (notifications.has(id)) return false;
        notifications.add(id);
        return true;
      },
      repo_controls: ({ repoId }: { repoId: string }) =>
        controls.get(repoId) ?? defaults(),
      save_repo_controls: ({
        repoId,
        controls: c,
      }: {
        repoId: string;
        controls: RepoControls;
      }) => {
        if (
          !Number.isInteger(c.maxConcurrent) ||
          c.maxConcurrent < 1 ||
          c.maxConcurrent > 64 ||
          [c.warningUsd, c.stopUsd].some(
            (v) => v !== null && (!Number.isFinite(v) || v <= 0),
          ) ||
          (c.warningUsd !== null &&
            c.stopUsd !== null &&
            c.warningUsd > c.stopUsd)
        )
          throw new Error("Invalid scheduling controls");
        controls.set(repoId, c);
        emit({ type: "repoControlsChanged", repoId, controls: c });
      },
      repo_resources: ({ repoId }: { repoId: string }) => summary(repoId),
      subtree_resources: ({ sessionId }: { sessionId: string }) =>
        summary(repoFor(sessionId) ?? "", sessionId),
    },
  };
}
