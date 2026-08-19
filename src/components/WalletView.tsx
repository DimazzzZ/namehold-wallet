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
import {
  useReadNames,
  useReadBalance,
  useReadRenewals,
  useNamesActionCapabilities,
  useActionHistory,
} from "../queries/read";
import { useStartFullSync, useSyncStatus, useCancelFullSync } from "../queries/sync";
import { useNodeLive, useStartHsd } from "../queries/node";
import { useSyncTriggerStore } from "../stores/syncTrigger";
import { auctionPhase, formatCountdown } from "../lib/auction";
import { displayName } from "../lib/idn";
import { NameActionsModal } from "./NameActionsModal";
import { NameInfoModal } from "./NameInfoModal";
import { BlockInfoModal } from "./BlockInfoModal";
import { TxInfoModal } from "./TxInfoModal";
import { ActivityRow } from "./ActivityView";
import { WalletManager } from "./WalletManager";
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
import { Alert } from "./ui/Alert";
import { Card } from "./ui/Card";
import { Disclosure } from "./ui/Disclosure";
import { CopyField } from "./ui/CopyField";
import {
  formatHns,
  hnsToDollarydoos,
  dollarydoosToHns,
  formatDate,
  latestTimestamp,
  isLikelyHnsAddress,
  truncateMiddle,
} from "../lib/utils";
import { mergeActivity } from "../lib/activity";
import { mapError } from "../lib/errors";
import {
  explorerAddressUrl,
} from "../lib/openExternal";
import { useUiStore } from "../stores/ui";
import { QRCodeSVG } from "qrcode.react";
import type { NameActionCapabilities, TxDraftSummary } from "../types";
import { subscribeAction } from "../lib/actionBus";

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
  const { data: history = [] } = useActionHistory();
  const [nameQuery, setNameQuery] = useState("");
  // Batch selection: names the user has checked for bulk operations.
  const [selectedNames, setSelectedNames] = useState<Set<string>>(new Set());
  // Batch renew mutation: calls build_batch_renew_draft, then signs + broadcasts.
  const batchRenewMutation = useNameAction("build_batch_renew_draft");
  // Batch reveal / redeem / finalize mutations (same generic pattern).
  const batchRevealMutation = useNameAction("build_batch_reveal_draft");
  const batchRedeemMutation = useNameAction("build_batch_redeem_draft");
  const batchFinalizeMutation = useNameAction("build_batch_finalize_draft");
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
  const { data: renewals } = useReadRenewals();
  // Capability-driven urgency alerts (F2 fix) — ONE batch fetch pinned to the
  // active wallet, replacing the old raw-phase filters below that showed a
  // false "you lost, redeem" for any CLOSED/unowned name even when this
  // wallet never placed a bid. Watch-only profiles never show these alerts
  // (no actions to take), so skip the fetch entirely for them.
  const { data: nameCaps = [], isError: nameCapsError } = useNamesActionCapabilities(
    profile?.watchOnly ? [] : names.map((n) => n.name),
    profile?.id ?? null,
  );

  const startSync = useStartFullSync();
  const startHsd = useStartHsd();
  const cancelSync = useCancelFullSync();
  const syncStatus = useSyncStatus();
  const manualSync = useSyncTriggerStore((s) => s.manualSync);
  const setManualSync = useSyncTriggerStore((s) => s.setManualSync);
  const nodeLive = useNodeLive();
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
  const [draft, setDraft] = useState<TxDraftSummary | null>(null);
  const [submitting, setSubmitting] = useState(false);
  // Toggle the QR alongside the receive address. Off by default — the address
  // text is the primary artifact; the QR is only useful when handing the
  // address to another device by camera.
  const [showQr, setShowQr] = useState(false);
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
  const [infoName, setInfoName] = useState<string | null>(null);
  const [infoBlock, setInfoBlock] = useState<number | null>(null);
  const [infoTx, setInfoTx] = useState<string | null>(null);
  // Batch confirmation modal state.
  const [batchModal, setBatchModal] = useState<{
    open: boolean;
    action: "renew" | "reveal" | "redeem" | "finalize";
    names: string[];
    feeDoos: number;
    draftId: string;
  } | null>(null);
  // Wallets manager modal (add / switch / delete). `addMode` opens it straight
  // to the add-wallet form.
  const [walletManagerOpen, setWalletManagerOpen] = useState(false);
  const [walletManagerAddMode, setWalletManagerAddMode] = useState(false);

  const unlocked = signer?.unlocked ?? false;
  const canWrite = writeCap?.canWrite ?? false;
  const isWatchOnly = profile?.watchOnly ?? false;
  const address = profile?.receiveAddress ?? null;
  // Spending uses node-synced coins (tracked_utxos), NOT the explorer balance.
  // If the explorer shows funds but nothing is synced yet, the user must connect
  // a node and Refresh before they can send.
  const spendable = balances?.liquidDoos ?? 0;
  const explorerBalance = readBalance?.confirmed ?? 0;
  const needsNodeSync = explorerBalance > 0 && spendable === 0;

  const isRunning = syncStatus.data?.running ?? false;

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
        if (canWrite) setSendOpen(true);
        break;
      case "wallet:sync":
        handleSync();
        break;
      case "wallet:toggleLock":
        if (unlocked) handleLock();
        else handleUnlock();
        break;
      case "wallet:toggleQr":
        setShowQr((v) => !v);
        break;
      case "wallet:focusFilter":
        filterInputRef.current?.focus();
        break;
      case "wallet:list:next":
        setSelectedNameIndex((i) =>
          Math.min(i + 1, filteredNames.length - 1),
        );
        break;
      case "wallet:list:prev":
        setSelectedNameIndex((i) => Math.max(i - 1, 0));
        break;
      case "wallet:list:open":
        if (selectedNameIndex >= 0 && filteredNames[selectedNameIndex]) {
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
  const toggleName = (name: string) =>
    setSelectedNames((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  const clearSelection = () => setSelectedNames(new Set());

  // Per-selection eligibility for batch actions. `nameCaps` is pinned to the
  // active profile via useNamesActionCapabilities and updates when phases move.
  // A batch action is "eligible" only when *every* selected name currently
  // supports it (canX.allowed = true). Empty selection ⇒ false.
  const batchEligibility = useMemo(() => {
    if (selectedNames.size === 0 || nameCaps.length === 0) {
      return { canReveal: false, canRedeem: false, canFinalize: false };
    }
    const capsByName = new Map(nameCaps.map((c) => [c.name, c]));
    let canReveal = true;
    let canRedeem = true;
    let canFinalize = true;
    for (const n of selectedNames) {
      const c = capsByName.get(n);
      if (!c) {
        canReveal = false;
        canRedeem = false;
        canFinalize = false;
        break;
      }
      if (!c.canReveal.allowed) canReveal = false;
      if (!c.canRedeem.allowed) canRedeem = false;
      if (!c.canFinalize.allowed) canFinalize = false;
    }
    return { canReveal, canRedeem, canFinalize };
  }, [selectedNames, nameCaps]);

  // Batch renew: build a single tx with multiple renewal covenants, sign, broadcast.
  // Compute the fee-rate arg once for all batch handlers (null = use setting default).
  const batchFeeRateArg = parseFeeRateArg(batchFeeRate) ?? undefined;

  const handleBatchRenew = async () => {
    const names = Array.from(selectedNames);
    if (names.length === 0) return;
    try {
      showToast(`Building batch renew draft…`, "info");
      const draft = await batchRenewMutation.mutateAsync({ names, feeRate: batchFeeRateArg });
      const feeDoos = draft.summary?.feeDoos ?? 0;
      setBatchModal({
        open: true,
        action: "renew",
        names,
        feeDoos,
        draftId: draft.id,
      });
    } catch (e) {
      showToast(`Batch renew failed: ${e}`, "error");
    }
  };

  // Batch reveal: build a single tx with multiple REVEAL covenants.
  const handleBatchReveal = async () => {
    const names = Array.from(selectedNames);
    if (names.length === 0) return;
    try {
      showToast(`Building batch reveal draft…`, "info");
      const draft = await batchRevealMutation.mutateAsync({ names, feeRate: batchFeeRateArg });
      const feeDoos = draft.summary?.feeDoos ?? 0;
      setBatchModal({
        open: true,
        action: "reveal",
        names,
        feeDoos,
        draftId: draft.id,
      });
    } catch (e) {
      showToast(`Batch reveal failed: ${e}`, "error");
    }
  };

  // Batch redeem: build a single tx to sweep losing-bid coins.
  const handleBatchRedeem = async () => {
    const names = Array.from(selectedNames);
    if (names.length === 0) return;
    try {
      showToast(`Building batch redeem draft…`, "info");
      const draft = await batchRedeemMutation.mutateAsync({ names, feeRate: batchFeeRateArg });
      const feeDoos = draft.summary?.feeDoos ?? 0;
      setBatchModal({
        open: true,
        action: "redeem",
        names,
        feeDoos,
        draftId: draft.id,
      });
    } catch (e) {
      showToast(`Batch redeem failed: ${e}`, "error");
    }
  };

  // Batch finalize: build a single tx with multiple FINALIZE covenants.
  const handleBatchFinalize = async () => {
    const names = Array.from(selectedNames);
    if (names.length === 0) return;
    try {
      showToast(`Building batch finalize draft…`, "info");
      const draft = await batchFinalizeMutation.mutateAsync({ names, feeRate: batchFeeRateArg });
      const feeDoos = draft.summary?.feeDoos ?? 0;
      setBatchModal({
        open: true,
        action: "finalize",
        names,
        feeDoos,
        draftId: draft.id,
      });
    } catch (e) {
      showToast(`Batch finalize failed: ${e}`, "error");
    }
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
            No wallet profile is active. Create or import one — your recovery phrase
            and passphrase are handled only in a secure window.
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
      ? `Enter a valid ${profile.network} address (starts with ${
          profile.network === "mainnet" ? "hs1" : profile.network === "testnet" ? "ts1" : "rs1"
        }…)`
      : null;
  const amountError =
    sendAmount.trim() && (isNaN(sendAmtDoos) || sendAmtDoos <= 0)
      ? "Enter a positive amount"
      : sendAmount.trim() && sendAmtDoos > spendable
        ? "Amount exceeds your spendable balance"
        : null;
  const canMax = !!sendAddress.trim() && !addressError && spendable > 0;

  return (
    <div className="space-y-6">
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
        <div className="bg-blue-50 border border-blue-200 rounded p-3 text-sm text-blue-800" data-testid="sync-status">
          <div className="font-medium">{syncStatus.data.progressLabel}</div>
          <div className="text-xs mt-1 space-y-0.5">
            <div>Step: {syncStatus.data.step}{syncStatus.data.waiting ? " (waiting for explorer…)" : ""}</div>
            {syncStatus.data.repairCandidates > 0 && (
              <div>
                Checked: {Math.max(0, syncStatus.data.repairCandidates - syncStatus.data.repairRemaining)}
                {" "}· Owned: +{syncStatus.data.repaired} · Remaining: ~{syncStatus.data.repairRemaining}
              </div>
            )}
            {syncStatus.data.step === "discover" && syncStatus.data.discoverAddressesTotal > 0 && (
              <div>Addresses: {syncStatus.data.discoverAddressesDone} / {syncStatus.data.discoverAddressesTotal}</div>
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
            <span
              className="text-xs text-gray-500"
              title={
                unlocked
                  ? "Your keys are in memory. They lock automatically after the session timeout."
                  : undefined
              }
            >
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

      {/* Receive & share — the receive address (primary), an opt-in QR, and the
          account xpub tucked behind a disclosure. The xpub is public material
          (reveals derived addresses; cannot spend), consulted rarely (Namebase
          setup), so it stays collapsed by default. If a multisig wallet kind is
          ever added, the xpub disclosure MUST gate on single-sig, since Namebase
          only supports single-signature xpubs. */}
      <Card
        title="Receive & share"
        actions={
          address ? (
            <Button size="sm" variant="ghost" onClick={() => setShowQr((v) => !v)}>
              {showQr ? "Hide QR" : "Show QR"}
            </Button>
          ) : undefined
        }
      >
        <div className="space-y-4" data-testid="account-xpub-card">
          {address ? (
            <>
              <CopyField
                label={
                  <span className="flex items-center gap-2">
                    <span>Receive Address</span>
                    <Badge variant={profile.network === "mainnet" ? "info" : "warning"}>
                      {profile.network}
                    </Badge>
                    {profile.network !== "mainnet" && (
                      <span className="text-xs text-amber-600">
                        — {profile.network} addresses differ from mainnet
                      </span>
                    )}
                  </span>
                }
                value={address}
                copyLabel="Copy Address"
                toastLabel="Address"
                externalUrl={
                  profile.network === "mainnet"
                    ? explorerAddressUrl(address)
                    : undefined
                }
                externalTestId="receive-address-explorer-link"
              />
              {showQr && (
                <div className="flex justify-center">
                  <QRCodeSVG value={address} size={128} level="M" />
                </div>
              )}
            </>
          ) : (
            <div className="text-gray-400">No address derived yet. Try syncing.</div>
          )}

          <Disclosure
            summary={
              <span className="flex items-center gap-2">
                <span>Show account public key (xpub) for Namebase</span>
                <span className="font-mono text-xs text-gray-400">
                  {truncateMiddle(profile.accountXpub)}
                </span>
              </span>
            }
          >
            <div className="text-sm text-gray-500 mb-1 flex items-center gap-2">
              <span>Account public key (xpub)</span>
              <Badge variant={profile.network === "mainnet" ? "info" : "warning"}>
                {profile.network}
              </Badge>
            </div>
            <CopyField
              value={profile.accountXpub}
              valueTestId="account-xpub-value"
              copyTestId="copy-xpub"
              copyLabel="Copy public key"
              toastLabel="Account public key"
            />
            <Alert
              tone="info"
              title="For Namebase / xpub-import payees only"
              className="mt-3"
            >
              Paste this into Namebase's "account public key (xpub)" field so
              buyers pay your wallet directly. This is a single-signature wallet,
              so the addresses derived from it are yours to spend. Anyone with
              this key can see every address and balance you'll ever use, but
              cannot move your funds.
            </Alert>
          </Disclosure>
        </div>
      </Card>

      {/* Balance — Spendable is the hero (what coin selection can actually use
          after a node sync); Confirmed/Unconfirmed and the name-bound totals
          are secondary text-xs cells. Confirmed/Unconfirmed come from the node
          when available, otherwise the explorer. */}
      <Card title="Balance">
        <div className="flex items-baseline gap-2">
          <span className="text-3xl font-bold tabular-nums">
            {formatHns(balances?.liquidDoos ?? 0)}
          </span>
          <span className="text-sm text-gray-500">HNS · spendable (from node sync)</span>
        </div>
        <div className="mt-3 grid grid-cols-2 sm:grid-cols-4 gap-x-4 gap-y-2 text-xs text-gray-500">
          <div>
            <div>Confirmed</div>
            <div className="text-sm text-gray-800 tabular-nums font-mono">
              {formatHns(readBalance?.confirmed ?? 0)}
            </div>
          </div>
          <div>
            <div>Unconfirmed</div>
            <div className="text-sm text-gray-800 tabular-nums font-mono">
              {formatHns(readBalance?.unconfirmed ?? 0)}
            </div>
          </div>
          {(balances?.nameLockupDoos ?? 0) > 0 && (
            <div data-testid="balance-locked-auctions">
              <div title="In-flight bids — returned on reveal/redeem">
                Locked in Auctions
              </div>
              <div className="text-sm text-gray-800 tabular-nums font-mono">
                {formatHns(balances!.nameLockupDoos)}
              </div>
            </div>
          )}
          {(balances?.nameControlDoos ?? 0) > 0 && (
            <div data-testid="balance-name-value">
              <div title="Value bound to names you control">Name Value</div>
              <div className="text-sm text-gray-800 tabular-nums font-mono">
                {formatHns(balances!.nameControlDoos)}
              </div>
            </div>
          )}
        </div>
      </Card>

      {/* Degraded notice (Task 12 review folded into Task 14): the batch
          capabilities query silently renders nothing while loading (the
          transient window is short and self-healing per Task 12's review),
          but a PERSISTENT failure (isError) must not look identical to "no
          urgent tasks" — it means we genuinely can't tell.
          Only show when the node is NOT live — when the node is live, the
          query self-heals on the next 30s poll, so the error is transient. */}
      {!isWatchOnly && nameCapsError && !nodeLive && (
        <div
          className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2"
          data-testid="urgent-tasks-degraded"
        >
          Couldn't verify urgent auction tasks — data may be stale.
        </div>
      )}

      {/* Auction urgency alerts (F2 fix) — driven ENTIRELY by the backend
          capability model (taskState), not raw phase filters. The old
          phase-based filters showed "you lost, redeem" for ANY CLOSED name
          with no owner — including names this wallet never bid on — and
          never showed a countdown. Capabilities are the single source of
          truth for "does this wallet actually have a redeemable/revealable/
          registerable coin for this name". */}
      {!isWatchOnly &&
        (() => {
          const capsByName = new Map<string, NameActionCapabilities>(
            nameCaps.map((c) => [c.name, c]),
          );
          // Countdown fragment for a name's capabilities — honest: when the
          // backend has no live countdown (e.g. node unreachable/no stats),
          // this is null and the banner renders WITHOUT a countdown fragment
          // rather than fabricating one.
          const countdownFragment = (name: string): string | null => {
            const c = capsByName.get(name);
            if (!c || c.countdownBlocks == null) return null;
            return formatCountdown({
              label: c.countdownLabel ?? "",
              blocks: c.countdownBlocks,
              hours: c.countdownHours,
            });
          };

          const revealNeeded = names.filter(
            (n) => capsByName.get(n.name)?.taskState === "readyToReveal",
          );
          const wonNeeded = names.filter(
            (n) => capsByName.get(n.name)?.taskState === "wonNeedsRegister",
          );
          // Only a name whose capability model says lostNeedsRedeem — i.e. we
          // hold a redeemable reveal coin for it — surfaces here. A CLOSED
          // name with no owner that this wallet never bid on (or already
          // redeemed) never reaches this state, so it never shows a false
          // "you lost" banner.
          const lostNeeded = names.filter(
            (n) => capsByName.get(n.name)?.taskState === "lostNeedsRedeem",
          );
          if (revealNeeded.length === 0 && wonNeeded.length === 0 && lostNeeded.length === 0) return null;

          const revealCountdown = revealNeeded.length > 0 ? countdownFragment(revealNeeded[0]!.name) : null;

          return (
            <>
              {revealNeeded.length > 0 && (
                <div
                  className="flex items-center justify-between gap-3 text-sm text-amber-900 bg-amber-50 border border-amber-300 rounded p-3"
                  data-testid="reveal-alert"
                >
                  <div>
                    <strong>Action required: reveal your bid</strong> —{" "}
                    {revealNeeded.map((n) => `.${displayName(n.name)}`).join(", ")}{" "}
                    {revealNeeded.length === 1 ? "is" : "are"} in the reveal phase. Reveal
                    before the window closes or your locked bid can't be reclaimed.
                    {revealCountdown && <> Reveal ends in {revealCountdown}.</>}
                  </div>
                  <Button size="sm" onClick={() => setManageName(revealNeeded[0]!.name)}>
                    Reveal
                  </Button>
                </div>
              )}
              {wonNeeded.length > 0 && (
                <div
                  className="flex items-center justify-between gap-3 text-sm text-green-900 bg-green-50 border border-green-300 rounded p-3"
                  data-testid="register-alert"
                >
                  <div>
                    <strong>Won! Register now</strong> —{" "}
                    {wonNeeded.map((n) => `.${displayName(n.name)}`).join(", ")}{" "}
                    {wonNeeded.length === 1 ? "was" : "were"} won. Register to
                    finalize ownership and set DNS records.
                  </div>
                  <Button size="sm" onClick={() => setManageName(wonNeeded[0]!.name)}>
                    Register
                  </Button>
                </div>
              )}
              {lostNeeded.length > 0 && (
                <div
                  className="flex items-center justify-between gap-3 text-sm text-red-900 bg-red-50 border border-red-300 rounded p-3"
                  data-testid="redeem-alert"
                >
                  <div>
                    <strong>Lost bid — redeem lockup</strong> —{" "}
                    {lostNeeded.map((n) => `.${displayName(n.name)}`).join(", ")}{" "}
                    {lostNeeded.length === 1 ? "was" : "were"} not won. Redeem your
                    reveal coin to reclaim the funds.
                  </div>
                  <Button size="sm" onClick={() => setManageName(lostNeeded[0]!.name)}>
                    Redeem
                  </Button>
                </div>
              )}
            </>
          );
        })()}

      {/* Renewal urgency alert — chain-sourced only: CSV-imported expiry data
          is stale by definition and must not fire a "renew now" alarm. */}
      {!isWatchOnly &&
        (() => {
          const expiring = (renewals?.names ?? []).filter(
            (r) => r.expiringSoon && r.source === "chain",
          );
          if (expiring.length === 0) return null;
          return (
            <div
              className="flex items-center justify-between gap-3 text-sm text-red-900 bg-red-50 border border-red-300 rounded p-3"
              data-testid="expiring-alert"
            >
              <div>
                <strong>Renew soon — name{expiring.length === 1 ? "" : "s"} expiring</strong>{" "}
                — {expiring.map((r) => `.${displayName(r.name)}`).join(", ")}{" "}
                {expiring.length === 1 ? "is" : "are"} close to the end of the renewal
                window. Renew now — an expired Handshake name is lost forever.
              </div>
              <Button size="sm" onClick={() => setManageName(expiring[0]!.name)}>
                Renew
              </Button>
            </div>
          );
        })()}

      {/* Actions — spend + go-to-Auctions merged into one card. The Send button
          gates on canWrite (writeCap.reason surfaced next to it) and the
          spendable balance; needs-node-sync warns when explorer shows funds
          but nothing is synced yet. */}
      {!isWatchOnly && (
        <Card title="Actions">
          <div className="space-y-2">
            <div className="flex flex-wrap items-center gap-3">
              <Button
                variant="primary"
                onClick={() => setSendOpen(true)}
                disabled={!canWrite || spendable === 0}
              >
                Send HNS
              </Button>
              <Button
                size="sm"
                variant="secondary"
                onClick={() => navigate("/auctions")}
              >
                Auctions
              </Button>
              {!canWrite && (
                <div className="flex items-center gap-2">
                  <span className="text-sm text-amber-600">
                    {writeCap?.reason ??
                      (needsNodeSync
                        ? "Sync your coins below, then unlock to send."
                        : "Connect a node, Refresh to sync your coins, then unlock to send.")}
                  </span>
                  <UnlockButton size="sm" variant="primary" label="Unlock" />
                </div>
              )}
            </div>
            <div className="text-xs text-gray-500">
              Get a TLD: acquire new Handshake domains through the Vickrey
              auction system.
            </div>
            {needsNodeSync && (
              <div
                className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2 space-y-2"
                data-testid="needs-node-sync"
              >
                <div>
                  Your balance is read from the explorer, but spending requires a
                  synced node. Start your node to load your spendable coins.
                </div>
                <div className="flex items-center gap-3">
                  <Button
                    size="sm"
                    variant="primary"
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
          </div>
        </Card>
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
                        checked={selectedNames.size > 0 && selectedNames.size === filteredNames.length}
                        ref={(el) => {
                          if (el) el.indeterminate = selectedNames.size > 0 && selectedNames.size < filteredNames.length;
                        }}
                        onChange={(e) => {
                          if (e.target.checked) setSelectedNames(new Set(filteredNames.map((n) => n.name)));
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
                      ref={filteredNames.indexOf(n) === selectedNameIndex ? selectedRowRef : undefined}
                      aria-selected={filteredNames.indexOf(n) === selectedNameIndex}
                      tabIndex={filteredNames.indexOf(n) === selectedNameIndex ? 0 : -1}
                      className={`border-t border-gray-100 hover:bg-gray-50 cursor-pointer ${
                        filteredNames.indexOf(n) === selectedNameIndex
                          ? "bg-blue-50 ring-1 ring-blue-300"
                          : ""
                      }`}
                      onClick={() => setManageName(n.name)}
                    >
                      <td className="py-1 pr-4">
                        <input
                          type="checkbox"
                          checked={selectedNames.has(n.name)}
                          onChange={() => toggleName(n.name)}
                          aria-label={`Select ${displayName(n.name)}`}
                        />
                      </td>
                      <td className="py-1 pr-4 text-xs font-mono">
                        <button
                          type="button"
                          className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                          onClick={() => setInfoName(n.name)}
                          title="View name info"
                          data-testid="owned-name-info-link"
                        >
                          .{displayName(n.name)}
                        </button>
                      </td>
                      <td className="py-1 pr-4">
                        {n.state ? (
                          <Badge variant={auctionPhase(n.state).variant}>
                            {auctionPhase(n.state).label}
                          </Badge>
                        ) : (
                          "—"
                        )}
                      </td>
                      <td className="py-1 pr-4 text-xs text-gray-500 font-mono">
                        {n.height ? (
                          <button
                            type="button"
                            className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                            onClick={() => setInfoBlock(n.height!)}
                            title="View block info"
                            data-testid="owned-name-height-info-link"
                          >
                            #{n.height}
                          </button>
                        ) : (
                          "—"
                        )}
                      </td>
                      <td className="py-1 pr-4 text-xs text-gray-500 font-mono">
                        {n.renewal ? (
                          <button
                            type="button"
                            className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                            onClick={() => setInfoBlock(n.renewal!)}
                            title="View block info"
                            data-testid="owned-name-renewal-info-link"
                          >
                            #{n.renewal}
                          </button>
                        ) : (
                          "—"
                        )}
                      </td>
                      <td className="py-1 text-right">
                        {!isWatchOnly && (
                          <Button size="sm" variant="ghost" onClick={() => setManageName(n.name)}>
                            Manage
                          </Button>
                        )}
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
                <span className="text-blue-800 font-medium">
                  {selectedNames.size} selected
                </span>
                <Button size="sm" variant="primary" onClick={handleBatchRenew}>
                  Renew Selected
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  onClick={handleBatchReveal}
                  disabled={!batchEligibility.canReveal}
                  title={
                    batchEligibility.canReveal
                      ? undefined
                      : "All selected names must be in the REVEAL phase"
                  }
                  data-testid="batch-reveal-btn"
                >
                  Reveal Selected
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  onClick={handleBatchRedeem}
                  disabled={!batchEligibility.canRedeem}
                  title={
                    batchEligibility.canRedeem
                      ? undefined
                      : "All selected names must have redeemable losing bids"
                  }
                  data-testid="batch-redeem-btn"
                >
                  Redeem Selected
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  onClick={handleBatchFinalize}
                  disabled={!batchEligibility.canFinalize}
                  title={
                    batchEligibility.canFinalize
                      ? undefined
                      : "All selected names must have a transfer ready to finalize"
                  }
                  data-testid="batch-finalize-btn"
                >
                  Finalize Selected
                </Button>
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
              : "No owned names found yet. Click Refresh to scan for names this wallet owns."}
          </div>
        )}
      </div>

      {/* Recent activity — real on-chain history from the node (node-indexed). */}
      {/* Merged activity: on-chain history + local drafts, deduped by txid. */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <div className="flex items-center justify-between mb-2">
          <div className="text-sm text-gray-500">Recent activity</div>
          <button
            className="text-xs text-blue-600 hover:underline"
            onClick={() => navigate("/activity")}
          >
            See all →
          </button>
        </div>
        {mergeActivity(history, drafts).length > 0 ? (
          <div className="max-h-72 overflow-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="py-1 pr-4">Date</th>
                  <th className="py-1 pr-4">Action</th>
                  <th className="py-1 pr-4">Name</th>
                  <th className="py-1 pr-4 text-right">Amount</th>
                  <th className="py-1 pr-4 text-right">Fee</th>
                  <th className="py-1 pr-4">Status</th>
                  <th className="py-1 pr-4">Block</th>
                  <th className="py-1 pr-4">Txid</th>
                  <th className="py-1">Actions</th>
                </tr>
              </thead>
              <tbody>
                {mergeActivity(history, drafts)
                  .slice(0, 10)
                  .map((row) => (
                    <ActivityRow
                      key={row.key}
                      row={row}
                      onNameClick={setInfoName}
                      onBlockClick={setInfoBlock}
                      onTxClick={setInfoTx}
                      enableDraftActions
                      profileId={profile?.id ?? null}
                    />
                  ))}
              </tbody>
            </table>
          </div>
        ) : (
          <div className="text-gray-400 text-sm py-4 text-center">
            No activity yet.
          </div>
        )}
      </div>

      {/* Diagnostics — collapsed by default; debug-only info that shouldn't
          compete for attention on the main screen. */}
      <Disclosure summary="Details">
        <div className="text-xs text-gray-400">
          Profile: {profile.id.slice(0, 8)}… | Last synced height:{" "}
          {profile.lastSyncedHeight ?? "—"} | Last successful sync:{" "}
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

      {infoName && (
        <NameInfoModal
          name={infoName}
          open={!!infoName}
          onClose={() => setInfoName(null)}
        />
      )}

      {infoBlock != null && (
        <BlockInfoModal
          height={infoBlock}
          open={infoBlock != null}
          onClose={() => setInfoBlock(null)}
          isMainnet={profile.network === "mainnet"}
        />
      )}

      {infoTx != null && (
        <TxInfoModal
          txid={infoTx}
          open={infoTx != null}
          onClose={() => setInfoTx(null)}
          isMainnet={profile.network === "mainnet"}
        />
      )}

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
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={!canMax || buildDraft.isPending}
                  onClick={() => handleBuildDraft({ max: true })}
                  title="Send your entire spendable balance (minus the network fee)"
                >
                  Max
                </Button>
              </div>
              {amountError && (
                <div className="mt-1 text-xs text-red-600" data-testid="send-amount-error">
                  {amountError}
                </div>
              )}
            </div>
            <FeeRateOverride value={sendFeeRate} onChange={setSendFeeRate} />
            <div className="flex gap-2 justify-end">
              <Button variant="ghost" onClick={resetSend}>Cancel</Button>
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
                <span className="font-mono">{formatHns(draft.summary?.sendTotalDoos ?? 0)} HNS</span>
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
              ⚠️ Beta — send a small test amount first and confirm it arrives before
              sending larger amounts.
            </div>
            {sendError && (
              <div
                className="bg-red-50 border border-red-300 rounded p-2 text-xs text-red-800"
                role="alert"
                data-testid="send-error"
              >
                <span className="font-semibold">Not sent.</span> {sendError} Your coins
                were not moved. You can adjust and try again.
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
                {submitting ? "Sending…" : sendError ? "Retry Sign & Broadcast" : "Sign & Broadcast"}
              </Button>
            </div>
          </div>
        )}
      </Dialog>
    </div>
  );
}
