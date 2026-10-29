import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { isSignerUnlockedFor } from "../../lib/signer";
import {
  useActiveProfile,
  useSignerSession,
  useWriteCapability,
  useNameAction,
  useExecuteDraft,
  useDeleteTxDraft,
} from "../../queries/wallet";
import {
  useReadNameInfo,
  useNameActionCapabilities,
  useRecoverBidCommitment,
  useBruteForceRecoverBid,
  useNameRecords,
} from "../../queries/read";
import { resolveSections } from "../../lib/nameSections";
import { useUiStore } from "../../stores/ui";
import { parseFeeRateArg } from "../../lib/feeRate";
import { mapError, stageOf, unwrapStaged } from "../../lib/errors";
import {
  auctionPhase,
  nextTransition,
  AUCTION_PHASE_GUIDE,
  REVOKED_REDEEM_GUIDE,
  taskSummaryFromCapabilities,
  validateBidInputs,
} from "../../lib/auction";
import { hnsToDollarydoos } from "../../lib/utils";
import { rowsToRecords, recordsToRows, type DnsRow } from "../../lib/dnsRecords";
import type { NameActionCapability } from "../../types";

export const RECORDS_NOT_FRESH_REASON = "Waiting for a fresh read of the current on-chain records";

/**
 * Every piece of state and the build→unlock→sign→broadcast runner behind the
 * name page, moved from the former modal. `name` is always the raw ACE form;
 * the page mounts this per name (`key={name}`), so a new name starts clean.
 */
