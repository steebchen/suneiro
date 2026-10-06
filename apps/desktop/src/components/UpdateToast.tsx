import { ask } from "@tauri-apps/plugin-dialog";
import { Download, Loader2 } from "lucide-react";
import { api, type UpdateStatus } from "../lib/api";
import { toast, useStore, workspaceActivity } from "../lib/store";

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

function waitingFor({ agents, workspaces }: UpdateStatus["activity"]) {
  if (agents) return `Restarts once ${plural(agents, "agent")} in ${plural(workspaces, "workspace")} finish${agents === 1 ? "es" : ""}.`;
  if (workspaces) return `Restarts once ${plural(workspaces, "workspace")} finish${workspaces === 1 ? "es" : ""} setting up.`;
  return "Restarting shortly…";
}

/** Whether restarting now would cut off an agent or a workspace setup. */
function busy() {
  const s = useStore.getState();
  return s.workspaces.some((w) => {
    const a = workspaceActivity(s, w.id);
    return a === "running" || a === "needs-input" || w.status === "creating" || w.status === "setting_up";
  });
}

export async function restartNow(version: string) {
  if (busy() && !(await ask("Agents are still working. Restarting stops them; you can continue the chats afterwards.", { title: `Restart to update to ${version}?`, okLabel: "Restart" }))) return;
  api.installUpdate(false).catch((e) => toast(`Update failed: ${e}`));
}

function whenIdle() {
  api.installUpdate(true).catch((e) => toast(`Update failed: ${e}`));
}

/** Bottom-right card offering a new version; nothing installs until the user picks a restart option. */
export function UpdateToast() {
  const update = useStore((s) => s.update);
  const dismissed = useStore((s) => s.updateDismissed);
  if (!update?.version || update.phase === "none") return null;
  const { version, phase } = update;
  if (phase === "available" && dismissed === version) return null;

  if (update.automatic) {
    return (
      <div className="w-80 rounded-lg border border-border bg-elevated p-3 shadow-lg">
        <div className="flex items-center gap-2 font-medium">
          <Loader2 size={14} className="animate-spin text-muted" /> Updating Suneiro to {version}…
        </div>
        <p className="mt-1 text-muted">Suneiro restarts in a moment.</p>
        {phase === "downloading" && (
          <div className="mt-3 flex justify-end">
            <button className="rounded-md px-2 py-1 hover:bg-hover" onClick={() => void api.cancelUpdate()}>
              Not now
            </button>
          </div>
        )}
      </div>
    );
  }

  const button = "rounded-md px-2 py-1 hover:bg-hover";
  const primary = "rounded-md bg-accent px-2 py-1 font-medium text-accent-fg";
  return (
    <div className="w-80 rounded-lg border border-border bg-elevated p-3 shadow-lg">
      <div className="flex items-center gap-2 font-medium">
        {phase === "available" ? <Download size={14} className="text-accent" /> : <Loader2 size={14} className="animate-spin text-muted" />}
        {phase === "available" && `Suneiro ${version} is available`}
        {phase === "downloading" && `Downloading Suneiro ${version}…`}
        {phase === "waiting" && `Update to ${version} queued`}
        {phase === "installing" && `Installing Suneiro ${version}…`}
      </div>
      {phase === "available" && <p className="mt-1 text-muted">Restart to install it. Running agents are only stopped if you restart now.</p>}
      {phase === "waiting" && <p className="mt-1 text-muted">{waitingFor(update.activity)}</p>}
      {phase === "available" && (
        <div className="mt-3 flex items-center justify-end gap-1">
          <button className={`${button} mr-auto text-muted`} onClick={() => useStore.setState({ updateDismissed: version })}>
            Dismiss
          </button>
          <button className={button} onClick={whenIdle} title="Install and restart once no agent is working and no workspace is being set up">
            Restart when idle
          </button>
          <button className={primary} onClick={() => void restartNow(version)}>
            Restart now
          </button>
        </div>
      )}
      {phase === "waiting" && (
        <div className="mt-3 flex items-center justify-end gap-1">
          <button className={`${button} mr-auto text-muted`} onClick={() => void api.cancelUpdate()}>
            Cancel
          </button>
          <button className={primary} onClick={() => void restartNow(version)}>
            Restart now
          </button>
        </div>
      )}
    </div>
  );
}
