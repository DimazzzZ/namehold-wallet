import { useState } from "react";
import { useWalletConnection, useWalletBalance, useWalletAddress, useWalletNames, useWalletTransactions, useSendHns } from "../queries/wallet";
import { useSettingsStore } from "../stores/settings";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { Input } from "./ui/Input";
import { Dialog } from "./ui/Dialog";
import { formatHns, hnsToDollarydoos } from "../lib/utils";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useUiStore } from "../stores/ui";

export function WalletView() {
  const { data: conn, isLoading: connLoading } = useWalletConnection();
  const { data: balance, isLoading: balLoading } = useWalletBalance();
  const { data: address } = useWalletAddress();
  const { data: names } = useWalletNames();
  const { data: transactions } = useWalletTransactions();
  const settings = useSettingsStore((s) => s.settings);
  const passphrase = useSettingsStore((s) => s.passphrase);
  const showToast = useUiStore((s) => s.showToast);
  const sendHns = useSendHns();

  const [sendDialogOpen, setSendDialogOpen] = useState(false);
  const [sendAddress, setSendAddress] = useState("");
  const [sendAmount, setSendAmount] = useState("");
  const [sendPassphrase, setSendPassphrase] = useState("");

  const walletUrl = settings?.hsd_wallet_api_url || "";
  const isLocalhost = walletUrl.includes("127.0.0.1") || walletUrl.includes("localhost");
  const writeMode = settings?.write_mode === "true";

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
      showToast(`Send failed: ${e}`, "error");
    }
  };

  return (
    <div className="space-y-6">
      <h2 className="text-xl font-bold">Wallet</h2>

      {!isLocalhost && (
        <div className="bg-red-50 border border-red-200 rounded p-3 text-sm text-red-700">
          Warning: Wallet API URL ({walletUrl}) is not localhost. Only use local connections for security.
        </div>
      )}

      <div className="grid grid-cols-2 gap-4">
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Connection</div>
          <div className="mt-1">
            {connLoading ? (
              <span className="text-gray-400">Checking...</span>
            ) : conn?.connected ? (
              <Badge variant="success">Connected</Badge>
            ) : (
              <Badge variant="error">{conn?.error || "Disconnected"}</Badge>
            )}
          </div>
        </div>
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Network</div>
          <div className="text-lg font-semibold">{settings?.hsd_network || "—"}</div>
        </div>
      </div>

      <div className="grid grid-cols-3 gap-4">
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Confirmed Balance</div>
          <div className="text-xl font-bold">
            {balLoading ? "..." : formatHns(balance?.confirmed)}
          </div>
          <div className="text-xs text-gray-400">HNS</div>
        </div>
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Unconfirmed</div>
          <div className="text-xl font-bold">
            {balLoading ? "..." : formatHns(balance?.unconfirmed)}
          </div>
          <div className="text-xs text-gray-400">HNS</div>
        </div>
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500">Locked</div>
          <div className="text-xl font-bold">
            {balLoading
              ? "..."
              : formatHns(
                  (balance?.locked_confirmed || 0) + (balance?.locked_unconfirmed || 0),
                )}
          </div>
          <div className="text-xs text-gray-400">HNS</div>
        </div>
      </div>

      <div className="bg-white rounded p-4 border border-gray-200">
        <div className="text-sm text-gray-500 mb-2">Receive Address</div>
        <div className="flex items-center gap-2">
          <code className="text-sm bg-gray-50 px-2 py-1 rounded flex-1 truncate">
            {address || "—"}
          </code>
          <Button
            size="sm"
            onClick={async () => {
              if (address) {
                await writeText(address);
                showToast("Address copied", "success");
              }
            }}
            disabled={!address}
          >
            Copy
          </Button>
          {writeMode && (
            <Button
              size="sm"
              variant="primary"
              onClick={() => setSendDialogOpen(true)}
            >
              Send HNS
            </Button>
          )}
        </div>
      </div>

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
          <div className="text-gray-400 text-sm">
            {conn?.connected ? "No names found in wallet" : "Connect to wallet first"}
          </div>
        )}
      </div>

      {transactions && transactions.length > 0 && (
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="text-sm text-gray-500 mb-2">
            Recent Transactions ({transactions.length})
          </div>
          <div className="max-h-60 overflow-auto">
            <pre className="text-xs bg-gray-50 p-2 rounded">
              {JSON.stringify(transactions.slice(0, 10), null, 2)}
            </pre>
          </div>
        </div>
      )}

      <div className="text-xs text-gray-400">
        Wallet ID: {settings?.hsd_wallet_id || "primary"} | API: {walletUrl}
      </div>

      <Dialog open={sendDialogOpen} onClose={() => setSendDialogOpen(false)} title="Send HNS">
        <div className="space-y-3">
          <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
            This will send real HNS from your wallet. This action cannot be undone.
          </div>
          <Input
            label="Destination Address"
            value={sendAddress}
            onChange={(e) => setSendAddress(e.target.value)}
            placeholder="rs1q..."
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
    </div>
  );
}
