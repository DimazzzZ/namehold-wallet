import { useState } from "react";
import { useReceiveAddresses, useDeriveNextReceiveAddress } from "../queries/read";
import { useActiveProfile } from "../queries/wallet";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { writeText } from "../lib/clipboard";
import { truncateMiddle } from "../lib/utils";
import { explorerAddressUrl } from "../lib/openExternal";
import { useUiStore } from "../stores/ui";

/**
 * Expandable list of all receive-branch addresses for the active wallet.
 * Each row shows the derivation index, truncated address, a used/unused badge,
 * and a copy button. A "Generate new address" button at the bottom allocates
 * the next unused index.
 */
export function ReceiveAddressList() {
  const { data: addresses, isLoading } = useReceiveAddresses();
  const { data: profile } = useActiveProfile();
  const deriveNext = useDeriveNextReceiveAddress();
  const showToast = useUiStore((s) => s.showToast);
  const [copiedIdx, setCopiedIdx] = useState<number | null>(null);

  const handleCopy = async (address: string, index: number) => {
    await writeText(address);
    setCopiedIdx(index);
    showToast("Address copied", "success");
    setTimeout(() => setCopiedIdx(null), 2000);
  };

  const handleDerive = async () => {
    try {
      await deriveNext.mutateAsync({ walletProfileId: profile?.id ?? null });
      showToast("New address generated", "success");
    } catch (e: unknown) {
      showToast(`Failed to generate address: ${e}`, "error");
    }
  };

  if (isLoading) {
    return <div className="text-sm text-gray-400 py-2">Loading addresses…</div>;
  }

  const rows = addresses ?? [];

  return (
    <div className="space-y-2" data-testid="receive-address-list">
      {rows.length === 0 ? (
        <div className="text-sm text-gray-400">
          No addresses derived yet. Generate one or run a sync.
        </div>
      ) : (
        <div className="max-h-64 overflow-auto border border-gray-200 rounded divide-y divide-gray-100">
          {rows.map((row) => (
            <div
              key={row.index}
              className="flex items-center gap-2 px-3 py-2 text-xs"
              data-testid={`addr-row-${row.index}`}
            >
              <span className="text-gray-400 w-6 text-right font-mono">
                {row.index}
              </span>
              <span className="font-mono text-gray-700 flex-1 truncate" title={row.address}>
                {truncateMiddle(row.address, 10, 8)}
              </span>
              <Badge variant={row.used ? "default" : "success"}>
                {row.used ? "used" : "fresh"}
              </Badge>
              {profile?.network === "mainnet" && (
                <a
                  href={explorerAddressUrl(row.address)}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-blue-500 hover:underline"
                  title="View on explorer"
                >
                  ↗
                </a>
              )}
              <button
                type="button"
                className="text-blue-600 hover:underline"
                onClick={() => handleCopy(row.address, row.index)}
                data-testid={`copy-addr-${row.index}`}
              >
                {copiedIdx === row.index ? "Copied" : "Copy"}
              </button>
            </div>
          ))}
        </div>
      )}
      <Button
        size="sm"
        variant="ghost"
        onClick={handleDerive}
        disabled={deriveNext.isPending}
        data-testid="derive-next-address-btn"
      >
        {deriveNext.isPending ? "Generating…" : "+ Generate new address"}
      </Button>
    </div>
  );
}
