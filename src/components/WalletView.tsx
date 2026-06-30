import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useWalletConnection, useWalletBalance, useWalletAddress, useWalletNames, useWalletTransactions, useWalletList, useSendHns } from "../queries/wallet";
import { useSettingsStore } from "../stores/settings";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { Input } from "./ui/Input";
import { Dialog } from "./ui/Dialog";
import { PageHeader } from "./ui/PageHeader";
import { formatHns, hnsToDollarydoos, formatDate } from "../lib/utils";
import { mapError } from "../lib/errors";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useUiStore } from "../stores/ui";
import { QRCodeSVG } from "qrcode.react";
import { invoke } from "../lib/invoke";

interface ParsedTx {
  hash: string;
  type: "send" | "receive" | "other";
  amount: number;
  address: string;
  confirmed: boolean;
  height: number | null;
  date: string | null;
}

function parseTransactions(txs: unknown[]): ParsedTx[] {
  if (!Array.isArray(txs)) return [];
  return txs.map((tx: any) => {
    const hash = tx.hash || tx.tx?.hash || "";
    const confirmed = (tx.confirmations || 0) > 0;
    const height = tx.height > 0 ? tx.height : null;
    const date = tx.date && tx.date !== "1970-01-01T00:00:00Z" ? tx.date : tx.mdate || null;

    // Determine type and amount from outputs
    let type: "send" | "receive" | "other" = "other";
    let amount = 0;
    let address = "";

    if (tx.outputs && Array.isArray(tx.outputs)) {
      const hasPath = tx.outputs.some((o: any) => o.path && !o.path.change);

      if (hasPath) {
        type = "receive";
        const receiveOutput = tx.outputs.find((o: any) => o.path && !o.path.change);
        amount = receiveOutput?.value || 0;
        address = receiveOutput?.address || "";
      } else if (tx.inputs && Array.isArray(tx.inputs)) {
        type = "send";
        const sendOutput = tx.outputs.find((o: any) => !o.path?.change);
        amount = sendOutput?.value || 0;
        address = sendOutput?.address || "";
      }
    }

    // Fallback: use fee to determine direction
    if (type === "other" && tx.fee) {
      amount = tx.fee;
    }

    return { hash, type, amount, address, confirmed, height, date };
  });
}

