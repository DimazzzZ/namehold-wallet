import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { ActionHint } from "./ActionHint";
import type { NameActionCapabilities, NameActionCapability } from "../../types";

/**
 * The "Ownership" section of the advanced actions block: recipient input +
 * Transfer / Finalize / Cancel transfer / Renew / Revoke buttons (Task 13 /
 * F6 extraction from `NameActionsModal`). All state (recipient, busy) and
 * the mutation runner stay in the orchestrator and flow down as props.
 *
 * The paid-swap entry points were withdrawn: only the holder of the TRANSFER
 * coin can finalize, which is the sender, so the "buyer" side could never be
 * pressed by a buyer — and with every input signed SIGHASH_ALL nothing about
 * the flow was atomic. See docs/specs/2026-09-21-paid-name-swaps.md. Claiming
 * an offer already recorded still works, in `PaidSwapClaim`.
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
}: OwnershipActionsProps) {
  // Paid swap: show "Buy with payment" button + payment address input when
  // the name is in TRANSFER state (transferPendingFinalize).

  return (
    <section className="space-y-2">
      <div className="font-medium text-gray-700">Ownership</div>
      <Input
        label="Transfer to address"
        value={recipient}
        onChange={(e) => onRecipientChange(e.target.value)}
        placeholder="hs1q… / rs1q…"
      />
      <div className="flex flex-wrap gap-2">
        <ActionHint reason={actionReason(caps?.canTransfer)}>
          <Button
            size="sm"
            variant="danger"
            disabled={actionDisabled("TRANSFER", caps?.canTransfer) || !recipient.trim()}
            onClick={onTransfer}
          >
            {busy === "TRANSFER" ? "…" : "Transfer"}
          </Button>
        </ActionHint>
        <ActionHint reason={actionReason(caps?.canFinalize)}>
          <Button
            size="sm"
            disabled={actionDisabled("FINALIZE", caps?.canFinalize)}
            onClick={onFinalize}
          >
            {busy === "FINALIZE" ? "…" : "Finalize"}
          </Button>
        </ActionHint>
        <ActionHint reason={actionReason(caps?.canCancelTransfer)}>
          <Button
            size="sm"
            disabled={actionDisabled("CANCEL_TRANSFER", caps?.canCancelTransfer)}
            onClick={onCancelTransfer}
          >
            {busy === "CANCEL" ? "…" : "Cancel transfer"}
          </Button>
        </ActionHint>
        <ActionHint reason={actionReason(caps?.canRenew)}>
          <Button size="sm" disabled={actionDisabled("RENEW", caps?.canRenew)} onClick={onRenew}>
            {busy === "RENEW" ? "…" : "Renew"}
          </Button>
        </ActionHint>
        <ActionHint reason={actionReason(caps?.canRevoke)}>
          <Button
            size="sm"
            variant="danger"
            disabled={actionDisabled("REVOKE", caps?.canRevoke)}
            onClick={onRevoke}
          >
            {busy === "REVOKE" ? "…" : "Revoke"}
          </Button>
        </ActionHint>
      </div>
    </section>
  );
}
