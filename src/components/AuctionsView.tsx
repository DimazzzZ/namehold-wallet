import { useState } from "react";
import { useWriteCapability } from "../queries/wallet";
import { useReadNames } from "../queries/read";
import { auctionPhase, recommendedAction } from "../lib/auction";
import { NameActionsModal } from "./NameActionsModal";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { PageHeader } from "./ui/PageHeader";
import { normalizeNameInput } from "../lib/utils";

/**
 * Auctions page — wallet-first entry point for acquiring new Handshake TLDs.
 *
 * Shows:
 *  1. A simple name lookup field to start an auction for any name.
 *  2. The user's in-flight auctions (OPENING / BIDDING / REVEAL) so they
 *     can track progress and act (reveal, register) without leaving the page.
 */
export function AuctionsView() {
  const { data: writeCap } = useWriteCapability();
  const { data: names = [] } = useReadNames();

  const canWrite = writeCap?.canWrite ?? false;

  const [lookupName, setLookupName] = useState("");
  const [manageName, setManageName] = useState<string | null>(null);

  // Names that are mid-auction or need a follow-up action.
  const activeAuctions = names.filter((n) => {
    const { phase } = auctionPhase(n.state);
    return phase === "OPENING" || phase === "BIDDING" || phase === "REVEAL";
  });

  const handleLookup = () => {
    const trimmed = lookupName.trim();
    if (trimmed) setManageName(trimmed);
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Auctions"
        subtitle="Acquire new Handshake TLDs through the Vickrey auction system."
      />

      {/* Name lookup — the primary action on this page */}
      <div className="bg-white rounded-lg p-6 border-2 border-blue-200 space-y-3">
        <div className="text-sm font-medium text-gray-900">Get a TLD</div>
        <div className="text-xs text-gray-500">
          Type any name to check availability and start an auction.
        </div>
        <div className="flex items-center gap-2">
          <div className="flex items-center">
            <span className="text-gray-400 text-sm mr-1">.</span>
            <input
              className="border border-gray-300 rounded px-2 py-1.5 text-sm w-48"
              value={lookupName}
              onChange={(e) => setLookupName(normalizeNameInput(e.target.value))}
              placeholder="example"
              onKeyDown={(e) => {
                if (e.key === "Enter" && lookupName.trim()) handleLookup();
              }}
            />
          </div>
          <Button
            size="sm"
            variant="primary"
            disabled={!lookupName.trim()}
            onClick={handleLookup}
          >
            Look up
          </Button>
        </div>
        {!canWrite && (
          <div className="text-xs text-amber-600">
            {writeCap?.reason ??
              "Connect a node in Settings, Refresh to sync your coins, then unlock to bid."}
          </div>
        )}
      </div>

      {/* In-flight auctions */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <div className="text-sm text-gray-500 mb-2">
          Active Auctions ({activeAuctions.length})
        </div>
        {activeAuctions.length > 0 ? (
          <div className="max-h-60 overflow-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="py-1">Name</th>
                  <th className="py-1">Phase</th>
                  <th className="py-1">Height</th>
                  <th className="py-1"></th>
                </tr>
              </thead>
              <tbody>
                {activeAuctions.map((n) => {
                  const phase = auctionPhase(n.state);
                  return (
                    <tr key={n.name} className="border-t border-gray-100">
                      <td className="py-1 font-mono">.{n.name}</td>
                      <td className="py-1">
                        <Badge variant={phase.variant}>{phase.label}</Badge>
                      </td>
                      <td className="py-1 text-xs text-gray-500">
                        {n.height ? `#${n.height}` : "—"}
                      </td>
                      <td className="py-1 text-right">
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => setManageName(n.name)}
                        >
                          {recommendedAction(n.state)?.label ?? "Manage"}
                        </Button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : (
          <div className="text-gray-400 text-sm py-4 text-center">
            No active auctions. Look up a name above to get started.
          </div>
        )}
      </div>

      {/* Name actions modal — reused for the full auction lifecycle */}
      {manageName && (
        <NameActionsModal
          name={manageName}
          open={!!manageName}
          onClose={() => setManageName(null)}
        />
      )}
    </div>
  );
}
