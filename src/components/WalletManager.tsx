import { useEffect, useState } from "react";
import {
  useWalletProfiles,
  useSetActiveProfile,
  useDeleteProfile,
  useRevealBackupPhrase,
} from "../queries/wallet";
import { useUiStore } from "../stores/ui";
import { Dialog } from "./ui/Dialog";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { AddWalletForm } from "./AddWalletForm";
import { mapError } from "../lib/errors";
import { walletKindLabel } from "../lib/walletKind";
import { writeText } from "../lib/clipboard";
import { Copy, Plus } from "lucide-react";
import { Icon } from "./ui/Icon";
import { NetworkBadge } from "./ui/NetworkBadge";
import { Tooltip } from "./ui/Tooltip";

/**
 * Manage all wallet profiles: switch the active one, reveal a recovery phrase
 * (hot wallets, in the secure window), delete, and add another. The backend
 * fully supports multiple profiles; this is the UI for it.
 */
export function WalletManager({
  open,
  onClose,
  startInAddMode = false,
}: {
  open: boolean;
  onClose: () => void;
  startInAddMode?: boolean;
}) {
  const showToast = useUiStore((s) => s.showToast);
  const { data: profiles = [] } = useWalletProfiles();
  const setActive = useSetActiveProfile();
  const deleteProfile = useDeleteProfile();
  const reveal = useRevealBackupPhrase();

  const [showAdd, setShowAdd] = useState(startInAddMode);

  // Reset to the requested mode whenever the dialog (re)opens.
  useEffect(() => {
    if (open) setShowAdd(startInAddMode);
  }, [open, startInAddMode]);

  const handleSwitch = (id: string) => {
    setActive.mutate(id, {
      onError: (e) => showToast(mapError(e), "error"),
    });
  };

  const handleReveal = (id: string) => {
    reveal.mutate(id, {
      onError: (e) => showToast(mapError(e), "error"),
    });
  };

  const handleDelete = async (id: string, label: string, network: string, wasActive: boolean) => {
    if (
      !confirm(
        `Remove wallet "${label}" (${network})? This deletes the local profile and its encrypted seed. Make sure you have its recovery phrase — this cannot be undone.`,
      )
    ) {
      return;
    }
    try {
      await deleteProfile.mutateAsync(id);
      // The backend clears the active pointer when the active profile is
      // deleted but does not pick a replacement — switch to a remaining one.
      if (wasActive) {
        const remaining = profiles.find((p) => p.id !== id);
        if (remaining) setActive.mutate(remaining.id);
      }
      showToast(`Removed "${label}"`, "success");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  return (
    <Dialog open={open} onClose={onClose} title="Wallets">
      <div className="space-y-4">
        <div className="divide-y divide-gray-100">
          {profiles.length === 0 ? (
            <div className="text-sm text-gray-500 py-2">No wallets yet.</div>
          ) : (
            profiles.map((p) => (
              <div key={p.id} className="flex items-center justify-between gap-2 py-2">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-medium truncate">{p.label}</span>
                    {p.active && <Badge variant="success">Active</Badge>}
                  </div>
                  <div className="flex items-center gap-1.5 mt-0.5">
                    <NetworkBadge network={p.network} />
                    <Badge variant={p.watchOnly ? "warning" : "default"}>
                      {walletKindLabel(p)}
                    </Badge>
                  </div>
                  <div className="mt-1 flex items-center gap-1 text-[11px] text-gray-500">
                    ID
                    <Tooltip content={p.id}>
                      <span className="font-mono" data-testid="wallet-id">
                        {p.id.slice(0, 8)}…
                      </span>
                    </Tooltip>
                    <Tooltip content="Copy wallet id">
                      <button
                        type="button"
                        aria-label="Copy wallet id"
                        className="p-0.5 rounded hover:bg-gray-100 hover:text-gray-700"
                        onClick={async () => {
                          await writeText(p.id);
                          showToast("Wallet id copied", "success");
                        }}
                        data-testid="copy-wallet-id"
                      >
                        <Icon icon={Copy} size={12} />
                      </button>
                    </Tooltip>
                  </div>
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  {!p.active && (
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={setActive.isPending}
                      onClick={() => handleSwitch(p.id)}
                    >
                      Switch
                    </Button>
                  )}
                  {!p.watchOnly && (
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={reveal.isPending}
                      onClick={() => handleReveal(p.id)}
                    >
                      Reveal phrase
                    </Button>
                  )}
                  <Button
                    size="sm"
                    variant="danger"
                    disabled={deleteProfile.isPending}
                    onClick={() => handleDelete(p.id, p.label, p.network, p.active)}
                  >
                    Delete
                  </Button>
                </div>
              </div>
            ))
          )}
        </div>

        {showAdd ? (
          <div className="border-t border-gray-100 pt-4 space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold text-gray-700">Add a wallet</h3>
              <Button size="sm" variant="ghost" onClick={() => setShowAdd(false)}>
                Cancel
              </Button>
            </div>
            <AddWalletForm
              defaultLabel={`Wallet ${profiles.length + 1}`}
              onDone={() => setShowAdd(false)}
            />
          </div>
        ) : (
          <div className="border-t border-gray-100 pt-3">
            <Button
              variant="primary"
              className="inline-flex items-center gap-1.5"
              onClick={() => setShowAdd(true)}
            >
              <Icon icon={Plus} size={14} /> Add wallet
            </Button>
          </div>
        )}
      </div>
    </Dialog>
  );
}
