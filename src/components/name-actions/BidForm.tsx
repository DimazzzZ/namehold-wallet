import type { ReactNode } from "react";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";

/**
 * The bid + lockup input pair, forfeit warning, and submit button — used by
 * both the guided BIDDING panel and the "advanced" auction section (Task 13
 * / F6: this used to be duplicated JSX in `NameActionsModal`). The two call
 * sites render meaningfully different layouts (the guided panel has a
 * description + reason banner above the inputs and a full-width primary
 * button below; the advanced section is a single inline row with a small
 * secondary button), so `variant` switches between the two — but the
 * underlying input/error/forfeit-warning markup, and the validation +
 * submit wiring, is shared.
 */
export interface BidFormProps {
  variant: "guided" | "advanced";
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
  /** Guided variant only: the phase description shown above the inputs. */
  description?: ReactNode;
  /** Guided variant only: the "why is this disabled" banner. */
  reasonBanner?: ReactNode;
  /** Advanced variant only: the submit button's `title` (disabled reason). */
  submitTitle?: string;
}

export function BidForm({
  variant,
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
  submitTitle,
}: BidFormProps) {
  const bidErrorTestId = variant === "guided" ? "bid-error" : "bid-error-advanced";
  const lockupErrorTestId = variant === "guided" ? "lockup-error" : "lockup-error-advanced";
  const forfeitTestId =
    variant === "guided" ? "bid-forfeit-warning" : "bid-forfeit-warning-advanced";

  const bidField = (
    <div className={variant === "guided" ? "flex-1" : undefined}>
      <Input
        label="Bid (HNS)"
        value={bidHns}
        onChange={(e) => onBidChange(e.target.value)}
        placeholder="10.0"
        type="number"
        step="0.000001"
      />
      {bidError && (
        <div className="mt-1 text-xs text-red-600" data-testid={bidErrorTestId}>
          {bidError}
        </div>
      )}
    </div>
  );

  const lockupField = (
    <div className={variant === "guided" ? "flex-1" : undefined}>
      <Input
        label="Lockup (HNS)"
        value={lockupHns}
        onChange={(e) => onLockupChange(e.target.value)}
        placeholder="≥ bid"
        type="number"
        step="0.000001"
      />
      {lockupError && (
        <div className="mt-1 text-xs text-red-600" data-testid={lockupErrorTestId}>
          {lockupError}
        </div>
      )}
    </div>
  );

  const forfeitWarning = (
    <div
      className="text-xs text-amber-700 bg-amber-50 border border-amber-200 rounded p-2"
      data-testid={forfeitTestId}
    >
      If you don't reveal during the reveal window, the entire lockup ({forfeitLockupText} HNS)
      is forfeited.
    </div>
  );

  if (variant === "advanced") {
    return (
      <>
        <div className="flex items-end gap-2">
          {bidField}
          {lockupField}
          <Button
            size="sm"
            variant="secondary"
            disabled={disabled}
            title={submitTitle ?? ""}
            onClick={onSubmit}
          >
            {busy ? busyLabel : idleLabel}
          </Button>
        </div>
        {forfeitWarning}
      </>
    );
  }

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
