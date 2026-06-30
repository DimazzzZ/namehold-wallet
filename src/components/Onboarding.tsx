import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { invoke } from "../lib/invoke";
import { useSettingsStore } from "../stores/settings";
import { useWalletList } from "../queries/wallet";
import { useUiStore } from "../stores/ui";
import { mapError } from "../lib/errors";

export function Onboarding() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const updateSetting = useSettingsStore((s) => s.update);
  const showToast = useUiStore((s) => s.showToast);
  const { data: walletList } = useWalletList();
  const [mode, setMode] = useState<"choose" | "create" | "import">("choose");

  // Create wallet state
  const [walletName, setWalletName] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [confirmPassphrase, setConfirmPassphrase] = useState("");
  const [creating, setCreating] = useState(false);

  // Import wallet state
  const [importName, setImportName] = useState("");
  const [importPassphrase, setImportPassphrase] = useState("");
  const [importMnemonic, setImportMnemonic] = useState("");
  const [importing, setImporting] = useState(false);

  // Seed phrase display
  const [generatedMnemonic, setGeneratedMnemonic] = useState("");
  const [confirmedSaved, setConfirmedSaved] = useState(false);

  const getUniqueName = (base: string) => {
    if (!walletList || !walletList.includes(base)) return base;
    let i = 2;
    while (walletList.includes(`${base}-${i}`)) i++;
    return `${base}-${i}`;
  };

  const handleCreate = async () => {
    if (passphrase !== confirmPassphrase) {
      showToast("Passphrases don't match", "error");
      return;
    }
    const name = walletName.trim() || getUniqueName("wallet");
    setCreating(true);
    try {
      const result: any = await invoke("create_wallet", { id: name, passphrase });
      setGeneratedMnemonic(result?.mnemonic?.phrase || "");
      setWalletName(name);
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setCreating(false);
    }
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
      showToast("Wallet imported successfully", "success");
      navigate("/wallet");
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setImporting(false);
    }
  };

  const handleConfirmSaved = async () => {
    await updateSetting("hsd_wallet_id", walletName);
    qc.invalidateQueries({ queryKey: ["wallet"] });
    showToast("Wallet created. Keep your seed phrase safe!", "success");
    navigate("/wallet");
  };

  // Seed phrase display after creation
  if (generatedMnemonic) {
    return (
      <div className="min-h-screen bg-gray-100 flex items-center justify-center p-4">
        <div className="bg-white rounded-lg p-8 max-w-lg w-full">
          <h2 className="text-xl font-bold mb-4 text-red-600">Save Your Seed Phrase</h2>
          <div className="bg-yellow-50 border border-yellow-200 rounded p-3 text-sm text-yellow-800 mb-4">
            Write down these 24 words in order. This is the ONLY way to recover your wallet.
            Never share this phrase with anyone.
          </div>
          <div className="bg-gray-50 rounded p-4 font-mono text-sm mb-4 break-all">
            {generatedMnemonic}
          </div>
          <div className="flex items-center gap-2 mb-4">
            <input type="checkbox" id="confirmed" checked={confirmedSaved} onChange={(e) => setConfirmedSaved(e.target.checked)} />
            <label htmlFor="confirmed" className="text-sm">I have written down my seed phrase and stored it safely</label>
          </div>
          <Button variant="primary" className="w-full" disabled={!confirmedSaved} onClick={handleConfirmSaved}>
            Continue to Wallet
          </Button>
        </div>
      </div>
    );
  }

  // Choose mode
  if (mode === "choose") {
    return (
      <div className="min-h-screen bg-gray-100 flex items-center justify-center p-4">
        <div className="bg-white rounded-lg p-8 max-w-md w-full text-center">
          <h1 className="text-2xl font-bold mb-2">Welcome to Namehold</h1>
          <p className="text-gray-500 mb-6">Your local Handshake wallet. Get started by creating or importing a wallet.</p>
          <div className="space-y-3">
            <Button variant="primary" className="w-full" onClick={() => setMode("create")}>Create New Wallet</Button>
            <Button variant="secondary" className="w-full" onClick={() => setMode("import")}>Import Existing Wallet</Button>
          </div>
          <p className="text-xs text-gray-400 mt-4">Requires hsd running locally. See Settings for connection config.</p>
        </div>
      </div>
    );
  }

  // Create wallet
  if (mode === "create") {
    return (
      <div className="min-h-screen bg-gray-100 flex items-center justify-center p-4">
        <div className="bg-white rounded-lg p-8 max-w-md w-full">
          <h2 className="text-xl font-bold mb-4">Create New Wallet</h2>
          <div className="space-y-3">
            <Input label="Wallet Name" value={walletName} onChange={(e) => setWalletName(e.target.value)} placeholder={getUniqueName("wallet")} />
            <Input label="Passphrase (optional)" type="password" value={passphrase} onChange={(e) => setPassphrase(e.target.value)} placeholder="Optional passphrase for encryption" />
            <Input label="Confirm Passphrase" type="password" value={confirmPassphrase} onChange={(e) => setConfirmPassphrase(e.target.value)} placeholder="Repeat passphrase" />
            <div className="flex gap-2">
              <Button variant="ghost" onClick={() => setMode("choose")}>Back</Button>
              <Button variant="primary" className="flex-1" onClick={handleCreate} disabled={creating}>
                {creating ? "Creating..." : "Create Wallet"}
              </Button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  // Import wallet
  return (
    <div className="min-h-screen bg-gray-100 flex items-center justify-center p-4">
      <div className="bg-white rounded-lg p-8 max-w-md w-full">
        <h2 className="text-xl font-bold mb-4">Import Wallet</h2>
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
          <div className="flex gap-2">
            <Button variant="ghost" onClick={() => setMode("choose")}>Back</Button>
            <Button variant="primary" className="flex-1" onClick={handleImport} disabled={importing || !importMnemonic.trim()}>
              {importing ? "Importing..." : "Import Wallet"}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
