// Uncaught UI errors -> Rust (`report_error`) -> PostHog Error Tracking.
// Rust decides whether anything is sent (release build + Settings → Send error
// reports) and masks the home folder; this file only shapes and throttles.

/** `chunk_id` names the bundle's uploaded source map (see `chunkIds`). */
export type StackFrame = { function?: string; filename?: string; lineno?: number; colno?: number; chunk_id?: string };
export type WebError = { mechanism: "error" | "unhandledrejection"; name: string; message: string; frames: StackFrame[] };

const MAX_REPORTS = 20;

/** Parse WebKit (`fn@url:1:2`) and V8 (`at fn (url:1:2)`) stacks, oldest call first. */
export function parseStack(stack: string | undefined): StackFrame[] {
  if (!stack) return [];
  const frames: StackFrame[] = [];
  for (const raw of stack.split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    const v8 = /^at (?:(.+?) \()?(.+?):(\d+):(\d+)\)?$/.exec(line);
    const webkit = !v8 && /^(?:(.*?)@)?(.+?):(\d+):(\d+)$/.exec(line);
    const m = v8 || webkit;
    if (!m) continue;
    frames.push({ function: m[1] || undefined, filename: m[2], lineno: Number(m[3]), colno: Number(m[4]) });
  }
  return frames.reverse();
}

/** Turn whatever was thrown into a report; `null` for nothing worth sending. */
export function toWebError(value: unknown, mechanism: WebError["mechanism"]): WebError | null {
  if (value instanceof Error) {
    return { mechanism, name: value.name || "Error", message: value.message, frames: parseStack(value.stack) };
  }
  if (value === undefined || value === null) return null;
  const message = typeof value === "string" ? value : safeJson(value);
  return { mechanism, name: mechanism === "unhandledrejection" ? "UnhandledRejection" : "Error", message, frames: [] };
}

/**
 * Release builds run `posthog-cli sourcemap inject`, which makes every bundle
 * record `_posthogChunkIds[<stack captured inside the bundle>] = <chunk ID>`.
 * Map each bundle URL to its chunk ID so PostHog can find the uploaded source map.
 */
export function chunkIds(registry: unknown = (globalThis as { _posthogChunkIds?: unknown })._posthogChunkIds): Map<string, string> {
  const map = new Map<string, string>();
  if (!registry || typeof registry !== "object") return map;
  for (const [stack, id] of Object.entries(registry)) {
    const file = parseStack(stack).at(-1)?.filename;
    if (file && typeof id === "string") map.set(file, id);
  }
  return map;
}

export function withChunkIds(error: WebError, ids: Map<string, string>): WebError {
  if (!ids.size) return error;
  return { ...error, frames: error.frames.map((f) => (f.filename && ids.has(f.filename) ? { ...f, chunk_id: ids.get(f.filename) } : f)) };
}

function safeJson(value: unknown) {
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

/** Forward window errors and unhandled rejections; duplicates and floods are dropped. */
export function installErrorReporting(send: (error: WebError) => Promise<unknown>, target: Pick<Window, "addEventListener"> = window) {
  const seen = new Set<string>();
  const report = (error: WebError | null) => {
    if (!error || seen.size >= MAX_REPORTS) return;
    const top = error.frames[error.frames.length - 1];
    const key = `${error.name}:${error.message}:${top?.filename}:${top?.lineno}`;
    if (seen.has(key)) return;
    seen.add(key);
    // Never let reporting itself raise another error event.
    send(withChunkIds(error, chunkIds())).catch(() => {});
  };
  target.addEventListener("error", (e: ErrorEvent) => {
    // Resource load failures (img/script) have no `error` and aren't app bugs.
    if (e.error === undefined && !e.message) return;
    report(toWebError(e.error ?? e.message, "error"));
  });
  target.addEventListener("unhandledrejection", (e: PromiseRejectionEvent) => report(toWebError(e.reason, "unhandledrejection")));
}