export function useNameActions(name: string) {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const { data: writeCap } = useWriteCapability();
  const { data: info, isLoading, isError, error } = useReadNameInfo(name);
  const {
    data: caps,
    isLoading: capsLoading,
    isFetched: capsFetched,
  } = useNameActionCapabilities(name, profile?.id ?? null);
  const exec = useExecuteDraft();
  const deleteDraft = useDeleteTxDraft();
  const recoverBid = useRecoverBidCommitment();
  const bruteForceRecover = useBruteForceRecoverBid();

  const build = {
    open: useNameAction("build_open_draft"),
    bid: useNameAction("build_bid_draft"),
    reveal: useNameAction("build_reveal_draft"),
    redeem: useNameAction("build_redeem_draft"),
    register: useNameAction("build_register_draft"),
    update: useNameAction("build_update_draft"),
    renew: useNameAction("build_renew_draft"),
    transfer: useNameAction("build_transfer_draft"),
    finalize: useNameAction("build_finalize_draft"),
    cancel: useNameAction("build_cancel_draft"),
    revoke: useNameAction("build_revoke_draft"),
  };

  // Bid inputs in HNS (human-readable), converted to doos on submit.
  const [bidHns, setBidHns] = useState("");
  const [lockupHns, setLockupHns] = useState("");
  // Per-transaction fee-rate override for the BID (doos/kvB, raw text).
  // Empty = use the global setting default. Only the bid path threads it (the
  // other single-name ceremony actions intentionally don't expose the knob).
  const [bidFeeRate, setBidFeeRate] = useState("");
  const [recoverHns, setRecoverHns] = useState("");
  const [recipient, setRecipient] = useState("");
  const [rows, setRows] = useState<DnsRow[]>([{ type: "TXT", value: "" }]);
  const [advanced, setAdvanced] = useState(false);
  const [recordsJson, setRecordsJson] = useState("[]");
  const [selling, setSelling] = useState(false);
  const [revokeOpen, setRevokeOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  // Mirrored for the unmount cleanup, which must see the latest value.
  const busyRef = useRef<string | null>(null);
  busyRef.current = busy;
  const [revealConfirming, setRevealConfirming] = useState(false);
  const [optimisticRevealTxid, setOptimisticRevealTxid] = useState<string | null>(null);
  const isLedger = profile?.kind === "ledger_hardware";

  const revealSubstate: string | null = exec.unlock.isPending
    ? "Unlocking…"
    : exec.sign.isPending
      ? isLedger
        ? "Confirm on your Ledger…"
        : "Signing…"
      : exec.broadcast.isPending
        ? "Broadcasting…"
        : null;

  // Ledger wallets don't use the local signer session — the device signs on
  // demand. Treat them as "unlocked" so exec.run() skips the unlock step.
  const unlocked = isLedger ? true : isSignerUnlockedFor(signer, profile?.id);
  const canWrite = writeCap?.canWrite ?? false;
  const lock = !!busy || !canWrite;

  const bidValidation = validateBidInputs(bidHns, lockupHns);
  const bidNum = Number(bidHns);
  const lockupNum = Number(lockupHns);
  // The forfeit warning's "X": the raw lockup input as typed.
  const forfeitLockupText = lockupHns.trim() || "0";

  const badge = auctionPhase(info?.state);
  // While a transfer is pending the next thing to happen is Finalize, which
  // only the backend's capability countdown knows (hsd's lockup stats).
  const countdown = caps?.transferPending
    ? caps.countdownLabel && caps.countdownBlocks != null
      ? {
          label: caps.countdownLabel,
          blocks: caps.countdownBlocks,
          hours: caps.countdownHours ?? null,
        }
      : // Not the expiry: "expires in …; then the name leaves this wallet" is false.
        null
    : nextTransition(info?.state, info?.stats);
  // A revoked name has no guided step of its own, except a losing reveal of
  // ours to redeem before it expires.
  const guide =
    AUCTION_PHASE_GUIDE[badge.phase] ??
    (badge.phase === "REVOKED" && caps?.taskState === "lostNeedsRedeem"
      ? REVOKED_REDEEM_GUIDE
      : null);
  const summary = taskSummaryFromCapabilities(caps);
  // Our losing reveals in the last block before the name expires, when hsd
  // takes no redeem any more: the backend raises `redeemWindowClosing` and
  // says why in `nextActionReason`.
  const noTaskReason = caps?.redeemWindowClosing ? (caps.nextActionReason ?? null) : null;
  // The status line says "Checking…" until the single caps query settles, so
  // it never disagrees with the row that opened the page.
  const capsPending = capsLoading || !capsFetched;

  // Which sections exist at this stage, and which are still ahead. `ownsName`
  // cannot answer that: during REVEAL hsd reports the highest revealer as the
  // owner. See `resolveSections`.
  const sections = resolveSections(caps);
  const recordsLive = sections.records.kind === "live";

  // Before REVEAL hsd reports `value`/`highest` as 0; our own bid is known
  // locally (`caps.bidValueDoos`), so show that instead.
  const preReveal = badge.phase === "BIDDING" || badge.phase === "OPENING";
  const showLocalBid = preReveal && caps?.hasBidCommitment === true && caps?.bidValueDoos != null;

  // Current DNS records, read fresh on every mount: seeding from a cached
  // pre-UPDATE snapshot would let the user overwrite their on-chain records
  // from a stale base.
  const {
    data: currentRecords,
    isFetching: recordsFetching,
    isError: recordsError,
    dataUpdatedAt: recordsUpdatedAt,
    refetch: refetchRecords,
  } = useNameRecords(recordsLive ? name : null, profile?.id ?? null, {
    forceFresh: true,
  });

  // When this page visit began (reset after a REGISTER/UPDATE broadcast). A
  // records read is fresh only if it landed at or after this moment.
  const openedAtRef = useRef<number>(0);
  useEffect(() => {
    openedAtRef.current = Date.now();
  }, []);

  const recordsFresh =
    recordsLive &&
    !recordsFetching &&
    !recordsError &&
    currentRecords !== undefined &&
    recordsUpdatedAt >= openedAtRef.current;

  // Seed the editor exactly once per visit (or once after a REGISTER/UPDATE),
  // so a refetch never clobbers the user's in-progress edits.
  const seededForName = useRef<string | null>(null);
  // Id of a draft built+persisted here but not yet broadcast. Drafts reserve
  // coins, and a leftover one trips the backend double-action guard.
  const pendingDraftRef = useRef<string | null>(null);

  // Discard a built-but-not-broadcast draft, releasing its reserved coins.
  // Never called after a broadcast-stage failure (the tx may be in flight).
  const discardPendingDraft = async () => {
    const id = pendingDraftRef.current;
    if (!id) return;
    pendingDraftRef.current = null;
    try {
      await deleteDraft.mutateAsync(id);
    } catch {
      // Best-effort: leave the row for the Activity view's Discard.
    }
  };
  useEffect(() => {
    if (!recordsLive) return;
    if (seededForName.current === name) return;
    if (!recordsFresh) return;
    // Only overwrite rows when there are records to prefill: an empty read
    // must not clobber anything typed while it was in flight.
    const seeded = recordsToRows(currentRecords?.records ?? []);
    if (seeded.length) {
      setRows(seeded);
    }
    setRecordsJson(JSON.stringify(currentRecords?.records ?? [], null, 2));
    seededForName.current = name;
  }, [recordsLive, name, recordsFresh, currentRecords]);

  // Leaving the page with a built-but-unsigned draft discards it. Not while a
  // run is in flight: `run` discards on a sign-stage failure itself and keeps
  // the draft on a broadcast-stage one.
  const discardRef = useRef(discardPendingDraft);
  discardRef.current = discardPendingDraft;
  useEffect(
    () => () => {
      if (!busyRef.current) void discardRef.current();
    },
    [],
  );

  // Whether the page offers anything beyond plain info, with a phase-based
  // fallback for the window before capabilities load.
  const hasRelevantActions =
    badge.phase === "AVAILABLE" ||
    badge.phase === "BIDDING" ||
    badge.phase === "REVEAL" ||
    caps?.taskState === "wonNeedsRegister" ||
    caps?.taskState === "lostNeedsRedeem" ||
    caps?.taskState === "transferPendingFinalize" ||
    // A registered name has update/transfer/renew/revoke actions. Merely
    // leading an auction does not — the owner coin is still a REVEAL.
    caps?.nameIsRegistered === true;

  // The auction exists but its bidding window has not opened.
  const biddingNotOpenYet = caps?.taskState === "waitingForBidding";

  // Whether the page actually offers something to sign right now.
  const hasSignableActions = hasRelevantActions && !biddingNotOpenYet;

  const actionDisabled = (_actionKey: string, cap?: NameActionCapability): boolean => {
    if (lock) return true;
    if (cap && !cap.allowed) return true;
    return false;
  };

  const actionReason = (cap?: NameActionCapability): string | null => {
    if (!canWrite) return writeCap?.reason ?? "Writing is not available";
    if (cap && !cap.allowed) return cap.reason;
    return null;
  };

  // Whether the page-wide write gate is on screen (it states the reason and
  // carries its own Unlock button).
  const writeGateVisible = !canWrite && hasSignableActions;

  // `actionReason`, minus the write-capability reason while the gate shows it.
  const guidedReason = (cap?: NameActionCapability): string | null => {
    if (!canWrite) return writeGateVisible ? null : actionReason(cap);
    return actionReason(cap);
  };

  // The page stays open after a broadcast: report it and reset what the
  // action consumed.
  const toastAfterBroadcast = (label: string) => (txid: string) => {
    showToast(`${label} broadcast — ${txid.slice(0, 12)}…`, "success");
    if (label === "BID") {
      setBidHns("");
      setLockupHns("");
      setBidFeeRate("");
    } else if (label === "TRANSFER") {
      setRecipient("");
    } else if (label === "REVOKE") {
      setRevokeOpen(false);
    } else if (label === "REGISTER" || label === "UPDATE") {
      // Reseed the editor from the next fresh read, once the tx is mined.
      seededForName.current = null;
      openedAtRef.current = Date.now();
    }
  };

  const run = async (
    label: string,
    builder: () => Promise<{ id: string }>,
    afterBroadcast: (txid: string) => void = toastAfterBroadcast(label),
  ) => {
    if (!profile) return;
    setBusy(label);
    busyRef.current = label;
    let draft: { id: string };
    try {
      draft = await builder();
    } catch (e) {
      showToast(mapError(e, "build"), "error");
      setBusy(null);
      busyRef.current = null;
      return;
    }
    // The draft is now persisted with coins reserved; track it so leaving
    // discards it instead of orphaning it.
    pendingDraftRef.current = draft.id;
    try {
      const result = await exec.run(draft.id, profile.id, unlocked);
      pendingDraftRef.current = null;
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
      afterBroadcast(result.txid);
    } catch (e) {
      showToast(mapError(unwrapStaged(e), stageOf(e)), "error");
      // A sign-stage rejection (cancel on Confirm & Sign, unlock/sign failure)
      // happens before broadcast — discard the orphan. A broadcast-stage one
      // may have a tx in flight, so the row stays.
      if (stageOf(e) !== "broadcast") {
        await discardPendingDraft();
      } else {
        pendingDraftRef.current = null;
      }
    } finally {
      setBusy(null);
      busyRef.current = null;
    }
  };

  // Reveal shows the pending card after broadcast and sets the optimistic
  // txid so the card renders before the next caps poll.
  const handleRevealConfirm = () =>
    run(
      "REVEAL",
      () => build.reveal.mutateAsync({ name }),
      (txid) => {
        setOptimisticRevealTxid(txid);
        setRevealConfirming(false);
      },
    );

  // Recover a lost bid_commitments row from the on-chain BID coin + a
  // user-remembered amount. Needs only the xpub, so no unlock.
  const handleRecoverBid = async () => {
    if (!recoverHns) return;
    setBusy("RECOVER");
    try {
      await recoverBid.mutateAsync({
        walletProfileId: profile?.id ?? null,
        name,
        bidValueDoos: hnsToDollarydoos(Number(recoverHns)),
      });
      showToast("Bid commitment recovered — you can reveal now.", "success");
      setRecoverHns("");
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setBusy(null);
    }
  };

  // Auto-recover without the amount: brute-forces the bid value against the
  // on-chain blind.
  const handleBruteForceRecover = async () => {
    setBusy("RECOVER");
    try {
      const res = await bruteForceRecover.mutateAsync({
        walletProfileId: profile?.id ?? null,
        name,
      });
      showToast(
        `Bid recovered (${(res.bidValueDoos / 1_000_000).toString()} HNS) — you can reveal now.`,
        "success",
      );
      setRecoverHns("");
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setBusy(null);
    }
  };

  // Records for submit: typed rows by default, raw-JSON array in Advanced mode.
  const recordsForSubmit = (): Record<string, unknown>[] | null => {
    if (advanced) {
      const v = JSON.parse(recordsJson);
      if (!Array.isArray(v)) throw new Error("records must be a JSON array");
      return v.length > 0 ? v : null;
    }
    return rowsToRecords(rows);
  };

  const submitRecords = (label: "REGISTER" | "UPDATE") => {
    let recs: Record<string, unknown>[] | null;
    try {
      recs = recordsForSubmit();
    } catch (e) {
      showToast(mapError(e), "error");
      return;
    }
    if (label === "REGISTER") {
      // `null` records → empty resource.
      run(label, () => build.register.mutateAsync({ name, records: recs }));
      return;
    }
    // UPDATE replaces the resource wholesale; an empty editor sends `[]`.
    run(label, () => build.update.mutateAsync({ name, records: recs ?? [] }));
  };

  const setRow = (i: number, patch: Partial<DnsRow>) =>
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const addRow = () => setRows((rs) => [...rs, { type: "TXT", value: "" }]);
  const removeRow = (i: number) =>
    setRows((rs) => (rs.length > 1 ? rs.filter((_, j) => j !== i) : rs));

  const submitBid = () =>
    run("BID", () =>
      build.bid.mutateAsync({
        name,
        bidValue: hnsToDollarydoos(bidNum),
        lockup: hnsToDollarydoos(lockupNum),
        feeRate: parseFeeRateArg(bidFeeRate) ?? undefined,
      }),
    );

  return {
    name,
    profile,
    writeCap,
    info,
    isLoading,
    isError,
    error,
    caps,
    build,
    bidHns,
    setBidHns,
    lockupHns,
    setLockupHns,
    bidFeeRate,
    setBidFeeRate,
    recoverHns,
    setRecoverHns,
    recipient,
    setRecipient,
    rows,
    advanced,
    setAdvanced,
    recordsJson,
    setRecordsJson,
    selling,
    setSelling,
    revokeOpen,
    setRevokeOpen,
    busy,
    revealConfirming,
    setRevealConfirming,
    optimisticRevealTxid,
    revealSubstate,
    canWrite,
    bidValidation,
    forfeitLockupText,
    badge,
    countdown,
    guide,
    summary,
    noTaskReason,
    capsPending,
    sections,
    recordsLive,
    showLocalBid,
    currentRecords,
    recordsFetching,
    recordsFresh,
    refetchRecords,
    biddingNotOpenYet,
    actionDisabled,
    actionReason,
    writeGateVisible,
    guidedReason,
    run,
    handleRevealConfirm,
    handleRecoverBid,
    handleBruteForceRecover,
    submitRecords,
    setRow,
    addRow,
    removeRow,
    submitBid,
  };
}

export type NameActions = ReturnType<typeof useNameActions>;
