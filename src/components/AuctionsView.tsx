import { useState } from "react";
import { useWriteCapability, useActiveProfile, useTxDrafts } from "../queries/wallet";
import { useReadNames, useNamesActionCapabilities } from "../queries/read";
import {
  auctionPhase,
  taskSummaryFromCapabilities,
  taskStateUrgencyRank,
  formatCountdown,
  type AuctionTaskSummary,
} from "../lib/auction";
import { NameActionsModal } from "./NameActionsModal";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { PageHeader } from "./ui/PageHeader";
import { normalizeNameInput } from "../lib/utils";
import { displayName } from "../lib/idn";
import type { HsdName, NameActionCapabilities } from "../types";

/**
 * Auctions page — wallet-first entry point for acquiring new Handshake TLDs.
 *
 * Shows:
 *  1. A simple name lookup field to start an auction for any name.
 *  2. The user's actionable auction tasks (including won/lost outcomes) so they
 *     can track progress and act (reveal, register, redeem) without leaving.
 */
export function AuctionsView() {
  const { data: writeCap } = useWriteCapability();
  const { data: names = [] } = useReadNames();
  // Guard against a resolved-but-null query result (e.g. a test double that
  // doesn't implement `list_tx_drafts` returns `null`, not `undefined`, so
  // the `= []` destructuring default alone wouldn't catch it).
  const { data: draftsData } = useTxDrafts();
  const drafts = draftsData ?? [];
  // Resolve the active wallet once here (not inside the inline TaskRow, which
  // remounts every render and would trigger a profile-refetch storm) so every
  // capability fetch is pinned to this wallet.
  const activeProfileId = useActiveProfile().data?.id ?? null;

  const canWrite = writeCap?.canWrite ?? false;

  const [lookupName, setLookupName] = useState("");
  const [manageName, setManageName] = useState<string | null>(null);

  // Names that are mid-auction or have actionable post-auction tasks.
  // We fetch capabilities for each name to determine the task state.
  const activeTasks = names.filter((n) => {
    const { phase } = auctionPhase(n.state);
    if (phase === "OPENING" || phase === "BIDDING" || phase === "REVEAL" || phase === "TRANSFER") return true;
    // CLOSED names: only include as a candidate when `registered` is
    // EXPLICITLY false (recently won, not yet registered). `registered`
    // being undefined/absent means "unknown, likely already registered"
    // (e.g. explorer-sourced data), so it must NOT be treated the same as
    // an explicit false — otherwise genuinely-owned names leak back into
    // Active Auctions. Mirrors WalletView's "won" alert gate.
    if (phase === "CLOSED") return n.owner != null && n.registered === false;
    return false;
  });

  // Just-broadcast OPEN drafts, surfaced as synthetic rows so a freshly
  // opened name doesn't sit invisible until the next sync tracks it as
  // OPENING (`useReadNames` only reflects names already in local state).
  // Deduped against `activeTasks` by name — once the name is genuinely
  // tracked (e.g. synced into OPENING), the real row wins and the synthetic
  // one disappears, so there's never a double entry for the same name.
  const activeTaskNames = new Set(activeTasks.map((n) => n.name));
  const pendingOpenNames = Array.from(
    new Set(
      drafts
        .filter(
          (d) =>
            d.action === "open" &&
            (d.status === "signed" ||
              d.status === "broadcast_pending" ||
              d.status === "broadcasted") &&
            !!d.summary?.name,
        )
        .map((d) => d.summary!.name as string),
    ),
  ).filter((name) => !activeTaskNames.has(name));

  // ONE batch capability fetch for the whole list (F5 fix — this used to be
  // an N+1 invoke, one `get_name_action_capabilities` call per row, fired
  // from a per-row component that remounted every render).
  const { data: capsList = [] } = useNamesActionCapabilities(
    activeTasks.map((n) => n.name),
    activeProfileId,
  );
  const capsByName = new Map<string, NameActionCapabilities>(
    capsList.map((c) => [c.name, c]),
  );

  // Sort by urgency: readyToReveal → wonNeedsRegister → lostNeedsRedeem →
  // expiringSoon → everything else, then by soonest countdown within a tier,
  // so a time-critical row (e.g. reveal window closing) can never be buried
  // under a long list of "waiting" rows.
  const sortedTasks = [...activeTasks].sort((a, b) => {
    const capsA = capsByName.get(a.name);
    const capsB = capsByName.get(b.name);
    const rankA = capsA ? taskStateUrgencyRank(capsA.taskState) : 4;
    const rankB = capsB ? taskStateUrgencyRank(capsB.taskState) : 4;
    if (rankA !== rankB) return rankA - rankB;
    const blocksA = capsA?.countdownBlocks ?? Number.POSITIVE_INFINITY;
    const blocksB = capsB?.countdownBlocks ?? Number.POSITIVE_INFINITY;
    return blocksA - blocksB;
  });

  const handleLookup = () => {
    const trimmed = lookupName.trim();
    if (trimmed) {
      setManageName(trimmed);
    }
  };

  const handleOpenManagement = (name: string) => {
    setManageName(name);
  };

  // Render a task row from the pre-fetched batch capabilities (no per-row
  // fetch — see `capsByName` above).
  const TaskRow = ({ name: n }: { name: HsdName }) => {
    const summary: AuctionTaskSummary | null = taskSummaryFromCapabilities(
      capsByName.get(n.name),
    );

    // Use capabilities when available; fall back to phase only as last resort.
    const displayLabel = summary?.label ?? auctionPhase(n.state).label;
    const displayVariant = summary?.variant ?? auctionPhase(n.state).variant;
    const nextLabel = summary?.nextActionLabel ?? auctionPhase(n.state).label;

    // Countdown column (F5 fix — this used to show the raw sync height,
    // which tells the user nothing about how much time an action has left).
    const countdownText =
      summary?.countdownBlocks != null
        ? formatCountdown({
            label: summary.countdownLabel ?? "",
            blocks: summary.countdownBlocks,
            hours: summary.countdownHours,
          })
        : null;

    return (
      <tr key={n.name} className="border-t border-gray-100">
        <td className="py-1 font-mono">.{displayName(n.name)}</td>
        <td className="py-1">
          <Badge variant={displayVariant}>{displayLabel}</Badge>
        </td>
        <td className="py-1 text-xs text-gray-500" title={summary?.countdownLabel ?? undefined}>
          {countdownText ?? "—"}
        </td>
        <td className="py-1 text-right">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => handleOpenManagement(n.name)}
          >
            {nextLabel === "Wait for Bidding" || nextLabel === "Owned" ? "View" : nextLabel}
          </Button>
        </td>
      </tr>
    );
  };

  // A just-broadcast OPEN with no tracked state yet — the modal's Open button
  // is already correctly disabled (Task 1's `canOpen`/`taskState` reflect the
  // pending open), so "View" just routes there with the RAW name.
  const PendingOpenRow = ({ name }: { name: string }) => (
    <tr key={name} className="border-t border-gray-100">
      <td className="py-1 font-mono">.{displayName(name)}</td>
      <td className="py-1">
        <Badge variant="warning">Opening — pending confirmation</Badge>
      </td>
      <td className="py-1 text-xs text-gray-500">—</td>
      <td className="py-1 text-right">
        <Button size="sm" variant="ghost" onClick={() => handleOpenManagement(name)}>
          View
        </Button>
      </td>
    </tr>
  );

  const totalActiveCount = activeTasks.length + pendingOpenNames.length;

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

      {/* Actionable auction tasks */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <div className="text-sm text-gray-500 mb-2">
          Active Auctions ({totalActiveCount})
        </div>
        {totalActiveCount > 0 ? (
          <div className="max-h-60 overflow-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="py-1">Name</th>
                  <th className="py-1">Task</th>
                  <th className="py-1">Countdown</th>
                  <th className="py-1"></th>
                </tr>
              </thead>
              <tbody>
                {sortedTasks.map((n) => (
                  <TaskRow key={n.name} name={n} />
                ))}
                {/* Pending-opens are not urgent (nothing to act on yet — the
                    Open button is already correctly disabled in the modal),
                    so they sit below the real, actionable tasks. */}
                {pendingOpenNames.map((name) => (
                  <PendingOpenRow key={name} name={name} />
                ))}
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
          onClose={() => {
            setManageName(null);
          }}
        />
      )}
    </div>
  );
}
