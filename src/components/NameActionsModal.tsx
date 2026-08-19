import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useActiveProfile,
  useSignerSession,
  useWriteCapability,
  useNameAction,
  useExecuteDraft,
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
import { BidForm } from "./name-actions/BidForm";
import { DnsRecordsEditor } from "./name-actions/DnsRecordsEditor";
import { GuidedAction } from "./name-actions/GuidedAction";
import { NameBidsPanel } from "./name-actions/NameBidsPanel";
import { NameSignMessage } from "./name-actions/NameSignMessage";
import { OwnershipActions } from "./name-actions/OwnershipActions";
import { PaidSwapClaim } from "./name-actions/PaidSwapClaim";
import { useUiStore } from "../stores/ui";
import { FeeRateOverride } from "./ui/FeeRateOverride";
import { parseFeeRateArg } from "../lib/feeRate";
import { mapError, stageOf, unwrapStaged } from "../lib/errors";
import { formatHns } from "../lib/utils";
import { displayName } from "../lib/idn";
import { WatchlistToggle } from "./WatchlistToggle";
import { explorerNameUrl, openExternal } from "../lib/openExternal";
import {
  auctionPhase,
  nextTransition,
  formatCountdown,
  AUCTION_PHASE_GUIDE,
  taskSummaryFromCapabilities,
  validateBidInputs,
} from "../lib/auction";
import { hnsToDollarydoos } from "../lib/utils";
import { rowsToRecords, recordsToRows, type DnsRow } from "../lib/dnsRecords";
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
  const { data: caps } = useNameActionCapabilities(
    open ? name : null,
    profile?.id ?? null,
  );
  const exec = useExecuteDraft();
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
  const revealSubstate: string | null =
    exec.unlock.isPending
      ? "Unlocking…"
      : exec.sign.isPending
        ? "Signing…"
        : exec.broadcast.isPending
          ? "Broadcasting…"
          : null;

  const unlocked = signer?.unlocked ?? false;
  const canWrite = writeCap?.canWrite ?? false;
  const lock = !!busy || !canWrite;

  // Client-side bid validation (F4 fix) — shared pure rule (`validateBidInputs`)
  // feeding the single `BidForm` component both the guided and advanced
  // sections render.
  const bidValidation = validateBidInputs(bidHns, lockupHns);
  const { formValid: bidFormValid, bidError: bidInputError, lockupError: lockupInputError } = bidValidation;
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

  // Whether the name is owned by the current wallet.
  const isOwned = caps?.ownsName ?? (!!info?.owner && info?.registered === true);

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
  } = useNameRecords(open && isOwned ? name : null, profile?.id ?? null, {
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
    isOwned &&
    !recordsFetching &&
    !recordsError &&
    currentRecords !== undefined &&
    recordsUpdatedAt >= openedAtRef.current;

  // Seed the editor from the loaded records EXACTLY ONCE per (name, open). A
  // refetch (Update invalidates the `["read"]` prefix) must NOT clobber the
  // user's in-progress edits; closing resets the guard so re-opening re-reads
  // the fresh (post-Update) records.
  const seededForName = useRef<string | null>(null);
  useEffect(() => {
    if (!open || !isOwned) return;
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
  }, [open, isOwned, name, recordsFresh, currentRecords]);
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

  // Owned names with no urgent auction task (already registered, nothing to
  // finalize) auto-expand the management section, so the user isn't forced to
  // click "Manage actions" to see their Transfer/Renew/Finalize/Revoke
  // controls. Names still mid-flow (just-won/needs-register, lost/needs-redeem)
  // keep their dedicated guided action up front instead, to avoid duplicating
  // it inside the advanced section. Gated on `caps` (not the pre-caps `isOwned`
  // fallback) so a still-loading response can't transiently look like
  // "owned, no task" and expand a section that collapses back once the real
  // taskState arrives. Fires once when this becomes true; the user can still
  // collapse it afterward via the toggle.
  const shouldAutoExpandManagement =
    caps?.ownsName === true &&
    caps.taskState !== "wonNeedsRegister" &&
    caps.taskState !== "lostNeedsRedeem";
  useEffect(() => {
    if (shouldAutoExpandManagement) setShowAllActions(true);
  }, [shouldAutoExpandManagement]);

  // Whether there are any user-actionable controls in this modal beyond plain info.
  // Falls back to phase-based check when capabilities haven't loaded yet.
  const hasRelevantActions =
    // Phase-based fallback (used when caps are null/loading)
    badge.phase === "AVAILABLE" ||
    badge.phase === "BIDDING" ||
    badge.phase === "REVEAL" ||
    // Capability-based (authoritative when loaded)
    caps?.taskState === "wonNeedsRegister" ||
    caps?.taskState === "lostNeedsRedeem" ||
    caps?.taskState === "transferPendingFinalize" ||
    // Owned names have update/transfer/renew/revoke actions
    (caps?.ownsName === true);

  // Show the advanced toggle only when there are meaningful extra actions behind it.
  const showAdvancedToggle = hasRelevantActions && (
    // Auction-phase advanced actions are always meaningful.
    badge.phase !== "CLOSED" ||
    // For CLOSED owned names: only show if there are ownership actions the user may want.
    (caps?.ownsName === true)
  );

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

  const run = async (
    label: string,
    builder: () => Promise<{ id: string }>,
  ) => {
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
    try {
      const result = await exec.run(draft.id, profile.id, unlocked);
      showToast(`${label} broadcast — ${result.txid.slice(0, 12)}…`, "success");
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
      onClose();
    } catch (e) {
      // exec.run() tags its rejection with which leg of unlock→sign→broadcast
      // threw (see useExecuteDraft) — thread that through to the toast.
      showToast(mapError(unwrapStaged(e), stageOf(e)), "error");
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
    try {
      const result = await exec.run(draft.id, profile.id, unlocked);
      // Success: stay in the modal, show the pending card.
      setOptimisticRevealTxid(result.txid);
      setRevealConfirming(false);
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
    } catch (e) {
      showToast(mapError(unwrapStaged(e), stageOf(e)), "error");
      // On failure, stay in the confirm panel so the user can retry.
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
      title={
        decodedName === name ? (
          `.${name}`
        ) : (
          <>
            .{decodedName}{" "}
            <span className="text-xs font-normal text-gray-400">(.{name})</span>
          </>
        )
      }
      >
      <div className="space-y-4 text-sm">
        {/* Explorer link + watchlist toggle */}
        <div className="flex items-center justify-between gap-2">
          <button
            type="button"
            className="text-xs text-blue-500 hover:text-blue-700 hover:underline cursor-pointer inline-flex items-center gap-1"
            onClick={() => openExternal(explorerNameUrl(name))}
            data-testid="name-explorer-link"
          >
            View on explorer ↗
          </button>
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
        {!isLoading && !isError && (
          <div
            className="flex items-center justify-between gap-3 bg-gray-50 border border-gray-200 rounded p-2"
            data-testid="name-phase"
          >
            <div className="flex items-center gap-2">
              {/* Show task state badge when available, fall back to phase */}
              {summary ? (
                <Badge variant={summary.variant}>{summary.label}</Badge>
              ) : (
                <Badge variant={badge.variant}>{badge.label}</Badge>
              )}
              {countdown && (
                <span className="text-xs text-gray-600" data-testid="name-countdown">
                  {countdown.label} {formatCountdown(countdown)}
                </span>
              )}
            </div>
            {(info?.highest ?? info?.value) != null && (
              <span className="text-xs text-gray-500">
                {info?.highest != null ? `High bid ${formatHns(info.highest)} HNS` : ""}
                {info?.value != null ? ` · value ${formatHns(info.value)} HNS` : ""}
              </span>
            )}
          </div>
        )}

        {/* Ownership indicator — shown when the wallet controls this name */}
        {isOwned && (
          <div className="bg-green-50 border border-green-200 rounded p-2 text-xs text-green-800" data-testid="ownership-indicator">
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
        {!canWrite && hasRelevantActions && (
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
        {!isLoading && !isError && guide && (
          (badge.phase !== "CLOSED" || caps?.ownsName || caps?.taskState === "wonNeedsRegister" || caps?.taskState === "lostNeedsRedeem") ? (
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
                actionReason={actionReason}
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
          ) : null
        )}

        <NameBidsPanel name={name} profileId={profile?.id ?? null} phase={badge.phase} />

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
                : caps?.ownsName
                  ? "Manage actions"
                  : "Show all actions"
              }
            </button>
          </div>
        )}

        {showAllActions && (
          <div className="space-y-4 border-t border-gray-200 pt-4">
            {/* Auction actions - always show for all names */}
            <section className="space-y-2">
              <div className="font-medium text-gray-700">Auction</div>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm" variant="secondary"
                  disabled={actionDisabled("OPEN", caps?.canOpen)}
                  title={actionReason(caps?.canOpen) ?? ""}
                  onClick={() => run("OPEN", () => build.open.mutateAsync({ name }))}
                >
                  {busy === "OPEN" ? "…" : "Open"}
                </Button>
                <Button
                  size="sm" variant="secondary"
                  disabled={actionDisabled("REVEAL", caps?.canReveal)}
                  title={actionReason(caps?.canReveal) ?? ""}
                  onClick={() => run("REVEAL", () => build.reveal.mutateAsync({ name }))}
                >
                  {busy === "REVEAL" ? "…" : "Reveal"}
                </Button>
                <Button
                  size="sm" variant="secondary"
                  disabled={actionDisabled("REDEEM", caps?.canRedeem)}
                  title={actionReason(caps?.canRedeem) ?? ""}
                  onClick={() => run("REDEEM", () => build.redeem.mutateAsync({ name }))}
                >
                  {busy === "REDEEM" ? "…" : "Redeem"}
                </Button>
              </div>
              <BidForm
                variant="advanced"
                bidHns={bidHns}
                onBidChange={setBidHns}
                lockupHns={lockupHns}
                onLockupChange={setLockupHns}
                bidError={bidInputError}
                lockupError={lockupInputError}
                forfeitLockupText={forfeitLockupText}
                disabled={actionDisabled("BID", caps?.canBid) || !bidFormValid}
                busy={busy === "BID"}
                onSubmit={submitBid}
                idleLabel="Bid"
                busyLabel="…"
                submitTitle={actionReason(caps?.canBid) ?? ""}
              />
            </section>

            {/* DNS records (REGISTER / UPDATE) - only show for owned names */}
            {isOwned && (
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
                      Can&apos;t read this name&apos;s current on-chain records. The
                      Update button is disabled to avoid overwriting your records from an
                      incomplete view — make sure your node is running and fully synced,
                      then retry.
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
                  <Button
                    size="sm" variant="secondary"
                    disabled={actionDisabled("REGISTER", caps?.canRegister) || !recordsFresh}
                    title={
                      !recordsFresh
                        ? "Waiting for a fresh read of the current on-chain records"
                        : actionReason(caps?.canRegister) ?? ""
                    }
                    onClick={() => submitRecords("REGISTER")}
                  >
                    {busy === "REGISTER" ? "…" : "Register"}
                  </Button>
                  <Button
                    size="sm"
                    disabled={actionDisabled("UPDATE", caps?.canUpdate) || !recordsFresh}
                    title={
                      !recordsFresh
                        ? "Waiting for a fresh read of the current on-chain records"
                        : actionReason(caps?.canUpdate) ?? ""
                    }
                    onClick={() => submitRecords("UPDATE")}
                  >
                    {busy === "UPDATE" ? "…" : "Update"}
                  </Button>
                </div>
              </section>
            )}

            {/* Ownership / lifecycle - only show for owned names */}
            {isOwned && (
              <OwnershipActions
                caps={caps}
                busy={busy}
                recipient={recipient}
                onRecipientChange={setRecipient}
                actionDisabled={actionDisabled}
                actionReason={actionReason}
                onTransfer={() => run("TRANSFER", () => build.transfer.mutateAsync({ name, recipient: recipient.trim() }))}
                onFinalize={() => run("FINALIZE", () => build.finalize.mutateAsync({ name }))}
                onCancelTransfer={() => run("CANCEL", () => build.cancel.mutateAsync({ name }))}
                onRenew={() => run("RENEW", () => build.renew.mutateAsync({ name }))}
                onRevoke={() => run("REVOKE", () => build.revoke.mutateAsync({ name }))}
                onBuyWithPayment={(paymentAddress, paymentValue) =>
                  run("FINALIZE_WITH_PAYMENT", () =>
                    build.finalizeWithPayment.mutateAsync({ name, paymentAddress, paymentValue })
                  )
                }
                onSellWithPayment={(buyerAddress, priceValue) =>
                  run("SELL_WITH_PAYMENT", async () => {
                    // 1. Record the offer for later claim verification.
                    await build.sellWithPayment.mutateAsync({
                      name,
                      buyerAddress,
                      priceDoos: priceValue,
                    });
                    // 2. Build the transfer draft to the buyer (normal TRANSFER
                    //    covenant — the payment happens in the buyer's finalize).
                    return build.transfer.mutateAsync({ name, recipient: buyerAddress });
                  })
                }
              />
            )}

            {/* Paid swap claim: shown when a paid_swap_offer exists for this name */}
            <PaidSwapClaim name={name} />

            {/* Sign message (Task 3) — Namebase-style domain-claim verification,
                owned names only; the component itself gates on caps.ownsName. */}
            <NameSignMessage name={name} profileId={profile?.id ?? null} caps={caps} />
          </div>
        )}

        <div className="flex justify-end">
          <Button variant="ghost" onClick={onClose} disabled={!!busy}>Close</Button>
        </div>
      </div>
    </Dialog>
  );
}
