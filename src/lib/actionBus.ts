/**
 * Lightweight typed action bus for route-scoped keyboard shortcuts.
 *
 * The global keyboard hook (useAppHotkeys) decides *when* an action should
 * fire (based on the active route + focus/modal rules) and dispatches an
 * `ActionId`. The view that owns the actual handler subscribes and *runs* it.
 *
 * This keeps view-local handlers (Send, Sync, focus lookup, …) where they
 * live — no lifting into a store/context — while still making them reachable
 * from a single global hook and the command palette.
 *
 * Mechanism: a `CustomEvent` on `window`. Fire-and-forget, so there's no
 * "pending action" state to clear and no cross-render race. Each view filters
 * to its own namespace (`view:verb`) in its handler.
 */

/** Namespaced action identifiers: "view:verb". */
export type ActionId =
  | "wallet:send"
  | "wallet:sync"
  | "wallet:toggleLock"
  | "wallet:toggleQr"
  | "wallet:focusFilter"
  | "wallet:list:next"
  | "wallet:list:prev"
  | "wallet:list:open"
  | "wallet:list:clear"
  | "auctions:focusLookup"
  | "auctions:batchBid"
  | "watchlist:focusAdd"
  | "watchlist:exportCsv"
  | "activity:focusSearch";

export interface ActionEventDetail {
  actionId: ActionId;
}

/** Event name used for all action-bus dispatches. Exported for tests. */
export const ACTION_EVENT_NAME = "namehold:action";

/**
 * Dispatch an action. Called by useAppHotkeys or the command palette.
 * Synchronous — listeners run before this returns.
 */
export function dispatchAction(actionId: ActionId): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(
    new CustomEvent<ActionEventDetail>(ACTION_EVENT_NAME, {
      detail: { actionId },
    }),
  );
}

/**
 * Subscribe to action events. Returns an unsubscribe function.
 *
 * Typically wired inside a view's `useEffect`. Because the handler often
 * closes over frequently-changing state (e.g. a filtered list), prefer the
 * ref-indirection pattern so you register a stable listener once instead of
 * re-subscribing on every keystroke:
 *
 *   const handlerRef = useRef(handler);
 *   handlerRef.current = handler;
 *   useEffect(() => subscribeAction((id) => handlerRef.current(id)), []);
 */
export function subscribeAction(
  handler: (actionId: ActionId) => void,
): () => void {
  if (typeof window === "undefined") return () => {};
  const listener = (e: Event) => {
    const detail = (e as CustomEvent<ActionEventDetail>).detail;
    if (detail?.actionId) handler(detail.actionId);
  };
  window.addEventListener(ACTION_EVENT_NAME, listener);
  return () => window.removeEventListener(ACTION_EVENT_NAME, listener);
}
