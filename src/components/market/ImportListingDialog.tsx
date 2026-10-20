import { useState } from "react";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { open as openFile } from "../../lib/dialog";
import { mapError } from "../../lib/errors";
import { useImportListing } from "../../queries/shakedex";
import { hiddenReasonText, importProblem, isListingLink, sentence } from "./marketText";
import type { ImportedRow } from "./marketModel";
import type { ImportSource } from "../../types";

interface ImportListingDialogProps {
  open: boolean;
  onClose: () => void;
  /** A listing the node verified, and how it came. */
  onImported: (imported: ImportedRow) => void;
  /** The backend's reason a market link cannot be imported here, if any. */
  linkRefusal: string | null;
}

/**
 * Bring in a listing a seller sent directly: one field takes a listing
 * file's JSON or a market link (told apart by what was pasted), or a file is
 * chosen from disk. Only a listing the node verified joins the list; for any
 * other verdict the dialog says why it cannot be bought.
 */
export function ImportListingDialog({
  open,
  onClose,
  onImported,
  linkRefusal,
}: ImportListingDialogProps) {
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const importListing = useImportListing();
  const busy = importListing.isPending;
  const pasted = text.trim();
  const link = isListingLink(pasted);
  // A link here cannot be imported at all: say so before the click.
  const blocked = link && linkRefusal !== null;

  const run = (source: ImportSource) => {
    setError(null);
    importListing.mutate(source, {
      onSuccess: (row) => {
        if (row.verdict.verdict !== "buyable") {
          setError(sentence(hiddenReasonText(row.verdict)));
          return;
        }
        setText("");
        onImported({ row, origin: source.kind === "link" ? "link" : "file" });
      },
      onError: (e) => setError(mapError(e)),
    });
  };

  const submit = () => {
    const problem = importProblem(pasted);
    if (problem) return setError(problem);
    // The raw text for a file: its own bytes, as pasted.
    run(link ? { kind: "link", url: pasted } : { kind: "text", json: text });
  };

  const pickFile = async () => {
    const picked = await openFile({
      title: "Choose a listing file",
      filters: [{ name: "Listing file", extensions: ["json"] }],
    });
    if (typeof picked === "string") run({ kind: "file", path: picked });
  };

  // Closing mid-check would hide its outcome.
  const close = () => {
    if (!busy) onClose();
  };

  return (
    <Dialog open={open} onClose={close} title="Import a listing" className="max-w-[480px]">
      <div className="space-y-3">
        <p className="text-sm text-gray-600 [text-wrap:pretty]">
          Got a listing from a seller directly? Paste the file's contents or a market.learnhns.com
          link. Your node checks it the same way as market listings.
        </p>
        <textarea
          data-testid="listing-paste"
          aria-label="Listing file or link"
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setError(null);
          }}
          rows={4}
          placeholder={'https://market.learnhns.com/listing/name  or  {"version":2,"name":…}'}
          className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 font-mono text-xs focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        {blocked && <p className="text-xs text-gray-500">{linkRefusal}</p>}
        {error && (
          <p role="alert" className="text-xs text-red-600" data-testid="import-error">
            {error}
          </p>
        )}
        <div className="flex items-center justify-between gap-3 pt-1">
          <button
            type="button"
            data-testid="import-listing-file"
            onClick={pickFile}
            disabled={busy}
            className="cursor-pointer text-sm text-blue-600 hover:underline disabled:text-gray-400"
          >
            Choose a file…
          </button>
          <div className="flex gap-2">
            <Button variant="ghost" data-testid="import-cancel" onClick={close} disabled={busy}>
              Cancel
            </Button>
            <Button
              variant="primary"
              data-testid={link ? "import-listing-link" : "import-listing-text"}
              disabled={busy || pasted === "" || blocked}
              onClick={submit}
            >
              {busy ? "Checking…" : link ? "Import link" : "Import"}
            </Button>
          </div>
        </div>
      </div>
    </Dialog>
  );
}
