import { create } from "zustand";
import { api, type CoreEvent, type InboxItem, type RepoControls } from "./api";

export const useSupervision = create<{
  inbox: InboxItem[];
  controls: Record<string, RepoControls>;
  revision: number;
  resourceFocus: { repoId: string; sessionId: string } | null;
}>(() => ({ inbox: [], controls: {}, revision: 0, resourceFocus: null }));

export async function loadInbox() {
  const inbox = await api.inbox();
  useSupervision.setState((s) => ({
    inbox: [
      ...new Map(
        [...inbox, ...s.inbox]
          .filter((i) => !resolvedIds.has(i.id))
          .map((i) => [i.id, i]),
      ).values(),
    ],
  }));
}

export function handleSupervisionEvents(events: CoreEvent[]) {
  const s = useSupervision.getState();
  let inbox = s.inbox;
  let controls = s.controls;
  let revision = s.revision;
  for (const e of events) {
    if (e.type === "inboxUpdated")
      inbox = [e.item, ...inbox.filter((i) => i.id !== e.item.id)];
    if (e.type === "inboxResolved") {
      resolvedIds.add(e.id);
      inbox = inbox.filter((i) => i.id !== e.id);
    }
    if (e.type === "repoControlsChanged")
      controls = { ...controls, [e.repoId]: e.controls };
    if (
      e.type === "usage" ||
      e.type === "sessionState" ||
      e.type === "repoControlsChanged"
    )
      revision++;
  }
  if (inbox !== s.inbox || controls !== s.controls || revision !== s.revision)
    useSupervision.setState({ inbox, controls, revision });
}

export async function dismissInbox(id: string) {
  await api.dismissInbox(id);
  resolvedIds.add(id);
  useSupervision.setState((s) => ({
    inbox: s.inbox.filter((i) => i.id !== id),
  }));
}
const resolvedIds = new Set<string>();
