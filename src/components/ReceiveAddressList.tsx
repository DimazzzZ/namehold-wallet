import { useState } from "react";
import { useReceiveAddresses, useRevealNextReceiveAddress } from "../queries/read";
import { useActiveProfile } from "../queries/wallet";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { writeText } from "../lib/clipboard";
import { formatDate, truncateMiddle } from "../lib/utils";
import { explorerAddressUrl } from "../lib/openExternal";
import { useUiStore } from "../stores/ui";
import { mapError } from "../lib/errors";
import { QRCodeSVG } from "qrcode.react";
import { Tooltip } from "./ui/Tooltip";

/**
 * Expandable list of all receive-branch addresses for the active wallet.
 * Each row shows the derivation index, truncated address, a used/unused badge,
 * a copy button, a QR toggle, and the first-seen date. A "Generate new
 * address" button at the bottom allocates the next unused index.
 */
export function ReceiveAddressList() {
  const { data: addresses, isLoading } = useReceiveAddresses();
  const { data: profile } = useActiveProfile();
  const revealNext = useRevealNextReceiveAddress();
  const showToast = useUiStore((s) => s.showToast);
  const [copiedIdx, setCopiedIdx] = useState<number | null>(null);
  const [qrIdx, setQrIdx] = useState<number | null>(null);

  const handleCopy = async (address: string, index: number) => {
    await writeText(address);
    setCopiedIdx(index);
    showToast("Address copied", "success");
    setTimeout(() => setCopiedIdx(null), 2000);
  };

  const handleDerive = async () => {
    try {
      await revealNext.mutateAsync({ walletProfileId: profile?.id ?? null });
      showToast("New address generated", "success");
    } catch (e: unknown) {
      showToast(mapError(e, "build"), "error");
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
            <div key={row.index}>
              <div
                className="flex items-center gap-2 px-3 py-2 text-xs"
                data-testid={`addr-row-${row.index}`}
              >
                <span className="text-gray-400 w-6 text-right font-mono">{row.index}</span>
                <Tooltip content={row.address} className="flex-1 min-w-0">
                  <span className="font-mono text-gray-700 block truncate">
                    {truncateMiddle(row.address, 10, 8)}
                  </span>
                </Tooltip>
                <Badge variant={row.used ? "default" : "success"}>
                  {row.used ? "used" : "fresh"}
                </Badge>
                <Tooltip content={`First derived ${row.firstSeenAt}`}>
                  <span className="text-gray-400 font-mono" data-testid={`first-seen-${row.index}`}>
                    {formatDate(row.firstSeenAt)}
                  </span>
                </Tooltip>
                {profile?.network === "mainnet" && (
                  <Tooltip content="View on explorer">
                    <a
                      href={explorerAddressUrl(row.address)}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-blue-500 hover:underline"
                      aria-label="View on explorer"
                    >
                      ↗
                    </a>
                  </Tooltip>
                )}
                <Tooltip content="Show QR code">
                  <button
                    type="button"
                    className="text-blue-600 hover:underline"
                    onClick={() => setQrIdx(qrIdx === row.index ? null : row.index)}
                    data-testid={`qr-btn-${row.index}`}
                  >
                    {qrIdx === row.index ? "Hide" : "QR"}
                  </button>
                </Tooltip>
                <button
                  type="button"
                  className="text-blue-600 hover:underline"
                  onClick={() => handleCopy(row.address, row.index)}
                  data-testid={`copy-addr-${row.index}`}
                >
                  {copiedIdx === row.index ? "Copied" : "Copy"}
                </button>
              </div>
              {qrIdx === row.index && (
                <div
                  className="px-3 py-3 bg-gray-50 flex justify-center"
                  data-testid={`qr-display-${row.index}`}
                >
                  <QRCodeSVG value={row.address} size={160} level="H" includeMargin />
                </div>
              )}
            </div>
          ))}
        </div>
      )}
      <Button
        size="sm"
        variant="ghost"
        onClick={handleDerive}
        disabled={revealNext.isPending}
        data-testid="derive-next-address-btn"
      >
        {revealNext.isPending ? "Generating…" : "+ Generate new address"}
      </Button>
    </div>
  );
}
