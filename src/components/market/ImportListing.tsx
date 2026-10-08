import { useState } from "react";
import { Button } from "../ui/Button";
import { open } from "../../lib/dialog";
import { mapError } from "../../lib/errors";
import { useImportListing } from "../../queries/shakedex";
import type { ImportSource, MarketRow } from "../../types";

interface ImportListingProps {
  /** The imported row and the kind of source it came from. */
  onImported: (row: MarketRow, source: ImportSource["kind"]) => void;
  /** The backend's reason a market link cannot be imported here, if any. */
  linkRefusal?: string | null;
}

/** A pasted `http(s)://` text is a market link, not a listing file. */
function isLink(text: string): boolean {
  return /^https?:\/\//i.test(text.trim());
}

/**
 * Bring in a listing: one field takes a listing file's JSON or a market link
 * (told apart by what was pasted), or a file is chosen from disk.
 */
export function ImportListing({ onImported, linkRefusal = null }: ImportListingProps) {
  const [text, setText] = useState("");
  const importListing = useImportListing();
  const pasted = text.trim();
  const link = isLink(pasted);
  // A link here cannot be imported at all: say so before the click.
  const blocked = link && linkRefusal !== null;

  const run = (source: ImportSource) => {
    importListing.mutate(source, {
      onSuccess: (row) => {
        onImported(row, source.kind);
        setText("");
      },
    });
  };

  const pickFile = async () => {
    const picked = await open({
      title: "Choose a listing file",
      filters: [{ name: "Listing file", extensions: ["json"] }],
    });
    if (typeof picked === "string") run({ kind: "file", path: picked });
  };

  return (
    <div className="space-y-2">
      <label htmlFor="listing-paste" className="block text-sm text-gray-600">
        Paste a listing file or a market.learnhns.com link
      </label>
      <textarea
        id="listing-paste"
        data-testid="listing-paste"
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={link ? 1 : 3}
        placeholder='{"version":2,"name":"…"} or https://market.learnhns.com/listing/name'
        className="w-full border border-gray-300 rounded-md px-3 py-2 text-xs font-mono bg-white focus:outline-none focus:ring-2 focus:ring-blue-500"
      />
      <div className="flex items-center gap-3">
        <Button
          size="sm"
          variant="primary"
          data-testid={link ? "import-listing-link" : "import-listing-text"}
          disabled={importListing.isPending || pasted === "" || blocked}
          onClick={() =>
            // The raw text for a file: its own bytes, as pasted.
            run(link ? { kind: "link", url: pasted } : { kind: "text", json: text })
          }
        >
          {importListing.isPending ? "Checking…" : link ? "Import link" : "Import"}
        </Button>
        <span className="text-sm text-gray-400">or</span>
        <button
          type="button"
          data-testid="import-listing-file"
          onClick={pickFile}
          disabled={importListing.isPending}
          className="text-sm text-blue-600 hover:text-blue-800 hover:underline disabled:text-gray-400 cursor-pointer"
        >
          choose a file…
        </button>
      </div>
      {blocked && <p className="text-xs text-gray-500">{linkRefusal}</p>}
      {importListing.isError && (
        <p role="alert" className="text-sm text-red-600">
          {mapError(importListing.error)}
        </p>
      )}
    </div>
  );
}
