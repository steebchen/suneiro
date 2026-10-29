import { invoke } from "@tauri-apps/api/core";
import type { Session } from "./api";

export type ProfilePreference = { model: string | null; effort: string | null };
export type AgentProfile = { id: string; name: string; instructions: string; preferences: Record<string, ProfilePreference> };
export type ContextMessage = { index: number; role: string; text: string };
export type ContextSelection = { task: string; acceptanceCriteria: string; messages: { sessionId: string; index: number }[]; files: string[]; diffs: string[] };
export type ContextPackage = { id: string; workspaceId: string; agentId: string; profileName: string; model: string | null; effort: string | null; warnings: string[]; text: string; bytes: number };

export const profilesApi = {
  list: () => invoke<AgentProfile[]>("list_profiles"),
  save: (profiles: AgentProfile[]) => invoke<void>("save_profiles", { profiles }),
  messages: (workspaceId: string, sessionId: string) => invoke<ContextMessage[]>("context_messages", { workspaceId, sessionId }),
  prepare: (workspaceId: string, profileId: string, agentId: string, selection: ContextSelection) => invoke<ContextPackage>("prepare_context", { workspaceId, profileId, agentId, selection }),
  launch: (workspaceId: string, packageId: string) => invoke<Session>("launch_profile", { workspaceId, packageId }),
};
