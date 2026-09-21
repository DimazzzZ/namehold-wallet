import type { ReactNode } from "react";
import { Tooltip } from "../ui/Tooltip";

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
 * receives no pointer events and a native `title` on it is never shown — the
 * tooltip went dead exactly when it had something to say. [`Tooltip`] listens
 * on a wrapper the pointer can still reach, and renders nothing extra when
 * `reason` is empty.
 */
export function ActionHint({ reason, children }: Props) {
  return <Tooltip content={reason}>{children}</Tooltip>;
}
