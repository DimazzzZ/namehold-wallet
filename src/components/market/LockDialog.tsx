import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Alert } from "../ui/Alert";
import { Tooltip } from "../ui/Tooltip";
import { FeeRateOverride } from "../ui/FeeRateOverride";
import { parseFeeRateArg } from "../../lib/feeRate";
import { mapError, stageOf, unwrapStaged } from "../../lib/errors";
import { formatHns } from "../../lib/utils";
import { displayName } from "../../lib/idn";
import { useBuildLock, useKeptDraft } from "../../queries/shakedex";
import {
  useActiveProfile,
  useDeleteTxDraft,
  useExecuteDraft,
  useWithSigner,
  useWriteCapability,
} from "../../queries/wallet";
import { useSettingsStore } from "../../stores/settings";
import { useUiStore } from "../../stores/ui";
import type { ShakedexListingMode, TxDraftSummary } from "../../types";
import {
  LISTING_BUTTON_LABEL,
  LOCK_COSTS,
  MY_LISTINGS_TAB,
  PUBLISH_PRIVACY,
  SHARED_AS_FILE_ONLY,
  canSellShakedex,
  sellRefusal,
  withKept,
} from "./marketText";

export interface LockDialogProps {
  open: boolean;
  name: string;
  onClose: () => void;
}

/**
 * Day 0 of a sale (R19): choose Buy Now or a reverse auction and, on
 * mainnet, whether to publish (R23); read what locking costs (R27); build
 * the lock TRANSFER, which the backend checks against R31 and answers with
 * its warning or its refusal; then sign and send it. The price, or the
 * schedule, is set at Finalize & sign, after the transfer lockup.
 */
