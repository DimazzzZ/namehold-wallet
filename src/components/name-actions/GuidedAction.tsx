import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { CopyField } from "../ui/CopyField";
import { BidForm } from "./BidForm";
import { DnsRecordsEditor } from "./DnsRecordsEditor";
import { formatCountdown } from "../../lib/auction";
import { formatHns } from "../../lib/utils";
import { openExternal, explorerTxUrl } from "../../lib/openExternal";
import type {
  AuctionPhaseGuide,
  AuctionTaskSummary,
  PhaseBadge,
  PhaseCountdown,
} from "../../lib/auction";
import type { DnsRow } from "../../lib/dnsRecords";
import type { NameActionCapabilities, NameActionCapability } from "../../types";

/**
 * The task-driven guided panel of `NameActionsModal` (Task 13 / F6
 * extraction): dispatches on the auction phase (+ capability taskState for
 * CLOSED) and renders the single most relevant action for the name. All
 * state and the build→sign→broadcast runner stay in the orchestrator; this
 * component only receives values and callbacks.
 */
export interface GuidedActionProps {
  badge: PhaseBadge;
  guide: AuctionPhaseGuide | null;
  countdown: PhaseCountdown | null;
  caps: NameActionCapabilities | null | undefined;
  summary: AuctionTaskSummary | null;
  busy: string | null;
  actionDisabled: (actionKey: string, cap?: NameActionCapability) => boolean;
  actionReason: (cap?: NameActionCapability) => string | null;
  // Simple actions.
  onOpen: () => void;
  onRedeem: () => void;
  onRegister: () => void;
  // Bid form (shared BidForm component; state owned by the orchestrator).
  bidHns: string;
  onBidChange: (value: string) => void;
  lockupHns: string;
  onLockupChange: (value: string) => void;
  bidError: string | null;
  lockupError: string | null;
  bidFormValid: boolean;
  forfeitLockupText: string;
  onBid: () => void;
  // Recover-bid widget (REVEAL phase, missing commitment).
  recoverHns: string;
  onRecoverHnsChange: (value: string) => void;
  onRecoverBid: () => void;
  // Reveal confirm-panel + pending-card wiring (this PR).
  /** True while the user has clicked Reveal but not yet confirmed — shows the
   *  confirm panel (with the bid amount) instead of the Reveal button. */
  revealConfirming: boolean;
  onRevealConfirmStart: () => void;
  onRevealConfirmCancel: () => void;
  onRevealConfirm: () => void;
  /** Fine-grained reveal substate label — "Unlocking…", "Signing…",
   *  "Broadcasting…", or null when idle. Feeds the button label. */
  revealSubstate: string | null;
  /** Fresh reveal txid captured optimistically at broadcast, so the pending
   *  card shows even before the capabilities poll catches up. */
  optimisticRevealTxid: string | null;
  // DNS rows for the guided REGISTER form (state owned by the orchestrator).
  rows: DnsRow[];
  onRowChange: (index: number, patch: Partial<DnsRow>) => void;
  onAddRow: () => void;
  onRemoveRow: (index: number) => void;
}

