import { isSignerUnlockedFor } from "../lib/signer";
import { useState, useEffect, useRef, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import {
  useWalletProfiles,
  useActiveProfile,
  useSignerSession,
  useWriteCapability,
  useWalletBalances,
  useTxDrafts,
  useUnlockSigner,
  useLockSigner,
  useSetActiveProfile,
  useBuildSendDraft,
  useSignTxDraft,
  useBroadcastTxDraft,
  useNameAction,
} from "../queries/wallet";
import { finalizeSentIds, usePurchaseFinalize } from "./market/usePurchase";
import { useReadNames, useReadBalance, nameCapabilitiesQueryKey } from "../queries/read";
import { useNeedsYou } from "../queries/needsYou";
import {
  useStartFullSync,
  useSyncStatus,
  useCancelFullSync,
  useNodeCoinsNotSyncedReason,
} from "../queries/sync";
import { useStartHsd } from "../queries/node";
import { useSyncTriggerStore } from "../stores/syncTrigger";
import {
  auctionPhase,
  pendingBroadcastBadge,
  shakedexStatusLabel,
  taskSummaryFromCapabilities,
} from "../lib/auction";
import { displayName } from "../lib/idn";
import { NameActionsModal } from "./NameActionsModal";
import { BlockInfoModal } from "./BlockInfoModal";
import { RecentActivityCard } from "./activity/RecentActivityCard";
import { WalletManager } from "./WalletManager";
import { NeedsYouCard } from "./NeedsYouCard";
import { ReceiveDialog } from "./ReceiveDialog";
import { AddWalletForm } from "./AddWalletForm";
import { UnlockButton } from "./UnlockButton";
import { BatchConfirmModal } from "./BatchConfirmModal";
import { Button } from "./ui/Button";
import { FeeRateOverride } from "./ui/FeeRateOverride";
import { parseFeeRateArg } from "../lib/feeRate";
import { Badge } from "./ui/Badge";
import { Input } from "./ui/Input";
import { Dialog } from "./ui/Dialog";
import { PageHeader } from "./ui/PageHeader";
import { Disclosure } from "./ui/Disclosure";
import {
  formatHns,
  formatHnsShort,
  hnsToDollarydoos,
  dollarydoosToHns,
  formatDate,
  latestTimestamp,
  isLikelyHnsAddress,
  hnsAddressPrefix,
  truncateMiddle,
} from "../lib/utils";
import { mapError } from "../lib/errors";
import { explorerCoversNetwork } from "../lib/openExternal";
import { useUiStore } from "../stores/ui";
import type { TxDraftSummary } from "../types";
import { subscribeAction } from "../lib/actionBus";
import { Tooltip } from "./ui/Tooltip";
import {
  MY_LISTINGS_TAB,
  canUseShakedex,
  listingStateLabel,
  shakedexRefusal,
} from "./market/marketText";

type BatchAction = "renew" | "reveal" | "redeem" | "finalize" | "transfer";

export function WalletView() {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);

  const { data: profiles = [] } = useWalletProfiles();
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const { data: writeCap } = useWriteCapability();
  const { data: balances } = useWalletBalances();
  const { data: readBalance } = useReadBalance();
  const { data: drafts = [] } = useTxDrafts();
  const { data: names = [] } = useReadNames();
  const [nameQuery, setNameQuery] = useState("");
  // Batch selection: names the user has checked for bulk operations.
  const [selectedNames, setSelectedNames] = useState<Set<string>>(new Set());
  // Batch renew mutation: calls build_batch_renew_draft, then signs + broadcasts.
  const batchRenewMutation = useNameAction("build_batch_renew_draft");
  // Batch reveal / redeem / finalize mutations (same generic pattern).
  const batchRevealMutation = useNameAction("build_batch_reveal_draft");
  const batchRedeemMutation = useNameAction("build_batch_redeem_draft");
  const batchFinalizeMutation = useNameAction("build_batch_finalize_draft");
  // Batch transfer mutation: transfers N selected names to one shared recipient.
  const batchTransferMutation = useNameAction("build_batch_transfer_draft");
  // Substring filter for the Owned Names list. Matches on BOTH the raw ACE
  // name (as stored on-chain) and its decoded displayName, so a unicode
  // substring (e.g. from a `.козёл`-style label) still finds the underlying
  // `xn--` row. Never run the query through normalizeNameInput — that strips
  // non-ASCII characters and would break unicode search entirely.
  const q = nameQuery.trim().toLowerCase();
  const filteredNames = q
    ? names.filter(
        (n) => n.name.toLowerCase().includes(q) || displayName(n.name).toLowerCase().includes(q),
      )
    : names;
  // Needs you and its ONE batch capability fetch (pinned to the active
  // wallet, empty for watch-only). The same index drives the batch-action
  // eligibility and the Owned Names State column, so a name is described the
  // same way everywhere on the page.
  const needsYou = useNeedsYou();
  const { capsByName } = needsYou;

  const { finalize: finalizePurchaseDraft, finalizingId } = usePurchaseFinalize();
  const finalizeSent = useMemo(() => finalizeSentIds(drafts), [drafts]);

  const startSync = useStartFullSync();
  const startHsd = useStartHsd();
  const cancelSync = useCancelFullSync();
  const syncStatus = useSyncStatus();
  const manualSync = useSyncTriggerStore((s) => s.manualSync);
  const setManualSync = useSyncTriggerStore((s) => s.setManualSync);
  const unlock = useUnlockSigner();
  const lock = useLockSigner();
  const setActive = useSetActiveProfile();
  const buildDraft = useBuildSendDraft();
  const signDraft = useSignTxDraft();
  const broadcast = useBroadcastTxDraft();

  const [sendOpen, setSendOpen] = useState(false);
  const [sendAddress, setSendAddress] = useState("");
  const [sendAmount, setSendAmount] = useState("");
  // Per-transaction fee-rate override (doos/kvB, raw text). Empty = use the
  // global setting default. Shared by the Send dialog and every batch action.
  const [sendFeeRate, setSendFeeRate] = useState("");
  const [batchFeeRate, setBatchFeeRate] = useState("");
  // Shared recipient for a batch transfer (one address for the whole batch).
  const [batchRecipient, setBatchRecipient] = useState("");
  const [draft, setDraft] = useState<TxDraftSummary | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [receiveOpen, setReceiveOpen] = useState(false);
  // A failed sign/broadcast must NOT look like success: we surface it as a
  // persistent in-dialog error (not just a transient toast) and keep the dialog
  // open so the user can see exactly what happened before deciding to retry.
  const [sendError, setSendError] = useState<string | null>(null);
  // When the inline "Start node" in the needs-node-sync callout fails, we
  // reveal an "Open Settings" escape hatch right in the callout (the happy
  // path never navigates). Reset whenever the callout is re-attempted.
  const [startNodeFailed, setStartNodeFailed] = useState(false);
  const navigate = useNavigate();
  const [manageName, setManageName] = useState<string | null>(null);
  const [infoBlock, setInfoBlock] = useState<number | null>(null);
  // Batch confirmation modal state.
  const [batchModal, setBatchModal] = useState<{
    open: boolean;
    action: BatchAction;
    names: string[];
    feeDoos: number;
    amountDoos: number;
    draftId: string;
    recipient?: string;
  } | null>(null);
  // Wallets manager modal (add / switch / delete). `addMode` opens it straight
  // to the add-wallet form.
  const [walletManagerOpen, setWalletManagerOpen] = useState(false);
  const [walletManagerAddMode, setWalletManagerAddMode] = useState(false);

  // Ledger wallets don't use the local signer session — the device signs on
  // demand. Treat them as "unlocked" so the send/action flow doesn't try to
  // prompt for a passphrase that doesn't exist.
  const isLedger = profile?.kind === "ledger_hardware";
  const unlocked = isLedger ? true : isSignerUnlockedFor(signer, profile?.id);
  const canWrite = writeCap?.canWrite ?? false;
  const isWatchOnly = profile?.watchOnly ?? false;
  const canFinalizePurchase = canUseShakedex(profile?.kind, writeCap);
  const finalizeRefusal = shakedexRefusal(profile?.kind, writeCap);
  // Spending uses node-synced coins (tracked_utxos), NOT the explorer balance.
  // If the explorer shows funds but nothing is synced yet, the user must connect
  // a node and Refresh before they can send.
  // Already net of immature coinbase: the backend applies the same maturity
  // predicate coin selection does, so a number shown here can actually be sent.
  const spendable = balances?.liquidDoos ?? 0;
  const immature = balances?.immatureDoos ?? 0;
  const immatureInBlocks = balances?.immatureInBlocks ?? null;
  const explorerBalance = readBalance?.confirmed ?? 0;
  const needsNodeSync = explorerBalance > 0 && spendable === 0;
  const sendBlockedReason =
    writeCap?.reason ??
    (needsNodeSync
      ? "Sync your coins below, then unlock to send."
      : "Connect a node, Refresh to sync your coins, then unlock to send.");

  const isRunning = syncStatus.data?.running ?? false;
  // The last sync could not read this wallet's coins from the node: the
  // balance below is the cache, and this says why.
  const nodeCoinsReason = useNodeCoinsNotSyncedReason(syncStatus.data);

  // Reset manualSync flag when sync finishes (running transitions true → false).
  // Uses a ref to detect the transition edge, not the initial state.
  const wasRunningRef = useRef(false);
  useEffect(() => {
    if (wasRunningRef.current && !isRunning && manualSync) {
      setManualSync(false);
    }
    wasRunningRef.current = isRunning;
  }, [isRunning, manualSync, setManualSync]);

  // --- Keyboard shortcut subscriptions (action bus) ---
  const filterInputRef = useRef<HTMLInputElement>(null);
  const [selectedNameIndex, setSelectedNameIndex] = useState<number>(-1);
  const selectedRowRef = useRef<HTMLTableRowElement>(null);

  // Reset list selection when the filter changes (list content shifts).
  useEffect(() => {
    setSelectedNameIndex(-1);
  }, [nameQuery]);

  // Scroll the selected row into view.
  useEffect(() => {
    selectedRowRef.current?.scrollIntoView({ block: "nearest" });
  }, [selectedNameIndex]);

  // Ref-indirection for the action handler — avoids re-subscribing on every
  // render while still capturing fresh closures.
  const actionHandlerRef = useRef<(id: string) => void>(() => {});
  actionHandlerRef.current = (actionId: string) => {
    switch (actionId) {
      case "wallet:send":
        if (canWrite) {
          setSendOpen(true);
        } else {
          // Match the Send button's UX: the keyboard path must not silently
          // no-op. Surface the same reason the disabled button shows.
          showToast(sendBlockedReason, "info");
        }
        break;
      case "wallet:sync":
        handleSync();
        break;
      case "wallet:toggleLock":
        if (unlocked) handleLock();
        else handleUnlock();
        break;
      case "wallet:openReceive":
        setReceiveOpen(true);
        break;
      case "wallet:focusFilter":
        filterInputRef.current?.focus();
        break;
      case "wallet:list:next":
        setSelectedNameIndex((i) => Math.min(i + 1, filteredNames.length - 1));
        break;
      case "wallet:list:prev":
        setSelectedNameIndex((i) => Math.max(i - 1, 0));
        break;
      case "wallet:list:open":
        // A purchase is not ours yet: there is nothing to manage.
        if (
          selectedNameIndex >= 0 &&
          filteredNames[selectedNameIndex] &&
          !filteredNames[selectedNameIndex]!.shakedex
        ) {
          setManageName(filteredNames[selectedNameIndex]!.name);
        }
        break;
      case "wallet:list:clear":
        setSelectedNameIndex(-1);
        break;
    }
  };
  useEffect(() => subscribeAction((id) => actionHandlerRef.current(id)), []);

  const resetSend = () => {
    setSendOpen(false);
    setSendAddress("");
    setSendAmount("");
    setSendFeeRate("");
    setDraft(null);
    setSubmitting(false);
    setSendError(null);
  };

  // Batch selection helpers.
  // Purchase rows are not ours yet: no batch action applies to them.
  const selectableNames = filteredNames.filter((n) => !n.shakedex);
  const toggleName = (name: string) =>
    setSelectedNames((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  const clearSelection = () => setSelectedNames(new Set());

  const finalizePurchase = (purchaseId: string) =>
    finalizePurchaseDraft(purchaseId, (txid) => `Finalize broadcast — ${txid.slice(0, 12)}…`);

  // Per-selection eligibility for batch actions. `nameCaps` is pinned to the
  // active profile via useNamesActionCapabilities and updates when phases move.
  // A batch action is "eligible" only when *every* selected name currently
  // supports it (canX.allowed = true). Empty selection ⇒ false.
  const batchEligibility = useMemo(() => {
    if (selectedNames.size === 0 || capsByName.size === 0) {
      return {
        canReveal: false,
        canRedeem: false,
        canFinalize: false,
        canTransfer: false,
        canRenew: false,
      };
    }
    let canReveal = true;
    let canRedeem = true;
    let canFinalize = true;
    let canTransfer = true;
    // R27: a name locked for sale (or not renewable yet) refuses a renewal;
    // the batch is refused by the backend too, but is not offered here.
    let canRenew = true;
    for (const n of selectedNames) {
      const c = capsByName.get(n);
      if (!c) {
        canReveal = false;
        canRedeem = false;
        canFinalize = false;
        canTransfer = false;
        canRenew = false;
        break;
      }
      if (!c.canReveal.allowed) canReveal = false;
      if (!c.canRedeem.allowed) canRedeem = false;
      if (!c.canFinalize.allowed) canFinalize = false;
      if (!c.canTransfer.allowed) canTransfer = false;
      if (!c.canRenew.allowed) canRenew = false;
    }
    return { canReveal, canRedeem, canFinalize, canTransfer, canRenew };
  }, [selectedNames, capsByName]);

  // Compute the fee-rate arg once for all batch handlers (null = use setting default).
  const batchFeeRateArg = parseFeeRateArg(batchFeeRate) ?? undefined;

  // Every batch action is the same three steps — build one draft for the
  // selection, then hand its fee and amount to the confirm modal — and every
  // failure is reported the same way.
  const runBatch = async (
    action: BatchAction,
    build: () => Promise<TxDraftSummary>,
    extra: { recipient?: string } = {},
  ) => {
    const names = Array.from(selectedNames);
    if (names.length === 0) return;
    try {
      showToast(`Building batch ${action} draft…`, "info");
      const draft = await build();
      setBatchModal({
        open: true,
        action,
        names,
        feeDoos: draft.summary?.feeDoos ?? 0,
        amountDoos: draft.summary?.sendTotalDoos ?? 0,
        draftId: draft.id,
        ...extra,
      });
    } catch (e) {
      showToast(`Batch ${action} failed: ${mapError(e)}`, "error");
    }
  };
  const batchNames = () => Array.from(selectedNames);

  const handleBatchRenew = () =>
    runBatch("renew", () =>
      batchRenewMutation.mutateAsync({ names: batchNames(), feeRate: batchFeeRateArg }),
    );
  const handleBatchReveal = () =>
    runBatch("reveal", () =>
      batchRevealMutation.mutateAsync({ names: batchNames(), feeRate: batchFeeRateArg }),
    );
  const handleBatchRedeem = () =>
    runBatch("redeem", () =>
      batchRedeemMutation.mutateAsync({ names: batchNames(), feeRate: batchFeeRateArg }),
    );
  const handleBatchFinalize = () =>
    runBatch("finalize", () =>
      batchFinalizeMutation.mutateAsync({ names: batchNames(), feeRate: batchFeeRateArg }),
    );
  const handleBatchTransfer = () => {
    const recipient = batchRecipient.trim();
    if (!recipient) return;
    return runBatch(
      "transfer",
      () =>
        batchTransferMutation.mutateAsync({
          names: batchNames(),
          recipient,
          feeRate: batchFeeRateArg,
        }),
      { recipient },
    );
  };

  // Confirm a pending batch draft: unlock (if needed) → sign → broadcast.
  const handleBatchConfirm = async () => {
    if (!batchModal || !profile) return;
    const { draftId, names, action } = batchModal;
    try {
      if (!unlocked) {
        await unlock.mutateAsync(profile.id);
      }
      await signDraft.mutateAsync(draftId);
      const result = await broadcast.mutateAsync(draftId);
      showToast(
        `Batch ${action} broadcast ${result.txid.slice(0, 12)}… (${names.length} name(s))`,
        "success",
      );
      setBatchModal(null);
      clearSelection();
      setBatchRecipient("");
      qc.invalidateQueries({ queryKey: ["wallet"] });
    } catch (e) {
      showToast(`Batch ${action} failed: ${mapError(e)}`, "error");
      // Keep the modal open so the user can retry or cancel.
      throw e;
    }
  };

  // Cancel a pending batch draft: discard the draft (frees reserved coins).
  const handleBatchCancel = () => {
    // Just close the modal. The draft row remains in the DB but its coin
    // reservations don't block anything — they'll be freed when the draft is
    // eventually replaced or the user builds another draft that supersedes
    // this one. Keeping the draft also lets the user replay it from Drafts.
    setBatchModal(null);
  };

  // Sync runs all reconciliation in a background thread.
  // The frontend polls status via useSyncStatus (persistent across navigation).
  const handleSync = async () => {
    try {
      setManualSync(true);
      await startSync.mutateAsync();
      showToast("Sync started in background", "info");
    } catch (e) {
      setManualSync(false);
      showToast(mapError(e), "error");
    }
  };

  const handleCancelSync = async () => {
    try {
      await cancelSync.mutateAsync();
      showToast("Stopping sync…", "info");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  // "Start node" from the needs-node-sync callout. Fulfils the old copy's
  // promise ("connect a node and Refresh") in one click: start hsd, then
  // trigger a full sync (same signal path as the Refresh button). On
  // failure, surface a toast and reveal an "Open Settings" link inline.
  const handleStartNodeAndSync = async () => {
    setStartNodeFailed(false);
    try {
      await startHsd.mutateAsync();
    } catch (e) {
      setStartNodeFailed(true);
      showToast(mapError(e), "error");
      return;
    }
    try {
      setManualSync(true);
      await startSync.mutateAsync();
      showToast("Node started — syncing your coins", "info");
    } catch (e) {
      // Node started but sync failed to kick off. Not fatal — the user can
      // hit Refresh manually; don't hide the callout behind a Settings link.
      setManualSync(false);
      showToast(mapError(e), "error");
    }
  };

  // Opening a name from Needs you: seed its single-name cache from the batch
  // so the modal's badge doesn't flash a loading state.
  const openName = (name: string) => {
    qc.setQueryData(nameCapabilitiesQueryKey(profile?.id ?? null, name), capsByName.get(name));
    setManageName(name);
  };

  const handleUnlock = async () => {
    if (!profile) return;
    try {
      await unlock.mutateAsync(profile.id);
      showToast("Wallet unlocked", "success");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleLock = async () => {
    try {
      await lock.mutateAsync();
      showToast("Wallet locked", "info");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleBuildDraft = async (opts?: { max?: boolean }) => {
    const max = opts?.max ?? false;
    if (!sendAddress.trim()) {
      showToast("Enter a destination address", "error");
      return;
    }
    const doos = hnsToDollarydoos(sendAmount);
    if (!max && (isNaN(doos) || doos <= 0)) {
      showToast("Invalid amount", "error");
      return;
    }
    setSendError(null);
    try {
      const d = await buildDraft.mutateAsync({
        toAddress: sendAddress.trim(),
        valueDoos: doos,
        max,
        feeRate: parseFeeRateArg(sendFeeRate) ?? undefined,
      });
      setDraft(d);
      // Reflect the swept amount in the field so "Max" is transparent.
      if (max && d.summary?.sendTotalDoos != null) {
        setSendAmount(dollarydoosToHns(d.summary.sendTotalDoos));
      }
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  // Unlock (if needed) → sign → broadcast. The send is only considered done when
  // the node confirms the broadcast; any failure keeps the dialog open with a
  // persistent error so it can never be mistaken for a successful send.
  const handleConfirmSend = async () => {
    if (!draft || !profile) return;
    setSubmitting(true);
    setSendError(null);
    try {
      if (!unlocked) {
        await unlock.mutateAsync(profile.id);
      }
      await signDraft.mutateAsync(draft.id);
      const result = await broadcast.mutateAsync(draft.id);
      // Only here — after the node accepted the tx — is the send complete.
      showToast(`Broadcast ${result.txid.slice(0, 12)}…`, "success");
      resetSend();
      qc.invalidateQueries({ queryKey: ["wallet"] });
    } catch (e) {
      const msg = mapError(e);
      setSendError(msg);
      showToast(msg, "error");
      setSubmitting(false);
    }
  };

  if (!profile) {
    return (
      <div className="space-y-6">
        <PageHeader title="Wallet" subtitle="No wallet profile yet." />
        <div className="bg-white border border-gray-200 rounded-lg p-6 space-y-4">
          <div className="text-sm text-gray-600">
            No wallet profile is active. Create or import one — your recovery phrase and passphrase
            are handled only in a secure window.
          </div>
          <AddWalletForm defaultLabel="Primary" onDone={() => {}} />
        </div>
      </div>
    );
  }

  // Inline send-form validation (profile is non-null here). The backend
  // address::decode stays authoritative at build; this is fast UI feedback.
  const sendAmtDoos = hnsToDollarydoos(sendAmount);
  const addressError =
    sendAddress.trim() && !isLikelyHnsAddress(sendAddress, profile.network)
      ? `Enter a valid ${profile.network} address (starts with ${hnsAddressPrefix(
          profile.network,
        )}…)`
      : null;
  const amountError =
    sendAmount.trim() && (isNaN(sendAmtDoos) || sendAmtDoos <= 0)
      ? "Enter a positive amount"
      : sendAmount.trim() && sendAmtDoos > spendable
        ? "Amount exceeds your spendable balance"
        : null;
  const canMax = !!sendAddress.trim() && !addressError && spendable > 0;

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Wallet"
        subtitle="Non-custodial. Keys are held locally and never leave this device."
        badges={
          <>
            <Badge variant="info">{profile.label}</Badge>
            <Badge>{profile.network}</Badge>
            {isWatchOnly ? (
              <Badge variant="warning">Watch-only</Badge>
            ) : unlocked ? (
              <Badge variant="success">Unlocked</Badge>
            ) : (
              <Badge variant="warning">Locked</Badge>
            )}
          </>
        }
        actions={
          syncStatus.data?.running
            ? manualSync
              ? [
                  {
                    label: syncStatus.data.cancelRequested ? "Stopping…" : "Stop",
                    variant: "danger",
                    disabled: syncStatus.data.cancelRequested,
                    loading: cancelSync.isPending,
                    onClick: handleCancelSync,
                  },
                ]
              : [
                  {
                    label: "Syncing…",
                    loading: true,
                    disabled: true,
                  },
                ]
            : [
                {
                  label: "Sync",
                  onClick: handleSync,
                },
              ]
        }
      />

      {syncStatus.data?.running && manualSync && (
        <div
          className="bg-blue-50 border border-blue-200 rounded p-3 text-sm text-blue-800"
          data-testid="sync-status"
        >
          <div className="font-medium">{syncStatus.data.progressLabel}</div>
          <div className="text-xs mt-1 space-y-0.5">
            <div>
              Step: {syncStatus.data.step}
              {syncStatus.data.waiting ? " (waiting for explorer…)" : ""}
            </div>
            {syncStatus.data.repairCandidates > 0 && (
              <div>
                Checked:{" "}
                {Math.max(0, syncStatus.data.repairCandidates - syncStatus.data.repairRemaining)} ·
                Owned: +{syncStatus.data.repaired} · Remaining: ~{syncStatus.data.repairRemaining}
              </div>
            )}
            {syncStatus.data.step === "discover" && syncStatus.data.discoverAddressesTotal > 0 && (
              <div>
                Addresses: {syncStatus.data.discoverAddressesDone} /{" "}
                {syncStatus.data.discoverAddressesTotal}
              </div>
            )}
            {syncStatus.data.step === "discover" && syncStatus.data.discoverTxsScanned > 0 && (
              <div>Transactions scanned: {syncStatus.data.discoverTxsScanned}</div>
            )}
            {syncStatus.data.discoverCandidates > 0 && (
              <div>Candidate names found: {syncStatus.data.discoverCandidates}</div>
            )}
            {syncStatus.data.discoverCurrentName && (
              <div>Checking: {syncStatus.data.discoverCurrentName}</div>
            )}
            {syncStatus.data.discovered > 0 && (
              <div>Newly discovered: {syncStatus.data.discovered}</div>
            )}
            {syncStatus.data.errors?.length > 0 && (
              <div className="text-red-600">Errors: {syncStatus.data.errors.join("; ")}</div>
            )}
          </div>
        </div>
      )}

      {/* Account bar — profile switch/add/manage + the lock action, condensed
          into one row. Connectivity/signer STATUS lives in the global
          StatusStrip; here we keep only the Lock/Unlock action plus a compact
          "Signer locked/unlocked" label and the passphrase helper hint. */}
      <div className="flex flex-wrap items-center gap-2 text-sm bg-white border border-gray-200 rounded-lg px-3 py-2">
        <span className="text-gray-500">Active wallet:</span>
        {profiles.length > 1 ? (
          <select
            className="border border-gray-300 rounded px-2 py-1"
            value={profile.id}
            onChange={(e) => setActive.mutate(e.target.value)}
          >
            {profiles.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label} ({p.network})
              </option>
            ))}
          </select>
        ) : (
          <span className="font-medium">
            {profile.label} ({profile.network})
          </span>
        )}
        <Button
          size="sm"
          variant="secondary"
          onClick={() => {
            setWalletManagerAddMode(true);
            setWalletManagerOpen(true);
          }}
        >
          + Add wallet
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            setWalletManagerAddMode(false);
            setWalletManagerOpen(true);
          }}
        >
          Manage wallets
        </Button>
        {!isWatchOnly && (
          <div className="ml-auto flex items-center gap-2">
            <Tooltip
              content={
                unlocked
                  ? "Your keys are in memory. They lock automatically after the session timeout."
                  : undefined
              }
            >
              <span className="text-xs text-gray-500">
                Signer {unlocked ? "unlocked" : "locked"}
                {!unlocked && (
                  <span className="hidden sm:inline">
                    {" — "}
                    {profile.hasPassphrase
                      ? "Unlock with your passphrase (in a secure window)"
                      : "no passphrase — just click Unlock"}
                  </span>
                )}
              </span>
            </Tooltip>
            {unlocked ? (
              <Button size="sm" variant="secondary" onClick={handleLock}>
                Lock
              </Button>
            ) : (
              <Button
                size="sm"
                variant="primary"
                onClick={handleUnlock}
                disabled={unlock.isPending}
              >
                {unlock.isPending ? "Unlocking…" : "Unlock"}
              </Button>
            )}
          </div>
        )}
      </div>

      <WalletManager
        open={walletManagerOpen}
        startInAddMode={walletManagerAddMode}
        onClose={() => setWalletManagerOpen(false)}
      />

      {/* Balance first: Spendable is the hero (what coin selection can
          actually use after a node sync), the rest is one muted line, and the
          page's one primary action — Send — sits right under it. Confirmed
          lives in the Spendable tooltip: it can differ from spendable while
          the node isn't synced. */}
      <section className="bg-white rounded-lg border border-gray-200 px-5 pt-5 pb-[18px]">
        <Tooltip
          content={`Spendable balance from node sync · confirmed ${formatHns(
            readBalance?.confirmed ?? 0,
          )} HNS`}
          className="flex w-fit"
        >
          <div className="text-[13px] text-gray-500">Spendable</div>
        </Tooltip>
        <Tooltip content={`${formatHns(spendable)} HNS`} className="flex w-fit">
          <div className="flex items-baseline gap-2 w-fit">
            <span
              className="text-[34px] font-bold tabular-nums text-gray-900"
              data-testid="balance-spendable"
            >
              {formatHnsShort(spendable)}
            </span>
            <span className="text-sm text-gray-500">HNS</span>
          </div>
        </Tooltip>
        <div className="flex flex-wrap gap-x-7 gap-y-1.5 mt-2.5 text-xs text-gray-500">
          <Tooltip
            content={`Received but not in a block yet · ${formatHns(readBalance?.unconfirmed ?? 0)} HNS`}
          >
            <span data-testid="balance-unconfirmed">
              Unconfirmed{" "}
              <span className="font-medium text-gray-700 tabular-nums">
                {formatHnsShort(readBalance?.unconfirmed ?? 0)}
              </span>
            </span>
          </Tooltip>
          {immature > 0 && (
            <Tooltip
              content={`Freshly mined coins — spendable once they mature · ${formatHns(immature)} HNS`}
            >
              <span data-testid="balance-immature">
                Immature
                {immatureInBlocks !== null && immatureInBlocks > 0
                  ? ` (${immatureInBlocks} ${immatureInBlocks === 1 ? "block" : "blocks"})`
                  : ""}{" "}
                <span className="font-medium text-gray-700 tabular-nums">
                  {formatHnsShort(immature)}
                </span>
              </span>
            </Tooltip>
          )}
          {(balances?.nameLockupDoos ?? 0) > 0 && (
            <Tooltip
              content={`In-flight bids — returned on reveal/redeem · ${formatHns(balances!.nameLockupDoos)} HNS`}
            >
              <span data-testid="balance-locked-auctions">
                Locked in auctions{" "}
                <span className="font-medium text-gray-700 tabular-nums">
                  {formatHnsShort(balances!.nameLockupDoos)}
                </span>
              </span>
            </Tooltip>
          )}
          {(balances?.nameControlDoos ?? 0) > 0 && (
            <Tooltip
              content={`Value bound to names you control · ${formatHns(balances!.nameControlDoos)} HNS`}
            >
              <span data-testid="balance-name-value">
                Name value{" "}
                <span className="font-medium text-gray-700 tabular-nums">
                  {formatHnsShort(balances!.nameControlDoos)}
                </span>
              </span>
            </Tooltip>
          )}
        </div>
        {nodeCoinsReason && (
          <div
            className="mt-3 text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2"
            data-testid="balance-node-not-authoritative"
          >
            Your node can't report this wallet's coins: {nodeCoinsReason}
          </div>
        )}
        <div className="mt-4 flex flex-wrap items-center gap-2">
          {!isWatchOnly && (
            <Tooltip content={!canWrite ? sendBlockedReason : undefined}>
              <Button
                variant="primary"
                onClick={() => setSendOpen(true)}
                disabled={!canWrite || spendable === 0}
                data-testid="wallet-send"
              >
                Send
              </Button>
            </Tooltip>
          )}
          <Button
            variant="secondary"
            onClick={() => setReceiveOpen(true)}
            data-testid="receive-open"
          >
            Receive
          </Button>
          {!canWrite && !isWatchOnly && (
            <>
              <span className="text-[13px] text-amber-700">{sendBlockedReason}</span>
              <UnlockButton size="sm" variant="secondary" label="Unlock" />
            </>
          )}
        </div>
        {isWatchOnly && (
          <div className="mt-2.5 text-xs text-gray-500">
            Watch-only wallet — it can receive and track balances and names, but it can't sign or
            spend.
          </div>
        )}
        {needsNodeSync && !isWatchOnly && (
          <div
            className="mt-3 text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2 space-y-2"
            data-testid="needs-node-sync"
          >
            <div>
              Your balance is read from the explorer, but spending requires a synced node. Start
              your node to load your spendable coins.
            </div>
            <div className="flex items-center gap-3">
              <Button
                size="sm"
                variant="secondary"
                onClick={() => void handleStartNodeAndSync()}
                disabled={startHsd.isPending || startSync.isPending}
                data-testid="needs-node-sync-start"
              >
                {startHsd.isPending ? "Starting…" : "Start node"}
              </Button>
              {startNodeFailed && (
                <button
                  type="button"
                  className="text-amber-900 underline hover:no-underline"
                  onClick={() => navigate("/settings")}
                  data-testid="needs-node-sync-settings"
                >
                  Open Settings
                </button>
              )}
            </div>
          </div>
        )}
      </section>

      {/* Every task the wallet owes the chain, one row each, most urgent
          first. Replaces the reveal/won/lost/expiring banners. */}
      {!isWatchOnly && (
        <NeedsYouCard
          items={needsYou.items}
          isLoading={needsYou.isLoading}
          isDegraded={needsYou.isDegraded}
          onOpenName={openName}
          onFinalizePurchase={(item) =>
            void finalizePurchaseDraft(
              item.purchaseId!,
              () =>
                `.${displayName(item.name)} finalize sent · it moves into your wallet once mined`,
            )
          }
          finalizingId={finalizingId}
          finalizeRefusal={finalizeRefusal}
        />
      )}

      {/* Owned Names (from local name-state cache) */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <div className="flex items-center justify-between mb-2 gap-2">
          <div className="text-sm text-gray-500">
            Owned Names ({filteredNames.length}
            {filteredNames.length !== names.length ? ` of ${names.length}` : ""})
          </div>
          <div className="flex items-center gap-2">
            <Input
              ref={filterInputRef}
              inputSize="md"
              className="w-48"
              value={nameQuery}
              onChange={(e) => setNameQuery(e.target.value)}
              placeholder="Filter…"
              data-testid="wallet-name-filter"
            />
          </div>
        </div>
        {names.length > 0 ? (
          filteredNames.length > 0 ? (
            <>
              <div className="max-h-60 overflow-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="text-left text-gray-500 border-b">
                      <th className="py-1 pr-4 w-8">
                        <input
                          type="checkbox"
                          checked={
                            selectedNames.size > 0 && selectedNames.size === selectableNames.length
                          }
                          ref={(el) => {
                            if (el)
                              el.indeterminate =
                                selectedNames.size > 0 &&
                                selectedNames.size < selectableNames.length;
                          }}
                          onChange={(e) => {
                            if (e.target.checked)
                              setSelectedNames(new Set(selectableNames.map((n) => n.name)));
                            else setSelectedNames(new Set());
                          }}
                          aria-label="Select all names"
                        />
                      </th>
                      <th className="py-1 pr-4">Name</th>
                      <th className="py-1 pr-4">State</th>
                      <th className="py-1 pr-4">Height</th>
                      <th className="py-1 pr-4">Renewal</th>
                      <th className="py-1"></th>
                    </tr>
                  </thead>
                  <tbody>
                    {filteredNames.map((n) => (
                      <tr
                        key={n.name}
                        ref={
                          filteredNames.indexOf(n) === selectedNameIndex
                            ? selectedRowRef
                            : undefined
                        }
                        aria-selected={filteredNames.indexOf(n) === selectedNameIndex}
                        tabIndex={filteredNames.indexOf(n) === selectedNameIndex ? 0 : -1}
                        // Not clickable: the row's actions are its own
                        // controls — the name, the block heights, Manage. A
                        // whole-row handler only duplicated them, and stacked a
                        // second dialog on top of whichever one was pressed.
                        // Keyboard selection is unaffected: `wallet:list:open`
                        // opens the selected row directly. `hover` stays for
                        // tracking a row across a wide table; `cursor-pointer`
                        // goes, since nothing here is clicking.
                        className={`border-t border-gray-100 hover:bg-gray-50 ${
                          filteredNames.indexOf(n) === selectedNameIndex
                            ? "bg-blue-50 ring-1 ring-blue-300"
                            : ""
                        }`}
                      >
                        <td className="py-1 pr-4">
                          {!n.shakedex && (
                            <input
                              type="checkbox"
                              checked={selectedNames.has(n.name)}
                              onChange={() => toggleName(n.name)}
                              aria-label={`Select ${displayName(n.name)}`}
                            />
                          )}
                        </td>
                        <td className="py-1 pr-4 text-xs font-mono">
                          {n.shakedex ? (
                            <span data-testid="owned-name-purchase-label">
                              .{displayName(n.name)}
                            </span>
                          ) : (
                            <Tooltip content="View name info">
                              <button
                                type="button"
                                className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                                onClick={() => setManageName(n.name)}
                                data-testid="owned-name-info-link"
                              >
                                .{displayName(n.name)}
                              </button>
                            </Tooltip>
                          )}
                        </td>
                        <td className="py-1 pr-4">
                          {/* What the name needs, not which auction phase it
                              is in. The same summary the Auctions list and the
                              name modal show, so one name is never described
                              two ways — a row reading "Closed" opened a modal
                              headed "Won — Register Now". The raw phase is
                              also nearly constant here: every name you own has
                              a closed auction. It stays as the fallback for
                              the window before capabilities load, and for a
                              watch-only profile that never fetches them. */}
                          {(() => {
                            // A purchase in flight comes first: it is not
                            // ours yet, so there are no capabilities for it.
                            if (n.shakedex) {
                              const s = shakedexStatusLabel(
                                n.shakedex,
                                finalizeSent.has(n.shakedex.purchaseId),
                              );
                              return <Badge variant={s.variant}>{s.label}</Badge>;
                            }
                            // R19: a name we listed shows the listing's
                            // state; its owner actions are the lock's now.
                            if (n.listing) {
                              return (
                                <Badge variant="info">
                                  {listingStateLabel(
                                    n.listing.state,
                                    n.listing.blocksUntilFinalize,
                                  )}
                                </Badge>
                              );
                            }
                            const task = taskSummaryFromCapabilities(capsByName.get(n.name));
                            // A transaction of ours for this name is in flight:
                            // the task is a verdict the chain has not reached
                            // and an instruction already followed. The modal
                            // and the auctions list both say so; this column
                            // was the last one still printing the task.
                            if (task?.pendingBroadcastAction) {
                              return (
                                <Badge variant="default">
                                  {pendingBroadcastBadge(task.pendingBroadcastAction)}
                                </Badge>
                              );
                            }
                            if (task) {
                              return <Badge variant={task.variant}>{task.label}</Badge>;
                            }
                            return n.state ? (
                              <Badge variant={auctionPhase(n.state).variant}>
                                {auctionPhase(n.state).label}
                              </Badge>
                            ) : (
                              "—"
                            );
                          })()}
                        </td>
                        <td className="py-1 pr-4 text-xs text-gray-500 font-mono">
                          {n.height ? (
                            <Tooltip content="View block info">
                              <button
                                type="button"
                                className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                                onClick={() => setInfoBlock(n.height!)}
                                data-testid="owned-name-height-info-link"
                              >
                                #{n.height}
                              </button>
                            </Tooltip>
                          ) : (
                            "—"
                          )}
                        </td>
                        <td className="py-1 pr-4 text-xs text-gray-500 font-mono">
                          {n.renewal ? (
                            <Tooltip content="View block info">
                              <button
                                type="button"
                                className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                                onClick={() => setInfoBlock(n.renewal!)}
                                data-testid="owned-name-renewal-info-link"
                              >
                                #{n.renewal}
                              </button>
                            </Tooltip>
                          ) : (
                            "—"
                          )}
                        </td>
                        <td className="py-1 text-right">
                          {!isWatchOnly &&
                            (n.shakedex ? (
                              n.shakedex.state === "awaitingFinalize" &&
                              n.shakedex.blocksRemaining === 0 &&
                              !finalizeSent.has(n.shakedex.purchaseId) && (
                                <Tooltip content={finalizeRefusal}>
                                  <Button
                                    size="sm"
                                    variant="primary"
                                    data-testid="owned-name-finalize"
                                    disabled={finalizingId !== null || !canFinalizePurchase}
                                    onClick={() => finalizePurchase(n.shakedex!.purchaseId)}
                                  >
                                    Finalize
                                  </Button>
                                </Tooltip>
                              )
                            ) : (
                              <>
                                {n.listing && (
                                  <Button
                                    size="sm"
                                    variant="ghost"
                                    data-testid="owned-name-my-listings"
                                    onClick={() => navigate("/market?tab=mine")}
                                  >
                                    {MY_LISTINGS_TAB}
                                  </Button>
                                )}
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  data-testid="owned-name-manage"
                                  onClick={() => setManageName(n.name)}
                                >
                                  Manage
                                </Button>
                              </>
                            ))}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {selectedNames.size > 0 && !isWatchOnly && (
                <>
                  <div
                    className="flex items-center gap-3 mt-3 p-2 bg-blue-50 border border-blue-200 rounded text-sm"
                    data-testid="batch-action-bar"
                  >
                    <span className="text-blue-800 font-medium">{selectedNames.size} selected</span>
                    <Tooltip
                      content={
                        batchEligibility.canRenew
                          ? undefined
                          : "All selected names must be renewable now"
                      }
                    >
                      <Button
                        size="sm"
                        variant="primary"
                        onClick={handleBatchRenew}
                        disabled={!batchEligibility.canRenew}
                        data-testid="batch-renew-btn"
                      >
                        Renew Selected
                      </Button>
                    </Tooltip>
                    <Tooltip
                      content={
                        batchEligibility.canReveal
                          ? undefined
                          : "All selected names must be in the REVEAL phase"
                      }
                    >
                      <Button
                        size="sm"
                        variant="primary"
                        onClick={handleBatchReveal}
                        disabled={!batchEligibility.canReveal}
                        data-testid="batch-reveal-btn"
                      >
                        Reveal Selected
                      </Button>
                    </Tooltip>
                    <Tooltip
                      content={
                        batchEligibility.canRedeem
                          ? undefined
                          : "All selected names must have redeemable losing bids"
                      }
                    >
                      <Button
                        size="sm"
                        variant="primary"
                        onClick={handleBatchRedeem}
                        disabled={!batchEligibility.canRedeem}
                        data-testid="batch-redeem-btn"
                      >
                        Redeem Selected
                      </Button>
                    </Tooltip>
                    <Tooltip
                      content={
                        batchEligibility.canFinalize
                          ? undefined
                          : "All selected names must have a transfer ready to finalize"
                      }
                    >
                      <Button
                        size="sm"
                        variant="primary"
                        onClick={handleBatchFinalize}
                        disabled={!batchEligibility.canFinalize}
                        data-testid="batch-finalize-btn"
                      >
                        Finalize Selected
                      </Button>
                    </Tooltip>
                    <Input
                      className="w-56"
                      placeholder="Transfer to hs1q… / rs1q…"
                      value={batchRecipient}
                      onChange={(e) => setBatchRecipient(e.target.value)}
                      data-testid="batch-transfer-recipient-input"
                    />
                    <Tooltip
                      content={
                        !batchEligibility.canTransfer
                          ? "All selected names must be transferable (owned, not mid-transfer)"
                          : !batchRecipient.trim()
                            ? "Enter a recipient address"
                            : undefined
                      }
                    >
                      <Button
                        size="sm"
                        variant="primary"
                        onClick={handleBatchTransfer}
                        disabled={!batchEligibility.canTransfer || !batchRecipient.trim()}
                        data-testid="batch-transfer-btn"
                      >
                        Transfer Selected
                      </Button>
                    </Tooltip>
                    <Button size="sm" variant="ghost" onClick={clearSelection}>
                      Clear
                    </Button>
                  </div>
                  <div className="mt-2">
                    <FeeRateOverride
                      value={batchFeeRate}
                      onChange={setBatchFeeRate}
                      label="Fee rate override"
                    />
                  </div>
                </>
              )}
            </>
          ) : (
            <div className="text-gray-400 text-sm py-4 text-center">
              No names match &quot;{nameQuery.trim()}&quot;
            </div>
          )
        ) : (
          <div className="text-gray-400 text-sm py-4 text-center">
            {false
              ? "Scanning the explorer for names this wallet owns…"
              : "No owned names found yet. Click Sync to scan for names this wallet owns."}
          </div>
        )}
      </div>

      <RecentActivityCard onNameClick={setManageName} />

      {/* Diagnostics — collapsed by default; debug-only info that shouldn't
          compete for attention on the main screen. */}
      <Disclosure summary="Details">
        <div className="text-xs text-gray-400">
          Profile: {profile.id.slice(0, 8)}… | Last synced height: {profile.lastSyncedHeight ?? "—"}{" "}
          | Last successful sync:{" "}
          {formatDate(latestTimestamp(profile.lastSyncedAt, profile.lastExplorerSyncAt))} | xpub:{" "}
          {truncateMiddle(profile.accountXpub)}
        </div>
      </Disclosure>

      {batchModal && (
        <BatchConfirmModal
          open={batchModal.open}
          action={batchModal.action}
          names={batchModal.names}
          estimatedFeeDoos={batchModal.feeDoos}
          amountDoos={batchModal.amountDoos}
          recipient={batchModal.recipient}
          onConfirm={handleBatchConfirm}
          onCancel={handleBatchCancel}
        />
      )}

      {manageName && (
        <NameActionsModal
          name={manageName}
          open={!!manageName}
          onClose={() => setManageName(null)}
        />
      )}

      {infoBlock != null && (
        <BlockInfoModal
          height={infoBlock}
          open={infoBlock != null}
          onClose={() => setInfoBlock(null)}
          isMainnet={explorerCoversNetwork(profile.network)}
        />
      )}

      <ReceiveDialog open={receiveOpen} onClose={() => setReceiveOpen(false)} profile={profile} />

      {/* Send dialog: form → preview → confirm */}
      <Dialog open={sendOpen} onClose={resetSend} title="Send HNS">
        {!draft ? (
          <div className="space-y-3">
            <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
              This sends real HNS. You'll review the fee and confirm before broadcasting.
            </div>
            <div>
              <Input
                label="Destination Address"
                value={sendAddress}
                onChange={(e) => setSendAddress(e.target.value)}
                placeholder={profile.network === "mainnet" ? "hs1q…" : "rs1q… / ts1q…"}
              />
              {addressError && (
                <div className="mt-1 text-xs text-red-600" data-testid="send-address-error">
                  {addressError}
                </div>
              )}
            </div>
            <div>
              <div className="flex items-end gap-2">
                <div className="flex-1">
                  <Input
                    label="Amount (HNS)"
                    value={sendAmount}
                    onChange={(e) => setSendAmount(e.target.value)}
                    placeholder="1.0"
                    type="number"
                    step="0.000001"
                  />
                </div>
                <Tooltip content="Send your entire spendable balance (minus the network fee)">
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={!canMax || buildDraft.isPending}
                    onClick={() => handleBuildDraft({ max: true })}
                  >
                    Max
                  </Button>
                </Tooltip>
              </div>
              {amountError && (
                <div className="mt-1 text-xs text-red-600" data-testid="send-amount-error">
                  {amountError}
                </div>
              )}
            </div>
            <FeeRateOverride value={sendFeeRate} onChange={setSendFeeRate} />
            <div className="flex gap-2 justify-end">
              <Button variant="ghost" onClick={resetSend}>
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={() => handleBuildDraft()}
                disabled={
                  !sendAddress.trim() ||
                  !sendAmount.trim() ||
                  !!addressError ||
                  !!amountError ||
                  buildDraft.isPending
                }
              >
                {buildDraft.isPending ? "Building…" : "Review"}
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="bg-gray-50 rounded p-3 text-sm space-y-1">
              <div className="flex justify-between">
                <span>Amount</span>
                <span className="font-mono">
                  {formatHns(draft.summary?.sendTotalDoos ?? 0)} HNS
                </span>
              </div>
              <div className="flex justify-between text-gray-500">
                <span>Fee</span>
                <span className="font-mono">{formatHns(draft.summary?.feeDoos ?? 0)} HNS</span>
              </div>
              <div className="flex justify-between text-gray-500">
                <span>Change</span>
                <span className="font-mono">{formatHns(draft.summary?.changeDoos ?? 0)} HNS</span>
              </div>
              <div className="flex justify-between text-gray-500">
                <span>Inputs</span>
                <span className="font-mono">{draft.summary?.numInputs ?? 0}</span>
              </div>
              {/* Show the FULL recipient address — never truncate it, so the
                  user always verifies exactly where funds are going. */}
              <div className="pt-1 border-t border-gray-200 mt-1">
                <div className="text-gray-500 mb-0.5">To</div>
                <div className="font-mono text-xs break-all" data-testid="send-recipient">
                  {draft.summary?.recipientAddress}
                </div>
              </div>
            </div>
            <div className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2">
              ⚠️ Beta — send a small test amount first and confirm it arrives before sending larger
              amounts.
            </div>
            {sendError && (
              <div
                className="bg-red-50 border border-red-300 rounded p-2 text-xs text-red-800"
                role="alert"
                data-testid="send-error"
              >
                <span className="font-semibold">Not sent.</span> {sendError} Your coins were not
                moved. You can adjust and try again.
              </div>
            )}
            {!unlocked && !sendError && (
              <div className="bg-blue-50 border border-blue-200 rounded p-2 text-xs text-blue-800 flex items-center justify-between gap-2">
                <span>You'll be asked for your passphrase in a secure window to sign.</span>
                <UnlockButton size="sm" variant="primary" />
              </div>
            )}
            <div className="flex gap-2 justify-end">
              <Button
                variant="ghost"
                onClick={() => {
                  setDraft(null);
                  setSendError(null);
                }}
                disabled={submitting}
              >
                Back
              </Button>
              <Button variant="danger" onClick={handleConfirmSend} disabled={submitting}>
                {submitting
                  ? isLedger
                    ? "Confirm on your Ledger…"
                    : "Sending…"
                  : sendError
                    ? "Retry Sign & Broadcast"
                    : "Sign & Broadcast"}
              </Button>
            </div>
          </div>
        )}
      </Dialog>
    </div>
  );
}
