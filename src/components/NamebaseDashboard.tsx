import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { useUiStore } from "../stores/ui";
import { useWalletAddress } from "../queries/wallet";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { Badge } from "./ui/Badge";
import { Dialog } from "./ui/Dialog";
import { mapError } from "../lib/errors";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

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
}

export function NamebaseDashboard() {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const { data: walletAddress } = useWalletAddress();
  const [cookie, setCookie] = useState("");
  const [importing, setImporting] = useState(false);
  const [selectedDomains, setSelectedDomains] = useState<Set<string>>(new Set());
  const [transferTarget, setTransferTarget] = useState<NamebaseDomain | null>(null);
  const [bulkTransferOpen, setBulkTransferOpen] = useState(false);
  const [transferPending, setTransferPending] = useState(false);

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
  const destAddress = receiveAddress || "Connect wallet to get address";

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
          {/* Account Balance */}
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
            <div className="bg-white rounded p-4 border border-gray-200">
              <div className="text-sm text-gray-500">Pending HNS</div>
              <div className="text-2xl font-bold">
                {account?.pendingHns || "0"}
              </div>
            </div>
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

          {/* Bulk Actions */}
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
                            <Button
                              size="sm"
                              variant="secondary"
                              onClick={() => setTransferTarget(d)}
                            >
                              Transfer
                            </Button>
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
            Transfer <strong>.{transferTarget?.name}</strong> from Namebase to your wallet.
          </p>
          <div>
            <label className="text-sm font-medium text-gray-700">Destination address</label>
            <div className="mt-1 flex items-center gap-2">
              <code className="flex-1 text-sm bg-gray-50 p-2 rounded font-mono truncate">
                {destAddress}
              </code>
              <Button
                size="sm"
                onClick={async () => {
                  if (receiveAddress) {
                    await writeText(receiveAddress);
                    showToast("Address copied", "success");
                  }
                }}
                disabled={!receiveAddress}
              >
                Copy
              </Button>
            </div>
          </div>
          <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
            This will initiate a transfer on Namebase. The domain will appear in your wallet after blockchain confirmation.
          </div>
          <div className="flex gap-2 justify-end">
            <Button variant="ghost" onClick={() => setTransferTarget(null)}>Cancel</Button>
            <Button
              variant="primary"
              disabled={!receiveAddress || transferPending}
              onClick={async () => {
                if (!transferTarget || !receiveAddress) return;
                setTransferPending(true);
                try {
                  await invoke("namebase_transfer_domain", { name: transferTarget.name, address: receiveAddress });
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
            Transfer <strong>{selectedDomains.size}</strong> domains from Namebase to your wallet.
          </p>
          <div>
            <label className="text-sm font-medium text-gray-700">Destination address</label>
            <div className="mt-1 font-mono text-sm bg-gray-50 p-2 rounded truncate">{destAddress}</div>
          </div>
          <div className="text-sm text-gray-600">
            <p className="mb-2"><strong>Selected domains ({selectedDomains.size}):</strong></p>
            <div className="max-h-32 overflow-auto bg-gray-50 rounded p-2 text-xs font-mono">
              {Array.from(selectedDomains).map((name) => `.${name}`).join(", ")}
            </div>
          </div>
          <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
            Each domain transfer will be initiated on Namebase. Domains will appear in your wallet after blockchain confirmation.
          </div>
          <div className="flex gap-2 justify-end">
            <Button variant="ghost" onClick={() => setBulkTransferOpen(false)}>Cancel</Button>
            <Button
              variant="primary"
              disabled={!receiveAddress || transferPending}
              onClick={async () => {
                if (!receiveAddress) return;
                setTransferPending(true);
                let successCount = 0;
                let failCount = 0;
                for (const name of selectedDomains) {
                  try {
                    await invoke("namebase_transfer_domain", { name, address: receiveAddress });
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
    </div>
  );
}
