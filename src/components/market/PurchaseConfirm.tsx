import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Alert } from "../ui/Alert";
import { FeeRateOverride } from "../ui/FeeRateOverride";
import { parseFeeRateArg } from "../../lib/feeRate";
import { mapError, stageOf, unwrapStaged } from "../../lib/errors";
import { formatHns } from "../../lib/utils";
import { displayName } from "../../lib/idn";
import { useBuildPurchase, usePurchasePreview } from "../../queries/shakedex";
import {
  useActiveProfile,
  useDeleteTxDraft,
  useExecuteDraft,
  useSignerSession,
  useWriteCapability,
} from "../../queries/wallet";
import { useUiStore } from "../../stores/ui";
import type { MarketRow } from "../../types";
import { canBuyShakedex, purchaseRefusal } from "./marketText";
import { useSettingsStore } from "../../stores/settings";

export interface PurchaseConfirmProps {
  open: boolean;
  row: MarketRow;
  /** True when the listing came from the LearnHNS Market, not a file or paste. */
  fromMarket: boolean;
  onClose: () => void;
}

/**
 * Confirms a Shakedex purchase: every line of the price, the market fee choice,
 * then build, sign and broadcast in one go.
 */
export function PurchaseConfirm({ open, row, fromMarket, onClose }: PurchaseConfirmProps) {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const { data: writeCap } = useWriteCapability();
  // null = the user has not touched the box; the default settles from the first preview.
  const [pay, setPay] = useState<boolean | null>(null);
  const [feeRateRaw, setFeeRateRaw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const feeRate = parseFeeRateArg(feeRateRaw);
  const preview = usePurchasePreview(
    open ? row.listingJson : null,
    pay ?? true,
    fromMarket,
    feeRate,
  );
  const build = useBuildPurchase();
  const exec = useExecuteDraft();
  const deleteDraft = useDeleteTxDraft();

  const data = preview.data;
  const fee = data?.marketFee ?? null;
  const payable = fee != null && fee.payable;

  useEffect(() => {
    if (pay === null && data) setPay(payable && fee.published);
  }, [pay, data, payable, fee]);

  const paying = pay === true && payable;
  const experimental = useSettingsStore((s) => s.settings?.shakedex_experimental);
  const allowed = canBuyShakedex(profile?.kind, writeCap, profile?.network, experimental);
  const refusal = purchaseRefusal(profile?.kind, writeCap, profile?.network, experimental);
  const settled = pay !== null && data !== undefined && !preview.isFetching && !preview.isError;
  const canBuy = allowed && settled && !busy && profile != null;

  const buy = async () => {
    if (!profile || !data) return;
    setBusy(true);
    setError(null);
    let draftId: string;
    try {
      const draft = await build.mutateAsync({
        listingJson: row.listingJson,
        acceptedMarketFeeDoos: paying && fee ? fee.valueDoos : null,
        fromMarket,
        feeRate,
      });
      draftId = draft.id;
    } catch (e) {
      setError(mapError(e));
      // The step may have moved: show the current figures again.
      void preview.refetch();
      setBusy(false);
      return;
    }
    try {
      await exec.run(draftId, profile.id, signer?.unlocked ?? false);
      showToast("Purchase sent — it appears under your names as an unconfirmed purchase");
      qc.invalidateQueries({ queryKey: ["read"] });
      qc.invalidateQueries({ queryKey: ["shakedex"] });
      onClose();
    } catch (e) {
      setError(mapError(unwrapStaged(e), stageOf(e)));
      // A failure before broadcast leaves an orphan draft holding coins (and
      // its purchase record). A broadcast the backend refused before sending
      // was discarded there; any other broadcast failure keeps its draft.
      if (stageOf(e) !== "broadcast") {
        try {
          await deleteDraft.mutateAsync(draftId);
        } catch {
          // Best-effort: the Activity view can still discard it.
        }
      } else {
        // The step may have moved: show the current figures.
        void preview.refetch();
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onClose={onClose} title={`Buy .${displayName(row.name)}`}>
      <div className="space-y-4">
        {preview.isError && (
          <Alert tone="error" title="Could not price this purchase">
            {mapError(preview.error)}
          </Alert>
        )}
        {data && (
          <dl className="text-sm space-y-2" data-testid="purchase-rows">
            <div className="flex justify-between">
              <dt>Price (to seller)</dt>
              <dd>{formatHns(data.priceDoos)} HNS</dd>
            </div>
            {fee && (
              <div>
                <div className="flex justify-between items-center">
                  <dt>
                    <label className="flex items-center gap-2">
                      <input
                        type="checkbox"
                        data-testid="purchase-market-fee-pay"
                        checked={paying}
                        disabled={!payable || busy}
                        onChange={(e) => setPay(e.target.checked)}
                      />
                      Market fee
                    </label>
                  </dt>
                  <dd data-testid="purchase-market-fee">
                    {formatHns(fee.valueDoos)} HNS
                    {fee.percentText != null && ` (${fee.percentText})`}
                  </dd>
                </div>
                {fee.warning && <p className="text-xs text-yellow-700 mt-1">{fee.warning}</p>}
              </div>
            )}
            <div className="flex justify-between">
              <dt>Network fee</dt>
              <dd>{formatHns(data.networkFeeDoos)} HNS</dd>
            </div>
            <div className="flex justify-between font-semibold border-t pt-2">
              <dt>Total</dt>
              <dd>{formatHns(data.totalDoos)} HNS</dd>
            </div>
          </dl>
        )}
        {data && (
          <p className="text-sm text-gray-700">The name becomes yours {data.finalizeWait}.</p>
        )}
        {data?.warnExpiry && (
          <Alert tone="warning">
            This name expires soon after the finalize becomes possible — finalize it in time, or the
            name is lost.
          </Alert>
        )}
        <FeeRateOverride value={feeRateRaw} onChange={setFeeRateRaw} label="Fee rate override" />
        {error && <Alert tone="error">{error}</Alert>}
        {refusal && profile != null && <p className="text-xs text-gray-600">{refusal}</p>}
        <div className="flex gap-2 justify-end pt-2">
          <Button variant="ghost" data-testid="purchase-cancel" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button variant="primary" data-testid="purchase-buy" onClick={buy} disabled={!canBuy}>
            {busy ? "Processing…" : "Buy"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
