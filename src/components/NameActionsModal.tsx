import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useActiveProfile,
  useSignerSession,
  useWriteCapability,
  useNameAction,
  useExecuteDraft,
  useDeleteTxDraft,
} from "../queries/wallet";
import {
  useReadNameInfo,
  useNameActionCapabilities,
  useRecoverBidCommitment,
  useBruteForceRecoverBid,
  useNameRecords,
} from "../queries/read";
import { Button } from "./ui/Button";
import { Dialog } from "./ui/Dialog";
import { Badge } from "./ui/Badge";
import { UnlockButton } from "./UnlockButton";
import { DnsRecordsEditor } from "./name-actions/DnsRecordsEditor";
import { GuidedAction } from "./name-actions/GuidedAction";
import { NameBidsPanel } from "./name-actions/NameBidsPanel";
import { NameSignMessage } from "./name-actions/NameSignMessage";
import { NameDetails } from "./name-actions/NameDetails";
import { OwnershipActions } from "./name-actions/OwnershipActions";
import { UpcomingSection } from "./name-actions/UpcomingSection";
import { resolveSections } from "../lib/nameSections";
import { PaidSwapClaim } from "./name-actions/PaidSwapClaim";
import { useUiStore } from "../stores/ui";
import { FeeRateOverride } from "./ui/FeeRateOverride";
import { parseFeeRateArg } from "../lib/feeRate";
import { mapError, stageOf, unwrapStaged } from "../lib/errors";
import { formatHns, formatHnsShort } from "../lib/utils";
import { Tooltip } from "./ui/Tooltip";
import { displayName } from "../lib/idn";
import { WatchlistToggle } from "./WatchlistToggle";
import { explorerNameUrl, openExternal } from "../lib/openExternal";
import {
  auctionPhase,
  nextTransition,
  formatCountdown,
  AUCTION_PHASE_GUIDE,
  taskSummaryFromCapabilities,
  pendingBroadcastBadge,
  redeemExplainer,
  validateBidInputs,
} from "../lib/auction";
import { hnsToDollarydoos } from "../lib/utils";
import { rowsToRecords, recordsToRows, type DnsRow } from "../lib/dnsRecords";
import { ActionHint } from "./name-actions/ActionHint";
import type { NameActionCapability } from "../types";

/**
 * One modal that exposes every name covenant action for a single name, wired to
 * the `build_*_draft` commands + the build→unlock→sign→broadcast runner.
 *
 * The modal is task-driven: it uses backend capability data to show the most
 * relevant action, with clear disabled reasons when actions aren't available.
 *
 * Task 13 (F6): this file is the thin orchestrator — it owns all state, the
 * mutation runner, and the modal layout; the widgets live in
 * `./name-actions/` (`GuidedAction`, `BidForm`, `DnsRecordsEditor`,
 * `OwnershipActions`) and receive state + callbacks as props.
 */
