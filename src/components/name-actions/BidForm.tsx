import type { ReactNode } from "react";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";

/**
 * The bid + lockup input pair, forfeit warning, and submit button of the
 * guided BIDDING panel on the name page. Its Bid is the page's one primary
 * button while bidding is open.
 */
export interface BidFormProps {
  bidHns: string;
  onBidChange: (value: string) => void;
  lockupHns: string;
  onLockupChange: (value: string) => void;
  bidError: string | null;
  lockupError: string | null;
  forfeitLockupText: string;
  disabled: boolean;
  busy: boolean;
  onSubmit: () => void;
  idleLabel: string;
  busyLabel: string;
  /** The phase description shown above the inputs. */
  description?: ReactNode;
  /** The "why is this disabled" banner. */
  reasonBanner?: ReactNode;
}

export function BidForm({
  bidHns,
  onBidChange,
  lockupHns,
  onLockupChange,
  bidError,
  lockupError,
  forfeitLockupText,
  disabled,
  busy,
  onSubmit,
  idleLabel,
  busyLabel,
  description,
  reasonBanner,
}: BidFormProps) {
  const bidField = (
    <div className="flex-1">
      <Input
        label="Bid (HNS)"
        value={bidHns}
        onChange={(e) => onBidChange(e.target.value)}
        placeholder="10.0"
        type="number"
        step="0.000001"
      />
      {bidError && (
        <div className="mt-1 text-xs text-red-600" data-testid="bid-error">
          {bidError}
        </div>
      )}
    </div>
  );

  const lockupField = (
    <div className="flex-1">
      <Input
        label="Lockup (HNS)"
        value={lockupHns}
        onChange={(e) => onLockupChange(e.target.value)}
        placeholder="≥ bid"
        type="number"
        step="0.000001"
      />
      {lockupError && (
        <div className="mt-1 text-xs text-red-600" data-testid="lockup-error">
          {lockupError}
        </div>
      )}
    </div>
  );

  const forfeitWarning = (
    <div
      className="text-xs text-amber-700 bg-amber-50 border border-amber-200 rounded p-2"
      data-testid="bid-forfeit-warning"
    >
      If you don't reveal during the reveal window, the entire lockup ({forfeitLockupText} HNS) is
      forfeited.
    </div>
  );

  return (
    <div className="space-y-3">
      <div className="text-sm text-gray-700">{description}</div>
      {reasonBanner}
      <div className="flex items-end gap-2">
        {bidField}
        {lockupField}
      </div>
      <div className="text-xs text-gray-500">
        Lockup must be ≥ bid. Excess is returned after reveal.
      </div>
      {forfeitWarning}
      <Button variant="primary" disabled={disabled} onClick={onSubmit}>
        {busy ? busyLabel : idleLabel}
      </Button>
    </div>
  );
}
