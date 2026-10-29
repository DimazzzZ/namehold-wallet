import { useEffect, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { parseFeeRateArg } from "../lib/feeRate";
import { mapError, unwrapStaged } from "../lib/errors";
import { useNameAction } from "../queries/wallet";
import { useDraftRunner } from "../queries/useDraftRunner";
import { useUiStore } from "../stores/ui";
import type { NameActionCapabilities, TxDraftSummary, WalletProfileSummary } from "../types";

export type BatchAction = "renew" | "reveal" | "redeem" | "finalize" | "transfer";

/**
 * Names per transaction. The backend builds at most this many into one draft
 * (`MAX_BATCH_SIZE`, spec 2026-09-15 B3); a larger selection is sent as a
 * series of transactions of this size, one confirm for the whole series.
 */
export const MAX_BATCH = 100;

export interface BatchModalState {
  open: boolean;
  action: BatchAction;
  /** Every selected name, across all transactions of the series. */
  names: string[];
  /** The fee of the first transaction times the number of transactions. */
  feeDoos: number;
  amountDoos: number;
  /** The first transaction's draft, built before the modal opens. */
  draftId: string;
  recipient?: string;
  /** How many transactions the series takes (1 for up to MAX_BATCH names). */
  batches: number;
}

export interface BatchProgress {
  action: BatchAction;
  step: "building" | "sending";
  /** 1-based index of the transaction being built or sent. */
  index: number;
  total: number;
}

// "Batch renew" for several names, "Renew" for one (a row menu's action).
const what = (action: BatchAction, count: number) =>
  count > 1 ? `Batch ${action}` : action[0]!.toUpperCase() + action.slice(1);

const chunk = (list: string[]): string[][] => {
  const out: string[][] = [];
  for (let i = 0; i < list.length; i += MAX_BATCH) out.push(list.slice(i, i + MAX_BATCH));
  return out;
};

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
  transferable,
}: {
  profile: WalletProfileSummary | null | undefined;
  capsByName: Map<string, NameActionCapabilities>;
  unlocked: boolean;
  /**
   * Names known to be transferable from the chain's own facts (registered,
   * not expired, no transfer pending). When given, a transfer is eligible as
   * soon as every selected name is in it, without waiting for the capability
   * query, which reads the node many times per name and takes minutes on a
   * wallet with hundreds of names.
   */
  transferable?: Set<string>;
}) {
  const qc = useQueryClient();
  const runner = useDraftRunner();
  const showToast = useUiStore((s) => s.showToast);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [recipient, setRecipient] = useState("");
  // Per-transaction fee-rate override (doos/kvB, raw text). Empty = use the
  // global setting default.
  const [feeRate, setFeeRate] = useState("");
  const [modal, setModal] = useState<BatchModalState | null>(null);
  // What a running series is doing right now, for the batch bar: building a
  // draft takes seconds per hundred names and signing several in a row longer.
  const [progress, setProgress] = useState<BatchProgress | null>(null);
  // The series behind an open modal: the chunks still to build after the
  // first, and the builder that makes a draft for one chunk.
  // `fromSelection` is false for a single name run from its row menu, which
  // must leave the user's selection as it is.
  const series = useRef<{
    rest: string[][];
    build: (names: string[]) => Promise<TxDraftSummary>;
    fromSelection: boolean;
  } | null>(null);

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
    if (selected.size === 0) return none;
    // Transfer: the chain's facts (when given) and the capability check (when
    // it has answered for a name) must both allow it. A name whose capability
    // has not arrived yet rests on the chain's facts alone, so a wallet with
    // hundreds of names can transfer before the slow capability query is done.
    const canTransfer = [...selected].every((n) => {
      const c = capsByName.get(n);
      if (transferable && !transferable.has(n)) return false;
      if (c) return c.canTransfer.allowed;
      return !!transferable;
    });
    const all = [...selected].map((n) => capsByName.get(n));
    if (all.some((c) => !c)) return { ...none, canTransfer };
    const caps = all as NameActionCapabilities[];
    return {
      canReveal: caps.every((c) => c.canReveal.allowed),
      canRedeem: caps.every((c) => c.canRedeem.allowed),
      canFinalize: caps.every((c) => c.canFinalize.allowed),
      canTransfer,
      // R27: a name locked for sale (or not renewable yet) refuses a renewal.
      canRenew: caps.every((c) => c.canRenew.allowed),
    };
  }, [selected, capsByName, transferable]);

  const feeRateArg = () => parseFeeRateArg(feeRate) ?? undefined;
  const names = () => Array.from(selected);

  // Every batch action is the same three steps — build one draft for the
  // selection, then hand its fee and amount to the confirm modal — and every
  // failure is reported the same way.
  const runBatch = async (
    action: BatchAction,
    build: (names: string[]) => Promise<TxDraftSummary>,
    extra: { recipient?: string } = {},
    only?: string[],
  ) => {
    const list = only ?? names();
    if (list.length === 0) return;
    const [first, ...rest] = chunk(list);
    try {
      setProgress({ action, step: "building", index: 1, total: rest.length + 1 });
      const draft = await build(first!);
      setProgress(null);
      series.current = { rest, build, fromSelection: !only };
      setModal({
        open: true,
        action,
        names: list,
        feeDoos: (draft.summary?.feeDoos ?? 0) * (rest.length + 1),
        amountDoos: draft.summary?.sendTotalDoos ?? 0,
        draftId: draft.id,
        batches: rest.length + 1,
        ...extra,
      });
    } catch (e) {
      setProgress(null);
      showToast(`${what(action, list.length)} failed: ${mapError(e)}`, "error");
    }
  };

  const builders = {
    renew: (n: string[]) => renewM.mutateAsync({ names: n, feeRate: feeRateArg() }),
    reveal: (n: string[]) => revealM.mutateAsync({ names: n, feeRate: feeRateArg() }),
    redeem: (n: string[]) => redeemM.mutateAsync({ names: n, feeRate: feeRateArg() }),
    finalize: (n: string[]) => finalizeM.mutateAsync({ names: n, feeRate: feeRateArg() }),
  };

  /**
   * Run one action for the given names instead of the selection (a row's
   * menu): the same build, confirm and send, and the selection stays as it is.
   */
  const runFor = (action: keyof typeof builders, only: string[]) =>
    runBatch(action, builders[action], {}, only);

  const run = {
    renew: () => runBatch("renew", builders.renew),
    reveal: () => runBatch("reveal", builders.reveal),
    redeem: () => runBatch("redeem", builders.redeem),
    finalize: () => runBatch("finalize", builders.finalize),
    transfer: () => {
      const to = recipient.trim();
      if (!to) return;
      return runBatch(
        "transfer",
        (n) => transferM.mutateAsync({ names: n, recipient: to, feeRate: feeRateArg() }),
        { recipient: to },
      );
    },
  };

  // Confirm a pending batch series: unlock (if needed) → sign → broadcast the
  // first draft, then build → sign → broadcast each remaining chunk. A failure
  // keeps the names not yet sent selected, so the series can be resumed.
  const confirm = async () => {
    if (!modal || !profile) return;
    const { draftId, names: list, action, batches } = modal;
    const s = series.current;
    const sent: string[] = [];
    // The first draft is the confirmation's: a failure keeps it for a retry
    // (Cancel abandons it). A later one was built here: a failure before its
    // broadcast discards it (useDraftRunner).
    const sendOne = async (id: string, chunkNames: string[], index: number) => {
      setProgress({ action, step: "sending", index, total: batches });
      const result = await runner.send(id, profile.id, index > 1 || unlocked, {
        keepOnFailure: index === 1,
      });
      sent.push(...chunkNames);
      showToast(
        `${what(action, list.length)} ${batches > 1 ? `${index} of ${batches} ` : ""}broadcast ${result.txid.slice(0, 12)}… (${chunkNames.length} name(s))`,
        "success",
      );
    };
    try {
      const first = chunk(list)[0] ?? [];
      await sendOne(draftId, first, 1);
      setModal(null);
      for (const [i, chunkNames] of (s?.rest ?? []).entries()) {
        setProgress({ action, step: "building", index: i + 2, total: batches });
        const draft = await s!.build(chunkNames);
        await sendOne(draft.id, chunkNames, i + 2);
      }
      series.current = null;
      setProgress(null);
      if (s?.fromSelection !== false) {
        clear();
        setRecipient("");
      }
      qc.invalidateQueries({ queryKey: ["wallet"] });
    } catch (e) {
      setProgress(null);
      const left = list.filter((n) => !sent.includes(n));
      showToast(
        `${what(action, list.length)} failed after ${sent.length} of ${list.length} name(s): ${mapError(unwrapStaged(e))}`,
        "error",
      );
      if (sent.length > 0) {
        // The modal's draft is spent; keep what is left selected for a retry.
        series.current = null;
        setModal(null);
        if (s?.fromSelection !== false) setSelected(new Set(left));
        qc.invalidateQueries({ queryKey: ["wallet"] });
        return;
      }
      // Nothing sent: keep the modal open so the user can retry or cancel.
      throw e;
    }
  };

  // Cancel discards the draft built for the confirmation: it reserves the
  // coins that pay its fee, and a draft nobody sends would keep them from every
  // later send ("not enough funds") until discarded by hand in Activity.
  const cancel = () => {
    if (modal) void runner.abandon(modal.draftId);
    series.current = null;
    setModal(null);
  };

  return {
    selected,
    toggle,
    setSelected,
    clear,
    eligibility,
    progress,
    recipient,
    setRecipient,
    feeRate,
    setFeeRate,
    run,
    runFor,
    modal,
    confirm,
    cancel,
  };
}

export type NameBatchActions = ReturnType<typeof useNameBatchActions>;
