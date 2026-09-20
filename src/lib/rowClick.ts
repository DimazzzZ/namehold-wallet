import type { MouseEvent } from "react";

/**
 * Selector for the things a click can legitimately land on that already have
 * their own meaning. Not a style choice — each of these either runs a handler
 * or changes state on its own.
 */
const INTERACTIVE = "button, a, input, select, textarea, label, [role='button']";

/**
 * True when a click inside a clickable row came from a control that handles it
 * itself — a cell's button, a link, the select checkbox.
 *
 * A row-level `onClick` sees every click in the row, including those already
 * handled by something inside it, because the event bubbles. Pressing a block
 * height in Owned Names ran the cell's handler AND the row's, and two dialogs
 * opened on top of each other.
 *
 * Deciding this once at the row is deliberate: the alternative is
 * `stopPropagation` in every inner handler, which is a rule the next button
 * added to the row has to remember, and the failure is silent.
 */
export function fromInteractiveChild(e: MouseEvent): boolean {
  const target = e.target as HTMLElement | null;
  return Boolean(target?.closest(INTERACTIVE));
}
