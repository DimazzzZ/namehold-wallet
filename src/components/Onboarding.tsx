import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { useSettingsStore } from "../stores/settings";
import { useUiStore } from "../stores/ui";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { mapError } from "../lib/errors";

type Path = "choose" | "import" | "create" | "watch";

/**
 * Wallet-first onboarding.
 *
 * The default and visually-primary path is importing an existing 24-word seed
 * phrase, since most early users are migrating from Namebase and already hold a
 * recovery phrase. A safe, no-key "watch-only" path lets users explore in
 * read-only mode without committing keys. Creating a brand new wallet and any
 * node/write setup are intentionally secondary and deferred.
 */
export function Onboarding() {
  const qc = useQueryClient();
  const updateSetting = useSettingsStore((s) => s.update);
  const saveAll = useSettingsStore((s) => s.saveAll);
  const showToast = useUiStore((s) => s.showToast);

  const [path, setPath] = useState<Path>("choose");
  const [walletName, setWalletName] = useState("primary");
  const [passphrase, setPassphrase] = useState("");
  const [mnemonic, setMnemonic] = useState("");
  const [createdMnemonic, setCreatedMnemonic] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);

  const finish = async (extra?: Parameters<typeof saveAll>[0]) => {
    await saveAll({ onboarding_complete: "true", ...(extra || {}) });
    qc.invalidateQueries({ queryKey: ["wallet"] });
    qc.invalidateQueries({ queryKey: ["read"] });
  };

  const handleImport = async () => {
    if (!mnemonic.trim()) {
      showToast("Enter your seed phrase", "error");
      return;
    }
    setBusy(true);
    try {
      await invoke("create_wallet", {
        id: walletName,
        passphrase,
        mnemonic: mnemonic.trim(),
      });
      await updateSetting("hsd_wallet_id", walletName);
      await finish();
      showToast(`Wallet "${walletName}" imported`, "success");
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const handleCreate = async () => {
    setBusy(true);
    try {
      const result = await invoke<{ mnemonic?: { phrase?: string } }>("create_wallet", {
        id: walletName,
        passphrase,
      });
      setCreatedMnemonic(result?.mnemonic?.phrase || "");
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const handleConfirmCreated = async () => {
    await updateSetting("hsd_wallet_id", walletName);
    await finish();
    showToast(`Wallet "${walletName}" created`, "success");
  };

  const handleWatchOnly = async () => {
    await finish({
      connection_mode: "external_read_only",
      external_read_provider: "hnsfans",
      write_mode: "false",
    });
    showToast("Read-only mode enabled", "success");
  };

  return (
    <div className="flex h-screen items-center justify-center bg-gray-100 p-6">
      <div className="bg-white rounded-lg shadow-lg max-w-lg w-full p-8">
        <h1 className="text-2xl font-bold text-gray-900 mb-2">Welcome to Namehold</h1>
        <p className="text-gray-500 mb-6">
          Your wallet for moving and managing Handshake names. Let's get you in.
        </p>

        {path === "choose" && (
          <div className="space-y-3">
            <button
              onClick={() => setPath("import")}
              className="w-full text-left p-4 border-2 border-blue-300 rounded-lg hover:border-blue-500 hover:bg-blue-50 transition"
            >
              <div className="font-medium text-gray-900">
                Import your wallet <span className="text-blue-600">· recommended</span>
              </div>
              <div className="text-sm text-gray-500">
                Restore from your 24-word seed phrase (e.g. exported from Namebase).
              </div>
            </button>

            <button
              onClick={handleWatchOnly}
              className="w-full text-left p-4 border border-gray-200 rounded-lg hover:border-blue-400 hover:bg-blue-50 transition"
            >
              <div className="font-medium text-gray-900">Just look around (read-only)</div>
              <div className="text-sm text-gray-500">
                Explore balances and names without entering any keys. Safe by default.
              </div>
            </button>

            <button
              onClick={() => setPath("create")}
              className="w-full text-left p-4 border border-gray-200 rounded-lg hover:border-blue-400 hover:bg-blue-50 transition"
            >
              <div className="font-medium text-gray-900">Create a new wallet</div>
              <div className="text-sm text-gray-500">Generate a fresh wallet and seed phrase.</div>
            </button>
          </div>
        )}

        {path === "import" && (
          <div className="space-y-4">
            <div className="flex flex-col gap-1">
              <label className="text-sm font-medium text-gray-700">Seed Phrase (24 words)</label>
              <textarea
                className="border border-gray-300 rounded px-3 py-2 text-sm h-24 resize-none focus:outline-none focus:ring-2 focus:ring-blue-500"
                value={mnemonic}
                onChange={(e) => setMnemonic(e.target.value)}
                placeholder="word1 word2 word3 ... word24"
              />
            </div>
            <Input label="Wallet Name" value={walletName} onChange={(e) => setWalletName(e.target.value)} />
            <Input
              label="Passphrase (optional)"
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
            />
            <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
              The passphrase affects address derivation. If your original wallet had no
              passphrase, leave this empty.
            </div>
            <div className="flex gap-2">
              <Button variant="ghost" onClick={() => setPath("choose")}>Back</Button>
              <Button onClick={handleImport} disabled={busy || !mnemonic.trim()}>
                {busy ? "Importing..." : "Import Wallet"}
              </Button>
            </div>
          </div>
        )}

        {path === "create" && (
          <div className="space-y-4">
            {!createdMnemonic ? (
              <>
                <Input label="Wallet Name" value={walletName} onChange={(e) => setWalletName(e.target.value)} />
                <Input
                  label="Passphrase (optional)"
                  type="password"
                  value={passphrase}
                  onChange={(e) => setPassphrase(e.target.value)}
                />
                <div className="flex gap-2">
                  <Button variant="ghost" onClick={() => setPath("choose")}>Back</Button>
                  <Button onClick={handleCreate} disabled={busy}>
                    {busy ? "Creating..." : "Create Wallet"}
                  </Button>
                </div>
              </>
            ) : (
              <>
                <div className="bg-yellow-50 border border-yellow-200 rounded p-3 text-sm text-yellow-800">
                  Write down these 24 words in order. This is the ONLY way to recover your wallet.
                </div>
                <div className="bg-gray-50 rounded p-4 font-mono text-sm break-all">
                  {createdMnemonic}
                </div>
                <label className="flex items-center gap-2 text-sm">
                  <input type="checkbox" checked={confirmed} onChange={(e) => setConfirmed(e.target.checked)} />
                  I have saved my seed phrase securely
                </label>
                <Button onClick={handleConfirmCreated} disabled={!confirmed} className="w-full">
                  Continue
                </Button>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
