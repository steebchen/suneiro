import * as React from "react";

export function subscriptionAuthAgent(agentId: string | undefined, cloud: boolean, text: string): string | null {
  if (cloud || (agentId !== "claude" && agentId !== "codex")) return null;
  const authFailure = /\bauthentication (?:required|failed)\b|\b(?:failed|unable) to authenticate\b|\bnot (?:logged|signed) in\b|\bunauthorized\b/i.test(text)
    || /\b(?:oauth|access[ _-]token|refresh[ _-]token)\b[^\n]{0,160}\b(?:expired|invalid|revoked|refresh|already used)\b/i.test(text)
    || /\b(?:expired|invalid|revoked)\b[^\n]{0,80}\b(?:oauth|access[ _-]token|refresh[ _-]token)\b/i.test(text);
  return authFailure ? (agentId === "claude" ? "Claude Code" : "Codex") : null;
}

export function AgentError({ text, agentId, cloud, onOpenSettings }: { text: string; agentId: string | undefined; cloud: boolean; onOpenSettings: () => void }): React.ReactElement {
  const agent = subscriptionAuthAgent(agentId, cloud, text);
  return (
    <div className="selectable my-2 rounded-md border border-del-fg/30 bg-del-bg px-3 py-2 text-xs text-del-fg">
      <div className="font-mono whitespace-pre-wrap">{text}</div>
      {agent && (
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <span>Sign in to {agent} again in Settings, then retry your message.</span>
          <button onClick={onOpenSettings} className="rounded-md border border-del-fg/30 px-2 py-1 font-medium hover:bg-hover">
            Open Settings
          </button>
        </div>
      )}
    </div>
  );
}
