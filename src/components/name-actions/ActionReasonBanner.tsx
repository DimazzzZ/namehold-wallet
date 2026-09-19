import { UnlockButton } from "../UnlockButton";

type Props = {
  /** Why the action is unavailable, or `null` to render nothing. */
  reason: string | null;
};

/**
 * The amber "why you can't do this" banner inside a guided action panel.
 *
 * It carries an Unlock button on the right so a locked wallet can be unlocked
 * where the notice appears, matching the wallet page and the modal's own
 * write-capability gate. [`UnlockButton`] hides itself when the signer is
 * already unlocked, so a purely capability-based reason ("not in REVEAL yet")
 * renders as text alone — the button only shows up for the reason it can fix.
 */
export function ActionReasonBanner({ reason }: Props) {
  if (!reason) return null;
  return (
    <div
      className="flex items-center justify-between gap-2 text-xs text-amber-700 bg-amber-50 border border-amber-200 rounded p-2"
      data-testid="action-reason"
    >
      <span>{reason}</span>
      <UnlockButton size="sm" variant="primary" />
    </div>
  );
}
