import { useState } from "react";
import { useNameAction } from "../queries/wallet";
import { Dialog } from "./ui/Dialog";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { validateBidInputs } from "../lib/auction";
import { hnsToDollarydoos } from "../lib/utils";
import { useUiStore } from "../stores/ui";

interface BatchBidModalProps {
  open: boolean;
  onClose: () => void;
  /** Pre-fill names from multi-select (e.g. watchlist selection). */
  initialNames?: string[];
}

/**
 * Modal for batch-bidding on multiple names. Users paste a list of names
 * (one per line), enter a shared bid value and lockup, and submit. The
 * backend allocates a fresh receive address per name, computes blinds,
 * persists commitments, and returns a draft for signing.
 */
export function BatchBidModal({ open, onClose, initialNames }: BatchBidModalProps) {
  const batchBidMutation = useNameAction("build_batch_bid_draft");
  const showToast = useUiStore((s) => s.showToast);

  const [namesText, setNamesText] = useState(initialNames?.join("\n") ?? "");
  const [bidHns, setBidHns] = useState("");
  const [lockupHns, setLockupHns] = useState("");
  const [feeRateOverride, setFeeRateOverride] = useState("");

  const names = namesText
    .split("\n")
    .map((n) => n.trim())
    .filter((n) => n.length > 0);

  const bidValidation = validateBidInputs(bidHns, lockupHns);
  const { formValid: bidFormValid } = bidValidation;

  const canSubmit = names.length > 0 && bidFormValid && !batchBidMutation.isPending;

  const handleSubmit = async () => {
    if (!canSubmit) return;
    try {
      showToast(`Building batch bid draft for ${names.length} names…`, "info");
      const draft = await batchBidMutation.mutateAsync({
        names,
        bidValue: hnsToDollarydoos(bidHns),
        lockup: hnsToDollarydoos(lockupHns),
        feeRate: feeRateOverride ? parseInt(feeRateOverride, 10) : undefined,
      });
      showToast(`Batch bid draft created (${draft.summary?.feeDoos ?? 0} doos fee)`, "success");
      // TODO: open the batch-action modal to sign/broadcast. For now, just close.
      onClose();
      setNamesText("");
      setBidHns("");
      setLockupHns("");
      setFeeRateOverride("");
    } catch (e: unknown) {
      showToast(`Batch bid failed: ${e}`, "error");
    }
  };

  return (
    <Dialog open={open} onClose={onClose} title="Batch Bid">
      <div className="space-y-4">
        <div>
          <label className="block text-sm font-medium mb-1">Names (one per line)</label>
          <textarea
            value={namesText}
            onChange={(e) => setNamesText(e.target.value)}
            placeholder="name1.hns&#10;name2.hns&#10;name3.hns"
            className="w-full h-32 p-2 border border-gray-300 rounded font-mono text-sm resize-none"
            data-testid="batch-bid-names-input"
          />
          {names.length > 0 && (
            <div className="text-xs text-gray-500 mt-1">
              {names.length} name{names.length !== 1 ? "s" : ""} to bid on
            </div>
          )}
        </div>

        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="block text-sm font-medium mb-1">Bid (HNS)</label>
            <Input
              type="number"
              value={bidHns}
              onChange={(e) => setBidHns(e.target.value)}
              placeholder="0.00"
              step="0.01"
              data-testid="batch-bid-value-input"
            />
            {bidValidation.bidError && (
              <div className="text-xs text-red-600 mt-1">{bidValidation.bidError}</div>
            )}
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">Lockup (HNS)</label>
            <Input
              type="number"
              value={lockupHns}
              onChange={(e) => setLockupHns(e.target.value)}
              placeholder="0.00"
              step="0.01"
              data-testid="batch-bid-lockup-input"
            />
            {bidValidation.lockupError && (
              <div className="text-xs text-red-600 mt-1">{bidValidation.lockupError}</div>
            )}
          </div>
        </div>

        <div>
          <label className="block text-sm font-medium mb-1">
            Fee rate override (doos/kvB, optional)
          </label>
          <Input
            type="number"
            value={feeRateOverride}
            onChange={(e) => setFeeRateOverride(e.target.value)}
            placeholder="Leave blank for auto"
            data-testid="batch-bid-fee-rate-input"
          />
        </div>

        <div className="flex gap-2 justify-end pt-2">
          <Button variant="ghost" onClick={onClose} disabled={batchBidMutation.isPending}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={handleSubmit}
            disabled={!canSubmit}
            data-testid="batch-bid-submit-btn"
          >
            {batchBidMutation.isPending ? "Building…" : "Build Draft"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
