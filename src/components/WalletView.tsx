import { ArrowRight, QrCode, RefreshCw, Send, TriangleAlert } from "lucide-react";
import { Icon } from "./ui/Icon";
import { isSignerUnlockedFor } from "../lib/signer";
import { useState, useEffect, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { settingsPath } from "../lib/settingsSections";
import { useQueryClient } from "@tanstack/react-query";
import {
  useActiveProfile,
  useSignerSession,
  useWriteCapability,
  useWalletBalances,
  useBuildSendDraft,
  useEstimateMaxSend,
} from "../queries/wallet";
import { useDraftRunner } from "../queries/useDraftRunner";
import { usePurchaseFinalize } from "./market/usePurchase";
import { useReadBalance } from "../queries/read";
import { useNeedsYou } from "../queries/needsYou";
import {
  useStartFullSync,
  useSyncStatus,
  useCancelFullSync,
  useNodeCoinsNotSyncedReason,
} from "../queries/sync";
import { useStartHsd } from "../queries/node";
import { useSyncTriggerStore } from "../stores/syncTrigger";
import { displayName } from "../lib/idn";
import { useOpenName } from "../hooks/useOpenName";
import { BlockInfoModal } from "./BlockInfoModal";
import { RecentActivityCard } from "./activity/RecentActivityCard";
import { useSignerToggle } from "../hooks/useSignerToggle";
import { NeedsYouCard } from "./NeedsYouCard";
import { ReceiveDialog } from "./ReceiveDialog";
import { AddWalletForm } from "./AddWalletForm";
import { UnlockButton } from "./UnlockButton";
import { YourNamesCard } from "./YourNamesCard";
import { Button } from "./ui/Button";
import { FeeRateOverride } from "./ui/FeeRateOverride";
import { parseFeeRateArg } from "../lib/feeRate";
import { Badge } from "./ui/Badge";
import { Input } from "./ui/Input";
import { Dialog } from "./ui/Dialog";
import { PageHeader } from "./ui/PageHeader";
import {
  formatHns,
  formatHnsShort,
  hnsToDollarydoos,
  isLikelyHnsAddress,
  hnsAddressPrefix,
} from "../lib/utils";
import { mapError, unwrapStaged } from "../lib/errors";
import { explorerCoversNetwork } from "../lib/openExternal";
import { useUiStore } from "../stores/ui";
import type { TxDraftSummary } from "../types";
import { subscribeAction } from "../lib/actionBus";
import { Tooltip } from "./ui/Tooltip";
import { shakedexRefusal } from "./market/marketText";

export function WalletView() {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);

  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const { data: writeCap } = useWriteCapability();
  const { data: balances } = useWalletBalances();
  const { data: readBalance } = useReadBalance();
  // Needs you and its ONE batch capability fetch (pinned to the active
  // wallet, empty for watch-only); Your names reads the same index, so a name
  // is described the same way everywhere on the page.
  const needsYou = useNeedsYou();
  const { capsByName } = needsYou;

  const { finalize: finalizePurchaseDraft, finalizingId } = usePurchaseFinalize();

  const startSync = useStartFullSync();
  const startHsd = useStartHsd();
  const cancelSync = useCancelFullSync();
  const syncStatus = useSyncStatus();
  const manualSync = useSyncTriggerStore((s) => s.manualSync);
  const setManualSync = useSyncTriggerStore((s) => s.setManualSync);
  const signerToggle = useSignerToggle();
  const buildDraft = useBuildSendDraft();
  const estimateMax = useEstimateMaxSend();
  // The amount Max put in the field: Review sends a sweep (no change left)
  // while the field still holds it, and that amount otherwise.
  const [maxFill, setMaxFill] = useState<number | null>(null);
  // The draft Review or Max builds reserves its coins: leaving the review
  // unsent abandons it, a failed send keeps it for Retry (useDraftRunner).
  const runner = useDraftRunner();

  const [sendOpen, setSendOpen] = useState(false);
  const [sendAddress, setSendAddress] = useState("");
  const [sendAmount, setSendAmount] = useState("");
  // Per-transaction fee-rate override (doos/kvB, raw text). Empty = use the
  // global setting default.
  const [sendFeeRate, setSendFeeRate] = useState("");
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
  const [infoBlock, setInfoBlock] = useState<number | null>(null);
  // Ledger wallets don't use the local signer session — the device signs on
  // demand. Treat them as "unlocked" so the send/action flow doesn't try to
  // prompt for a passphrase that doesn't exist.
  const isLedger = profile?.kind === "ledger_hardware";
  const unlocked = isLedger ? true : isSignerUnlockedFor(signer, profile?.id);
  const canWrite = writeCap?.canWrite ?? false;
  const isWatchOnly = profile?.watchOnly ?? false;
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
        // The shell's lock does the same; a Ledger or watch-only wallet has none.
        if (signerToggle.canLock) void signerToggle.toggle();
        break;
      case "wallet:openReceive":
        setReceiveOpen(true);
        break;
    }
  };
  useEffect(() => subscribeAction((id) => actionHandlerRef.current(id)), []);

  const resetSend = () => {
    void runner.abandon(draft?.id);
    setSendOpen(false);
    setSendAddress("");
    setSendAmount("");
    setSendFeeRate("");
    setDraft(null);
    setMaxFill(null);
    setSubmitting(false);
    setSendError(null);
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
  // so the page's status line doesn't flash a loading state.
  const openNamePage = useOpenName();
  const openName = (name: string) => openNamePage(name, { seedCaps: capsByName.get(name) });

  // Max fills the amount and stays on the form: the user may lower it first.
  const handleMax = async () => {
    try {
      const est = await estimateMax.mutateAsync(parseFeeRateArg(sendFeeRate) ?? undefined);
      // Plain digits: the amount field is a number input, which drops a value
      // with thousands separators (dollarydoosToHns adds them).
      setSendAmount((est.maxSendDoos / 1_000_000).toFixed(6));
      setMaxFill(est.maxSendDoos);
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleBuildDraft = async () => {
    const max = maxFill != null && hnsToDollarydoos(sendAmount) === maxFill;
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
      const result = await runner.send(draft.id, profile.id, unlocked, { keepOnFailure: true });
      // Only here — after the node accepted the tx — is the send complete.
      showToast(`Broadcast ${result.txid.slice(0, 12)}…`, "success");
      setDraft(null);
      resetSend();
      qc.invalidateQueries({ queryKey: ["wallet"] });
    } catch (e) {
      const msg = mapError(unwrapStaged(e));
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
          isWatchOnly ? (
            <Tooltip content="This wallet tracks an account xpub. It can receive and show balances, but it can't sign or spend.">
              <Badge variant="warning">Watch-only</Badge>
            </Tooltip>
          ) : undefined
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
                  icon: RefreshCw,
                  tooltip: "Scan the chain for this wallet's coins and names",
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
            className="mt-3 text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2 flex gap-1.5"
            data-testid="balance-node-not-authoritative"
          >
            <Icon icon={TriangleAlert} size={14} className="mt-px" />
            <span>Your node can't report this wallet's coins: {nodeCoinsReason}</span>
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
                className="inline-flex items-center gap-1.5"
              >
                <Icon icon={Send} size={14} /> Send
              </Button>
            </Tooltip>
          )}
          <Button
            variant="secondary"
            onClick={() => setReceiveOpen(true)}
            data-testid="receive-open"
            className="inline-flex items-center gap-1.5"
          >
            <Icon icon={QrCode} size={14} /> Receive
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
                  onClick={() => navigate(settingsPath("node"))}
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
          allNamesLink={
            <button
              type="button"
              className="text-xs text-blue-600 hover:underline inline-flex items-center gap-1"
              onClick={() => navigate("/names?filter=needs")}
              data-testid="needs-you-all-names"
            >
              All names <Icon icon={ArrowRight} size={12} />
            </button>
          }
        />
      )}

      <YourNamesCard
        capsByName={capsByName}
        needs={needsYou.items}
        onOpenName={openName}
        network={profile.network}
      />

      <RecentActivityCard onNameClick={(name) => openNamePage(name)} />

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
                <Tooltip content="Fill in your whole spendable balance minus the network fee; you can lower it before Review">
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={!canMax || buildDraft.isPending || estimateMax.isPending}
                    onClick={() => void handleMax()}
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
            <div className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2 flex gap-1.5">
              <Icon icon={TriangleAlert} size={14} className="mt-px" />
              <span>
                Beta — send a small test amount first and confirm it arrives before sending larger
                amounts.
              </span>
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
                <UnlockButton size="sm" variant="secondary" />
              </div>
            )}
            <div className="flex gap-2 justify-end">
              <Button
                variant="ghost"
                onClick={() => {
                  void runner.abandon(draft?.id);
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
