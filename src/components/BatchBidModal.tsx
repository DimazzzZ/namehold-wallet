import { isSignerUnlockedFor } from "../lib/signer";
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNameAction, useSignerSession } from "../queries/wallet";
import { useDraftRunner } from "../queries/useDraftRunner";
import { useNamesActionCapabilities } from "../queries/read";
import { Dialog } from "./ui/Dialog";
import { BatchConfirmModal } from "./BatchConfirmModal";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { BID_FORFEIT_RULE, validateBidInputs } from "../lib/auction";
import { formatHns } from "../lib/utils";
import { mapError, unwrapStaged } from "../lib/errors";
import { useUiStore } from "../stores/ui";
import { FeeRateOverride } from "./ui/FeeRateOverride";
import { parseFeeRateArg } from "../lib/feeRate";

interface BatchBidModalProps {
  open: boolean;
  onClose: () => void;
  activeProfileId?: string | null;
}

/**
 * Modal for batch-bidding on multiple names. Users paste a list of names
 * (one per line), enter a shared bid value and lockup, and submit. The
 * backend allocates a fresh receive address per name, computes blinds,
 * persists commitments, and returns a draft for signing.
 *
 * Flow: input → preflight (check availability) → build draft → sign/broadcast confirm.
 */
export function BatchBidModal({ open, onClose, activeProfileId }: BatchBidModalProps) {
  const batchBidMutation = useNameAction("build_batch_bid_draft");
  // The built batch bid reserves its coins: Back or closing abandons it, a
  // failed send keeps it for a retry (useDraftRunner).
  const runner = useDraftRunner();
  const showToast = useUiStore((s) => s.showToast);
  const qc = useQueryClient();
  const { data: signer } = useSignerSession();

  const [namesText, setNamesText] = useState("");
  const [bidHns, setBidHns] = useState("");
  const [lockupHns, setLockupHns] = useState("");
  const [feeRateOverride, setFeeRateOverride] = useState("");
  const [step, setStep] = useState<"input" | "preflight" | "confirm">("input");
  const [pendingDraft, setPendingDraft] = useState<{
    id: string;
    feeDoos: number;
    amountDoos: number;
    names: string[];
  } | null>(null);

  const names = namesText
    .split("\n")
    .map((n) => n.trim())
    .filter((n) => n.length > 0);

  const bidValidation = validateBidInputs(bidHns, lockupHns);
  const { formValid: bidFormValid, bidDoos, lockupDoos } = bidValidation;

  // Preflight: fetch capabilities for all pasted names to show biddability.
  // Only fires when the user advances to the preflight step.
  const { data: allCaps = [], isFetching: capsLoading } = useNamesActionCapabilities(
    step === "preflight" ? names : [],
    activeProfileId,
  );

  // Categorize names by biddability.
  const biddableNames = allCaps.filter((c) => c.canBid.allowed).map((c) => c.name);
  const notBiddableNames = allCaps.filter((c) => !c.canBid.allowed);

  const unlocked = isSignerUnlockedFor(signer, activeProfileId);

  const handleCheckNames = () => {
    if (names.length === 0 || !bidFormValid) return;
    setStep("preflight");
  };

  const handleRemoveNotBiddable = () => {
    setNamesText(biddableNames.join("\n"));
  };

  const handleBuildDraft = async () => {
    if (biddableNames.length === 0 || bidDoos === null || lockupDoos === null || !bidFormValid) {
      return;
    }
    try {
      showToast(`Building batch bid draft for ${biddableNames.length} names…`, "info");
      const draft = await batchBidMutation.mutateAsync({
        names: biddableNames,
        bidValue: bidDoos,
        lockup: lockupDoos,
        feeRate: parseFeeRateArg(feeRateOverride) ?? undefined,
      });
      setPendingDraft({
        id: draft.id,
        feeDoos: draft.summary?.feeDoos ?? 0,
        amountDoos: draft.summary?.sendTotalDoos ?? 0,
        names: biddableNames,
      });
      setStep("confirm");
    } catch (e: unknown) {
      showToast(mapError(e, "build"), "error");
    }
  };

  const handleBatchConfirm = async () => {
    if (!pendingDraft) return;
    try {
      const result = await runner.send(pendingDraft.id, activeProfileId ?? "", unlocked, {
        keepOnFailure: true,
      });
      showToast(
        `Batch bid broadcast ${result.txid.slice(0, 12)}… (${pendingDraft.names.length} name(s))`,
        "success",
      );
      // Invalidate both wallet and read queries so positions refresh.
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
      resetAndClose();
    } catch (e: unknown) {
      showToast(mapError(unwrapStaged(e)), "error");
      // Keep the confirm modal open so the user can retry.
      throw e;
    }
  };

  const resetAndClose = () => {
    void runner.abandon(pendingDraft?.id);
    setNamesText("");
    setBidHns("");
    setLockupHns("");
    setFeeRateOverride("");
    setStep("input");
    setPendingDraft(null);
    onClose();
  };

  const handleBack = () => {
    void runner.abandon(pendingDraft?.id);
    setStep("input");
    setPendingDraft(null);
  };

  // Render the confirm modal if a draft is pending.
  if (step === "confirm" && pendingDraft) {
    return (
      <BatchConfirmModal
        open={open}
        action="bid"
        names={pendingDraft.names}
        estimatedFeeDoos={pendingDraft.feeDoos}
        amountDoos={pendingDraft.amountDoos}
        onConfirm={handleBatchConfirm}
        onCancel={handleBack}
      />
    );
  }

  // Preflight step: show per-name biddability results.
  if (step === "preflight") {
    return (
      <Dialog open={open} onClose={handleBack} title="Batch Bid — Check Names">
        <div className="space-y-4">
          {capsLoading ? (
            <div className="text-sm text-gray-500 py-4 text-center">
              Checking name availability…
            </div>
          ) : (
            <>
              <div className="text-sm text-gray-600">
                {biddableNames.length} of {names.length} name
                {names.length !== 1 ? "s" : ""} are ready to bid on.
              </div>

              {biddableNames.length > 0 && (
                <div className="border border-green-200 bg-green-50 rounded p-3">
                  <div className="text-sm font-medium text-green-900 mb-2">
                    Biddable ({biddableNames.length})
                  </div>
                  <ul className="space-y-1 max-h-32 overflow-auto">
                    {biddableNames.map((name) => (
                      <li key={name} className="text-sm text-green-800 font-mono">
                        ✓ .{name}
                      </li>
                    ))}
                  </ul>
                </div>
              )}

              {notBiddableNames.length > 0 && (
                <div className="border border-red-200 bg-red-50 rounded p-3">
                  <div className="text-sm font-medium text-red-900 mb-2">
                    Not available ({notBiddableNames.length})
                  </div>
                  <ul className="space-y-1 max-h-32 overflow-auto">
                    {notBiddableNames.map((cap) => (
                      <li key={cap.name} className="text-sm text-red-800 font-mono">
                        ✗ .{cap.name} — {cap.canBid.reason || "not biddable"}
                      </li>
                    ))}
                  </ul>
                </div>
              )}

              {biddableNames.length > 0 && lockupDoos !== null && (
                <BatchBidLockupNote count={biddableNames.length} lockupDoos={lockupDoos} />
              )}
            </>
          )}

          <div className="flex gap-2 justify-end pt-2">
            <Button variant="ghost" onClick={handleBack}>
              Back
            </Button>
            {notBiddableNames.length > 0 && biddableNames.length > 0 && (
              <Button variant="ghost" onClick={handleRemoveNotBiddable}>
                Remove not available
              </Button>
            )}
            <Button
              variant="primary"
              onClick={handleBuildDraft}
              disabled={
                biddableNames.length === 0 ||
                !bidFormValid ||
                batchBidMutation.isPending ||
                capsLoading
              }
              data-testid="batch-bid-build-draft-btn"
            >
              {batchBidMutation.isPending ? "Building…" : "Build Draft"}
            </Button>
          </div>
        </div>
      </Dialog>
    );
  }

  // Input step (default).
  return (
    <Dialog open={open} onClose={resetAndClose} title="Batch Bid">
      <div className="space-y-4">
        <div>
          <label className="block text-sm font-medium mb-1">Names (one per line)</label>
          <textarea
            value={namesText}
            onChange={(e) => setNamesText(e.target.value)}
            placeholder="name1&#10;name2&#10;name3"
            className="w-full h-32 p-2 border border-gray-300 rounded font-mono text-sm resize-none"
            data-testid="batch-bid-names-input"
          />
          {names.length > 0 && (
            <div className="text-xs text-gray-500 mt-1">
              {names.length} name{names.length !== 1 ? "s" : ""} entered
            </div>
          )}
        </div>

        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="block text-sm font-medium mb-1">Bid (HNS)</label>
            <Input
              inputMode="decimal"
              autoComplete="off"
              value={bidHns}
              onChange={(e) => setBidHns(e.target.value)}
              placeholder="0.00"
              data-testid="batch-bid-value-input"
            />
            {bidValidation.bidError && (
              <div className="text-xs text-red-600 mt-1" data-testid="batch-bid-value-error">
                {bidValidation.bidError}
              </div>
            )}
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">Lockup (HNS)</label>
            <Input
              inputMode="decimal"
              autoComplete="off"
              value={lockupHns}
              onChange={(e) => setLockupHns(e.target.value)}
              placeholder="0.00"
              data-testid="batch-bid-lockup-input"
            />
            {bidValidation.lockupError && (
              <div className="text-xs text-red-600 mt-1" data-testid="batch-bid-lockup-error">
                {bidValidation.lockupError}
              </div>
            )}
          </div>
        </div>

        <FeeRateOverride
          value={feeRateOverride}
          onChange={setFeeRateOverride}
          label="Fee rate override"
        />

        <div className="flex gap-2 justify-end pt-2">
          <Button variant="ghost" onClick={resetAndClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={handleCheckNames}
            disabled={names.length === 0 || !bidFormValid}
            data-testid="batch-bid-check-names-btn"
          >
            Check names
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

/**
 * What a batch bid locks and what comes back, under the names it will bid on.
 * Each name's BID coin holds the whole lockup until it is revealed; a REVEAL
 * keeps the bid in a coin and returns the rest; the revealed bid then stays in
 * the name at the second-highest bid (a win) or comes back through a REDEEM
 * (a loss). An unrevealed bid forfeits its lockup (`BID_FORFEIT_RULE`).
 */
function BatchBidLockupNote({ count, lockupDoos }: { count: number; lockupDoos: number }) {
  const totalDoos = count * lockupDoos;
  return (
    <div className="space-y-2">
      <div className="bg-blue-50 border border-blue-200 rounded p-3 text-sm space-y-1">
        <div data-testid="batch-bid-total-lockup">
          <strong>Locked until you reveal:</strong> {count} × {formatHns(lockupDoos)} HNS ={" "}
          {formatHns(totalDoos)} HNS
        </div>
        <div className="text-xs text-gray-600">
          At reveal, the part of each lockup above its bid comes back to you. The bid stays locked
          until the auction closes: if you win, the second-highest bid stays locked in the name; if
          you lose, Redeem gives it back.
        </div>
      </div>
      <div
        className="text-xs text-amber-700 bg-amber-50 border border-amber-200 rounded p-2"
        data-testid="batch-bid-forfeit-warning"
      >
        Reveal every bid. {BID_FORFEIT_RULE}{" "}
        {count === 1
          ? `That is ${formatHns(lockupDoos)} HNS.`
          : `That is ${formatHns(lockupDoos)} HNS a name, ${formatHns(totalDoos)} HNS for all ${count}.`}
      </div>
    </div>
  );
}
