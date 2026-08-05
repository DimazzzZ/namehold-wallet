import { useState } from "react";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import type { NameActionCapabilities, NameActionCapability } from "../../types";

/**
 * The "Ownership" section of the advanced actions block: recipient input +
 * Transfer / Finalize / Cancel transfer / Renew / Revoke buttons (Task 13 /
 * F6 extraction from `NameActionsModal`). All state (recipient, busy) and
 * the mutation runner stay in the orchestrator and flow down as props.
 *
 * `onBuyWithPayment` is the paid name swap flow: buyer finalizes a TRANSFER
 * and pays the seller in the same transaction. Only shown when the name is
 * in TRANSFER state (taskState === "transferPendingFinalize").
 */
export interface OwnershipActionsProps {
  caps: NameActionCapabilities | null | undefined;
  busy: string | null;
  recipient: string;
  onRecipientChange: (value: string) => void;
  actionDisabled: (actionKey: string, cap?: NameActionCapability) => boolean;
  actionReason: (cap?: NameActionCapability) => string | null;
  onTransfer: () => void;
  onFinalize: () => void;
  onCancelTransfer: () => void;
  onRenew: () => void;
  onRevoke: () => void;
  onBuyWithPayment?: (paymentAddress: string, paymentValue: number) => void;
}

export function OwnershipActions({
  caps,
  busy,
  recipient,
  onRecipientChange,
  actionDisabled,
  actionReason,
  onTransfer,
  onFinalize,
  onCancelTransfer,
  onRenew,
  onRevoke,
  onBuyWithPayment,
}: OwnershipActionsProps) {
  // Paid swap: show "Buy with payment" button + payment address input when
  // the name is in TRANSFER state (transferPendingFinalize).
  const canFinalize = caps?.canFinalize;
  const [showPayForm, setShowPayForm] = useState(false);
  const [payAddr, setPayAddr] = useState("");
  const [payAmount, setPayAmount] = useState("");

  const handleBuy = () => {
    const amount = parseFloat(payAmount);
    if (!payAddr.trim() || isNaN(amount) || amount <= 0) return;
    // Convert HNS to dollarydoos (1 HNS = 1,000,000 dollarydoos)
    const doos = Math.round(amount * 1_000_000);
    onBuyWithPayment?.(payAddr.trim(), doos);
    setShowPayForm(false);
    setPayAddr("");
    setPayAmount("");
  };

  return (
    <section className="space-y-2">
      <div className="font-medium text-gray-700">Ownership</div>
      <Input label="Transfer to address" value={recipient} onChange={(e) => onRecipientChange(e.target.value)} placeholder="hs1q… / rs1q…" />
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="danger"
          disabled={actionDisabled("TRANSFER", caps?.canTransfer) || !recipient.trim()}
          title={actionReason(caps?.canTransfer) ?? ""}
          onClick={onTransfer}
        >
          {busy === "TRANSFER" ? "…" : "Transfer"}
        </Button>
        <Button
          size="sm"
          disabled={actionDisabled("FINALIZE", caps?.canFinalize)}
          title={actionReason(caps?.canFinalize) ?? ""}
          onClick={onFinalize}
        >
          {busy === "FINALIZE" ? "…" : "Finalize"}
        </Button>
        <Button
          size="sm"
          disabled={actionDisabled("CANCEL_TRANSFER", caps?.canCancelTransfer)}
          title={actionReason(caps?.canCancelTransfer) ?? ""}
          onClick={onCancelTransfer}
        >
          {busy === "CANCEL" ? "…" : "Cancel transfer"}
        </Button>
        <Button
          size="sm"
          disabled={actionDisabled("RENEW", caps?.canRenew)}
          title={actionReason(caps?.canRenew) ?? ""}
          onClick={onRenew}
        >
          {busy === "RENEW" ? "…" : "Renew"}
        </Button>
        <Button
          size="sm" variant="danger"
          disabled={actionDisabled("REVOKE", caps?.canRevoke)}
          title={actionReason(caps?.canRevoke) ?? ""}
          onClick={onRevoke}
        >
          {busy === "REVOKE" ? "…" : "Revoke"}
        </Button>
        {canFinalize && !actionDisabled("FINALIZE", canFinalize) && onBuyWithPayment && (
          <Button
            size="sm"
            variant="primary"
            disabled={busy === "FINALIZE_WITH_PAYMENT"}
            onClick={() => setShowPayForm((v) => !v)}
          >
            {busy === "FINALIZE_WITH_PAYMENT" ? "…" : "Buy with payment"}
          </Button>
        )}
      </div>
      {showPayForm && (
        <div className="space-y-2 pt-2 border-t border-gray-100">
          <Input
            label="Seller's payment address"
            value={payAddr}
            onChange={(e) => setPayAddr(e.target.value)}
            placeholder="hs1q… / rs1q…"
          />
          <Input
            label="Payment amount (HNS)"
            value={payAmount}
            onChange={(e) => setPayAmount(e.target.value)}
            placeholder="0.00"
            type="number"
            step="0.000001"
            min="0"
          />
          <div className="flex gap-2">
            <Button
              size="sm"
              variant="primary"
              disabled={!payAddr.trim() || !payAmount || parseFloat(payAmount) <= 0}
              onClick={handleBuy}
            >
              Confirm & build draft
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setShowPayForm(false)}
            >
              Cancel
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}
