import { useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { Button } from "../ui/Button";
import { Icon } from "../ui/Icon";
import { FeeRateOverride } from "../ui/FeeRateOverride";
import { ActionHint } from "../name-actions/ActionHint";
import { GuidedAction } from "../name-actions/GuidedAction";
import { NameBidsPanel } from "../name-actions/NameBidsPanel";
import { NameDetails } from "../name-actions/NameDetails";
import { UpcomingSection } from "../name-actions/UpcomingSection";
import { useNameBids } from "../../queries/read";
import { countdownForNetwork } from "../../lib/nameStatus";
import { redeemExplainer } from "../../lib/auction";
import { explorerCoversNetwork } from "../../lib/openExternal";
import { formatHns, formatHnsShort } from "../../lib/utils";
import type { NameTab } from "../../lib/nameRoute";
import type { NameActions } from "./useNameActions";

export const TAB_GRID = "grid gap-5 items-start lg:grid-cols-[minmax(0,1.55fr)_minmax(300px,1fr)]";

/** Overview: what to do next (guided panel), the auction, and the Details card. */
export function OverviewTab({ a, onTab }: { a: NameActions; onTab: (tab: NameTab) => void }) {
  const { name, caps, info, badge, guide, summary, busy, profile, sections, run, build } = a;
  const network = profile?.network;
  const countdown = a.countdown && countdownForNetwork(a.countdown, network);
  const [manualOpen, setManualOpen] = useState(false);
  const { data: bids } = useNameBids(name, profile?.id ?? null);
  const loaded = !a.isLoading && !a.isError;

  const guidedTitle =
    caps?.taskState === "ownedNoUrgentAction"
      ? "You own this name"
      : caps?.taskState === "expiringSoon"
        ? "Renew before it expires"
        : (summary?.nextActionLabel ?? guide?.title);

  const showGuided =
    loaded &&
    guide &&
    (badge.phase !== "CLOSED" ||
      caps?.ownsName ||
      caps?.taskState === "wonNeedsRegister" ||
      caps?.taskState === "lostNeedsRedeem" ||
      caps?.taskState === "lostUnrevealed" ||
      caps?.taskState === "closedResultUnknown");

  const bidCount = bids?.bids.filter((b) => !b.pending).length ?? 0;
  const auctionMeta =
    badge.phase === "CLOSED" && bids?.highest != null ? `Closed · ${bidCount} bids` : undefined;

  return (
    <div className={TAB_GRID}>
      <div className="flex flex-col gap-4 min-w-0">
        {caps?.transferPending && (
          <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 flex flex-col gap-2">
            <div className="font-medium text-blue-900">Transfer in progress</div>
            <div className="text-gray-700">
              Started at height {info?.transfer}. Once the transfer lockup is over, finalize it and
              the name leaves this wallet. Until then you can cancel.
            </div>
            <div className="flex items-center gap-3">
              <ActionHint reason={a.actionReason(caps.canCancelTransfer)}>
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={a.actionDisabled("CANCEL_TRANSFER", caps.canCancelTransfer)}
                  onClick={() => run("CANCEL", () => build.cancel.mutateAsync({ name }))}
                >
                  {busy === "CANCEL" ? "…" : "Cancel transfer"}
                </Button>
              </ActionHint>
              <button
                type="button"
                className="text-xs text-blue-600 hover:underline"
                onClick={() => onTab("transfer")}
              >
                Transfer &amp; sell tab
              </button>
            </div>
          </div>
        )}

        {showGuided ? (
          <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
            <div className="font-medium text-blue-900 mb-2">{guidedTitle}</div>
            <GuidedAction
              badge={badge}
              guide={guide}
              countdown={countdown}
              caps={caps}
              summary={summary}
              busy={busy}
              actionDisabled={a.actionDisabled}
              actionReason={a.guidedReason}
              onOpen={() => run("OPEN", () => build.open.mutateAsync({ name }))}
              onRedeem={() => run("REDEEM", () => build.redeem.mutateAsync({ name }))}
              onRegister={() => a.submitRecords("REGISTER")}
              bidHns={a.bidHns}
              onBidChange={a.setBidHns}
              lockupHns={a.lockupHns}
              onLockupChange={a.setLockupHns}
              bidError={a.bidValidation.bidError}
              lockupError={a.bidValidation.lockupError}
              bidFormValid={a.bidValidation.formValid}
              forfeitLockupText={a.forfeitLockupText}
              onBid={a.submitBid}
              recoverHns={a.recoverHns}
              onRecoverHnsChange={a.setRecoverHns}
              onRecoverBid={a.handleRecoverBid}
              onBruteForceRecover={a.handleBruteForceRecover}
              revealConfirming={a.revealConfirming}
              onRevealConfirmStart={() => a.setRevealConfirming(true)}
              onRevealConfirmCancel={() => a.setRevealConfirming(false)}
              onRevealConfirm={a.handleRevealConfirm}
              revealSubstate={a.revealSubstate}
              optimisticRevealTxid={a.optimisticRevealTxid}
              rows={a.rows}
              onRowChange={a.setRow}
              onAddRow={a.addRow}
              onRemoveRow={a.removeRow}
              isMainnet={explorerCoversNetwork(network)}
            />
            {badge.phase === "BIDDING" && caps?.canBid?.allowed ? (
              <div className="mt-3">
                <FeeRateOverride
                  value={a.bidFeeRate}
                  onChange={a.setBidFeeRate}
                  label="Fee rate override"
                />
              </div>
            ) : null}
          </div>
        ) : loaded && guide && badge.phase === "CLOSED" && !a.noTaskReason ? (
          <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
            <div className="font-medium text-blue-900 mb-2">Name details</div>
            <div className="text-sm text-gray-700">
              This name is already registered. No auction actions are needed for this name.
              <div className="mt-1 text-xs text-gray-500">
                Phase: {badge.label}
                {info?.value != null && ` · Value: ${formatHns(info.value)} HNS`}
              </div>
            </div>
          </div>
        ) : null}

        {loaded && a.noTaskReason && (
          <div
            className="text-sm text-gray-700 bg-gray-50 border border-gray-200 rounded p-3"
            data-testid="no-task-reason"
          >
            {a.noTaskReason}
          </div>
        )}

        {sections.auction.kind === "upcoming" && (
          <UpcomingSection
            id="auction"
            title="Manual auction actions"
            when={sections.auction.when}
          />
        )}
        {sections.auction.kind === "live" && (
          <section className="bg-white border border-gray-200 rounded-lg">
            <button
              type="button"
              className="w-full flex items-center gap-1.5 px-4 py-3 text-left font-medium text-gray-700"
              onClick={() => setManualOpen((o) => !o)}
              aria-expanded={manualOpen}
              data-testid="manual-auction-toggle"
            >
              <Icon icon={manualOpen ? ChevronDown : ChevronRight} size={14} />
              Manual auction actions
            </button>
            {manualOpen && (
              <div className="space-y-2 px-4 pb-4">
                <div className="text-xs text-gray-500">
                  The guided panel above already does this. Use these only if it has fallen out of
                  step with the chain.
                </div>
                {/* Only what this stage actually allows; each live one says
                    what pressing it does. */}
                {caps?.canOpen?.allowed && (
                  <div className="space-y-1">
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={a.actionDisabled("OPEN", caps?.canOpen)}
                      onClick={() => run("OPEN", () => build.open.mutateAsync({ name }))}
                    >
                      {busy === "OPEN" ? "…" : "Open"}
                    </Button>
                    <div className="text-xs text-gray-500">Start the auction for this name.</div>
                  </div>
                )}
                {caps?.canReveal?.allowed && (
                  <div className="space-y-1">
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={a.actionDisabled("REVEAL", caps?.canReveal)}
                      onClick={() => run("REVEAL", () => build.reveal.mutateAsync({ name }))}
                    >
                      {busy === "REVEAL" ? "…" : "Reveal"}
                    </Button>
                    <div className="text-xs text-gray-500">
                      Disclose what you bid. Every bid you placed on this name reveals together.
                    </div>
                  </div>
                )}
                {caps?.canRedeem?.allowed && (
                  <div className="space-y-1">
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={a.actionDisabled("REDEEM", caps?.canRedeem)}
                      onClick={() => run("REDEEM", () => build.redeem.mutateAsync({ name }))}
                    >
                      {busy === "REDEEM" ? "…" : "Redeem"}
                    </Button>
                    <div className="text-xs text-gray-500" data-testid="redeem-explainer">
                      {redeemExplainer(caps)}
                    </div>
                  </div>
                )}
              </div>
            )}
          </section>
        )}

        <NameBidsPanel
          name={name}
          profileId={profile?.id ?? null}
          phase={badge.phase}
          suppressEmptyHint={a.biddingNotOpenYet}
          meta={auctionMeta}
        >
          {/* A bid of ours from an auction of this name that is over, whose
              lockup is stranded. The list above is scoped to the current
              auction, so without this the coin is simply missing. */}
          {(caps?.strandedBidCount ?? 0) > 0 && (
            <div
              className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2"
              data-testid="stranded-bids"
            >
              <span className="font-semibold">
                {caps!.strandedBidCount === 1
                  ? "1 bid of yours from an auction that is over"
                  : `${caps!.strandedBidCount} bids of yours from auctions that are over`}
              </span>{" "}
              {formatHnsShort(caps!.strandedLockupDoos ?? 0)} HNS is still locked in{" "}
              {caps!.strandedBidCount === 1 ? "it" : "them"}. A bid can be revealed, and a reveal
              redeemed or registered, only while its own auction is the name&apos;s current one.
              That auction has ended, so the lockup cannot be recovered.
            </div>
          )}
          {/* Our coins of an ended auction no node could be asked about: not
              known, which is not the same as nothing stranded. */}
          {(caps?.strandedUncheckedCount ?? 0) > 0 && (
            <div
              className="text-xs text-gray-700 bg-gray-50 border border-gray-200 rounded p-2"
              data-testid="stranded-unchecked"
            >
              Some of your bids on this name could not be checked — sync with your node to see
              whether a lockup was lost.
            </div>
          )}
        </NameBidsPanel>
      </div>

      {loaded && (
        <NameDetails
          name={name}
          profileId={profile?.id ?? null}
          info={info}
          caps={caps}
          network={network}
          countdown={a.countdown}
          showLocalBid={a.showLocalBid}
          recordsLive={a.recordsLive}
          onOpenRecords={() => onTab("dns")}
        />
      )}
    </div>
  );
}
