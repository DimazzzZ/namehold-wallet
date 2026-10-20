import { useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { isSignerUnlockedFor } from "../../lib/signer";
import { mapError, stageOf, unwrapStaged } from "../../lib/errors";
import {
  invalidatePurchaseViews,
  useBuildPurchase,
  useBuildPurchaseFinalize,
  type BuildPurchaseArgs,
} from "../../queries/shakedex";
import {
  useActiveProfile,
  useDeleteTxDraft,
  useExecuteDraft,
  useSignerSession,
} from "../../queries/wallet";
import { useUiStore } from "../../stores/ui";
import type { TxDraftSummary } from "../../types";

/**
 * Buy a listing: build the purchase draft, then unlock → secure confirm →
 * sign → broadcast in one go. `buy` resolves true once the purchase is sent;
 * on any failure it resolves false with the reason in `error`.
 */
export function usePurchase() {
  const qc = useQueryClient();
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const build = useBuildPurchase();
  const exec = useExecuteDraft();
  const deleteDraft = useDeleteTxDraft();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // `busy` is state, so two calls in one tick (a click and Enter) both see it
  // false; this ref lets only the first one through.
  const inFlight = useRef(false);

  /** `refreshPreview` shows the current figures again: the step may have moved. */
  const buy = async (args: BuildPurchaseArgs, refreshPreview: () => void): Promise<boolean> => {
    if (!profile || inFlight.current) return false;
    inFlight.current = true;
    setBusy(true);
    setError(null);
    let draftId: string;
    try {
      draftId = (await build.mutateAsync(args)).id;
    } catch (e) {
      setError(mapError(e));
      refreshPreview();
      inFlight.current = false;
      setBusy(false);
      return false;
    }
    try {
      await exec.run(draftId, profile.id, isSignerUnlockedFor(signer, profile.id));
      return true;
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
        refreshPreview();
      }
      return false;
    } finally {
      invalidatePurchaseViews(qc);
      inFlight.current = false;
      setBusy(false);
    }
  };

  return { buy, busy, error, clearError: () => setError(null) };
}

/**
 * Finalize a purchased name: build the draft, then unlock → secure confirm →
 * sign → broadcast. The outcome is toasted: `sentText` once it is sent,
 * then the draft's warnings (the name still has the seller's DNS records,
 * R14), or the failure. `finalize` resolves true once it is sent.
 */
export function usePurchaseFinalize() {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const buildFinalize = useBuildPurchaseFinalize();
  const executeDraft = useExecuteDraft();
  const deleteDraft = useDeleteTxDraft();
  // Purchase id of the Finalize in flight, to disable that row's button.
  const [finalizingId, setFinalizingId] = useState<string | null>(null);

  const finalize = async (
    purchaseId: string,
    sentText: (txid: string) => string,
  ): Promise<boolean> => {
    if (!profile) return false;
    setFinalizingId(purchaseId);
    try {
      let draft: TxDraftSummary;
      try {
        draft = await buildFinalize.mutateAsync({ purchaseId, feeRate: null });
      } catch (e) {
        showToast(mapError(e, "build"), "error");
        return false;
      }
      try {
        const result = await executeDraft.run(
          draft.id,
          profile.id,
          isSignerUnlockedFor(signer, profile.id),
        );
        qc.invalidateQueries({ queryKey: ["wallet"] });
        qc.invalidateQueries({ queryKey: ["read"] });
        showToast(sentText(result.txid), "success");
        for (const w of draft.summary?.warnings ?? []) showToast(w, "info");
        return true;
      } catch (e) {
        showToast(mapError(unwrapStaged(e), stageOf(e)), "error");
        // A cancelled confirm or an unlock/sign failure leaves an orphan draft
        // holding coins; a broadcast-stage failure may have a tx in flight.
        if (stageOf(e) === "sign") {
          try {
            await deleteDraft.mutateAsync(draft.id);
          } catch {
            // Best-effort; the Activity view can still discard it.
          }
        }
        return false;
      }
    } finally {
      setFinalizingId(null);
    }
  };

  return { finalize, finalizingId };
}

/**
 * Purchases whose finalize may already be on the chain. Owned Names and Your
 * purchases keep them ready to finalize until the purchase job sees it
 * mined, and the backend refuses a second finalize meanwhile.
 */
export function finalizeSentIds(drafts: TxDraftSummary[]): Set<string | undefined> {
  return new Set(
    drafts
      .filter(
        (d) =>
          d.action === "shakedex_purchase_finalize" &&
          (d.status === "broadcasted" ||
            d.status === "confirmed" ||
            d.status === "broadcast_pending"),
      )
      .map((d) => d.summary?.purchaseId),
  );
}
