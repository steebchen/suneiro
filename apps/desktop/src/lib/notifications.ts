import { api } from "./api";

let permission: Promise<boolean> | null = null;
export async function notifyOnce(id: string, title: string, body: string) {
  if (!("__TAURI_INTERNALS__" in window) || location.protocol === "http:")
    return;
  try {
    const n = await import("@tauri-apps/plugin-notification");
    permission ??= n
      .isPermissionGranted()
      .then(
        async (granted) =>
          granted || (await n.requestPermission()) === "granted",
      );
    if ((await permission) && (await api.claimNotification(id)))
      n.sendNotification({ title, body });
  } catch {
    // OS delivery is best effort; claims are durable and at-most-once.
  }
}
