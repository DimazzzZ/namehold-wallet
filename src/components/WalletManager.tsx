import { useEffect, useState } from "react";
import {
  useWalletProfiles,
  useSetActiveProfile,
  useDeleteProfile,
  useRenameProfile,
  useRevealBackupPhrase,
} from "../queries/wallet";
import { useUiStore } from "../stores/ui";
import { Dialog } from "./ui/Dialog";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { Badge } from "./ui/Badge";
import { AddWalletForm } from "./AddWalletForm";
import { isUserRejected, mapError } from "../lib/errors";
import { walletKindLabel } from "../lib/walletKind";
import { writeText } from "../lib/clipboard";
import { Copy, Plus } from "lucide-react";
import { Icon } from "./ui/Icon";
import { NetworkBadge } from "./ui/NetworkBadge";
import { Tooltip } from "./ui/Tooltip";

/**
 * Manage all wallet profiles: switch the active one, rename, reveal a recovery
 * phrase (hot wallets, in the secure window), delete, and add another. The backend
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
  const rename = useRenameProfile();
  const reveal = useRevealBackupPhrase();

  const [showAdd, setShowAdd] = useState(startInAddMode);
  const [editing, setEditing] = useState<{ id: string; label: string } | null>(null);

  // Reset to the requested mode whenever the dialog (re)opens.
  useEffect(() => {
    if (open) {
      setShowAdd(startInAddMode);
      setEditing(null);
    }
  }, [open, startInAddMode]);

  const handleRename = (current: string) => {
    const label = editing?.label.trim();
    if (!editing || !label || label === current) return;
    rename.mutate(
      { walletProfileId: editing.id, label },
      {
        onSuccess: () => setEditing(null),
        // Declined in the secure window: the edit stays, with nothing to report.
        onError: (e) => {
          if (!isUserRejected(e)) showToast(mapError(e), "error");
        },
      },
    );
  };

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

  // The backend asks for the confirmation in the secure window (a wallet
  // whose recovery phrase is stored here also has its label typed there), so
  // nothing on this page can delete a wallet by itself. Declining there is
  // the user's answer and needs no toast.
  const handleDelete = async (id: string, label: string, wasActive: boolean) => {
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
      if (isUserRejected(e)) return;
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
                <div className="min-w-0 flex-1">
                  {editing?.id === p.id ? (
                    <form
                      className="flex items-center gap-1.5"
                      onSubmit={(e) => {
                        e.preventDefault();
                        handleRename(p.label);
                      }}
                    >
                      <div className="flex-1 min-w-0">
                        <Input
                          aria-label="Wallet name"
                          inputSize="sm"
                          autoFocus
                          maxLength={64}
                          value={editing.label}
                          onChange={(e) => setEditing({ id: p.id, label: e.target.value })}
                          onKeyDown={(e) => {
                            // Escape leaves the edit, not the whole dialog.
                            if (e.key === "Escape") {
                              e.stopPropagation();
                              setEditing(null);
                            }
                          }}
                        />
                      </div>
                      <Button
                        type="submit"
                        size="sm"
                        variant="secondary"
                        disabled={
                          rename.isPending ||
                          !editing.label.trim() ||
                          editing.label.trim() === p.label
                        }
                      >
                        {rename.isPending ? "Saving…" : "Save"}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        onClick={() => setEditing(null)}
                      >
                        Cancel
                      </Button>
                    </form>
                  ) : (
                    <div className="flex items-center gap-2">
                      <span className="font-medium truncate">{p.label}</span>
                      {p.active && <Badge variant="success">Active</Badge>}
                    </div>
                  )}
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
                {editing?.id !== p.id && (
                  <div className="flex items-center gap-1 shrink-0">
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => setEditing({ id: p.id, label: p.label })}
                    >
                      Rename
                    </Button>
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
                      onClick={() => handleDelete(p.id, p.label, p.active)}
                      data-testid="wallet-delete"
                    >
                      Delete
                    </Button>
                  </div>
                )}
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
