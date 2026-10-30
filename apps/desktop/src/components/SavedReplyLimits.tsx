import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { allowRepliesControls, replyLimitsActive } from "../lib/replyLimits";
import { toast } from "../lib/store";
import { useSupervision } from "../lib/supervision";

export function SavedReplyLimits({ repoId }: { repoId: string }) {
  const controls = useSupervision((s) => s.controls[repoId]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    const before = useSupervision.getState().controls[repoId];
    void api.repoControls(repoId).then((next) => {
      if (!active || useSupervision.getState().controls[repoId] !== before) return;
      useSupervision.setState((s) => ({ controls: { ...s.controls, [repoId]: next } }));
    }).catch((error) => {
      if (active) toast(`Couldn't load saved reply limits: ${error}`);
    });
    return () => { active = false; };
  }, [repoId]);

  if (!replyLimitsActive(controls)) return null;

  const allowReplies = async () => {
    setBusy(true);
    try {
      const next = allowRepliesControls(await api.repoControls(repoId));
      await api.saveRepoControls(repoId, next);
      useSupervision.setState((s) => ({ controls: { ...s.controls, [repoId]: next } }));
    } catch (error) {
      toast(`Couldn't allow replies: ${error}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div role="status" className="flex shrink-0 items-center gap-3 border-b border-border bg-warn/10 px-3 py-2 text-xs">
      <div className="min-w-0 flex-1">
        <p className="font-medium">Saved reply limits are active</p>
        <p className="text-muted">Allow replies clears the saved pause and spend limits. Other limits stay unchanged.</p>
      </div>
      <button disabled={busy} onClick={() => void allowReplies()} className="shrink-0 rounded border border-border px-2 py-1 hover:bg-hover disabled:opacity-50">
        {busy ? "Allowing…" : "Allow replies"}
      </button>
    </div>
  );
}
