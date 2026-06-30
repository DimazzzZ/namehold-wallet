import { useState, useEffect, useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { useUiStore } from "../stores/ui";
import { useActiveProfile } from "../queries/wallet";
import {
  useNamebaseDomainWithdrawals,
  useNamebaseRenewals,
  useWithdrawHns,
  namebaseStatus,
} from "../queries/namebase";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { Badge } from "./ui/Badge";
import { Dialog } from "./ui/Dialog";
import { mapError } from "../lib/errors";
import { formatDate } from "../lib/utils";

/** Whole days from now until an ISO date (negative = already past). */
function daysUntil(iso: string): number | null {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return null;
  return Math.floor((t - Date.now()) / 86_400_000);
}

/** Expiry urgency color, matching the Portfolio Renewals thresholds. */
function expiryColor(days: number | null): string {
  if (days == null) return "text-gray-400";
  if (days < 30) return "text-red-600";
  if (days < 90) return "text-yellow-600";
  return "text-green-600";
}

interface NamebaseDomain {
  name: string;
  owner_id: string;
  owned_since: string;
  auto_renew_active: boolean;
  status: string;
}

interface NamebaseAccount {
  balance: { hns: number; btc: number };
  pendingHns: number;
  has2fa: boolean;
  withdrawalFeeHns: number;
  minimums?: { hns?: number; btc?: number };
}

export function NamebaseDashboard() {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const { data: activeProfile } = useActiveProfile();
  const walletAddress = activeProfile?.receiveAddress ?? null;
  const { data: domainTransfers = [] } = useNamebaseDomainWithdrawals();
  // Latest Namebase transfer status per domain, so the list mirrors Namebase.
  const transferByDomain = useMemo(() => {
    const m = new Map<string, string>();
    for (const t of domainTransfers) {
      if (t.domain && !m.has(t.domain)) m.set(t.domain, t.status);
    }
    return m;
  }, [domainTransfers]);
  const [cookie, setCookie] = useState("");
  const [importing, setImporting] = useState(false);
  const [selectedDomains, setSelectedDomains] = useState<Set<string>>(new Set());
  const [transferTarget, setTransferTarget] = useState<NamebaseDomain | null>(null);
  const [bulkTransferOpen, setBulkTransferOpen] = useState(false);
  const [transferPending, setTransferPending] = useState(false);
  // Editable destination for transfers/withdrawals — defaults to the user's
  // wallet (set when a modal opens) but can be changed to a third-party address.
  const [destInput, setDestInput] = useState("");
  const [withdrawOpen, setWithdrawOpen] = useState(false);
  const [withdrawAmount, setWithdrawAmount] = useState("");
  const withdrawHns = useWithdrawHns();

  const { data: nbStatus, isLoading: statusLoading } = useQuery({
    queryKey: ["namebase", "status"],
    queryFn: () => invoke<{ connected: boolean; has_cookie: boolean; account?: NamebaseAccount; error?: string }>("get_namebase_status"),
    retry: false,
  });

  const { data: domainsData } = useQuery({
    queryKey: ["namebase", "domains"],
    queryFn: () => invoke<{ domains: NamebaseDomain[] }>("fetch_namebase_domains"),
    enabled: nbStatus?.connected === true,
    retry: false,
  });

  const { data: stakedData } = useQuery({
    queryKey: ["namebase", "staked"],
    queryFn: () => invoke<{ stakedDomains: NamebaseDomain[] }>("fetch_namebase_staked"),
    enabled: nbStatus?.connected === true,
    retry: false,
  });

  const isConnected = nbStatus?.connected ?? false;
  const account = nbStatus?.account;
  const domains = domainsData?.domains || [];
  const stakedDomains = stakedData?.stakedDomains || [];

  // Namebase's renewal calendar (soonest-first). Lets the user renew/move a
  // custodial domain before it lapses mid-migration.
  const { data: renewals = [] } = useNamebaseRenewals(isConnected);
  const autoRenewByDomain = useMemo(() => {
    const m = new Map<string, boolean>();
    for (const d of domains) m.set(d.name, d.auto_renew_active);
    return m;
  }, [domains]);

  const handleConnect = async () => {
    if (!cookie.trim()) return;
    try {
      await invoke("connect_namebase", { cookie: cookie.trim() });
      showToast("Connected to Namebase", "success");
      qc.invalidateQueries({ queryKey: ["namebase"] });
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleDisconnect = async () => {
    try {
      await invoke("disconnect_namebase");
      showToast("Disconnected from Namebase", "success");
      qc.invalidateQueries({ queryKey: ["namebase"] });
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleImport = async () => {
    setImporting(true);
    try {
      const result: any = await invoke("import_from_namebase");
      showToast(`Imported ${result.imported} TLDs (${result.staked_count} staked)`, "success");
      qc.invalidateQueries({ queryKey: ["assets"] });
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setImporting(false);
    }
  };

  const toggleDomain = (name: string) => {
    setSelectedDomains((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const toggleAll = () => {
    if (selectedDomains.size === domains.length) {
      setSelectedDomains(new Set());
    } else {
      setSelectedDomains(new Set(domains.map((d) => d.name)));
    }
  };

  const receiveAddress = walletAddress || null;

  // When a transfer/withdraw modal opens, default the destination to the wallet.
  useEffect(() => {
    if (transferTarget || bulkTransferOpen || withdrawOpen) {
      setDestInput(receiveAddress ?? "");
    }
    if (withdrawOpen) setWithdrawAmount("");
  }, [transferTarget, bulkTransferOpen, withdrawOpen, receiveAddress]);

  const dest = destInput.trim();
  const isThirdParty = dest.length > 0 && dest !== receiveAddress;

  // HNS withdrawal amounts are denominated in HNS (Namebase's create endpoint,
  // balance, fee, and minimum are all HNS — NOT dollarydoos). The user enters the
  // NET amount the recipient receives; Namebase's `amount` is the GROSS (debited
  // from balance) with the fee deducted, so we send `net + fee`.
  const availableHns = account?.balance?.hns ?? 0;
  const feeHns = account?.withdrawalFeeHns ?? 0;
  const minHns = account?.minimums?.hns ?? 0;
  const amountNet = parseFloat(withdrawAmount);
  const grossHns = Number.isFinite(amountNet) ? Number((amountNet + feeHns).toFixed(6)) : 0;
  const overBalance = Number.isFinite(amountNet) && amountNet > 0 && grossHns > availableHns;
  const amountValid =
    Number.isFinite(amountNet) &&
    amountNet > 0 &&
    grossHns <= availableHns &&
    grossHns >= minHns;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-bold">Namebase</h2>
        {isConnected && (
          <div className="flex gap-2">
            <Button size="sm" onClick={handleImport} disabled={importing}>
              {importing ? "Importing..." : "Import All TLDs"}
            </Button>
            <Button size="sm" variant="ghost" onClick={handleDisconnect}>
              Logout
            </Button>
          </div>
        )}
      </div>

      {!isConnected ? (
        <div className="bg-white rounded p-6 border border-gray-200 max-w-md">
          <h3 className="text-sm font-semibold mb-3">Connect to Namebase</h3>
          <p className="text-xs text-gray-500 mb-3">
            Paste your session cookie from Namebase browser. Open Namebase → F12 → Network → copy Cookie header.
          </p>
          <Input
            label="Session Cookie"
            type="password"
            value={cookie}
            onChange={(e) => setCookie(e.target.value)}
            placeholder="Paste Namebase session cookie"
          />
          <div className="mt-3">
            <Button
              variant="primary"
              onClick={handleConnect}
              disabled={!cookie.trim() || statusLoading}
            >
              {statusLoading ? "Connecting..." : "Connect"}
            </Button>
          </div>
          {nbStatus?.error && (
            <div className="mt-2 text-sm text-red-600">{nbStatus.error}</div>
          )}
        </div>
      ) : (
        <>
          {/* Account Balance (custodial — held by Namebase, not the on-chain wallet) */}
          <div className="text-xs text-gray-400 mb-1">
            Custodial balance held by Namebase — separate from your on-chain wallet
            (the HNSFans/explorer balance).
          </div>
          <div className="grid grid-cols-3 gap-4">
            <div className="bg-white rounded p-4 border border-gray-200">
              <div className="text-sm text-gray-500">HNS Balance</div>
              <div className="text-2xl font-bold">
                {account?.balance?.hns?.toLocaleString() || "—"}
              </div>
            </div>
            <div className="bg-white rounded p-4 border border-gray-200">
              <div className="text-sm text-gray-500">BTC Balance</div>
              <div className="text-2xl font-bold">
                {account?.balance?.btc || "—"}
              </div>
            </div>
            <div
              className="bg-white rounded p-4 border border-gray-200"
              title="HNS in your Namebase custodial account that Namebase reports as pending/reserved and isn't part of your available balance. Separate from your on-chain wallet (the HNSFans/explorer balance)."
            >
              <div className="text-sm text-gray-500">Pending HNS</div>
              <div className="text-2xl font-bold">
                {account?.pendingHns || "0"}
                <span className="text-base font-normal text-gray-400"> HNS</span>
              </div>
              <div className="text-xs text-gray-400">Reserved on Namebase</div>
            </div>
          </div>

          <div>
            <Button
              variant="primary"
              size="sm"
              onClick={() => setWithdrawOpen(true)}
              disabled={availableHns <= feeHns}
            >
              Withdraw HNS
            </Button>
            {availableHns <= feeHns && (
              <span className="ml-2 text-xs text-gray-400">
                Not enough balance to withdraw.
              </span>
            )}
          </div>

          {/* Domain Summary */}
          <div className="grid grid-cols-2 gap-4">
            <div className="bg-white rounded p-4 border border-gray-200">
              <div className="text-sm text-gray-500">Total Domains</div>
              <div className="text-2xl font-bold">{domains.length}</div>
            </div>
            <div className="bg-white rounded p-4 border border-gray-200">
              <div className="text-sm text-gray-500">Staked Domains</div>
              <div className="text-2xl font-bold text-purple-700">{stakedDomains.length}</div>
            </div>
          </div>

          {/* Expiring soon — Namebase renewal calendar */}
          {renewals.length > 0 && (
            <div
              className="bg-white rounded p-4 border border-gray-200"
              data-testid="namebase-expiring"
            >
              <h3 className="text-sm font-semibold mb-1">Expiring soon ({renewals.length})</h3>
              <p className="text-xs text-gray-500 mb-3">
                These custodial domains expire soonest — renew on Namebase or move them
                out before they lapse. Names with auto-renew <strong>off</strong> are the
                highest risk.
              </p>
              <div className="max-h-72 overflow-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="text-left text-gray-500 border-b">
                      <th className="px-2 py-1">Name</th>
                      <th className="px-2 py-1">Expires</th>
                      <th className="px-2 py-1">Block</th>
                      <th className="px-2 py-1">Auto-renew</th>
                    </tr>
                  </thead>
                  <tbody>
                    {renewals.map((r) => {
                      const days = daysUntil(r.estimated_date);
                      const autoRenew = autoRenewByDomain.get(r.domain);
                      return (
                        <tr key={r.domain} className="border-t border-gray-100">
                          <td className="px-2 py-1 font-mono">.{r.domain}</td>
                          <td className={`px-2 py-1 ${expiryColor(days)}`}>
                            {formatDate(r.estimated_date)}
                            {days != null && (
                              <span className="text-xs text-gray-400">
                                {" "}
                                ({days <= 0 ? "now" : `${days}d`})
                              </span>
                            )}
                          </td>
                          <td className="px-2 py-1 text-xs text-gray-500">#{r.expire_block}</td>
                          <td className="px-2 py-1">
                            {autoRenew === false ? (
                              <Badge variant="error">Off</Badge>
                            ) : autoRenew ? (
                              <Badge variant="success">On</Badge>
                            ) : (
                              <span className="text-xs text-gray-400">—</span>
                            )}
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {/* Bulk actions — acts on the Your Domains selection, so it sits directly
              above that table (not above the Expiring-soon panel). */}
          {selectedDomains.size > 0 && (
            <div className="flex items-center gap-3 bg-blue-50 border border-blue-200 rounded px-3 py-2">
              <span className="text-sm text-blue-700">{selectedDomains.size} selected</span>
              <Button size="sm" variant="primary" onClick={() => setBulkTransferOpen(true)}>
                Transfer Selected
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setSelectedDomains(new Set())}>
                Clear
              </Button>
            </div>
          )}

          {/* Domain List */}
          {domains.length > 0 && (
            <div className="bg-white rounded p-4 border border-gray-200">
              <h3 className="text-sm font-semibold mb-3">Your Domains ({domains.length})</h3>
              <div className="max-h-96 overflow-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="text-left text-gray-500 border-b">
                      <th className="px-2 py-1 w-8">
                        <input
                          type="checkbox"
                          checked={selectedDomains.size === domains.length && domains.length > 0}
                          onChange={toggleAll}
                        />
                      </th>
                      <th className="px-2 py-1">Name</th>
                      <th className="px-2 py-1">Status</th>
                      <th className="px-2 py-1">Auto-Renew</th>
                      <th className="px-2 py-1">Owned Since</th>
                      <th className="px-2 py-1 w-20">Action</th>
                    </tr>
                  </thead>
                  <tbody>
                    {domains.map((d) => {
                      const isStaked = stakedDomains.some((s) => s.name === d.name);
                      return (
                        <tr key={d.name} className="border-t border-gray-100">
                          <td className="px-2 py-1">
                            <input
                              type="checkbox"
                              checked={selectedDomains.has(d.name)}
                              onChange={() => toggleDomain(d.name)}
                            />
                          </td>
                          <td className="px-2 py-1 font-mono">.{d.name}</td>
                          <td className="px-2 py-1">
                            {isStaked ? (
                              <Badge variant="warning">Staked</Badge>
                            ) : (
                              <Badge>{d.status}</Badge>
                            )}
                          </td>
                          <td className="px-2 py-1">{d.auto_renew_active ? "Yes" : "No"}</td>
                          <td className="px-2 py-1 text-xs text-gray-400">{d.owned_since?.slice(0, 10)}</td>
                          <td className="px-2 py-1">
                            {transferByDomain.has(d.name) ? (
                              (() => {
                                // Show Namebase's own live status for the transfer.
                                const status = transferByDomain.get(d.name)!;
                                const { label, tone } = namebaseStatus(status);
                                return (
                                  <Badge variant={tone} title={status}>{label}</Badge>
                                );
                              })()
                            ) : (
                              <Button
                                size="sm"
                                variant="secondary"
                                onClick={() => setTransferTarget(d)}
                              >
                                Transfer
                              </Button>
                            )}
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </>
      )}

      {/* Single Transfer Modal */}
      <Dialog
        open={!!transferTarget}
        onClose={() => setTransferTarget(null)}
        title={`Transfer .${transferTarget?.name || ""}`}
      >
        <div className="space-y-3">
          <p className="text-sm text-gray-600">
            Transfer <strong>.{transferTarget?.name}</strong> from Namebase to an HNS address.
          </p>
          <div>
            <Input
              label="Destination address"
              value={destInput}
              onChange={(e) => setDestInput(e.target.value)}
              placeholder="hs1…"
            />
            <div className="mt-1 flex items-center justify-between text-xs text-gray-500">
              <span>Defaults to your wallet — change it to transfer to a third party.</span>
              {receiveAddress && dest !== receiveAddress && (
                <button
                  type="button"
                  className="text-blue-600 hover:underline"
                  onClick={() => setDestInput(receiveAddress)}
                >
                  Use my wallet
                </button>
              )}
            </div>
          </div>
          {isThirdParty && (
            <div className="bg-amber-50 border border-amber-300 rounded p-2 text-xs text-amber-800">
              Transferring to an address <strong>outside this wallet</strong>. Double-check
              it — Namebase withdrawals are irreversible.
            </div>
          )}
          <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
            This will initiate a transfer on Namebase. The domain will appear at the
            destination after blockchain confirmation.
          </div>
          <div className="flex gap-2 justify-end">
            <Button variant="ghost" onClick={() => setTransferTarget(null)}>Cancel</Button>
            <Button
              variant="primary"
              disabled={!dest || transferPending}
              onClick={async () => {
                if (!transferTarget || !dest) return;
                setTransferPending(true);
                try {
                  await invoke("namebase_transfer_domain", { name: transferTarget.name, address: dest });
                  showToast(`Transfer initiated for .${transferTarget.name}`, "success");
                  setTransferTarget(null);
                  qc.invalidateQueries({ queryKey: ["namebase-withdrawals"] });
                  qc.invalidateQueries({ queryKey: ["namebase-domain-withdrawals"] });
                } catch (e) {
                  showToast(mapError(e), "error");
                } finally {
                  setTransferPending(false);
                }
              }}
            >
              {transferPending ? "Transferring..." : "Confirm Transfer"}
            </Button>
          </div>
        </div>
      </Dialog>

      {/* Bulk Transfer Modal */}
      <Dialog
        open={bulkTransferOpen}
        onClose={() => setBulkTransferOpen(false)}
        title={`Transfer ${selectedDomains.size} domains`}
      >
        <div className="space-y-3">
          <p className="text-sm text-gray-600">
            Transfer <strong>{selectedDomains.size}</strong> domains from Namebase to an HNS address.
          </p>
          <div className="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded p-2">
            ⚠️ Beta — transfers are irreversible. Move <strong>one</strong> domain first
            and confirm it arrives before transferring the rest.
          </div>
          <div>
            <Input
              label="Destination address"
              value={destInput}
              onChange={(e) => setDestInput(e.target.value)}
              placeholder="hs1…"
            />
            <div className="mt-1 flex items-center justify-between text-xs text-gray-500">
              <span>All selected domains go to this address. Defaults to your wallet.</span>
              {receiveAddress && dest !== receiveAddress && (
                <button
                  type="button"
                  className="text-blue-600 hover:underline"
                  onClick={() => setDestInput(receiveAddress)}
                >
                  Use my wallet
                </button>
              )}
            </div>
          </div>
          {isThirdParty && (
            <div className="bg-amber-50 border border-amber-300 rounded p-2 text-xs text-amber-800">
              Transferring to an address <strong>outside this wallet</strong>. Double-check
              it — Namebase withdrawals are irreversible.
            </div>
          )}
          <div className="text-sm text-gray-600">
            <p className="mb-2"><strong>Selected domains ({selectedDomains.size}):</strong></p>
            <div className="max-h-32 overflow-auto bg-gray-50 rounded p-2 text-xs font-mono">
              {Array.from(selectedDomains).map((name) => `.${name}`).join(", ")}
            </div>
          </div>
          <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
            Each domain transfer will be initiated on Namebase. Domains will appear at the
            destination after blockchain confirmation.
          </div>
          <div className="flex gap-2 justify-end">
            <Button variant="ghost" onClick={() => setBulkTransferOpen(false)}>Cancel</Button>
            <Button
              variant="primary"
              disabled={!dest || transferPending}
              onClick={async () => {
                if (!dest) return;
                setTransferPending(true);
                let successCount = 0;
                let failCount = 0;
                for (const name of selectedDomains) {
                  try {
                    await invoke("namebase_transfer_domain", { name, address: dest });
                    successCount++;
                  } catch {
                    failCount++;
                  }
                }
                showToast(
                  `Transferred ${successCount} domains${failCount > 0 ? `, ${failCount} failed` : ""}`,
                  failCount > 0 ? "error" : "success",
                );
                setBulkTransferOpen(false);
                setSelectedDomains(new Set());
                setTransferPending(false);
                qc.invalidateQueries({ queryKey: ["namebase-withdrawals"] });
                qc.invalidateQueries({ queryKey: ["namebase-domain-withdrawals"] });
              }}
            >
              {transferPending ? "Transferring..." : `Transfer ${selectedDomains.size} Domains`}
            </Button>
          </div>
        </div>
      </Dialog>

      {/* Withdraw HNS Modal */}
      <Dialog open={withdrawOpen} onClose={() => setWithdrawOpen(false)} title="Withdraw HNS">
        <div className="space-y-3">
          <p className="text-sm text-gray-600">
            Withdraw HNS from your Namebase balance to an HNS address.
          </p>
          <div>
            <Input
              label="Amount to send (HNS)"
              type="number"
              step="0.000001"
              value={withdrawAmount}
              onChange={(e) => setWithdrawAmount(e.target.value)}
              placeholder="0.0"
            />
            <div className="mt-1 flex items-center justify-between text-xs text-gray-500">
              <span>
                The recipient receives this amount; the {feeHns} HNS network fee is added
                on top. Available: {availableHns.toLocaleString()} HNS.
              </span>
              <button
                type="button"
                className="text-blue-600 hover:underline"
                onClick={() =>
                  setWithdrawAmount(String(Number(Math.max(availableHns - feeHns, 0).toFixed(6))))
                }
              >
                Max
              </button>
            </div>
          </div>

          {/* Fee breakdown — make the fee explicit. */}
          {Number.isFinite(amountNet) && amountNet > 0 && (
            <div className="bg-gray-50 border border-gray-200 rounded p-3 text-sm space-y-1">
              <div className="flex justify-between text-gray-600">
                <span>Send to recipient</span>
                <span className="font-mono">{amountNet} HNS</span>
              </div>
              <div className="flex justify-between text-gray-600">
                <span>Network fee</span>
                <span className="font-mono">+ {feeHns} HNS</span>
              </div>
              <div className="flex justify-between font-semibold text-gray-900 border-t border-gray-200 pt-1">
                <span>Total debited</span>
                <span className="font-mono">{grossHns} HNS</span>
              </div>
            </div>
          )}
          {overBalance && (
            <div className="bg-red-50 border border-red-300 rounded p-2 text-xs text-red-800">
              Not enough balance — need {grossHns} HNS including the {feeHns} HNS fee
              (available {availableHns.toLocaleString()} HNS).
            </div>
          )}
          <div>
            <Input
              label="Destination address"
              value={destInput}
              onChange={(e) => setDestInput(e.target.value)}
              placeholder="hs1…"
            />
            <div className="mt-1 flex items-center justify-between text-xs text-gray-500">
              <span>Defaults to your wallet — change it to withdraw to a third party.</span>
              {receiveAddress && dest !== receiveAddress && (
                <button
                  type="button"
                  className="text-blue-600 hover:underline"
                  onClick={() => setDestInput(receiveAddress)}
                >
                  Use my wallet
                </button>
              )}
            </div>
          </div>
          {isThirdParty && (
            <div className="bg-amber-50 border border-amber-300 rounded p-2 text-xs text-amber-800">
              Withdrawing to an address <strong>outside this wallet</strong>. Double-check
              it — Namebase withdrawals are irreversible.
            </div>
          )}
          <div className="flex gap-2 justify-end">
            <Button variant="ghost" onClick={() => setWithdrawOpen(false)}>Cancel</Button>
            <Button
              variant="primary"
              disabled={!amountValid || !dest || withdrawHns.isPending}
              onClick={async () => {
                if (!amountValid || !dest) return;
                try {
                  // Namebase's `amount` is the GROSS (HNS) debited; recipient gets
                  // gross − fee. The user entered the net, so send net + fee.
                  await withdrawHns.mutateAsync({ address: dest, amount: String(grossHns) });
                  showToast(
                    `Withdrawal of ${amountNet} HNS requested (+${feeHns} HNS fee)`,
                    "success",
                  );
                  setWithdrawOpen(false);
                } catch (e) {
                  showToast(mapError(e), "error");
                }
              }}
            >
              {withdrawHns.isPending ? "Requesting…" : "Withdraw"}
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}
