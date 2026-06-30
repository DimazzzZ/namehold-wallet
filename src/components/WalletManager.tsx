import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useWalletList } from "../queries/wallet";
import { useSettingsStore } from "../stores/settings";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { Input } from "./ui/Input";
import { Dialog } from "./ui/Dialog";
import { invoke } from "../lib/invoke";
import { useUiStore } from "../stores/ui";
import { mapError } from "../lib/errors";

export function WalletManager() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { data: walletList, isLoading, refetch } = useWalletList();
  const settings = useSettingsStore((s) => s.settings);
  const updateSetting = useSettingsStore((s) => s.update);
  const showToast = useUiStore((s) => s.showToast);
  const currentWalletId = settings?.hsd_wallet_id || "";

  const [createOpen, setCreateOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);

  // Create state
  const [newName, setNewName] = useState("");
  const [newPassphrase, setNewPassphrase] = useState("");
  const [creating, setCreating] = useState(false);
  const [createdMnemonic, setCreatedMnemonic] = useState("");
  const [confirmedSaved, setConfirmedSaved] = useState(false);

  // Import state
  const [importName, setImportName] = useState("");
  const [importPassphrase, setImportPassphrase] = useState("");
  const [importMnemonic, setImportMnemonic] = useState("");
  const [importing, setImporting] = useState(false);

  const getUniqueName = (base: string) => {
    if (!walletList || !walletList.includes(base)) return base;
    let i = 2;
    while (walletList.includes(`${base}-${i}`)) i++;
    return `${base}-${i}`;
  };

  const handleSwitch = async (id: string) => {
    await updateSetting("hsd_wallet_id", id);
    qc.invalidateQueries({ queryKey: ["wallet"] });
    showToast(`Switched to "${id}"`, "success");
    navigate("/wallet");
  };

  const handleDelete = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      const result = await invoke<string>("delete_wallet", { id: deleteTarget });
      showToast(result || `Wallet "${deleteTarget}" removed from list`, "success");
      setDeleteTarget(null);
      // If deleted the current wallet, switch to another
      if (deleteTarget === currentWalletId) {
        const remaining = walletList?.filter((w) => w !== deleteTarget) || [];
        if (remaining.length > 0) {
          await updateSetting("hsd_wallet_id", remaining[0]!);
        } else {
          await updateSetting("hsd_wallet_id", "");
        }
      }
      qc.invalidateQueries({ queryKey: ["wallet"] });
      refetch();
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setDeleting(false);
    }
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
    refetch();
    showToast(`Wallet "${newName.trim()}" created`, "success");
    navigate("/wallet");
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
      refetch();
      showToast(`Wallet "${name}" imported`, "success");
      navigate("/wallet");
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-bold">Wallet Manager</h2>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="secondary"
            onClick={async () => {
              showToast("Refreshing...", "info");
              await refetch();
              showToast("Wallet list refreshed", "success");
            }}
          >
            Refresh
          </Button>
          <Button size="sm" onClick={() => setCreateOpen(true)}>Create Wallet</Button>
          <Button size="sm" variant="secondary" onClick={() => setImportOpen(true)}>Import Wallet</Button>
        </div>
      </div>

      {isLoading ? (
        <div className="text-gray-500">Loading wallets...</div>
      ) : !walletList || walletList.length === 0 ? (
        <div className="bg-white rounded p-8 border text-center">
          <div className="text-gray-500 mb-3">No wallets found.</div>
          <div className="text-sm text-gray-400 mb-4">Create a new wallet or import an existing one.</div>
        </div>
      ) : (
        <div className="space-y-2">
          {walletList.map((id) => (
            <div
              key={id}
              className={`bg-white rounded border p-4 flex items-center justify-between cursor-pointer hover:bg-gray-50 ${
                id === currentWalletId ? "border-blue-300 bg-blue-50" : "border-gray-200"
              }`}
              onClick={() => handleSwitch(id)}
            >
              <div className="flex items-center gap-3">
                <span className="font-mono font-medium">{id}</span>
                {id === currentWalletId && <Badge variant="success">Active</Badge>}
              </div>
              <div className="flex gap-2">
                <Button size="sm" variant="ghost">
                  {id === currentWalletId ? "Current" : "Switch"}
                </Button>
                <Button
                  size="sm"
                  variant="danger"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDeleteTarget(id);
                  }}
                >
                  Delete
                </Button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Delete Confirmation Dialog */}
      <Dialog open={!!deleteTarget} onClose={() => setDeleteTarget(null)} title="Remove Wallet">
        <div className="space-y-3">
          <div className="text-sm text-gray-600">
            Remove wallet <strong>"{deleteTarget}"</strong> from the list?
          </div>
          <div className="text-xs text-gray-500">
            The wallet will be hidden from Namehold but still exists in hsd. You can re-import it later with the same seed phrase.
          </div>
          <div className="flex gap-2 justify-end">
            <Button variant="ghost" onClick={() => setDeleteTarget(null)}>Cancel</Button>
            <Button variant="danger" onClick={handleDelete} disabled={deleting}>
              {deleting ? "Removing..." : "Remove from List"}
            </Button>
          </div>
        </div>
      </Dialog>

      {/* Create Dialog */}
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

      {/* Import Dialog */}
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