export function GuidedAction({
  badge,
  guide,
  countdown,
  caps,
  summary,
  busy,
  actionDisabled,
  actionReason,
  onOpen,
  onRedeem,
  onRegister,
  bidHns,
  onBidChange,
  lockupHns,
  onLockupChange,
  bidError,
  lockupError,
  bidFormValid,
  forfeitLockupText,
  onBid,
  recoverHns,
  onRecoverHnsChange,
  onRecoverBid,
  revealConfirming,
  onRevealConfirmStart,
  onRevealConfirmCancel,
  onRevealConfirm,
  revealSubstate,
  optimisticRevealTxid,
  rows,
  onRowChange,
  onAddRow,
  onRemoveRow,
}: GuidedActionProps) {
  if (!guide) return null;

  switch (badge.phase) {
    case "AVAILABLE":
      return (
        <div className="space-y-2">
          <div className="text-sm text-gray-700">{guide.description}</div>
          {actionReason(caps?.canOpen) && (
            <div className="text-xs text-amber-700 bg-amber-50 border border-amber-200 rounded p-2">
              {actionReason(caps?.canOpen)}
            </div>
          )}
          <Button
            variant="primary"
            disabled={actionDisabled("OPEN", caps?.canOpen)}
            onClick={onOpen}
          >
            {busy === "OPEN" ? "Opening…" : guide.action}
          </Button>
        </div>
      );

    case "OPENING":
      return (
        <div className="text-sm text-gray-600">
          {guide.description}
          {countdown && (
            <div className="mt-1 font-medium">
              {countdown.label} {formatCountdown(countdown)}
            </div>
          )}
        </div>
      );

    case "BIDDING":
      return (
        <BidForm
          variant="guided"
          bidHns={bidHns}
          onBidChange={onBidChange}
          lockupHns={lockupHns}
          onLockupChange={onLockupChange}
          bidError={bidError}
          lockupError={lockupError}
          forfeitLockupText={forfeitLockupText}
          disabled={actionDisabled("BID", caps?.canBid) || !bidFormValid}
          busy={busy === "BID"}
          onSubmit={onBid}
          idleLabel={guide.action}
          busyLabel="Placing bid…"
          description={guide.description}
          reasonBanner={
            actionReason(caps?.canBid) && (
              <div className="text-xs text-amber-700 bg-amber-50 border border-amber-200 rounded p-2">
                {actionReason(caps?.canBid)}
              </div>
            )
          }
        />
      );

    case "REVEAL":
      // Reveal has three UI states before the auction closes:
      //   1. In-flight / done   — a pending-confirmation or done card (no button)
      //   2. Confirm panel      — user clicked Reveal, hasn't confirmed yet
      //   3. Default            — the Reveal button (+ Recover-bid widget)
      // The card path is chosen from BOTH the derived taskState (surviving
      // reload / cross-device) AND our local optimistic revealTxid (so a
      // fresh broadcast shows the card immediately, before the caps poll).
      {
        const showCard =
          !!optimisticRevealTxid ||
          caps?.taskState === "revealBroadcastPending" ||
          caps?.taskState === "revealDoneWaitingForClose";
        if (showCard) {
          const isConfirmed = caps?.taskState === "revealDoneWaitingForClose";
          const txid = optimisticRevealTxid ?? caps?.revealTxid ?? null;
          return (
            <div
              className="space-y-2 rounded border p-3"
              data-testid="reveal-status-card"
            >
              <div
                className={
                  isConfirmed
                    ? "text-sm font-medium text-gray-800"
                    : "text-sm font-medium text-blue-800"
                }
              >
                {isConfirmed
                  ? "Your reveal is confirmed."
                  : "Your reveal is broadcast and waiting to confirm."}
              </div>
              <div className="text-xs text-gray-600">
                {isConfirmed
                  ? "The auction stays open for other reveals until the window closes."
                  : "Confirmation usually takes about ten minutes. It's safe to close this modal — you'll get a toast when it lands."}
              </div>
              {txid && (
                <div className="text-xs">
                  <CopyField
                    label={
                      <span className="text-gray-600">Reveal txid</span>
                    }
                    value={txid}
                    display={`${txid.slice(0, 10)}…${txid.slice(-6)}`}
                    toastLabel="Reveal txid"
                    valueTestId="reveal-txid"
                  />
                  <button
                    type="button"
                    className="mt-1 inline-block text-blue-600 hover:underline cursor-pointer text-xs"
                    onClick={() => openExternal(explorerTxUrl(txid))}
                    data-testid="reveal-explorer-link"
                  >
                    View on explorer ↗
                  </button>
                </div>
              )}
              {/* Two-clocks copy: reveal confirmation vs auction close. */}
              {countdown && !isConfirmed && (
                <div className="text-xs text-gray-500">
                  Auction closes in {formatCountdown(countdown)} (separate from
                  this reveal's confirmation).
                </div>
              )}
              {countdown && isConfirmed && (
                <div className="text-xs text-gray-500">
                  Auction closes in {formatCountdown(countdown)}.
                </div>
              )}
            </div>
          );
        }

        if (revealConfirming) {
          const bidText =
            caps?.bidValueDoos != null ? formatHns(caps.bidValueDoos) : null;
          const substate = revealSubstate;
          return (
            <div
              className="space-y-2 rounded border border-blue-200 bg-blue-50 p-3"
              data-testid="reveal-confirm-panel"
            >
              <div className="text-sm font-medium text-blue-900">
                Confirm reveal
              </div>
              <div className="text-xs text-blue-900">
                You're about to reveal
                {bidText ? (
                  <>
                    {" "}
                    a bid of <b>{bidText}</b>
                  </>
                ) : (
                  " your bid"
                )}{" "}
                for <b>{caps?.name ?? "this name"}</b>. Once broadcast this
                cannot be undone.
              </div>
              <div className="flex gap-2">
                <Button
                  variant="primary"
                  disabled={!!substate}
                  onClick={onRevealConfirm}
                  data-testid="reveal-confirm-broadcast"
                >
                  {substate ?? "Confirm & reveal"}
                </Button>
                <Button
                  variant="secondary"
                  disabled={!!substate}
                  onClick={onRevealConfirmCancel}
                >
                  Cancel
                </Button>
              </div>
            </div>
          );
        }

        return (
          <div className="space-y-2">
            <div className="text-sm text-gray-700">{guide.description}</div>
            {actionReason(caps?.canReveal) && (
              <div className="text-xs text-amber-700 bg-amber-50 border border-amber-200 rounded p-2">
                {actionReason(caps?.canReveal)}
              </div>
            )}
            {caps && !caps.hasBidCommitment && (
              <div
                className="rounded border border-gray-200 bg-gray-50 p-2 space-y-2"
                data-testid="recover-bid"
              >
                <div className="text-xs text-gray-600">
                  Lost your bid commitment? If you remember the exact amount
                  you bid, you can recover it from the on-chain bid coin.
                </div>
                <div className="flex items-end gap-2">
                  <div className="flex-1">
                    <Input
                      label="Your bid amount (HNS)"
                      value={recoverHns}
                      onChange={(e) => onRecoverHnsChange(e.target.value)}
                      placeholder="10.0"
                      type="number"
                      step="0.000001"
                    />
                  </div>
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={!recoverHns || busy === "RECOVER"}
                    onClick={onRecoverBid}
                  >
                    {busy === "RECOVER" ? "Recovering…" : "Recover bid"}
                  </Button>
                </div>
              </div>
            )}
            <div className="flex gap-2">
              <Button
                variant="primary"
                disabled={actionDisabled("REVEAL", caps?.canReveal)}
                onClick={onRevealConfirmStart}
                data-testid="reveal-primary"
              >
                {guide.action}
              </Button>
              <Button
                variant="secondary"
                disabled={actionDisabled("REDEEM", caps?.canRedeem)}
                onClick={onRedeem}
              >
                {busy === "REDEEM" ? "…" : "Redeem (lost bid)"}
              </Button>
            </div>
            {summary?.urgency && (
              <div className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2 mt-2">
                {summary.urgency}
              </div>
            )}
          </div>
        );
      }

    case "CLOSED":
      // Task-driven guidance: the appropriate copy depends on capability state.
      if (caps?.taskState === "wonNeedsRegister") {
        return (
          <div className="space-y-3">
            <div className="text-sm text-green-800">
              {caps.nextActionReason ?? "You won the auction! Register the name to finalize ownership."}
            </div>
            {actionReason(caps?.canRegister) && (
              <div className="text-xs text-amber-700 bg-amber-50 border border-amber-200 rounded p-2">
                {actionReason(caps?.canRegister)}
              </div>
            )}
            <DnsRecordsEditor
              variant="guided"
              rows={rows}
              onRowChange={onRowChange}
              onAddRow={onAddRow}
              onRemoveRow={onRemoveRow}
            />
            <Button
              variant="primary"
              disabled={actionDisabled("REGISTER", caps?.canRegister)}
              onClick={onRegister}
            >
              {busy === "REGISTER" ? "Registering…" : "Register"}
            </Button>
          </div>
        );
      }
      if (caps?.taskState === "lostNeedsRedeem") {
        return (
          <div className="space-y-2">
            <div className="text-sm text-red-800">
              {caps.nextActionReason ?? "Your bid lost. Redeem your reveal coin to reclaim the funds."}
            </div>
            <Button
              variant="primary"
              disabled={actionDisabled("REDEEM", caps?.canRedeem)}
              onClick={onRedeem}
            >
              {busy === "REDEEM" ? "…" : "Redeem"}
            </Button>
          </div>
        );
      }
      // Owned registered name — no auction urgency, just manage.
      if (caps?.ownsName) {
        // Explorer-confirmed ownership but no local node-synced owner coin:
        // every spend-capable action (register/update/transfer/finalize/
        // cancelTransfer/renew/revoke) is force-disabled by the backend.
        // Surface that reason here so the user isn't left staring at
        // buttons that quietly do nothing.
        const ownerCoinNotSynced = caps.hasOwnerCoin === false;
        const notSyncedReason =
          caps.canUpdate?.reason ?? caps.canRenew?.reason ?? caps.canTransfer?.reason ?? null;
        return (
          <div className="space-y-2">
            <div className="text-sm text-gray-700">
              You own this name. Use the controls below to update DNS, transfer, or renew.
            </div>
            {ownerCoinNotSynced && (
              <div
                className="text-xs text-amber-700 bg-amber-50 border border-amber-200 rounded p-2"
                data-testid="owner-coin-not-synced"
              >
                This name is yours, but its owner coin hasn&apos;t synced locally yet.{" "}
                {notSyncedReason ?? "Connect a node and Refresh to manage it."}
              </div>
            )}
          </div>
        );
      }
      // Third-party CLOSED name — not owned by us.
      return (
        <div className="space-y-2">
          <div className="text-sm text-gray-700">
            This name is already registered by another wallet. No actions are available
            for you on this name.
          </div>
        </div>
      );

    default:
      return null;
  }
}
