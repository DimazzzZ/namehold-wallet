import { useEffect, useState, type RefObject } from "react";

/** Below this a panel is too cramped to read; it may then run past the fold. */
const MIN_HEIGHT = 280;

/** The nearest ancestor that scrolls vertically, or null. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const o = getComputedStyle(p).overflowY;
    if (o === "auto" || o === "scroll") return p;
  }
  return null;
}

/**
 * How tall the sticky element `ref` may be to end on screen: from its top
 * (the area's top padding edge once it sticks there) down to the bottom
 * padding edge of the area it scrolls in. Kept current as that area scrolls
 * and resizes; null until measured, or where nothing scrolls.
 */
export function useScrollportHeight(ref: RefObject<HTMLElement | null>): number | null {
  const [height, setHeight] = useState<number | null>(null);
  useEffect(() => {
    const el = ref.current;
    const port = el && scrollParent(el);
    if (!el || !port) return;
    const measure = () => {
      if (port.clientHeight === 0) return setHeight(null);
      const cs = getComputedStyle(port);
      const box = port.getBoundingClientRect();
      const top = Math.max(
        el.getBoundingClientRect().top,
        box.top + (parseFloat(cs.paddingTop) || 0),
      );
      const bottom = box.top + port.clientHeight - (parseFloat(cs.paddingBottom) || 0);
      setHeight(Math.max(MIN_HEIGHT, Math.round(bottom - top)));
    };
    measure();
    port.addEventListener("scroll", measure, { passive: true });
    window.addEventListener("resize", measure);
    const ro = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    ro?.observe(port);
    return () => {
      port.removeEventListener("scroll", measure);
      window.removeEventListener("resize", measure);
      ro?.disconnect();
    };
  }, [ref]);
  return height;
}
