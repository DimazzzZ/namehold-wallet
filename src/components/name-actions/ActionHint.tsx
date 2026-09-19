import type { ReactNode } from "react";

type Props = {
  /** Why the wrapped action is unavailable, or `null`/empty for no hint. */
  reason: string | null | undefined;
  children: ReactNode;
};

/**
 * Wraps an action button so its "why is this disabled" hint is actually
 * reachable.
 *
 * `Button` carries `disabled:pointer-events-none`, so a disabled button
 * receives no pointer events and the browser never shows its `title` — the
 * tooltip went dead exactly when it had something to say. Hanging the title on
 * a wrapper the pointer CAN reach restores it, without giving disabled buttons
 * hover styling or click handling.
 */
export function ActionHint({ reason, children }: Props) {
  if (!reason) return <>{children}</>;
  return (
    <span className="inline-flex" title={reason}>
      {children}
    </span>
  );
}
