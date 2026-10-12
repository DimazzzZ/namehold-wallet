import { useState } from "react";
import { Alert } from "../ui/Alert";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { open } from "../../lib/dialog";
import { mapError } from "../../lib/errors";
import { displayName } from "../../lib/idn";
import { normalizeNameInputAce } from "../../lib/utils";
import { useImportOwnListing, useRestoreLock } from "../../queries/shakedex";
import { useActiveProfile, useWithSigner, useWriteCapability } from "../../queries/wallet";
import { useUiStore } from "../../stores/ui";
import type { ShakedexListingSummary } from "../../types";
import { RESTORED_LABEL, listingStateLabel, shakedexRefusal } from "./marketText";

/**
 * R32: adopt a lock of this wallet by the name's chain state, after the
 * wallet was restored from its recovery phrase. The backend derives the
 * name's lock key from the unlocked seed (recovery-phrase wallets on a node
 * that can send, R16, R6; no experimental flag, R15) and refuses anything
 * that is not this wallet's lock, with the reason.
 */
export function RestoreLock() {
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: writeCap } = useWriteCapability();
  const withSigner = useWithSigner();
  const restore = useRestoreLock();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const gate = shakedexRefusal(profile?.kind, writeCap);
  // The on-chain (ACE) name, as the lock key was derived for it at Lock.
  const typed = normalizeNameInputAce(name);
  // The normalization drops what a name cannot hold (spaces, punctuation):
  // say which name is restored when it is not the one typed, beyond case and
  // a leading dot.
  const target = typed === "" ? "" : displayName(typed);
  const asTyped = name.trim().toLowerCase().replace(/^\./, "");
  const showTarget = target !== "" && target !== asTyped;

  const submit = async () => {
    if (!profile || typed === "") return;
    setBusy(true);
    setError(null);
    try {
      await withSigner.run(profile.id, () => restore.mutateAsync(typed));
      showToast(`Restored .${displayName(typed)}: ${RESTORED_LABEL}`);
      setName("");
    } catch (e) {
      setError(mapError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="mt-6 space-y-2" aria-labelledby="restore-lock-heading">
      <h3 id="restore-lock-heading" className="text-sm font-semibold text-gray-900">
        Restore a lock by name
      </h3>
      <p className="text-xs text-gray-500">
        After restoring this wallet from its recovery phrase, type the name of a listing it had: the
        wallet checks on your node that the name sits in this wallet&apos;s lock and adopts it.
      </p>
      <div className="flex items-center gap-2">
        <input
          data-testid="restore-name"
          aria-label="Name to restore"
          className="block w-full max-w-xs rounded-md border border-gray-300 px-3 py-1.5 text-sm font-mono"
          value={name}
          disabled={busy}
          onChange={(e) => setName(e.target.value)}
        />
        <Button
          size="sm"
          variant="primary"
          data-testid="restore-submit"
          disabled={busy || typed === "" || profile == null || gate !== null}
          onClick={submit}
        >
          {busy ? "Checking…" : "Restore"}
        </Button>
      </div>
      {showTarget && (
        <p data-testid="restore-target" className="text-xs text-gray-600">
          Restores .{target}
        </p>
      )}
      {gate && <p className="text-xs text-gray-600">{gate}</p>}
      {error && (
        <p role="alert" className="text-sm text-red-600">
          {error}
        </p>
      )}
    </section>
  );
}

export interface OwnFileImportProps {
  listing: ShakedexListingSummary;
  onClose: () => void;
}

/**
 * R32: a Restored lock's own saved listing file, pasted or chosen on disk
 * (the backend reads the chosen path; the renderer has no file access).
 * The backend checks the file against the lock and the seed and makes the
 * lock a full listing, or refuses it with the reason.
 */
export function OwnFileImport({ listing, onClose }: OwnFileImportProps) {
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: writeCap } = useWriteCapability();
  const withSigner = useWithSigner();
  const importOwn = useImportOwnListing();
  // R16, R6: the import derives the lock key from the seed.
  const gate = shakedexRefusal(profile?.kind, writeCap);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async (source: { kind: "text"; text: string } | { kind: "file"; path: string }) => {
    if (!profile) return;
    setBusy(true);
    setError(null);
    try {
      await withSigner.run(profile.id, () => importOwn.mutateAsync(source));
      showToast(`Listing file imported: the listing is ${listingStateLabel("listed", null)}.`);
      onClose();
    } catch (e) {
      setError(mapError(e));
    } finally {
      setBusy(false);
    }
  };

  const pickFile = async () => {
    let picked: string | string[] | null;
    try {
      picked = await open({
        title: "Choose the saved listing file",
        filters: [{ name: "Listing file", extensions: ["json"] }],
      });
    } catch (e) {
      setError(mapError(e));
      return;
    }
    if (typeof picked === "string") await run({ kind: "file", path: picked });
  };

  return (
    <Dialog
      open
      onClose={() => !busy && onClose()}
      title={`Import the listing file of .${displayName(listing.name)}`}
    >
      <div className="space-y-3 text-sm text-gray-700">
        <label htmlFor="own-file-text" className="block text-gray-600">
          Paste the listing file this wallet saved for this lock
        </label>
        <textarea
          id="own-file-text"
          data-testid="own-file-text"
          value={text}
          disabled={busy}
          onChange={(e) => setText(e.target.value)}
          rows={4}
          className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 font-mono text-xs"
        />
        {gate && <p className="text-xs text-gray-600">{gate}</p>}
        {error && (
          <Alert tone="error">
            <span data-testid="own-file-error">{error}</span>
          </Alert>
        )}
        <div className="flex items-center justify-end gap-3 pt-2">
          <button
            type="button"
            data-testid="own-file-pick"
            onClick={() => void pickFile()}
            disabled={busy || gate !== null}
            className="cursor-pointer text-sm text-blue-600 hover:text-blue-800 hover:underline disabled:text-gray-400"
          >
            choose a file…
          </button>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            Close
          </Button>
          <Button
            variant="primary"
            data-testid="own-file-import"
            disabled={busy || gate !== null || text.trim() === ""}
            onClick={() => void run({ kind: "text", text })}
          >
            {busy ? "Checking…" : "Import"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
