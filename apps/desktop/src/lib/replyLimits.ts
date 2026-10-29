import type { RepoControls } from "./api";

export function replyLimitsActive(controls: RepoControls | null | undefined): boolean {
  return !!controls && (controls.paused || controls.stopUsd !== null || controls.warningUsd !== null);
}

export function allowRepliesControls(controls: RepoControls): RepoControls {
  return { ...controls, paused: false, stopUsd: null, warningUsd: null };
}
