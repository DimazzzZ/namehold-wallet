import type { ReactNode } from "react";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { ActionHint } from "./ActionHint";
import { RevokeConfirm } from "../name-page/RevokeConfirm";
import type { NameActionCapabilities, NameActionCapability } from "../../types";

/**
 * The name page's Transfer & sell cards for a name this wallet owns:
 * Transfer, Sell, Renew and the Danger zone (type-to-confirm Revoke). All
 * state (recipient, busy, the revoke box) and the mutation runner stay in the
 * name page and flow down as props.
 *
 * The paid-swap entry points were withdrawn: only the holder of the TRANSFER
 * coin can finalize, which is the sender, so the "buyer" side could never be
 * pressed by a buyer — and with every input signed SIGHASH_ALL nothing about
 * the flow was atomic. See docs/specs/2026-09-21-paid-name-swaps.md. Offers
 * recorded before that are listed read-only in Activity (`PaidSwapOffers`).
 */
export interface OwnershipActionsProps {
  name: string;
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
  /** Open the Lock dialog (Shakedex selling, R19). */
  onSell: () => void;
  /** Why this profile cannot sell (`marketText.ts::sellRefusal`), or null. */
  sellRefusal: string | null;
  transferPending: boolean;
  /** The Renew card's sentence (expiry as this network can tell it). */
  renewText: string;
  revokeOpen: boolean;
  onRevokeOpenChange: (open: boolean) => void;
}

function Card({
  title,
  aside,
  danger,
  children,
}: {
  title: ReactNode;
  aside?: ReactNode;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <section
      className={`bg-white border rounded-lg ${danger ? "border-red-200" : "border-gray-200"}`}
    >
      <div
        className={`flex items-center justify-between gap-2 px-4 py-3 border-b ${
          danger ? "border-red-100" : "border-gray-100"
        }`}
      >
        <h3 className={`text-sm font-semibold ${danger ? "text-red-800" : "text-gray-900"}`}>
          {title}
        </h3>
        {aside}
      </div>
      <div className="p-4 flex flex-col gap-3 text-[13px] text-gray-700">{children}</div>
    </section>
  );
}

export function OwnershipActions({
  name,
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
  onSell,
  sellRefusal,
  transferPending,
  renewText,
  revokeOpen,
  onRevokeOpenChange,
}: OwnershipActionsProps) {
  const revokeDisabled = actionDisabled("REVOKE", caps?.canRevoke);
  return (
    <>
      <Card
        title="Transfer to another wallet"
        aside={transferPending && <Badge variant="info">Transfer in progress</Badge>}
      >
        <Input
          label="Recipient address"
          value={recipient}
          onChange={(e) => onRecipientChange(e.target.value)}
          placeholder="hs1q… / rs1q…"
          className="font-mono"
        />
        <p className="text-xs text-gray-500">
          The name moves in two steps: send the transfer, then finalize it after the lockup. You can
          cancel until it&apos;s finalized.
        </p>
        <div className="flex flex-wrap gap-2">
          <ActionHint reason={actionReason(caps?.canTransfer)}>
            <Button
              size="sm"
              variant="secondary"
              disabled={actionDisabled("TRANSFER", caps?.canTransfer) || !recipient.trim()}
              onClick={onTransfer}
            >
              {busy === "TRANSFER" ? "…" : "Start transfer"}
            </Button>
          </ActionHint>
          <ActionHint reason={actionReason(caps?.canFinalize)}>
            <Button
              size="sm"
              variant="secondary"
              disabled={actionDisabled("FINALIZE", caps?.canFinalize)}
              onClick={onFinalize}
            >
              {busy === "FINALIZE" ? "…" : "Finalize"}
            </Button>
          </ActionHint>
          <ActionHint reason={actionReason(caps?.canCancelTransfer)}>
            <Button
              size="sm"
              variant="secondary"
              disabled={actionDisabled("CANCEL_TRANSFER", caps?.canCancelTransfer)}
              onClick={onCancelTransfer}
            >
              {busy === "CANCEL" ? "…" : "Cancel transfer"}
            </Button>
          </ActionHint>
        </div>
      </Card>

      <Card title="Sell on the market">
        <div className="flex items-center justify-between gap-3">
          <p>
            Lock the name for sale at one price or a price that falls step by step. Buyers&apos;
            nodes check the listing before they pay.
          </p>
          <ActionHint reason={sellRefusal ?? actionReason(caps?.canTransfer)}>
            <Button
              size="sm"
              variant="secondary"
              data-testid="name-sell"
              // A lock is a TRANSFER to the lock address: the same covenant
              // gate, R27's "locked for sale" included.
              disabled={sellRefusal !== null || actionDisabled("TRANSFER", caps?.canTransfer)}
              onClick={onSell}
            >
              Sell…
            </Button>
          </ActionHint>
        </div>
      </Card>

      <Card title="Renew">
        <div className="flex items-center justify-between gap-3">
          <p data-testid="renew-text">{renewText}</p>
          <ActionHint reason={actionReason(caps?.canRenew)}>
            <Button
              size="sm"
              variant="secondary"
              disabled={actionDisabled("RENEW", caps?.canRenew)}
              onClick={onRenew}
            >
              {busy === "RENEW" ? "…" : "Renew"}
            </Button>
          </ActionHint>
        </div>
      </Card>

      <Card title="Danger zone" danger>
        <div className="flex items-center justify-between gap-3">
          <p>
            <b>Revoke</b> burns the name for good: nobody, including you, can use it until it
            expires. Use it only if your key was stolen.
          </p>
          <ActionHint reason={actionReason(caps?.canRevoke)}>
            <button
              type="button"
              className="inline-flex items-center rounded border border-red-200 bg-white px-2.5 py-1 text-xs font-medium text-red-700 hover:bg-red-50 disabled:opacity-50 disabled:pointer-events-none"
              disabled={revokeDisabled}
              onClick={() => onRevokeOpenChange(true)}
              data-testid="revoke-open"
            >
              Revoke…
            </button>
          </ActionHint>
        </div>
        {revokeOpen && (
          <RevokeConfirm
            name={name}
            allowed={caps?.canRevoke?.allowed ?? false}
            busy={busy === "REVOKE"}
            onConfirm={onRevoke}
            onCancel={() => onRevokeOpenChange(false)}
          />
        )}
      </Card>
    </>
  );
}
