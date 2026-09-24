import { useState } from "react";
import { NetworkSelect } from "./ui/NetworkSelect";
import { useUiStore } from "../stores/ui";
import {
  useSecureCreateWallet,
  useSecureImportWallet,
  useImportLedgerWallet,
} from "../queries/wallet";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { mapError } from "../lib/errors";
import type { WalletNetwork } from "../types";

type Path = "choose" | "import" | "create" | "watch" | "ledger";

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
  defaultNetwork = "mainnet",
}: {
  onDone: () => void | Promise<void>;
  defaultLabel?: string;
  /**
   * Preselect the network picker. G1: onboarding lifts the network choice into
   * the connection step (so the "Test connection" probe can validate the
   * node's chain against it) and threads the user's pick down here, keeping
   * the two screens in agreement.
   */
  defaultNetwork?: WalletNetwork;
}) {
  const showToast = useUiStore((s) => s.showToast);
  const createWallet = useSecureCreateWallet();
  const importWallet = useSecureImportWallet();
  const importLedger = useImportLedgerWallet();

  const [path, setPath] = useState<Path>("choose");
  const [label, setLabel] = useState(defaultLabel);
  const [network, setNetwork] = useState<WalletNetwork>(defaultNetwork);

  const busy = createWallet.isPending || importWallet.isPending || importLedger.isPending;

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

  const handleLedger = async () => {
    try {
      await importLedger.mutateAsync({
        label: label.trim() || "Ledger",
        network,
      });
      await onDone();
      showToast("Ledger wallet imported", "success");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const NetworkPicker = (
    <div className="flex flex-col gap-1">
      <label className="text-sm font-medium text-gray-700">Network</label>
      <NetworkSelect value={network} onChange={setNetwork} testId="add-wallet-network-select" />
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
          onClick={() => setPath("ledger")}
          className="w-full text-left p-4 border border-gray-200 rounded-lg hover:border-blue-400 hover:bg-blue-50 transition"
        >
          <div className="font-medium text-gray-900">Connect a Ledger device</div>
          <div className="text-sm text-gray-500">
            Import a hardware wallet. Keys stay on the device; every spend is confirmed on-device.
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
          You'll enter your recovery phrase and a device passphrase in a separate secure window.
          Namehold's main screen never sees them.
        </div>
        <div className="flex gap-2">
          <Button variant="ghost" onClick={() => setPath("choose")}>
            Back
          </Button>
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
          You'll paste an account-level xpub in a secure window. Watch-only wallets cannot spend.
        </div>
        <div className="flex gap-2">
          <Button variant="ghost" onClick={() => setPath("choose")}>
            Back
          </Button>
          <Button onClick={handleWatchOnly} disabled={busy}>
            {busy ? "Adding..." : "Add watch-only wallet"}
          </Button>
        </div>
      </div>
    );
  }

  if (path === "ledger") {
    return (
      <div className="space-y-4">
        <Input label="Wallet Name" value={label} onChange={(e) => setLabel(e.target.value)} />
        {NetworkPicker}
        <div className="bg-blue-50 border border-blue-200 rounded p-2 text-xs text-blue-800">
          Make sure your Ledger is connected via USB, unlocked, and the Handshake app is open.
          Confirm the export prompt on the device when it appears — only the account xpub is
          exported (no keys ever leave the device).
        </div>
        <div className="flex gap-2">
          <Button variant="ghost" onClick={() => setPath("choose")}>
            Back
          </Button>
          <Button onClick={handleLedger} disabled={busy}>
            {busy ? "Connecting to device..." : "Import from Ledger"}
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
        A secure window will ask you to set a device passphrase, then show your recovery phrase to
        back up. Write it down — it's the only way to recover your wallet.
      </div>
      <div className="flex gap-2">
        <Button variant="ghost" onClick={() => setPath("choose")}>
          Back
        </Button>
        <Button onClick={handleCreate} disabled={busy}>
          {busy ? "Creating..." : "Create in secure window"}
        </Button>
      </div>
    </div>
  );
}
