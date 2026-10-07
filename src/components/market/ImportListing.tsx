import { useState } from "react";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { open } from "../../lib/dialog";
import { useImportListing } from "../../queries/shakedex";
import type { ImportSource, MarketRow } from "../../types";

interface ImportListingProps {
  /** The imported row and the kind of source it came from. */
  onImported: (row: MarketRow, source: ImportSource["kind"]) => void;
  /** The backend's reason a market link cannot be imported here, if any. */
  linkRefusal?: string | null;
}

/** Bring in a listing from a listing file, pasted JSON or a market link. */
export function ImportListing({ onImported, linkRefusal = null }: ImportListingProps) {
  const [json, setJson] = useState("");
  const [url, setUrl] = useState("");
  const importListing = useImportListing();

  const run = (source: ImportSource) => {
    importListing.mutate(source, { onSuccess: (row) => onImported(row, source.kind) });
  };

  const pickFile = async () => {
    const picked = await open({
      title: "Choose a listing file",
      filters: [{ name: "Listing file", extensions: ["json"] }],
    });
    if (typeof picked === "string") run({ kind: "file", path: picked });
  };

  return (
    <div className="space-y-3">
      <div>
        <Button
          size="sm"
          data-testid="import-listing-file"
          onClick={pickFile}
          disabled={importListing.isPending}
        >
          Choose listing file…
        </Button>
      </div>
      <div className="space-y-1">
        <label htmlFor="listing-paste" className="text-sm font-medium text-gray-700">
          Or paste a listing file
        </label>
        <textarea
          id="listing-paste"
          data-testid="listing-paste"
          value={json}
          onChange={(e) => setJson(e.target.value)}
          rows={4}
          className="w-full border border-gray-300 rounded px-3 py-1.5 text-xs font-mono focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <Button
          size="sm"
          data-testid="import-listing-text"
          disabled={importListing.isPending || json.trim() === ""}
          onClick={() => run({ kind: "text", json })}
        >
          Import
        </Button>
      </div>
      <div className="flex items-end gap-2">
        <div className="flex-1">
          <Input
            label="Or a market link"
            placeholder="https://market.learnhns.com/listing/name"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
          />
        </div>
        <Button
          size="sm"
          data-testid="import-listing-link"
          disabled={importListing.isPending || url.trim() === "" || linkRefusal !== null}
          onClick={() => run({ kind: "link", url: url.trim() })}
        >
          Import link
        </Button>
      </div>
      {linkRefusal && <p className="text-xs text-gray-500">{linkRefusal}</p>}
      {importListing.isError && (
        <p role="alert" className="text-sm text-red-600">
          {String(importListing.error)}
        </p>
      )}
    </div>
  );
}
