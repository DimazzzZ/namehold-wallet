import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useWalletProfiles,
  useActiveProfile,
  useSignerSession,
  useWriteCapability,
  useWalletBalances,
  useTxDrafts,
  useSyncWalletState,
  useDiscoverOwnedNames,
  useUnlockSigner,
  useLockSigner,
  useSetActiveProfile,
  useBuildSendDraft,
  useSignTxDraft,
  useBroadcastTxDraft,
} from "../queries/wallet";
import { useReadNames, useReadBalance } from "../queries/read";
import { auctionPhase } from "../lib/auction";
import { NameActionsModal } from "./NameActionsModal";
import { WalletManager } from "./WalletManager";
import { AddWalletForm } from "./AddWalletForm";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { Input } from "./ui/Input";
import { Dialog } from "./ui/Dialog";
import { PageHeader } from "./ui/PageHeader";
import {
  formatHns,
  hnsToDollarydoos,
  dollarydoosToHns,
  formatDate,
  isLikelyHnsAddress,
} from "../lib/utils";
import { mapError } from "../lib/errors";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useUiStore } from "../stores/ui";
import { QRCodeSVG } from "qrcode.react";
import type { TxDraftSummary } from "../types";

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

  const sync = useSyncWalletState();
  const discoverNames = useDiscoverOwnedNames();
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
  const [copied, setCopied] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  // A failed sign/broadcast must NOT look like success: we surface it as a
  // persistent in-dialog error (not just a transient toast) and keep the dialog
  // open so the user can see exactly what happened before deciding to retry.
  const [sendError, setSendError] = useState<string | null>(null);
  const [manageName, setManageName] = useState<string | null>(null);
  const [arbitraryName, setArbitraryName] = useState("");
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

  const handleCopyAddress = async () => {
    if (!address) return;
    await writeText(address);
    setCopied(true);
    showToast("Address copied", "success");
    setTimeout(() => setCopied(false), 2000);
  };

  // Refresh is best-effort. Reads (balance/names) and owned-name discovery come
  // from the explorer (node-free); the local node is OPTIONAL and only needed to
  // sync spendable coins. Neither a missing node nor a busy explorer should raise
  // a scary error toast — they're expected conditions, surfaced as calm info.
  const handleSync = async () => {
    // Node sync: soft-fail. A missing node already returns nodeReachable:false;
    // any other node error just means "couldn't sync spendable coins right now".
    let nodeReachable = false;
    try {
      const res = (await sync.mutateAsync(undefined)) as { nodeReachable?: boolean } | undefined;
      nodeReachable = res?.nodeReachable !== false;
    } catch {
      nodeReachable = false;
    }

    // Explorer discovery of owned names: best-effort, may be partial if the
    // explorer rate-limits mid-crawl.
    const found = (await discoverNames.mutateAsync().catch(() => undefined)) as
      | { discovered?: number; partial?: boolean }
      | undefined;

    if (found?.partial) {
      showToast(
        "The explorer is busy (rate-limited). Some names may be missing — Refresh again shortly.",
        "info",
      );
    } else if (nodeReachable) {
      showToast("Synced", "success");
    } else {
      const n = found?.discovered ?? 0;
      showToast(
        n > 0
          ? `Found ${n} owned name${n === 1 ? "" : "s"}. Connect a local node to sync spendable coins.`
          : "Reads refreshed. Connect a local node to sync spendable coins.",
        "info",
      );
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
        actions={[
          {
            label: sync.isPending || discoverNames.isPending ? "Refreshing…" : "Refresh",
            onClick: handleSync,
          },
        ]}
      />

      {/* Profile quick-switch + manage */}
      <div className="flex items-center gap-2 text-sm">
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
      </div>

      <WalletManager
        open={walletManagerOpen}
        startInAddMode={walletManagerAddMode}
        onClose={() => setWalletManagerOpen(false)}
      />

      {/* Signer status / lock controls */}
      {!isWatchOnly && (
        <div className="bg-white border border-gray-200 rounded-lg p-4 flex items-center justify-between">
          <div className="text-sm">
            <div className="font-medium text-gray-900">
              Signer {unlocked ? "unlocked" : "locked"}
            </div>
            <div className="text-gray-500">
              {unlocked
                ? "Your keys are in memory. They lock automatically after the session timeout."
                : profile.hasPassphrase
                  ? "Unlock with your passphrase (in a secure window) to sign transactions."
                  : "This wallet has no passphrase — just click Unlock to enable signing."}
            </div>
          </div>
          {unlocked ? (
            <Button variant="secondary" onClick={handleLock}>Lock</Button>
          ) : (
            <Button variant="primary" onClick={handleUnlock} disabled={unlock.isPending}>
              {unlock.isPending ? "Unlocking…" : "Unlock"}
            </Button>
          )}
        </div>
      )}

      {/* Receive Address */}
      <div className="bg-white rounded-lg p-6 border-2 border-blue-200">
        <div className="text-sm text-gray-500 mb-2 flex items-center gap-2">
          <span>Receive Address</span>
          <Badge variant={profile.network === "mainnet" ? "info" : "warning"}>
            {profile.network}
          </Badge>
          {profile.network !== "mainnet" && (
            <span className="text-xs text-amber-600">
              — {profile.network} addresses differ from mainnet
            </span>
          )}
        </div>
        {address ? (
          <div className="flex items-center gap-6">
            <div className="flex-1">
              <div className="font-mono text-lg font-bold break-all bg-gray-50 p-3 rounded">
                {address}
              </div>
              <div className="mt-2">
                <Button onClick={handleCopyAddress} variant="primary">
                  {copied ? "Copied!" : "Copy Address"}
                </Button>
              </div>
            </div>
            <div className="shrink-0">
              <QRCodeSVG value={address} size={150} level="M" />
            </div>
          </div>
        ) : (
          <div className="text-gray-400">No address derived yet. Try syncing.</div>
        )}
      </div>

      {/* Balances — confirmed/unconfirmed come from the HNSFans explorer
          (node-free); "Spendable (synced)" is what coin selection can use after
          a node sync. */}
      <div className="grid grid-cols-3 gap-4">
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Confirmed</div>
          <div className="text-2xl font-bold">{formatHns(readBalance?.confirmed ?? 0)}</div>
          <div className="text-xs text-gray-400">HNS · via explorer</div>
        </div>
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Unconfirmed</div>
          <div className="text-2xl font-bold">{formatHns(readBalance?.unconfirmed ?? 0)}</div>
          <div className="text-xs text-gray-400">HNS · via explorer</div>
        </div>
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Spendable (synced)</div>
          <div className="text-2xl font-bold">{formatHns(balances?.liquidDoos ?? 0)}</div>
          <div className="text-xs text-gray-400">HNS · from node sync</div>
        </div>
      </div>

      {/* Name-bound balances — surfaced only when there's value tied up in names
          or in-flight auction bids, so the user can see funds that aren't liquid. */}
      {((balances?.nameLockupDoos ?? 0) > 0 || (balances?.nameControlDoos ?? 0) > 0) && (
        <div className="grid grid-cols-2 gap-4">
          <div
            className="bg-white rounded p-4 border border-gray-200"
            data-testid="balance-locked-auctions"
          >
            <div className="text-sm text-gray-500">Locked in Auctions</div>
            <div className="text-2xl font-bold">
              {formatHns(balances?.nameLockupDoos ?? 0)}
            </div>
            <div className="text-xs text-gray-400">
              HNS · in-flight bids (returned on reveal/redeem)
            </div>
          </div>
          <div
            className="bg-white rounded p-4 border border-gray-200"
            data-testid="balance-name-value"
          >
            <div className="text-sm text-gray-500">Name Value</div>
            <div className="text-2xl font-bold">
              {formatHns(balances?.nameControlDoos ?? 0)}
            </div>
            <div className="text-xs text-gray-400">HNS · bound to names you control</div>
          </div>
        </div>
      )}

      {/* Reveal alert — names whose bids are in the REVEAL phase need a reveal tx
          or the bid lockup stays stuck. Surfaced prominently so it isn't missed. */}
      {!isWatchOnly &&
        (() => {
          const revealNeeded = names.filter(
            (n) => (n.state ?? "").toUpperCase() === "REVEAL",
          );
          const first = revealNeeded[0];
          if (!first) return null;
          return (
            <div
              className="flex items-center justify-between gap-3 text-sm text-amber-900 bg-amber-50 border border-amber-300 rounded p-3"
              data-testid="reveal-alert"
            >
              <div>
                <strong>Action required: reveal your bid</strong> —{" "}
                {revealNeeded.map((n) => `.${n.name}`).join(", ")}{" "}
                {revealNeeded.length === 1 ? "is" : "are"} in the reveal phase. Reveal
                before the window closes or your locked bid can't be reclaimed.
              </div>
              <Button size="sm" onClick={() => setManageName(first.name)}>
                Reveal
              </Button>
            </div>
          );
        })()}

      {/* Actions */}
      {!isWatchOnly && (
        <div className="space-y-2">
          <div className="flex items-center gap-3">
            <Button
              variant="primary"
              onClick={() => setSendOpen(true)}
              disabled={!canWrite || spendable === 0}
            >
              Send HNS
            </Button>
            {!canWrite && (
              <span className="text-sm text-amber-600">
                {writeCap?.reason ??
                  "Connect a node in Settings, Refresh to sync your coins, then unlock to send."}
              </span>
            )}
          </div>
          {needsNodeSync && (
            <div
              className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2"
              data-testid="needs-node-sync"
            >
              Your balance is read from the explorer, but spending requires a
              synced node. Connect a node in <strong>Settings</strong> and click{" "}
              <strong>Refresh</strong> to load your spendable coins.
            </div>
          )}
        </div>
      )}

      {/* Owned Names (from local name-state cache) */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <div className="flex items-center justify-between mb-2">
          <div className="text-sm text-gray-500">Owned Names ({names.length})</div>
          {!isWatchOnly && (
            <div className="flex items-center gap-2">
              <input
                className="border border-gray-300 rounded px-2 py-1 text-xs"
                value={arbitraryName}
                onChange={(e) => setArbitraryName(e.target.value.toLowerCase())}
                placeholder="name to act on (e.g. open/bid)"
              />
              <Button
                size="sm"
                disabled={!arbitraryName.trim()}
                onClick={() => setManageName(arbitraryName.trim())}
              >
                Name actions
              </Button>
            </div>
          )}
        </div>
        {names.length > 0 ? (
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
                {names.map((n) => (
                  <tr key={n.name} className="border-t border-gray-100">
                    <td className="py-1 font-mono">.{n.name}</td>
                    <td className="py-1">
                      {n.state ? (
                        <Badge variant={auctionPhase(n.state).variant}>
                          {auctionPhase(n.state).label}
                        </Badge>
                      ) : (
                        "—"
                      )}
                    </td>
                    <td className="py-1 text-xs text-gray-500">{n.height ? `#${n.height}` : "—"}</td>
                    <td className="py-1 text-xs text-gray-500">{n.renewal ? `#${n.renewal}` : "—"}</td>
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
            {discoverNames.isPending
              ? "Scanning the explorer for names this wallet owns…"
              : "No owned names found yet. Click Refresh to scan for names this wallet owns."}
          </div>
        )}
      </div>

      {/* Recent drafts */}
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
                    <td className="py-2 pr-4">{d.action}</td>
                    <td className="py-2 pr-4 font-mono">
                      {d.summary ? formatHns(d.summary.sendTotalDoos) : "—"}
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
                            ? `Confirmed · #${d.confirmationHeight}`
                            : "Confirmed"
                          : d.status === "broadcasted"
                          ? "Pending"
                          : d.status === "dropped"
                          ? "Not confirmed"
                          : d.status}
                      </Badge>
                    </td>
                    <td className="py-2 text-xs font-mono truncate max-w-[120px]">
                      {d.txid ? `${d.txid.slice(0, 10)}…` : "—"}
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

      <div className="text-xs text-gray-400">
        Profile: {profile.id.slice(0, 8)}… | Last synced height:{" "}
        {profile.lastSyncedHeight ?? "—"} | xpub: {profile.accountXpub.slice(0, 16)}…
      </div>

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