export function WalletView() {
  const qc = useQueryClient();
  const { data: conn, isLoading: connLoading } = useWalletConnection();
  const { data: balance, isLoading: balLoading } = useWalletBalance();
  const { data: address } = useWalletAddress();
  const { data: names } = useWalletNames();
  const { data: rawTransactions } = useWalletTransactions();
  const { data: walletList } = useWalletList();
  const settings = useSettingsStore((s) => s.settings);
  const updateSetting = useSettingsStore((s) => s.update);
  const passphrase = useSettingsStore((s) => s.passphrase);
  const showToast = useUiStore((s) => s.showToast);
  const sendHns = useSendHns();

  const [sendDialogOpen, setSendDialogOpen] = useState(false);
  const [sendAddress, setSendAddress] = useState("");
  const [sendAmount, setSendAmount] = useState("");
  const [sendPassphrase, setSendPassphrase] = useState("");
  const [switchDialogOpen, setSwitchDialogOpen] = useState(false);
  const [newWalletId, setNewWalletId] = useState("");
  const [copied, setCopied] = useState(false);

  // Create wallet state
  const [createOpen, setCreateOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newPassphrase, setNewPassphrase] = useState("");
  const [creating, setCreating] = useState(false);
  const [createdMnemonic, setCreatedMnemonic] = useState("");
  const [confirmedSaved, setConfirmedSaved] = useState(false);

  // Import wallet state
  const [importOpen, setImportOpen] = useState(false);
  const [importName, setImportName] = useState("");
  const [importPassphrase, setImportPassphrase] = useState("");
  const [importMnemonic, setImportMnemonic] = useState("");
  const [importing, setImporting] = useState(false);

  const walletUrl = settings?.hsd_wallet_api_url || "";
  const isLocalhost = walletUrl.includes("127.0.0.1") || walletUrl.includes("localhost");
  const writeMode = settings?.write_mode === "true";
  const currentWalletId = settings?.hsd_wallet_id || "primary";

  const transactions = parseTransactions(rawTransactions || []);

  const handleCopyAddress = async () => {
    if (!address) return;
    await writeText(address);
    setCopied(true);
    showToast("Address copied", "success");
    setTimeout(() => setCopied(false), 2000);
  };

  const handleSendHns = async () => {
    if (!sendAddress.trim() || !sendAmount.trim()) return;
    const dollarydoos = hnsToDollarydoos(sendAmount);
    if (isNaN(dollarydoos) || dollarydoos <= 0) {
      showToast("Invalid amount", "error");
      return;
    }
    const pw = sendPassphrase || passphrase;
    if (!pw) {
      showToast("Enter wallet passphrase in Settings or below", "error");
      return;
    }
    try {
      await sendHns.mutateAsync({ address: sendAddress, value: dollarydoos, passphrase: pw });
      showToast(`Sent ${sendAmount} HNS to ${sendAddress}`, "success");
      setSendDialogOpen(false);
      setSendAddress("");
      setSendAmount("");
      setSendPassphrase("");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleRefresh = () => {
    qc.invalidateQueries({ queryKey: ["wallet"] });
  };

  const handleSwitchWallet = async (walletId: string) => {
    if (!walletId.trim() || walletId === currentWalletId) return;
    try {
      await updateSetting("hsd_wallet_id", walletId.trim());
      qc.invalidateQueries({ queryKey: ["wallet"] });
      showToast(`Switched to wallet "${walletId.trim()}"`, "success");
      setSwitchDialogOpen(false);
      setNewWalletId("");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleDisconnect = async () => {
    try {
      await updateSetting("hsd_wallet_id", "");
      qc.invalidateQueries({ queryKey: ["wallet"] });
      showToast("Disconnected", "success");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const getUniqueName = (base: string) => {
    if (!walletList || !walletList.includes(base)) return base;
    let i = 2;
    while (walletList.includes(`${base}-${i}`)) i++;
    return `${base}-${i}`;
  };

  const handleCreate = async () => {
    const name = newName.trim() || getUniqueName("wallet");
    setCreating(true);
    try {
      const result: any = await invoke("create_wallet", { id: name, passphrase: newPassphrase });
      setCreatedMnemonic(result?.mnemonic?.phrase || "");
      setNewName(name);
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setCreating(false);
    }
  };

  const handleConfirmCreated = async () => {
    await updateSetting("hsd_wallet_id", newName.trim());
    qc.invalidateQueries({ queryKey: ["wallet"] });
    setCreateOpen(false);
    setCreatedMnemonic("");
    setNewName("");
    setNewPassphrase("");
    setConfirmedSaved(false);
    showToast(`Wallet "${newName.trim()}" created`, "success");
  };

  const handleImport = async () => {
    if (!importMnemonic.trim()) {
      showToast("Enter your seed phrase", "error");
      return;
    }
    const name = importName.trim() || getUniqueName("wallet");
    setImporting(true);
    try {
      await invoke("create_wallet", { id: name, passphrase: importPassphrase, mnemonic: importMnemonic.trim() });
      await updateSetting("hsd_wallet_id", name);
      qc.invalidateQueries({ queryKey: ["wallet"] });
      setImportOpen(false);
      setImportName("");
      setImportPassphrase("");
      setImportMnemonic("");
      showToast(`Wallet "${name}" imported`, "success");
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Wallet"
        subtitle="Manage your Handshake wallet, balance, names, and transactions."
        badges={
          <>
            <Badge variant="info">{currentWalletId}</Badge>
            {conn?.connected ? (
              <Badge variant="success">Connected</Badge>
            ) : connLoading ? (
              <Badge>Checking...</Badge>
            ) : (
              <Badge variant="error">Disconnected</Badge>
            )}
          </>
        }
        actions={[
          { label: "Create Wallet", onClick: () => setCreateOpen(true) },
          { label: "Import Wallet", variant: "secondary", onClick: () => setImportOpen(true) },
          { label: "Switch Wallet", variant: "secondary", onClick: () => setSwitchDialogOpen(true) },
          { label: "Refresh", onClick: handleRefresh },
        ]}
      />

      {!isLocalhost && (
        <div className="bg-red-50 border border-red-200 rounded p-3 text-sm text-red-700">
          Warning: Wallet API URL ({walletUrl}) is not localhost. Only use local connections for security.
        </div>
      )}

      {!conn?.connected && !connLoading && (
        <div className="bg-yellow-50 border border-yellow-200 rounded p-3 text-sm text-yellow-800">
          Wallet API not available. Make sure hsd is running with wallet plugin enabled (without <code>--no-wallet</code> flag).
          {!walletList?.length && (
            <span className="block mt-1">No wallets found. Create or import a wallet below.</span>
          )}
        </div>
      )}

      {/* Receive Address - Prominent */}
      <div className="bg-white rounded-lg p-6 border-2 border-blue-200">
        <div className="text-sm text-gray-500 mb-2">Receive Address</div>
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
          <div className="text-gray-400">
            {conn?.connected ? "Loading address..." : "Connect to wallet first"}
          </div>
        )}
      </div>

      {/* Balance */}
      <div className="grid grid-cols-3 gap-4">
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Confirmed Balance</div>
          <div className="text-2xl font-bold">
            {balLoading ? "..." : formatHns(balance?.confirmed)}
          </div>
          <div className="text-xs text-gray-400">HNS</div>
        </div>
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Unconfirmed</div>
          <div className="text-2xl font-bold">
            {balLoading ? "..." : formatHns(balance?.unconfirmed)}
          </div>
          <div className="text-xs text-gray-400">HNS</div>
        </div>
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Locked</div>
          <div className="text-2xl font-bold">
            {balLoading
              ? "..."
              : formatHns(
                  (balance?.locked_confirmed || 0) + (balance?.locked_unconfirmed || 0),
                )}
          </div>
          <div className="text-xs text-gray-400">HNS</div>
        </div>
      </div>

      {/* Actions */}
      <div className="flex gap-3">
        {writeMode && (
          <Button variant="primary" onClick={() => setSendDialogOpen(true)}>
            Send HNS
          </Button>
        )}
      </div>

      {/* Owned Names */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <div className="text-sm text-gray-500 mb-2">
          Owned Names ({names?.length ?? 0})
        </div>
        {names && names.length > 0 ? (
          <div className="max-h-60 overflow-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500">
                  <th className="py-1">Name</th>
                  <th className="py-1">State</th>
                  <th className="py-1">Height</th>
                  <th className="py-1">Renewal</th>
                  <th className="py-1">Expires</th>
                </tr>
              </thead>
              <tbody>
                {names.map((n) => (
                  <tr key={n.name} className="border-t border-gray-100">
                    <td className="py-1 font-mono">.{n.name}</td>
                    <td className="py-1">{n.state || "—"}</td>
                    <td className="py-1 text-xs text-gray-500">{n.height ? `#${n.height}` : "—"}</td>
                    <td className="py-1 text-xs text-gray-500">{n.renewal ? `#${n.renewal}` : "—"}</td>
                    <td className="py-1">
                      {n.stats?.days_until_expire
                        ? `${Math.round(n.stats.days_until_expire)}d`
                        : "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <div className="text-gray-400 text-sm py-4 text-center">
            {conn?.connected
              ? "No names in wallet yet. Names will appear here after transfer."
              : "Connect to wallet first"}
          </div>
        )}
      </div>

      {/* Transaction History */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <div className="text-sm text-gray-500 mb-2">
          Transaction History ({transactions.length})
        </div>
        {transactions.length > 0 ? (
          <div className="max-h-80 overflow-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="py-2 pr-4">Date</th>
                  <th className="py-2 pr-4">Type</th>
                  <th className="py-2 pr-4">Amount</th>
                  <th className="py-2 pr-4">Address</th>
                  <th className="py-2 pr-4">Status</th>
                  <th className="py-2">Hash</th>
                </tr>
              </thead>
              <tbody>
                {transactions.map((tx, i) => (
                  <tr key={tx.hash || i} className="border-t border-gray-100">
                    <td className="py-2 pr-4 text-xs text-gray-500">
                      {tx.date ? formatDate(tx.date) : "—"}
                    </td>
                    <td className="py-2 pr-4">
                      <Badge variant={tx.type === "receive" ? "success" : tx.type === "send" ? "warning" : "default"}>
                        {tx.type}
                      </Badge>
                    </td>
                    <td className="py-2 pr-4 font-mono">
                      {tx.amount > 0 ? formatHns(tx.amount) : "—"}
                    </td>
                    <td className="py-2 pr-4 text-xs font-mono truncate max-w-[120px]">
                      {tx.address || "—"}
                    </td>
                    <td className="py-2 pr-4">
                      <Badge variant={tx.confirmed ? "success" : "warning"}>
                        {tx.confirmed ? "Confirmed" : "Pending"}
                      </Badge>
                    </td>
                    <td className="py-2 text-xs font-mono truncate max-w-[100px]">
                      {tx.hash ? `${tx.hash.slice(0, 8)}...` : "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <div className="text-gray-400 text-sm py-4 text-center">
            {conn?.connected
              ? "No transactions yet. Transactions will appear here after sending or receiving."
              : "Connect to wallet first"}
          </div>
        )}
      </div>

      {/* Wallet info */}
      <div className="text-xs text-gray-400">
        Wallet ID: {currentWalletId} | Network: {settings?.hsd_network || "—"} | API: {walletUrl}
      </div>

      {/* Send HNS Dialog */}
      <Dialog open={sendDialogOpen} onClose={() => setSendDialogOpen(false)} title="Send HNS">
        <div className="space-y-3">
          <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
            This will send real HNS from your wallet. This action cannot be undone.
          </div>
          <Input
            label="Destination Address"
            value={sendAddress}
            onChange={(e) => setSendAddress(e.target.value)}
            placeholder="hs1q..."
          />
          <Input
            label="Amount (HNS)"
            value={sendAmount}
            onChange={(e) => setSendAmount(e.target.value)}
            placeholder="1.0"
            type="number"
            step="0.000001"
          />
          <Input
            label="Wallet Passphrase"
            type="password"
            value={sendPassphrase}
            onChange={(e) => setSendPassphrase(e.target.value)}
            placeholder={passphrase ? "Using saved passphrase" : "Enter passphrase"}
          />
          {sendAmount && sendAddress && (
            <div className="bg-gray-50 rounded p-3 text-sm">
              <div className="flex justify-between">
                <span>Amount:</span>
                <span className="font-mono">{sendAmount} HNS</span>
              </div>
              <div className="flex justify-between text-gray-500">
                <span>To:</span>
                <span className="font-mono truncate max-w-[200px]">{sendAddress}</span>
              </div>
            </div>
          )}
          <div className="flex gap-2 justify-end">
            <Button variant="ghost" onClick={() => setSendDialogOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="danger"
              onClick={handleSendHns}
              disabled={!sendAddress.trim() || !sendAmount.trim() || sendHns.isPending}
            >
              {sendHns.isPending ? "Sending..." : "Send HNS"}
            </Button>
          </div>
        </div>
      </Dialog>

      {/* Switch Wallet Dialog */}
      <Dialog open={switchDialogOpen} onClose={() => setSwitchDialogOpen(false)} title="Switch Wallet">
        <div className="space-y-3">
          <div className="text-sm text-gray-600">
            Current wallet: <strong>{currentWalletId}</strong>
          </div>

          {walletList && walletList.length > 0 && (
            <div>
              <label className="text-sm font-medium text-gray-700">Available Wallets</label>
              <div className="mt-1 border border-gray-300 rounded max-h-40 overflow-auto">
                {walletList.map((wid) => (
                  <div
                    key={wid}
                    className={`px-3 py-2 text-sm cursor-pointer hover:bg-gray-50 flex items-center justify-between ${wid === currentWalletId ? "bg-blue-50" : ""}`}
                    onClick={() => handleSwitchWallet(wid)}
                  >
                    <span className="font-mono">{wid}</span>
                    {wid === currentWalletId && <Badge variant="success">Current</Badge>}
                  </div>
                ))}
              </div>
            </div>
          )}

          <div className="border-t pt-3">
            <Input
              label="Or enter wallet ID"
              value={newWalletId}
              onChange={(e) => setNewWalletId(e.target.value)}
              placeholder="e.g. my-wallet"
            />
            <div className="flex gap-2 justify-end mt-2">
              <Button
                size="sm"
                onClick={() => handleSwitchWallet(newWalletId)}
                disabled={!newWalletId.trim() || newWalletId === currentWalletId}
              >
                Switch
              </Button>
            </div>
          </div>

          <div className="border-t pt-3 flex justify-between">
            <Button variant="ghost" size="sm" onClick={handleDisconnect}>
              Disconnect
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setSwitchDialogOpen(false)}>
              Cancel
            </Button>
          </div>
        </div>
      </Dialog>

      {/* Create Wallet Dialog */}
      <Dialog open={createOpen} onClose={() => { setCreateOpen(false); setCreatedMnemonic(""); }} title="Create Wallet">
        {createdMnemonic ? (
          <div className="space-y-3">
            <div className="bg-yellow-50 border border-yellow-200 rounded p-3 text-sm text-yellow-800">
              Write down these 24 words. This is the ONLY way to recover your wallet.
            </div>
            <div className="bg-gray-50 rounded p-4 font-mono text-sm break-all">{createdMnemonic}</div>
            <div className="flex items-center gap-2">
              <input type="checkbox" id="confirmed-save" checked={confirmedSaved} onChange={(e) => setConfirmedSaved(e.target.checked)} />
              <label htmlFor="confirmed-save" className="text-sm">I have saved my seed phrase</label>
            </div>
            <Button variant="primary" className="w-full" disabled={!confirmedSaved} onClick={handleConfirmCreated}>Done</Button>
          </div>
        ) : (
          <div className="space-y-3">
            <Input label="Wallet Name" value={newName} onChange={(e) => setNewName(e.target.value)} placeholder={getUniqueName("wallet")} />
            <Input label="Passphrase (optional)" type="password" value={newPassphrase} onChange={(e) => setNewPassphrase(e.target.value)} placeholder="Optional passphrase" />
            <div className="flex gap-2 justify-end">
              <Button variant="ghost" onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button variant="primary" onClick={handleCreate} disabled={creating}>
                {creating ? "Creating..." : "Create"}
              </Button>
            </div>
          </div>
        )}
      </Dialog>

      {/* Import Wallet Dialog */}
      <Dialog open={importOpen} onClose={() => setImportOpen(false)} title="Import Wallet">
        <div className="space-y-3">
          <Input label="Wallet Name" value={importName} onChange={(e) => setImportName(e.target.value)} placeholder={getUniqueName("wallet")} />
          <Input label="Passphrase (optional)" type="password" value={importPassphrase} onChange={(e) => setImportPassphrase(e.target.value)} placeholder="Leave empty if original wallet had no passphrase" />
          <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
            The passphrase affects address derivation. If your original wallet had no passphrase, leave this empty.
          </div>
          <div className="flex flex-col gap-1">
            <label className="text-sm font-medium text-gray-700">Seed Phrase (24 words)</label>
            <textarea
              className="border border-gray-300 rounded px-3 py-2 text-sm h-24 resize-none focus:outline-none focus:ring-2 focus:ring-blue-500"
              value={importMnemonic}
              onChange={(e) => setImportMnemonic(e.target.value)}
              placeholder="word1 word2 word3 ... word24"
            />
          </div>
          <div className="flex gap-2 justify-end">
            <Button variant="ghost" onClick={() => setImportOpen(false)}>Cancel</Button>
            <Button variant="primary" onClick={handleImport} disabled={!importMnemonic.trim() || importing}>
              {importing ? "Importing..." : "Import"}
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}
