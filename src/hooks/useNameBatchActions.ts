import { useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { parseFeeRateArg } from "../lib/feeRate";
import { mapError } from "../lib/errors";
import {
  useBroadcastTxDraft,
  useNameAction,
  useSignTxDraft,
  useUnlockSigner,
} from "../queries/wallet";
import { useUiStore } from "../stores/ui";
import type { NameActionCapabilities, TxDraftSummary, WalletProfileSummary } from "../types";

export type BatchAction = "renew" | "reveal" | "redeem" | "finalize" | "transfer";

export interface BatchModalState {
  open: boolean;
  action: BatchAction;
  names: string[];
  feeDoos: number;
  amountDoos: number;
  draftId: string;
  recipient?: string;
}

/**
 * Selection + the five batch name actions (renew, reveal, redeem, finalize,
 * transfer): build one draft for the selection, confirm it in
 * BatchConfirmModal, then unlock → sign → broadcast. Selection resets when
 * the wallet changes.
 */
export function useNameBatchActions({
  profile,
  capsByName,
  unlocked,
}: {
  profile: WalletProfileSummary | null | undefined;
  capsByName: Map<string, NameActionCapabilities>;
  unlocked: boolean;
}) {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [recipient, setRecipient] = useState("");
  // Per-transaction fee-rate override (doos/kvB, raw text). Empty = use the
  // global setting default.
  const [feeRate, setFeeRate] = useState("");
  const [modal, setModal] = useState<BatchModalState | null>(null);

  useEffect(() => {
    setSelected(new Set());
    setRecipient("");
  }, [profile?.id]);

  const renewM = useNameAction("build_batch_renew_draft");
  const revealM = useNameAction("build_batch_reveal_draft");
  const redeemM = useNameAction("build_batch_redeem_draft");
  const finalizeM = useNameAction("build_batch_finalize_draft");
  // Transfers N selected names to one shared recipient.
  const transferM = useNameAction("build_batch_transfer_draft");
  const unlock = useUnlockSigner();
  const signDraft = useSignTxDraft();
  const broadcast = useBroadcastTxDraft();

  const toggle = (name: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  const clear = () => setSelected(new Set());

  // A batch action is "eligible" only when *every* selected name currently
  // supports it (canX.allowed = true). Empty selection ⇒ false.
  const eligibility = useMemo(() => {
    const none = {
      canReveal: false,
      canRedeem: false,
      canFinalize: false,
      canTransfer: false,
      canRenew: false,
    };
    if (selected.size === 0 || capsByName.size === 0) return none;
    const e = {
      canReveal: true,
      canRedeem: true,
      canFinalize: true,
      canTransfer: true,
      canRenew: true,
    };
    for (const n of selected) {
      const c = capsByName.get(n);
      if (!c) return none;
      if (!c.canReveal.allowed) e.canReveal = false;
      if (!c.canRedeem.allowed) e.canRedeem = false;
      if (!c.canFinalize.allowed) e.canFinalize = false;
      if (!c.canTransfer.allowed) e.canTransfer = false;
      // R27: a name locked for sale (or not renewable yet) refuses a renewal.
      if (!c.canRenew.allowed) e.canRenew = false;
    }
    return e;
  }, [selected, capsByName]);

  const feeRateArg = () => parseFeeRateArg(feeRate) ?? undefined;
  const names = () => Array.from(selected);

  // Every batch action is the same three steps — build one draft for the
  // selection, then hand its fee and amount to the confirm modal — and every
  // failure is reported the same way.
  const runBatch = async (
    action: BatchAction,
    build: () => Promise<TxDraftSummary>,
    extra: { recipient?: string } = {},
  ) => {
    const list = names();
    if (list.length === 0) return;
    try {
      showToast(`Building batch ${action} draft…`, "info");
      const draft = await build();
      setModal({
        open: true,
        action,
        names: list,
        feeDoos: draft.summary?.feeDoos ?? 0,
        amountDoos: draft.summary?.sendTotalDoos ?? 0,
        draftId: draft.id,
        ...extra,
      });
    } catch (e) {
      showToast(`Batch ${action} failed: ${mapError(e)}`, "error");
    }
  };

  const run = {
    renew: () =>
      runBatch("renew", () => renewM.mutateAsync({ names: names(), feeRate: feeRateArg() })),
    reveal: () =>
      runBatch("reveal", () => revealM.mutateAsync({ names: names(), feeRate: feeRateArg() })),
    redeem: () =>
      runBatch("redeem", () => redeemM.mutateAsync({ names: names(), feeRate: feeRateArg() })),
    finalize: () =>
      runBatch("finalize", () => finalizeM.mutateAsync({ names: names(), feeRate: feeRateArg() })),
    transfer: () => {
      const to = recipient.trim();
      if (!to) return;
      return runBatch(
        "transfer",
        () => transferM.mutateAsync({ names: names(), recipient: to, feeRate: feeRateArg() }),
        { recipient: to },
      );
    },
  };

  // Confirm a pending batch draft: unlock (if needed) → sign → broadcast.
  const confirm = async () => {
    if (!modal || !profile) return;
    const { draftId, names: list, action } = modal;
    try {
      if (!unlocked) await unlock.mutateAsync(profile.id);
      await signDraft.mutateAsync(draftId);
      const result = await broadcast.mutateAsync(draftId);
      showToast(
        `Batch ${action} broadcast ${result.txid.slice(0, 12)}… (${list.length} name(s))`,
        "success",
      );
      setModal(null);
      clear();
      setRecipient("");
      qc.invalidateQueries({ queryKey: ["wallet"] });
    } catch (e) {
      showToast(`Batch ${action} failed: ${mapError(e)}`, "error");
      // Keep the modal open so the user can retry or cancel.
      throw e;
    }
  };

  // Cancel: just close. The draft row stays and its coin reservations block
  // nothing; it can be replayed or discarded from Activity.
  const cancel = () => setModal(null);

  return {
    selected,
    toggle,
    setSelected,
    clear,
    eligibility,
    recipient,
    setRecipient,
    feeRate,
    setFeeRate,
    run,
    modal,
    confirm,
    cancel,
  };
}

export type NameBatchActions = ReturnType<typeof useNameBatchActions>;
