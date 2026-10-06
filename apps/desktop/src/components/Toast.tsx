import clsx from "clsx";
import { useStore } from "../lib/store";

export function Toast() {
  const toast = useStore((s) => s.toast);
  if (!toast) return null;
  return (
    <div
      onClick={() => useStore.setState({ toast: null })}
      className={clsx(
        "selectable max-w-md rounded-lg border px-3 py-2 whitespace-pre-wrap shadow-lg",
        toast.kind === "error" ? "border-del-fg/40 bg-elevated text-del-fg" : "border-border bg-elevated text-fg",
      )}
    >
      {toast.text}
    </div>
  );
}