export function LockDialog({ open, name, onClose }: LockDialogProps) {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: writeCap } = useWriteCapability();
  const experimental = useSettingsStore((s) => s.settings?.shakedex_experimental);
  const mainnet = profile?.network === "mainnet";
  const [mode, setMode] = useState<ShakedexListingMode>("buyNow");
  const [publish, setPublish] = useState(true);
  const [feeRateRaw, setFeeRateRaw] = useState("");
  const [draft, setDraft] = useState<TxDraftSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const signer = useWithSigner();
  const build = useBuildLock();
  const keptDraft = useKeptDraft();
  const exec = useExecuteDraft();
  const deleteDraft = useDeleteTxDraft();

  const refusal = sellRefusal(profile?.kind, writeCap, profile?.network, experimental);
  const allowed = canSellShakedex(profile?.kind, writeCap, profile?.network, experimental);

  // An unsent lock draft holds coins, and its Locking listing goes with it.
  const discard = async () => {
    if (!draft) return;
    try {
      await deleteDraft.mutateAsync(draft.id);
    } catch {
      // Best-effort: Activity can still discard it.
    }
    setDraft(null);
  };

  const lock = async () => {
    if (!profile) return;
    setBusy(true);
    setError(null);
    try {
      // The backend checks the signer before anything else (it derives the
      // lock key), so a locked wallet is unlocked for this profile first.
      const built = await signer.run(profile.id, () =>
        build.mutateAsync({
          name,
          mode,
          publish: mainnet && publish,
          feeRate: parseFeeRateArg(feeRateRaw),
        }),
      );
      setDraft(built);
    } catch (e) {
      setError(mapError(e));
    } finally {
      setBusy(false);
    }
  };

  const send = async () => {
    if (!profile || !draft) return;
    setBusy(true);
    setError(null);
    try {
      // The build just passed the signer check; should the session lapse
      // before signing, the sign is refused "Wallet locked" and run again
      // after one unlock.
      await signer.run(profile.id, () => exec.run(draft.id, profile.id, true));
      showToast(
        `Lock sent. Once the transfer lockup is over, set the price with ${LISTING_BUTTON_LABEL.finalizeAndSign} in Market › ${MY_LISTINGS_TAB}.`,
      );
      // The listing and the name's actions follow the sent lock.
      qc.invalidateQueries({ queryKey: ["shakedex"] });
      qc.invalidateQueries({ queryKey: ["read"] });
      onClose();
    } catch (e) {
      const why = mapError(unwrapStaged(e), stageOf(e));
      // After a failed broadcast the lock is not this dialog's to delete or
      // to send again. Refused at the gates it stays signed with its Locking
      // listing (Send or Discard in My listings); not taken, or no answer,
      // it is held `broadcast_pending` (Send again); refused by the node it
      // is Activity's; refused by R31's recheck the backend deleted it with
      // its listing. A fresh read of My listings says which.
      if (stageOf(e) === "broadcast") {
        const id = draft.id;
        setDraft(null);
        setError(withKept(why, await keptDraft(id)));
      } else {
        setError(why);
        await discard();
      }
    } finally {
      setBusy(false);
    }
  };

  // Closing mid-send would hide the outcome; closing after the build drops
  // the unsent draft.
  const close = () => {
    if (busy) return;
    void discard().then(onClose);
  };

  // R31's warning, or any other the backend put on the draft; the costs
  // are already on screen.
  const warnings = (draft?.summary?.warnings ?? []).filter((w) => w !== LOCK_COSTS);

  return (
    <Dialog open={open} onClose={close} title={`Sell .${displayName(name)}`}>
      <div className="space-y-4 text-sm">
        {!draft && (
          <>
            <fieldset className="space-y-1" disabled={busy}>
              <legend className="font-medium text-gray-700">How to sell</legend>
              <label className="flex items-center gap-2">
                <input
                  type="radio"
                  data-testid="lock-mode-buy-now"
                  checked={mode === "buyNow"}
                  onChange={() => setMode("buyNow")}
                />
                Buy Now — one price
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="radio"
                  data-testid="lock-mode-reverse"
                  checked={mode === "reverseAuction"}
                  onChange={() => setMode("reverseAuction")}
                />
                Reverse auction — the price falls step by step to a floor
              </label>
              <p className="text-xs text-gray-500">
                You set the price, or the schedule, at Finalize &amp; sign, once the transfer lockup
                is over.
              </p>
            </fieldset>
            {mainnet ? (
              <div>
                <label className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    data-testid="lock-publish"
                    checked={publish}
                    disabled={busy}
                    onChange={(e) => setPublish(e.target.checked)}
                  />
                  Publish on LearnHNS Market
                </label>
                <p className="mt-1 text-xs text-gray-500">{PUBLISH_PRIVACY}</p>
              </div>
            ) : (
              profile != null && <p className="text-xs text-gray-500">{SHARED_AS_FILE_ONLY}</p>
            )}
          </>
        )}
        <p data-testid="lock-costs" className="text-gray-700">
          {LOCK_COSTS}
        </p>
        {draft && (
          <dl data-testid="lock-rows">
            <div className="flex justify-between">
              <dt>Network fee</dt>
              <dd>{formatHns(draft.summary?.feeDoos)} HNS</dd>
            </div>
          </dl>
        )}
        {warnings.map((w) => (
          <Alert key={w} tone="warning">
            <span data-testid="lock-warning">{w}</span>
          </Alert>
        ))}
        {!draft && (
          <FeeRateOverride value={feeRateRaw} onChange={setFeeRateRaw} label="Fee rate override" />
        )}
        {error && (
          <Alert tone="error">
            <span data-testid="lock-error">{error}</span>
          </Alert>
        )}
        {refusal && profile != null && <p className="text-xs text-gray-600">{refusal}</p>}
        <div className="flex justify-end gap-2 pt-2">
          {draft ? (
            <>
              <Button
                variant="ghost"
                data-testid="lock-back"
                disabled={busy}
                onClick={() => void discard()}
              >
                Back
              </Button>
              <Button variant="primary" data-testid="lock-send" disabled={busy} onClick={send}>
                {busy ? "Processing…" : "Sign and send the lock"}
              </Button>
            </>
          ) : (
            <Tooltip content={allowed ? null : refusal}>
              <Button
                variant="primary"
                data-testid="lock-build"
                disabled={!allowed || busy || profile == null}
                onClick={lock}
              >
                {busy ? "Checking…" : "Lock name"}
              </Button>
            </Tooltip>
          )}
        </div>
      </div>
    </Dialog>
  );
}
