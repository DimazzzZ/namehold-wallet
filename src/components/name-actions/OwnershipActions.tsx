import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import type { NameActionCapabilities, NameActionCapability } from "../../types";

/**
 * The "Ownership" section of the advanced actions block: recipient input +
 * Transfer / Finalize / Cancel transfer / Renew / Revoke buttons (Task 13 /
 * F6 extraction from `NameActionsModal`). All state (recipient, busy) and
 * the mutation runner stay in the orchestrator and flow down as props.
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
      </div>
    </section>
  );
}
