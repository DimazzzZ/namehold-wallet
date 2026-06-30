import { useState } from "react";
import { useUiStore } from "../stores/ui";
import { useSecureCreateWallet, useSecureImportWallet } from "../queries/wallet";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { mapError } from "../lib/errors";
import type { WalletNetwork } from "../types";

type Path = "choose" | "import" | "create" | "watch";

/**
 * Add a wallet (create / import / watch-only). Secrets never touch React: the
 * recovery phrase and passphrase are handled only in the Rust-owned secure
 * window. This form collects a non-secret label + network, triggers the backend
 * flow (which auto-activates the new profile), and calls `onDone` on success.
 *
 * Reused by first-run onboarding, the Wallet "no profile" fallback, and the
 * Wallets manager.
 */
export function AddWalletForm({
  onDone,
  defaultLabel = "",
}: {
  onDone: () => void | Promise<void>;
  defaultLabel?: string;
}) {
  const showToast = useUiStore((s) => s.showToast);
  const createWallet = useSecureCreateWallet();
  const importWallet = useSecureImportWallet();

  const [path, setPath] = useState<Path>("choose");
  const [label, setLabel] = useState(defaultLabel);
  const [network, setNetwork] = useState<WalletNetwork>("mainnet");

  const busy = createWallet.isPending || importWallet.isPending;

  const handleCreate = async () => {
    try {
      await createWallet.mutateAsync({ label: label.trim() || "Wallet", network });
      await onDone();
      showToast("Wallet created. Back up your recovery phrase!", "success");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleImport = async () => {
    try {
      await importWallet.mutateAsync({
        label: label.trim() || "Wallet",
        network,
        kind: "mnemonic_hot",
      });
      await onDone();
      showToast("Wallet imported", "success");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleWatchOnly = async () => {
    try {
      await importWallet.mutateAsync({
        label: label.trim() || "Watch-only",
        network,
        kind: "watch_only_xpub",
      });
      await onDone();
      showToast("Watch-only wallet added", "success");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const NetworkPicker = (
    <div className="flex flex-col gap-1">
      <label className="text-sm font-medium text-gray-700">Network</label>
      <select
        className="border border-gray-300 rounded px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
        value={network}
        onChange={(e) => setNetwork(e.target.value as WalletNetwork)}
      >
        <option value="mainnet">Mainnet</option>
        <option value="testnet">Testnet</option>
        <option value="regtest">Regtest (local testing only)</option>
      </select>
    </div>
  );

  if (path === "choose") {
    return (
      <div className="space-y-3">
        <button
          onClick={() => setPath("import")}
          className="w-full text-left p-4 border-2 border-blue-300 rounded-lg hover:border-blue-500 hover:bg-blue-50 transition"
        >
          <div className="font-medium text-gray-900">
            Import your wallet <span className="text-blue-600">· recommended</span>
          </div>
          <div className="text-sm text-gray-500">
            Restore from your 12/24-word recovery phrase (entered in a secure window).
          </div>
        </button>

        <button
          onClick={() => setPath("watch")}
          className="w-full text-left p-4 border border-gray-200 rounded-lg hover:border-blue-400 hover:bg-blue-50 transition"
        >
          <div className="font-medium text-gray-900">Watch-only (read-only)</div>
          <div className="text-sm text-gray-500">
            Track an account xpub without entering any secret. No spending.
          </div>
        </button>

        <button
          onClick={() => setPath("create")}
          className="w-full text-left p-4 border border-gray-200 rounded-lg hover:border-blue-400 hover:bg-blue-50 transition"
        >
          <div className="font-medium text-gray-900">Create a new wallet</div>
          <div className="text-sm text-gray-500">
            Generate a fresh wallet. Your recovery phrase appears in a secure window.
          </div>
        </button>
      </div>
    );
  }

  if (path === "import") {
    return (
      <div className="space-y-4">
        <Input label="Wallet Name" value={label} onChange={(e) => setLabel(e.target.value)} />
        {NetworkPicker}
        <div className="bg-blue-50 border border-blue-200 rounded p-2 text-xs text-blue-800">
          You'll enter your recovery phrase and a device passphrase in a separate secure
          window. Namehold's main screen never sees them.
        </div>
        <div className="flex gap-2">
          <Button variant="ghost" onClick={() => setPath("choose")}>Back</Button>
          <Button onClick={handleImport} disabled={busy}>
            {busy ? "Importing..." : "Import in secure window"}
          </Button>
        </div>
      </div>
    );
  }

  if (path === "watch") {
    return (
      <div className="space-y-4">
        <Input label="Wallet Name" value={label} onChange={(e) => setLabel(e.target.value)} />
        {NetworkPicker}
        <div className="bg-blue-50 border border-blue-200 rounded p-2 text-xs text-blue-800">
          You'll paste an account-level xpub in a secure window. Watch-only wallets cannot
          spend.
        </div>
        <div className="flex gap-2">
          <Button variant="ghost" onClick={() => setPath("choose")}>Back</Button>
          <Button onClick={handleWatchOnly} disabled={busy}>
            {busy ? "Adding..." : "Add watch-only wallet"}
          </Button>
        </div>
      </div>
    );
  }

  // path === "create"
  return (
    <div className="space-y-4">
      <Input label="Wallet Name" value={label} onChange={(e) => setLabel(e.target.value)} />
      {NetworkPicker}
      <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
        A secure window will ask you to set a device passphrase, then show your recovery
        phrase to back up. Write it down — it's the only way to recover your wallet.
      </div>
      <div className="flex gap-2">
        <Button variant="ghost" onClick={() => setPath("choose")}>Back</Button>
        <Button onClick={handleCreate} disabled={busy}>
          {busy ? "Creating..." : "Create in secure window"}
        </Button>
      </div>
    </div>
  );
}
