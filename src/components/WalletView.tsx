import { useState } from "react";
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
} from "../queries/wallet";
import {
  useReadNames,
  useReadBalance,
  useReadRenewals,
  useNamesActionCapabilities,
  useActionHistory,
} from "../queries/read";
import { useStartFullSync, useSyncStatus, useCancelFullSync } from "../queries/sync";
import { auctionPhase, formatCountdown } from "../lib/auction";
import { displayName } from "../lib/idn";
import { NameActionsModal } from "./NameActionsModal";
import { ACTION_META, FALLBACK_META } from "./ActivityView";
import { WalletManager } from "./WalletManager";
import { AddWalletForm } from "./AddWalletForm";
import { Button } from "./ui/Button";
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
  netSpendDoos,
  hnsToDollarydoos,
  dollarydoosToHns,
  formatDate,
  formatDateLong,
  amountTone,
  latestTimestamp,
  isLikelyHnsAddress,
  truncateMiddle,
} from "../lib/utils";
import { mapError } from "../lib/errors";
import {
  explorerAddressUrl,
  explorerBlockUrl,
  explorerNameUrl,
  explorerTxUrl,
  openExternal,
} from "../lib/openExternal";
import { useUiStore } from "../stores/ui";
import { QRCodeSVG } from "qrcode.react";
import type { NameActionCapabilities, TxDraftSummary } from "../types";

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
  const cancelSync = useCancelFullSync();
  const syncStatus = useSyncStatus();
  const unlock = useUnlockSigner();
  const lock = useLockSigner();
  const setActive = useSetActiveProfile();
  const buildDraft = useBuildSendDraft();
  const signDraft = useSignTxDraft();
  const broadcast = useBroadcastTxDraft();

  const [sendOpen, setSendOpen] = useState(false);
  const [sendAddress, setSendAddress] = useState("");
  const [sendAmount, setSendAmount] = useState("");
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
  const navigate = useNavigate();
  const [manageName, setManageName] = useState<string | null>(null);
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

  const resetSend = () => {
    setSendOpen(false);
    setSendAddress("");
    setSendAmount("");
    setDraft(null);
    setSubmitting(false);
    setSendError(null);
  };

  // Sync runs all reconciliation in a background thread.
  // The frontend polls status via useSyncStatus (persistent across navigation).
  const handleSync = async () => {
    try {
      await startSync.mutateAsync();
      showToast("Sync started in background", "info");
    } catch (e) {
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
                  label: "Sync",
                  onClick: handleSync,
                },
              ]
        }
      />

      {syncStatus.data?.running && (
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
          urgent tasks" — it means we genuinely can't tell. */}
      {!isWatchOnly && nameCapsError && (
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
                <span className="text-sm text-amber-600">
                  {writeCap?.reason ??
                    "Connect a node in Settings, Refresh to sync your coins, then unlock to send."}
                </span>
              )}
            </div>
            <div className="text-xs text-gray-500">
              Get a TLD: acquire new Handshake domains through the Vickrey
              auction system.
            </div>
            {needsNodeSync && (
              <div
                className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2"
                data-testid="needs-node-sync"
              >
                Your balance is read from the explorer, but spending requires a
                synced node. Connect a node in <strong>Settings</strong> and
                click <strong>Refresh</strong> to load your spendable coins.
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
          <input
            className="border border-gray-300 rounded px-2 py-1.5 text-sm w-48"
            value={nameQuery}
            onChange={(e) => setNameQuery(e.target.value)}
            placeholder="Filter…"
          />
        </div>
        {names.length > 0 ? (
          filteredNames.length > 0 ? (
            <div className="max-h-60 overflow-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-gray-500 border-b">
                    <th className="py-1">Name</th>
                    <th className="py-1">State</th>
                    <th className="py-1">Height</th>
                    <th className="py-1">Renewal</th>
                    <th className="py-1"></th>
                  </tr>
                </thead>
                <tbody>
                  {filteredNames.map((n) => (
                    <tr key={n.name} className="border-t border-gray-100">
                      <td className="py-1 font-mono">
                        {profile.network === "mainnet" ? (
                          <button
                            type="button"
                            className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                            onClick={() => openExternal(explorerNameUrl(n.name))}
                            title="View on explorer"
                            data-testid="owned-name-explorer-link"
                          >
                            .{displayName(n.name)}
                          </button>
                        ) : (
                          `.${displayName(n.name)}`
                        )}
                      </td>
                      <td className="py-1">
                        {n.state ? (
                          <Badge variant={auctionPhase(n.state).variant}>
                            {auctionPhase(n.state).label}
                          </Badge>
                        ) : (
                          "—"
                        )}
                      </td>
                      <td className="py-1 text-xs text-gray-500">
                        {n.height ? (
                          profile.network === "mainnet" ? (
                            <button
                              type="button"
                              className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                              onClick={() => openExternal(explorerBlockUrl(n.height!))}
                              title="View on explorer"
                              data-testid="owned-name-height-explorer-link"
                            >
                              #{n.height}
                            </button>
                          ) : (
                            `#${n.height}`
                          )
                        ) : (
                          "—"
                        )}
                      </td>
                      <td className="py-1 text-xs text-gray-500">
                        {n.renewal ? (
                          profile.network === "mainnet" ? (
                            <button
                              type="button"
                              className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                              onClick={() => openExternal(explorerBlockUrl(n.renewal!))}
                              title="View on explorer"
                              data-testid="owned-name-renewal-explorer-link"
                            >
                              #{n.renewal}
                            </button>
                          ) : (
                            `#${n.renewal}`
                          )
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
        {history.length > 0 ? (
          <div className="max-h-72 overflow-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="py-1 pr-4">Date</th>
                  <th className="py-1 pr-4">Action</th>
                  <th className="py-1 pr-4">Name</th>
                  <th className="py-1 pr-4 text-right">Amount</th>
                  <th className="py-1">Status</th>
                </tr>
              </thead>
              <tbody>
                {history.slice(0, 10).map((h) => {
                  const tone = amountTone(h);
                  const toneClass =
                    tone === "income"
                      ? "text-green-600"
                      : tone === "spend"
                      ? "text-red-600"
                      : "text-gray-700";
                  const sign = tone === "income" ? "+" : tone === "spend" ? "-" : "";
                  return (
                    <tr key={h.txid} className="border-t border-gray-100 hover:bg-gray-50">
                      <td className="py-1 pr-4 text-gray-500 whitespace-nowrap">
                        {h.time ? formatDateLong(new Date(h.time * 1000).toISOString()) : "Pending"}
                      </td>
                      <td className="py-1 pr-4">
                        <Badge variant={(ACTION_META[h.action] ?? FALLBACK_META).variant}>
                          {(ACTION_META[h.action] ?? FALLBACK_META).label}
                        </Badge>
                      </td>
                      <td className="py-1 pr-4 font-mono">
                        {h.name ? (
                          profile.network === "mainnet" ? (
                            <button
                              type="button"
                              className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                              onClick={() => openExternal(explorerNameUrl(h.name!))}
                              title="View on explorer"
                              data-testid="recent-activity-name-explorer-link"
                            >
                              .{displayName(h.name)}
                            </button>
                          ) : (
                            `.${displayName(h.name)}`
                          )
                        ) : (
                          "—"
                        )}
                      </td>
                      <td
                        className="py-1 pr-4 font-mono text-right whitespace-nowrap"
                        title={
                          h.valueDoos === 0 && h.direction !== "receive"
                            ? "Name's locked value is re-homed to your own coin — no HNS spent beyond the fee."
                            : undefined
                        }
                      >
                        <span className={toneClass}>
                          {sign}
                          {formatHns(Math.abs(h.valueDoos))}
                        </span>
                      </td>
                      <td className="py-1">
                        <Badge variant={h.confirmed ? "success" : "warning"}>
                          {h.confirmed ? "Confirmed" : "Pending"}
                        </Badge>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : (
          <div className="text-gray-400 text-sm py-4 text-center">
            No activity yet. Requires a synced node with the address index.
          </div>
        )}
      </div>

      {/* Recent drafts — local send/name drafts and their broadcast status. */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <div className="text-sm text-gray-500 mb-2">Recent transactions ({drafts.length})</div>
        {drafts.length > 0 ? (
          <div className="max-h-72 overflow-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="py-2 pr-4">Date</th>
                  <th className="py-2 pr-4">Action</th>
                  <th className="py-2 pr-4">Amount</th>
                  <th className="py-2 pr-4">Fee</th>
                  <th className="py-2 pr-4">Status</th>
                  <th className="py-2">Txid</th>
                </tr>
              </thead>
              <tbody>
                {drafts.map((d) => (
                  <tr key={d.id} className="border-t border-gray-100">
                    <td className="py-2 pr-4 text-xs text-gray-500">{formatDate(d.createdAt)}</td>
                    <td className="py-2 pr-4">
                      {d.action}{d.summary?.name ? (
                        profile.network === "mainnet" ? (
                          <>
                            {" · "}
                            <button
                              type="button"
                              className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer font-mono"
                              onClick={() => openExternal(explorerNameUrl(d.summary!.name!))}
                              title="View on explorer"
                              data-testid="recent-tx-name-explorer-link"
                            >
                              .{displayName(d.summary.name)}
                            </button>
                          </>
                        ) : (
                          ` · .${displayName(d.summary.name)}`
                        )
                      ) : ""}
                    </td>
                    <td
                      className="py-2 pr-4 font-mono"
                      title={
                        d.summary &&
                        d.summary.recipientAddress == null &&
                        d.summary.sendTotalDoos > 0
                          ? `Name value ${formatHns(
                              d.summary.sendTotalDoos,
                            )} HNS is carried to your own new coin — not spent; only the fee applies.`
                          : undefined
                      }
                    >
                      {d.summary ? formatHns(netSpendDoos(d.summary)) : "—"}
                    </td>
                    <td className="py-2 pr-4 font-mono text-xs text-gray-500">
                      {d.summary ? formatHns(d.summary.feeDoos) : "—"}
                    </td>
                    <td className="py-2 pr-4">
                      <Badge
                        variant={
                          d.status === "confirmed"
                            ? "success"
                            : d.status === "broadcasted"
                            ? "warning"
                            : d.status === "failed" || d.status === "dropped"
                            ? "error"
                            : "default"
                        }
                        title={d.errorMessage ?? undefined}
                      >
                      {d.status === "confirmed"
                         ? d.confirmationHeight
                            ? (profile.network === "mainnet" ? (
                              <>
                                Confirmed ·{" "}
                                <button
                                  type="button"
                                  className="underline cursor-pointer"
                                  onClick={() => openExternal(explorerBlockUrl(d.confirmationHeight!))}
                                  title="View block on explorer"
                                  data-testid="recent-tx-block-explorer-link"
                                >
                                  #{d.confirmationHeight}
                                </button>
                              </>
                            ) : `Confirmed · #${d.confirmationHeight}`)
                            : "Confirmed"
                          : d.status === "broadcasted"
                          ? "Pending"
                          : d.status === "dropped"
                          ? "Not confirmed"
                          : d.status}
                      </Badge>
                    </td>
                    <td className="py-2 text-xs font-mono truncate max-w-[120px]">
                      {d.txid ? (
                        profile.network === "mainnet" ? (
                         <button
                           type="button"
                            className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                            onClick={() => openExternal(explorerTxUrl(d.txid!))}
                            title="View on explorer"
                            data-testid="recent-tx-explorer-link"
                          >
                            {`${d.txid.slice(0, 10)}…`}
                          </button>
                        ) : (
                          `${d.txid.slice(0, 10)}…`
                        )
                      ) : (
                        "—"
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <div className="text-gray-400 text-sm py-4 text-center">No transactions yet.</div>
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

      {manageName && (
        <NameActionsModal
          name={manageName}
          open={!!manageName}
          onClose={() => setManageName(null)}
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
              <div className="bg-blue-50 border border-blue-200 rounded p-2 text-xs text-blue-800">
                You'll be asked for your passphrase in a secure window to sign.
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