export function NameActionsModal({
  name,
  open,
  onClose,
}: {
  name: string;
  open: boolean;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const { data: writeCap } = useWriteCapability();
  const { data: info, isLoading, isError, error } = useReadNameInfo(open ? name : null);
  const {
    data: caps,
    isLoading: capsLoading,
    isFetched: capsFetched,
  } = useNameActionCapabilities(open ? name : null, profile?.id ?? null);
  const exec = useExecuteDraft();
  const deleteDraft = useDeleteTxDraft();
  const recoverBid = useRecoverBidCommitment();
  const bruteForceRecover = useBruteForceRecoverBid();

  // Display-only: the decoded Unicode form of `name`, if it's an IDN. Every
  // backend call in this component keeps using the raw `name` prop.
  const decodedName = displayName(name);

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
    finalizeWithPayment: useNameAction("build_finalize_with_payment_draft"),
    sellWithPayment: useNameAction("create_paid_swap_offer"),
  };

  // Bid inputs in HNS (human-readable), converted to doos on submit.
  const [bidHns, setBidHns] = useState("");
  const [lockupHns, setLockupHns] = useState("");
  // Per-transaction fee-rate override for the BID (doos/kvB, raw text).
  // Empty = use the global setting default. Scope decision: override lives on
  // Send + Bid + bulk actions, so within this single-name modal only the bid
  // path threads it (the other single-name ceremony actions rotate fast and
  // intentionally don't expose the knob).
  const [bidFeeRate, setBidFeeRate] = useState("");
  const [recoverHns, setRecoverHns] = useState("");
  const [recipient, setRecipient] = useState("");
  const [rows, setRows] = useState<DnsRow[]>([{ type: "TXT", value: "" }]);
  const [advanced, setAdvanced] = useState(false);
  const [recordsJson, setRecordsJson] = useState("[]");
  const [showAllActions, setShowAllActions] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  // Reveal confirm-panel + pending-card state (this PR).
  const [revealConfirming, setRevealConfirming] = useState(false);
  const [optimisticRevealTxid, setOptimisticRevealTxid] = useState<string | null>(null);
  // Fine-grained substate label during the reveal build→sign→broadcast.
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
  // demand. Treat them as "unlocked" so exec.run() skips the unlock step and
  // goes straight to sign (which dispatches to the device).
  const unlocked = isLedger ? true : (signer?.unlocked ?? false);
  const canWrite = writeCap?.canWrite ?? false;
  const lock = !!busy || !canWrite;

  // Client-side bid validation (F4 fix) — shared pure rule (`validateBidInputs`)
  // feeding the single `BidForm` component both the guided and advanced
  // sections render.
  const bidValidation = validateBidInputs(bidHns, lockupHns);
  const {
    formValid: bidFormValid,
    bidError: bidInputError,
    lockupError: lockupInputError,
  } = bidValidation;
  const bidNum = Number(bidHns);
  const lockupNum = Number(lockupHns);
  // What the forfeit warning shows as "X" — the raw lockup input as typed, so
  // it always matches what the user is about to lock up (falls back to "0"
  // before anything is entered, never a stale/guessed number).
  const forfeitLockupText = lockupHns.trim() || "0";

  const badge = auctionPhase(info?.state);
  const countdown = nextTransition(info?.state, info?.stats);
  const guide = AUCTION_PHASE_GUIDE[badge.phase];
  const summary = taskSummaryFromCapabilities(caps);
  // Loading gate for the phase badge: the yellow `badge.label` fallback is the
  // raw on-chain phase (e.g. "Bidding"), which contradicts the table's
  // task-state label (e.g. "Waiting for Bidding") while the SINGLE caps query
  // is still in flight. Suppress the fallback until caps has settled, so the
  // modal never renders a badge that disagrees with the row that opened it.
  // (With the cache bridge in AuctionsView, `caps` is normally seeded on open
  // and `summary` is already present, so this only fires on a cold open or a
  // node-preflight-not-ready case where caps legitimately resolves to null.)
  const capsPending = capsLoading || !capsFetched;

  // Whether the name is owned by the current wallet.
  // Which of the advanced sections exist at this stage, and which are still
  // ahead. `ownsName` cannot answer that: during REVEAL hsd reports the
  // highest revealer as the owner, so it is true for a name the wallet has
  // only bid on. See `resolveSections` for the rules.
  const sections = resolveSections(caps);

  // The editable records section owns the DNS read, the freshness gate and the
  // one-shot seeding. All three follow the section, so they cannot drift apart
  // from what is on screen.
  const recordsLive = sections.records.kind === "live";

  // Before REVEAL, hsd reports the on-chain `value`/`highest` as 0 — every bid
  // is blinded, so the network cannot know the amounts yet. But OUR own bid is
  // not a secret to us: the backend persisted its plaintext in `bid_commitments`
  // and surfaces it as `caps.bidValueDoos`. So when this wallet has a bid
  // commitment on a still-bidding/opening name, show that local value instead
  // of a misleading on-chain 0. On-chain `highest`/`value` are only meaningful
  // once amounts are revealed (REVEAL/CLOSED), so we defer to them there.
  const preReveal = badge.phase === "BIDDING" || badge.phase === "OPENING";
  const showLocalBid = preReveal && caps?.hasBidCommitment === true && caps?.bidValueDoos != null;

  // Current DNS records for owned names, read from the node (`getnameresource`).
  // Used to seed the editor once per open so the user sees/edits/deletes the
  // name's existing records (UPDATE replaces the resource wholesale, so the
  // editor must start from the full current set).
  //
  // `forceFresh` disables the 15s react-query cache and refetches on every
  // open: seeding from a cached pre-UPDATE snapshot would let the user
  // overwrite their on-chain records from a stale base (the reported bug).
  const {
    data: currentRecords,
    isFetching: recordsFetching,
    isError: recordsError,
    dataUpdatedAt: recordsUpdatedAt,
    refetch: refetchRecords,
  } = useNameRecords(open && recordsLive ? name : null, profile?.id ?? null, {
    forceFresh: true,
  });

  // Timestamp of when THIS modal-open began. A records read only counts as
  // "fresh enough to seed / to allow UPDATE" if it landed at or after this
  // moment — i.e. it reflects the current open, not a value cached from a
  // prior session.
  const openedAtRef = useRef<number>(0);
  useEffect(() => {
    if (open) openedAtRef.current = Date.now();
  }, [open]);

  // The records read is guaranteed-fresh when: not currently fetching, we
  // have data, no error, and the data landed at/after this open. Until then
  // the editor must not seed and UPDATE must stay disabled.
  const recordsFresh =
    open &&
    recordsLive &&
    !recordsFetching &&
    !recordsError &&
    currentRecords !== undefined &&
    recordsUpdatedAt >= openedAtRef.current;

  // Seed the editor from the loaded records EXACTLY ONCE per (name, open). A
  // refetch (Update invalidates the `["read"]` prefix) must NOT clobber the
  // user's in-progress edits; closing resets the guard so re-opening re-reads
  // the fresh (post-Update) records.
  const seededForName = useRef<string | null>(null);
  // Id of a draft that was built+persisted for this modal but not yet
  // successfully broadcast. `build_*_draft` persists eagerly (status `draft`)
  // and reserves coins, so an un-broadcast draft left behind will trip the
  // backend double-action guard ("already being opened") on the next attempt.
  // We clear it on broadcast success and discard it on cancel / modal close.
  const pendingDraftRef = useRef<string | null>(null);

  // Discard a built-but-not-broadcast draft, releasing its reserved coins.
  // Safe to call for `draft`/`signed`/`failed` rows; the backend refuses
  // `broadcasted`/`broadcast_pending`/`confirmed` rows, so we never call this
  // after a broadcast-stage failure (the tx may be in flight).
  const discardPendingDraft = async () => {
    const id = pendingDraftRef.current;
    if (!id) return;
    pendingDraftRef.current = null;
    try {
      await deleteDraft.mutateAsync(id);
    } catch {
      // Best-effort cleanup: if the backend refuses (e.g. it raced into a
      // broadcast state), leave the row for the Activity view's Discard.
    }
  };
  useEffect(() => {
    if (!open || !recordsLive) return;
    if (seededForName.current === name) return;
    // Seed ONLY from a guaranteed-fresh read. Never seed from a stale cache
    // or an in-flight/undefined value — that's the stale-editor bug.
    if (!recordsFresh) return;
    // Seed the editor from the fresh on-chain set. Only OVERWRITE the rows
    // when the name actually has records to prefill: an empty fresh read means
    // "no records" — the editor's default blank row already represents that,
    // and overwriting here would clobber anything the user typed while the
    // read was in flight (e.g. a REGISTER-from-scratch in the guided flow).
    // The stale-editor bug is already prevented upstream: `recordsFresh`
    // gates this effect, so a stale non-empty read can never seed.
    const seeded = recordsToRows(currentRecords?.records ?? []);
    if (seeded.length) {
      setRows(seeded);
    }
    setRecordsJson(JSON.stringify(currentRecords?.records ?? [], null, 2));
    seededForName.current = name;
  }, [open, recordsLive, name, recordsFresh, currentRecords]);
  useEffect(() => {
    if (!open) seededForName.current = null;
  }, [open]);

  // Reset the reveal confirm-panel + optimistic-txid state whenever the modal
  // closes or the name changes, so reopening starts clean (a stale optimistic
  // txid must never leak across names). The derived taskState (which survives
  // reload) is what re-shows the pending card on reopen — not this local state.
  useEffect(() => {
    setRevealConfirming(false);
    setOptimisticRevealTxid(null);
  }, [open, name]);

  // Close cleanup: if the user dismisses the modal (open → false) while a
  // built-but-not-broadcast draft is still tracked — e.g. they closed the
  // action modal rather than pressing Cancel on the Confirm & Sign overlay —
  // discard it so its reserved coins are freed and the next attempt isn't
  // blocked by the double-action guard. Only fires on the true→false edge, so
  // it never runs on mount or while the modal is open.
  const prevOpenRef = useRef(open);
  useEffect(() => {
    if (prevOpenRef.current && !open) {
      void discardPendingDraft();
    }
    prevOpenRef.current = open;
    // discardPendingDraft is a stable closure over refs/hooks; intentionally
    // gated on `open` only so it fires exactly on the close edge.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Owned names with no urgent auction task (already registered, nothing to
  // finalize) auto-expand the management section, so the user isn't forced to
  // click "Manage actions" to see their Transfer/Renew/Finalize/Revoke
  // controls. Names still mid-flow (just-won/needs-register, lost/needs-redeem)
  // keep their dedicated guided action up front instead, to avoid duplicating
  // it inside the advanced section. Gated on `caps` (not a pre-caps guess
  // fallback) so a still-loading response can't transiently look like
  // "owned, no task" and expand a section that collapses back once the real
  // taskState arrives. Fires once when this becomes true; the user can still
  // collapse it afterward via the toggle.
  const shouldAutoExpandManagement =
    caps?.nameIsRegistered === true &&
    caps.taskState !== "wonNeedsRegister" &&
    caps.taskState !== "lostNeedsRedeem";
  useEffect(() => {
    if (shouldAutoExpandManagement) setShowAllActions(true);
  }, [shouldAutoExpandManagement]);

  // Whether the modal offers anything beyond plain info, for the guided panel
  // and the unlock notice. Deliberately looser than `sections.anyLive`, which
  // governs the advanced area alone: this one has a phase-based fallback for
  // the window before capabilities load.
  const hasRelevantActions =
    // Phase-based fallback (used when caps are null/loading)
    badge.phase === "AVAILABLE" ||
    badge.phase === "BIDDING" ||
    badge.phase === "REVEAL" ||
    // Capability-based (authoritative when loaded)
    caps?.taskState === "wonNeedsRegister" ||
    caps?.taskState === "lostNeedsRedeem" ||
    caps?.taskState === "transferPendingFinalize" ||
    // A registered name has update/transfer/renew/revoke actions. Merely
    // leading an auction does not — the owner coin is still a REVEAL.
    caps?.nameIsRegistered === true;

  // The auction exists but its bidding window has not opened: the name is in
  // OPENING, or an OPEN this wallet broadcast is still waiting for a block.
  // (It once also meant "this wallet has already bid", back when a second bid
  // was refused; several bids are allowed now, so BIDDING never reports this.)
  const biddingNotOpenYet = caps?.taskState === "waitingForBidding";

  // Whether the modal actually offers something to sign/broadcast right now.
  // `hasRelevantActions` includes a phase-based fallback that is true before
  // the bidding window opens, when every action is caps-disabled and there is
  // nothing to submit — the "unlock to sign" notice would be pointless there.
  // Gate signable UI on this instead of the looser `hasRelevantActions`.
  const hasSignableActions = hasRelevantActions && !biddingNotOpenYet;

  // One rule for the toggle: open it only when something behind it can be
  // acted on. Every "is this phase meaningful?" special case this used to
  // carry is now a section state, so a menu of nothing but upcoming lines
  // never gets a button to open it.
  const showAdvancedToggle = sections.anyLive;

  // Use capabilities to determine if an action is disabled and why.
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

  // Whether the modal-wide write gate below is on screen. It already states the
  // write-capability reason and carries its own Unlock button.
  const writeGateVisible = !canWrite && hasSignableActions;

  // The reason a guided panel shows in its banner. Same as `actionReason`,
  // except it does not repeat the write-capability reason while the gate is
  // showing it — otherwise the same sentence, and its Unlock button, appear
  // twice. Tooltips keep using `actionReason`: a title on a disabled button
  // isn't a duplicate of anything.
  const guidedReason = (cap?: NameActionCapability): string | null => {
    if (!canWrite) return writeGateVisible ? null : actionReason(cap);
    return actionReason(cap);
  };

  const run = async (label: string, builder: () => Promise<{ id: string }>) => {
    if (!profile) return;
    setBusy(label);
    let draft: { id: string };
    try {
      draft = await builder();
    } catch (e) {
      showToast(mapError(e, "build"), "error");
      setBusy(null);
      return;
    }
    // The draft is now persisted (status `draft`) with coins reserved. Track it
    // so a cancel / close discards it instead of orphaning it.
    pendingDraftRef.current = draft.id;
    try {
      const result = await exec.run(draft.id, profile.id, unlocked);
      // Broadcast succeeded — the draft is now owned by the chain, not us.
      pendingDraftRef.current = null;
      showToast(`${label} broadcast — ${result.txid.slice(0, 12)}…`, "success");
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
      onClose();
    } catch (e) {
      // exec.run() tags its rejection with which leg of unlock→sign→broadcast
      // threw (see useExecuteDraft) — thread that through to the toast.
      showToast(mapError(unwrapStaged(e), stageOf(e)), "error");
      // A cancel on the Confirm & Sign overlay (or an unlock/sign failure)
      // rejects at the `sign` stage, before broadcast — discard the orphan so
      // the next attempt isn't blocked by the double-action guard. A
      // `broadcast`-stage failure may have a tx in flight, so we keep the row.
      if (stageOf(e) !== "broadcast") {
        await discardPendingDraft();
      } else {
        pendingDraftRef.current = null;
      }
    } finally {
      setBusy(null);
    }
  };

  // Reveal confirm-and-broadcast: builds the reveal draft, runs the
  // unlock→sign→broadcast pipeline, then stays in the modal (shows the
  // pending card) rather than closing. On success, sets the optimistic txid
  // so the card renders immediately (before the next caps poll).
  const handleRevealConfirm = async () => {
    if (!profile) return;
    setBusy("REVEAL");
    let draft: { id: string };
    try {
      draft = await build.reveal.mutateAsync({ name });
    } catch (e) {
      showToast(mapError(e, "build"), "error");
      setBusy(null);
      return;
    }
    pendingDraftRef.current = draft.id;
    try {
      const result = await exec.run(draft.id, profile.id, unlocked);
      // Success: stay in the modal, show the pending card.
      pendingDraftRef.current = null;
      setOptimisticRevealTxid(result.txid);
      setRevealConfirming(false);
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
    } catch (e) {
      showToast(mapError(unwrapStaged(e), stageOf(e)), "error");
      // On failure, stay in the confirm panel so the user can retry.
      // A pre-broadcast cancel/failure orphans the reveal draft — discard it so
      // a retry isn't blocked. Keep it only if broadcast may be in flight.
      if (stageOf(e) !== "broadcast") {
        await discardPendingDraft();
      } else {
        pendingDraftRef.current = null;
      }
    } finally {
      setBusy(null);
    }
  };

  // Recover a lost bid_commitments row from the on-chain BID coin + a
  // user-remembered bid amount (see `recover_bid_commitment`). Needs only the
  // account xpub (public), so it works without unlocking the signer.
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

  // Auto-recover a lost bid_commitments row WITHOUT the user remembering the
  // amount (see `brute_force_recover_bid`). Brute-forces the bid value against
  // the on-chain blind; works for bids made in any hsd-compatible wallet.
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
      // REGISTER keeps the existing semantics: `null` records → empty resource.
      run(label, () => build.register.mutateAsync({ name, records: recs }));
      return;
    }
    // UPDATE replaces the resource wholesale. An empty editor means "delete all
    // records", which must send `[]` (empty resource), NOT `null` —
    // `build_update_draft` expects a `Vec`, so `null` would break it.
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

  return (
    <Dialog
      open={open}
      onClose={onClose}
      className="max-w-2xl"
      title={
        <span className="flex items-center gap-2 flex-wrap">
          <span>
            {decodedName === name ? (
              `.${name}`
            ) : (
              <>
                .{decodedName} <span className="text-xs font-normal text-gray-400">(.{name})</span>
              </>
            )}
          </span>
          {/* Phase badge (moved from the body so it sits right of the name,
              matching the former NameInfoModal). Same loading gate: task-state
              summary when known, a neutral placeholder while caps is pending,
              and only then the raw on-chain phase. data-testid stays here so
              the tests that assert on the phase label follow it. */}
          {!isLoading &&
            !isError &&
            (summary?.pendingBroadcastAction ? (
              // A transaction of ours for this name is in flight, so the task
              // label is a verdict the chain has not reached and an
              // instruction the user has already followed. The guided panel
              // below says the same thing; the header used to contradict it
              // in red. The auctions list already reads this way.
              <Badge variant="default" data-testid="name-phase">
                {pendingBroadcastBadge(summary.pendingBroadcastAction)}
              </Badge>
            ) : summary ? (
              <Badge variant={summary.variant} data-testid="name-phase">
                {summary.label}
              </Badge>
            ) : capsPending ? (
              <Badge variant="default" data-testid="name-phase">
                <span data-testid="name-phase-loading">Checking…</span>
              </Badge>
            ) : (
              <Badge variant={badge.variant} data-testid="name-phase">
                {badge.label}
              </Badge>
            ))}
        </span>
      }
    >
      <div className="space-y-4 text-sm">
        {/* Explorer link (mainnet only — Shakeshift indexes no other chain, so
            the link would 404) + watchlist toggle */}
        <div className="flex items-center justify-between gap-2">
          {profile?.network === "mainnet" ? (
            <button
              type="button"
              className="text-xs text-blue-500 hover:text-blue-700 hover:underline cursor-pointer inline-flex items-center gap-1"
              onClick={() => openExternal(explorerNameUrl(name))}
              data-testid="name-explorer-link"
            >
              View on explorer ↗
            </button>
          ) : (
            <span />
          )}
          <WatchlistToggle name={name} />
        </div>

        {/* Loading state */}
        {isLoading && (
          <div className="text-center py-4">
            <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-blue-600 mx-auto"></div>
            <div className="mt-2 text-sm text-gray-600">Loading name info...</div>
          </div>
        )}

        {/* Error state */}
        {isError && (
          <div className="bg-red-50 border border-red-300 rounded p-3 text-sm text-red-800">
            <div className="font-medium">Failed to load name info</div>
            <div className="mt-1 text-xs">{error?.message || "Unknown error"}</div>
          </div>
        )}

        {/* Phase header - only show when data is loaded and no error */}
        {/* Phase meta row — the phase badge itself now lives in the modal
            title; here we keep the countdown and high-bid/value summary. Only
            render the row when there's something to show. */}
        {!isLoading &&
          !isError &&
          (countdown || showLocalBid || (info?.highest ?? info?.value) != null) && (
            <div className="flex items-center justify-between gap-3 bg-gray-50 border border-gray-200 rounded p-2">
              <div className="flex items-center gap-2">
                {countdown && (
                  <span className="text-xs text-gray-600" data-testid="name-countdown">
                    {countdown.label} {formatCountdown(countdown)}
                  </span>
                )}
              </div>
              {showLocalBid ? (
                <span className="text-xs text-gray-700" data-testid="name-your-bid">
                  {(caps?.myBidCount ?? 0) > 1 ? "Latest bid" : "Your bid"}{" "}
                  <Tooltip content={<>{formatHns(caps?.bidValueDoos)} HNS</>}>
                    <span className="cursor-help underline decoration-dotted underline-offset-2">
                      {formatHnsShort(caps?.bidValueDoos)} HNS
                    </span>
                  </Tooltip>
                  {caps?.lockupValueDoos != null && (
                    <span data-testid="name-your-lockup">
                      {" · lockup "}
                      <Tooltip content={<>{formatHns(caps.lockupValueDoos)} HNS</>}>
                        <span className="cursor-help underline decoration-dotted underline-offset-2">
                          {formatHnsShort(caps.lockupValueDoos)} HNS
                        </span>
                      </Tooltip>
                    </span>
                  )}
                  {(caps?.myBidCount ?? 0) > 1 && (
                    <span className="text-gray-400" data-testid="name-your-bid-count">
                      {` · ${caps?.myBidCount} of yours`}
                    </span>
                  )}
                </span>
              ) : (
                (info?.highest ?? info?.value) != null && (
                  <span className="text-xs text-gray-500">
                    {info?.highest != null ? `High bid ${formatHns(info.highest)} HNS` : ""}
                    {info?.value != null ? ` · value ${formatHns(info.value)} HNS` : ""}
                  </span>
                )
              )}
            </div>
          )}

        {/* Ownership indicator — shown when the wallet genuinely holds the
            name. Not `ownsName`: during REVEAL hsd reports the highest
            revealer as the owner, and a green "Owned by this wallet" on a
            name still being auctioned is the claim a user has least reason to
            question. `wonNeedsRegister` counts — the name is held, only the
            first resource has yet to be published. */}
        {(caps?.nameIsRegistered === true || caps?.taskState === "wonNeedsRegister") && (
          <div
            className="bg-green-50 border border-green-200 rounded p-2 text-xs text-green-800"
            data-testid="ownership-indicator"
          >
            <span className="font-semibold">Owned by this wallet</span>
            {caps?.taskState === "ownedNoUrgentAction" && (
              <span> — This name is registered and controlled by your wallet.</span>
            )}
            {caps?.taskState === "wonNeedsRegister" && (
              <span> — You won the auction. Register to finalize ownership.</span>
            )}
          </div>
        )}

        {/* Write-capability gate — only show when there are relevant actions */}
        {/* Write-capability gate — only when there is actually something to
            sign. If the modal has nothing to submit (e.g. this wallet already
            bid and is just waiting), the "unlock to sign" notice is noise. */}
        {writeGateVisible && (
          <div
            className="bg-red-50 border border-red-300 rounded p-2 text-xs text-red-800"
            role="alert"
            data-testid="name-actions-blocked"
          >
            <div className="flex items-center justify-between gap-2">
              <span>
                <span className="font-semibold">Name actions unavailable.</span>{" "}
                {writeCap?.reason ??
                  "Connect a fully-synced, address-indexed node and unlock your signer to manage names."}
              </span>
              <UnlockButton size="sm" variant="primary" />
            </div>
          </div>
        )}

        {/* Guided action - only show when loaded and no error.
            For CLOSED phase, only show when there is an actionable task
            (won/register, lost/redeem, or owned).
            Skip for third-party CLOSED names. */}
        {!isLoading &&
          !isError &&
          guide &&
          (badge.phase !== "CLOSED" ||
          caps?.ownsName ||
          caps?.taskState === "wonNeedsRegister" ||
          caps?.taskState === "lostNeedsRedeem" ? (
            <div className="bg-blue-50 border border-blue-200 rounded p-3">
              <div className="font-medium text-blue-900 mb-2">
                {summary?.nextActionLabel ?? guide.title}
              </div>
              <GuidedAction
                badge={badge}
                guide={guide}
                countdown={countdown}
                caps={caps}
                summary={summary}
                busy={busy}
                actionDisabled={actionDisabled}
                actionReason={guidedReason}
                onOpen={() => run("OPEN", () => build.open.mutateAsync({ name }))}
                onRedeem={() => run("REDEEM", () => build.redeem.mutateAsync({ name }))}
                onRegister={() => submitRecords("REGISTER")}
                bidHns={bidHns}
                onBidChange={setBidHns}
                lockupHns={lockupHns}
                onLockupChange={setLockupHns}
                bidError={bidInputError}
                lockupError={lockupInputError}
                bidFormValid={bidFormValid}
                forfeitLockupText={forfeitLockupText}
                onBid={submitBid}
                recoverHns={recoverHns}
                onRecoverHnsChange={setRecoverHns}
                onRecoverBid={handleRecoverBid}
                onBruteForceRecover={handleBruteForceRecover}
                revealConfirming={revealConfirming}
                onRevealConfirmStart={() => setRevealConfirming(true)}
                onRevealConfirmCancel={() => setRevealConfirming(false)}
                onRevealConfirm={handleRevealConfirm}
                revealSubstate={revealSubstate}
                optimisticRevealTxid={optimisticRevealTxid}
                rows={rows}
                onRowChange={setRow}
                onAddRow={addRow}
                onRemoveRow={removeRow}
                isMainnet={profile?.network === "mainnet"}
              />
              {badge.phase === "BIDDING" && caps?.canBid?.allowed ? (
                <div className="mt-3">
                  <FeeRateOverride
                    value={bidFeeRate}
                    onChange={setBidFeeRate}
                    label="Fee rate override"
                  />
                </div>
              ) : null}
            </div>
          ) : badge.phase === "CLOSED" ? (
            <div className="bg-blue-50 border border-blue-200 rounded p-3">
              <div className="font-medium text-blue-900 mb-2">Name details</div>
              <div className="text-sm text-gray-700">
                This name is already registered. No auction actions are needed for this name.
                <div className="mt-1 text-xs text-gray-500">
                  Phase: {badge.label}
                  {info?.value != null && ` · Value: ${formatHns(info.value)} HNS`}
                </div>
              </div>
            </div>
          ) : null)}

        <NameBidsPanel
          name={name}
          profileId={profile?.id ?? null}
          phase={badge.phase}
          suppressEmptyHint={biddingNotOpenYet}
        />

        {/* A bid this wallet placed in an EARLIER auction of this name whose
            lockup is stranded. The bids panel above is scoped to the current
            auction, so without this the coin is simply missing from the UI
            with nothing to explain where the money went. */}
        {(caps?.strandedBidCount ?? 0) > 0 && (
          <div
            className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2"
            data-testid="stranded-bids"
          >
            <span className="font-semibold">
              {caps!.strandedBidCount === 1
                ? "1 bid from an earlier auction of this name"
                : `${caps!.strandedBidCount} bids from earlier auctions of this name`}
            </span>{" "}
            {formatHnsShort(caps!.strandedLockupDoos ?? 0)} HNS is still locked in{" "}
            {caps!.strandedBidCount === 1 ? "it" : "them"}. That auction closed without a reveal,
            and a bid can only be revealed while its own auction is running — so the lockup cannot
            be recovered.
          </div>
        )}

        {/* Read-only on-chain details (heights, transfer, owner UTXO, closed
            values, DNS records) — the former NameInfoModal, folded in so one
            modal serves both inspection and actions. For owned names the
            editable DnsRecordsEditor below owns the records, so suppress the
            read-only DNS block here to avoid showing them twice. */}
        {!isLoading && !isError && (
          <NameDetails
            name={name}
            profileId={profile?.id ?? null}
            info={info}
            hideDnsRecords={recordsLive}
          />
        )}

        {/* Advanced actions toggle — only when relevant actions exist */}
        {showAdvancedToggle && (
          <div>
            <button
              type="button"
              className="text-xs text-blue-600 hover:underline"
              onClick={() => setShowAllActions((a) => !a)}
              data-testid="all-actions-toggle"
            >
              {showAllActions
                ? "Hide advanced actions"
                : // "Manage" is only true once there is a name to manage; on a
                  // name still being auctioned it promises controls that the
                  // sections below deliberately do not render.
                  caps?.nameIsRegistered
                  ? "Manage actions"
                  : "Show all actions"}
            </button>
          </div>
        )}

        {/* `anyLive` guards the container as well as the toggle: auto-expand can
            leave `showAllActions` true while every section has since gone
            absent — a broadcast going out does exactly that — and the bordered
            box would render with nothing in it. */}
        {showAllActions && sections.anyLive && (
          <div className="space-y-4 border-t border-gray-200 pt-4" data-testid="advanced-actions">
            {sections.auction.kind === "upcoming" && (
              <UpcomingSection
                id="auction"
                title="Manual auction actions"
                when={sections.auction.when}
              />
            )}
            {sections.auction.kind === "live" && (
              <section className="space-y-2">
                <div className="font-medium text-gray-700">Manual auction actions</div>
                <div className="text-xs text-gray-500">
                  The guided panel above already does this. Use these only if it has fallen out of
                  step with the chain.
                </div>
                {/* Only what this stage actually allows. Open / Reveal /
                    Redeem used to all render here, two of them permanently
                    greyed, which is the wall of dead controls the section
                    states exist to remove — and at button granularity it is
                    worse, because a covenant name with no explanation reads as
                    a thing the user failed to understand. Each live one says
                    what pressing it does. */}
                {caps?.canOpen?.allowed && (
                  <div className="space-y-1">
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={actionDisabled("OPEN", caps?.canOpen)}
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
                      disabled={actionDisabled("REVEAL", caps?.canReveal)}
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
                      disabled={actionDisabled("REDEEM", caps?.canRedeem)}
                      onClick={() => run("REDEEM", () => build.redeem.mutateAsync({ name }))}
                    >
                      {busy === "REDEEM" ? "…" : "Redeem"}
                    </Button>
                    <div className="text-xs text-gray-500" data-testid="redeem-explainer">
                      {redeemExplainer(caps)}
                    </div>
                  </div>
                )}
              </section>
            )}

            {sections.records.kind === "upcoming" && (
              <UpcomingSection id="records" title="DNS records" when={sections.records.when} />
            )}
            {recordsLive && (
              <section className="space-y-2">
                <div className="flex items-center justify-between">
                  <div className="font-medium text-gray-700">DNS records (REGISTER / UPDATE)</div>
                  <button
                    type="button"
                    className="text-xs text-blue-600 hover:underline"
                    onClick={() => setAdvanced((a) => !a)}
                    data-testid="dns-advanced-toggle"
                  >
                    {advanced ? "Use row editor" : "Advanced (raw JSON)"}
                  </button>
                </div>

                {/* Freshness gate. The editor seeds and UPDATE is enabled ONLY
                    from a guaranteed-fresh read of the current on-chain
                    records — otherwise the user could overwrite their resource
                    from a stale base (the stale-editor bug). */}
                {recordsFetching && (
                  <div className="text-xs text-gray-500" data-testid="dns-records-loading">
                    Loading current on-chain records…
                  </div>
                )}
                {!recordsFetching && !recordsFresh && (
                  <div
                    className="text-xs text-red-700 bg-red-50 border border-red-200 rounded p-2 flex items-center justify-between gap-2"
                    data-testid="dns-records-stale-banner"
                  >
                    <span>
                      Can&apos;t read this name&apos;s current on-chain records. The Update button
                      is disabled to avoid overwriting your records from an incomplete view — make
                      sure your node is running and fully synced, then retry.
                    </span>
                    <button
                      type="button"
                      className="shrink-0 text-blue-600 hover:underline"
                      onClick={() => refetchRecords()}
                      data-testid="dns-records-retry"
                    >
                      Retry
                    </button>
                  </div>
                )}
                {recordsFresh && currentRecords?.records?.length === 0 && (
                  <div className="text-xs text-gray-400" data-testid="dns-records-hint">
                    This name has no records yet. Add records below and Update to publish them.
                  </div>
                )}

                {/* Only render the editor once the fresh read has seeded it —
                    prevents the user from typing into a not-yet-seeded editor
                    whose rows would be clobbered by the incoming seed. */}
                {!recordsFresh ? null : advanced ? (
                  <textarea
                    className="w-full border border-gray-300 rounded px-2 py-1 font-mono text-xs h-20"
                    value={recordsJson}
                    onChange={(e) => setRecordsJson(e.target.value)}
                    placeholder='[{"type":"TXT","txt":["hello"]}]'
                    data-testid="dns-json"
                  />
                ) : (
                  <DnsRecordsEditor
                    variant="advanced"
                    rows={rows}
                    onRowChange={setRow}
                    onAddRow={addRow}
                    onRemoveRow={removeRow}
                  />
                )}

                <div className="flex gap-2">
                  {/* The guided panel above owns Register whenever it is the
                      step the name is on. Two identical live buttons for one
                      action leave the user choosing between them with nothing
                      to choose on. */}
                  {caps?.taskState !== "wonNeedsRegister" && (
                    <ActionHint
                      reason={
                        !recordsFresh
                          ? "Waiting for a fresh read of the current on-chain records"
                          : actionReason(caps?.canRegister)
                      }
                    >
                      <Button
                        size="sm"
                        variant="secondary"
                        disabled={actionDisabled("REGISTER", caps?.canRegister) || !recordsFresh}
                        onClick={() => submitRecords("REGISTER")}
                      >
                        {busy === "REGISTER" ? "…" : "Register"}
                      </Button>
                    </ActionHint>
                  )}
                  <ActionHint
                    reason={
                      !recordsFresh
                        ? "Waiting for a fresh read of the current on-chain records"
                        : actionReason(caps?.canUpdate)
                    }
                  >
                    <Button
                      size="sm"
                      disabled={actionDisabled("UPDATE", caps?.canUpdate) || !recordsFresh}
                      onClick={() => submitRecords("UPDATE")}
                    >
                      {busy === "UPDATE" ? "…" : "Update"}
                    </Button>
                  </ActionHint>
                </div>
              </section>
            )}

            {sections.ownership.kind === "upcoming" && (
              <UpcomingSection id="ownership" title="Ownership" when={sections.ownership.when} />
            )}
            {sections.ownership.kind === "live" && (
              <>
                <OwnershipActions
                  caps={caps}
                  busy={busy}
                  recipient={recipient}
                  onRecipientChange={setRecipient}
                  actionDisabled={actionDisabled}
                  actionReason={actionReason}
                  onTransfer={() =>
                    run("TRANSFER", () =>
                      build.transfer.mutateAsync({ name, recipient: recipient.trim() }),
                    )
                  }
                  onFinalize={() => run("FINALIZE", () => build.finalize.mutateAsync({ name }))}
                  onCancelTransfer={() => run("CANCEL", () => build.cancel.mutateAsync({ name }))}
                  onRenew={() => run("RENEW", () => build.renew.mutateAsync({ name }))}
                  onRevoke={() => run("REVOKE", () => build.revoke.mutateAsync({ name }))}
                />
                {/* Proving ownership belongs to the ownership section, and needs
                  the same registration: a signature over a name the wallet has
                  only bid on is a claim every verifier resolves as false. */}
                <NameSignMessage name={name} profileId={profile?.id ?? null} caps={caps} />
              </>
            )}

            {/* Paid swap claim: shown when a paid_swap_offer exists for this name */}
            <PaidSwapClaim name={name} />
          </div>
        )}

        <div className="flex justify-end">
          <Button variant="ghost" onClick={onClose} disabled={!!busy}>
            Close
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
