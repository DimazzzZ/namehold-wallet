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
 * on a wrapper the pointer can still reach, and shows nothing when `reason` is
 * empty — the wrapper itself is always rendered, for the reason its own code
 * gives.
 *
 * It is a thin wrapper on purpose. What it adds is the name: at a call site
 * `reason` says this is a capability's refusal, where `content` would say only
 * that some text exists.
 */
export function ActionHint({ reason, children }: Props) {
  return <Tooltip content={reason}>{children}</Tooltip>;
}
