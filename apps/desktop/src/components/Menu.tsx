import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

export type MenuItem = { label: string; onSelect: () => void };

/** Minimal dropdown: click to open, click outside or Escape to close.
 *  The list is portaled to <body> so scrolling/clipping containers (e.g. the tab bar) can't hide it. */
export function Menu({
  label,
  items,
  align = "right",
  empty = "Nothing here",
}: {
  label: ReactNode;
  items: MenuItem[];
  align?: "left" | "right";
  empty?: string;
}) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left?: number; right?: number }>();
  const ref = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (!open || !ref.current) return;
    const place = () => {
      const r = ref.current!.getBoundingClientRect();
      setPos(align === "right" ? { top: r.bottom + 4, right: window.innerWidth - r.right } : { top: r.bottom + 4, left: r.left });
    };
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [open, align]);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent) {
        if (e.key === "Escape") setOpen(false);
        return;
      }
      const t = e.target as Node;
      if (!ref.current?.contains(t) && !listRef.current?.contains(t)) setOpen(false);
    };
    // Scrolling the anchor's container would detach the fixed list from its button.
    const onScroll = (e: Event) => {
      if (!listRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative shrink-0">
      <button onClick={() => setOpen((o) => !o)} className="rounded px-1.5 py-1 text-xs text-muted hover:bg-hover hover:text-fg">
        {label}
      </button>
      {open &&
        pos &&
        createPortal(
          <div
            ref={listRef}
            style={pos}
            className="fixed z-50 max-h-[70vh] min-w-40 overflow-y-auto rounded-md border border-border bg-elevated p-1 shadow-xl"
          >
            {items.length === 0 && <div className="px-2 py-1.5 text-[13px] text-muted">{empty}</div>}
            {items.map((item) => (
              <button
                key={item.label}
                onClick={() => {
                  setOpen(false);
                  item.onSelect();
                }}
                className="block w-full rounded px-2 py-1.5 text-left text-[13px] hover:bg-hover"
              >
                {item.label}
              </button>
            ))}
          </div>,
          document.body,
        )}
    </div>
  );
}
