import { useState } from "react";
import { useWriteCapability, useActiveProfile } from "../queries/wallet";
import { useReadNames, useNamesActionCapabilities, useAuctionPositions } from "../queries/read";
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
import { explorerNameUrl, openExternal } from "../lib/openExternal";
import type { HsdName, NameActionCapabilities, AuctionTaskState } from "../types";

/**
 * Live task states that count as "still in the auction" for a position name
 * (one with a draft/commitment but no owner coin). A position is only ever
 * DISPLAYED when its current capabilities land in this set — this is what
 * makes the list self-cleaning: a won+registered name flips to `ownsName`
 * (excluded upstream by the backend before it even becomes a position) and a
 * lost+redeemed one settles into `unavailableOther`/no-caps, so both quietly
 * drop off without any extra bookkeeping here.
 */
const ACTIVE_POSITION_TASK_STATES = new Set<AuctionTaskState>([
  "availableToOpen",
  "waitingForBidding",
  "readyToBid",
  "readyToReveal",
  // Keep the row visible while the reveal is in flight and after it confirms
  // (until the auction closes), so a successful reveal never makes the row
  // silently vanish — the exact "where did it go?" confusion in another form.
  "revealBroadcastPending",
  "revealDoneWaitingForClose",
  "wonNeedsRegister",
  "lostNeedsRedeem",
]);

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
  // Resolve the active wallet once here (not inside the inline TaskRow, which
  // remounts every render and would trigger a profile-refetch storm) so every
  // capability fetch is pinned to this wallet.
  const activeProfile = useActiveProfile().data ?? null;
  const activeProfileId = activeProfile?.id ?? null;
  const isMainnet = activeProfile?.network === "mainnet";
  const { data: positionNames = [] } = useAuctionPositions(activeProfileId);

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

  // Auction positions (open/bid/reveal drafts + bid commitments) that aren't
  // already owned. Dedup against ALL owned names (not just `activeTasks`) so
  // a won+registered name that's still lingering in the positions table
  // (backend hasn't caught up) never double-shows: it's already present via
  // `activeTasks`/owned, and this is purely additive for names not otherwise
  // tracked yet (e.g. a confirmed OPEN not yet synced into OPENING).
  const ownedNames = new Set(names.map((n) => n.name));
  const dedupedPositionNames = positionNames.filter((n) => !ownedNames.has(n));

  // ONE batch capability fetch for the whole list (F5 fix — this used to be
  // an N+1 invoke, one `get_name_action_capabilities` call per row, fired
  // from a per-row component that remounted every render). Positions ride
  // along in the SAME batch so they get real phase/taskState/countdown —
  // this is what lets a position row be display-filtered by live caps below,
  // instead of showing a floor "pending confirmation" placeholder forever.
  const { data: capsList = [] } = useNamesActionCapabilities(
    [...activeTasks.map((n) => n.name), ...dedupedPositionNames],
    activeProfileId,
  );
  const capsByName = new Map<string, NameActionCapabilities>(
    capsList.map((c) => [c.name, c]),
  );

  // Only show a position once its live caps land in an active-auction task
  // state — this is what makes the list self-clean by lifecycle (a won and
  // registered name is excluded upstream by the backend before it's even a
  // position; a lost-and-redeemed one settles into `unavailableOther`/no
  // caps and quietly drops here) rather than needing separate bookkeeping.
  const positionRows: HsdName[] = dedupedPositionNames
    .filter((name) => {
      const caps = capsByName.get(name);
      return caps != null && ACTIVE_POSITION_TASK_STATES.has(caps.taskState);
    })
    .map((name) => ({
      name,
      state: null,
      height: null,
      renewal: null,
      owner: null,
      stats: null,
      registered: false,
    }));

  const allTasks = [...activeTasks, ...positionRows];

  // Sort by urgency: readyToReveal → wonNeedsRegister → lostNeedsRedeem →
  // expiringSoon → everything else, then by soonest countdown within a tier,
  // so a time-critical row (e.g. reveal window closing) can never be buried
  // under a long list of "waiting" rows.
  const sortedTasks = [...allTasks].sort((a, b) => {
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

    // A confirmed-open position whose live caps haven't caught up to a real
    // phase yet (node/explorer not synced) still reads as `availableToOpen`
    // from the capability model — but this row only exists because we KNOW
    // an open is already in flight for it, so the label/button must never
    // read as "not yet started" / invite a re-open ("Open Auction").
    const isPendingPhaseOpen = summary?.taskState === "availableToOpen";

    // Use capabilities when available; fall back to phase only as last resort.
    const displayLabel = isPendingPhaseOpen
      ? "In auction"
      : (summary?.label ?? auctionPhase(n.state).label);
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
        <td className="py-1 font-mono">
          {isMainnet ? (
            <button
              type="button"
              className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
              onClick={() => openExternal(explorerNameUrl(n.name))}
              title="View on explorer"
              data-testid="auction-name-explorer-link"
            >
              .{displayName(n.name)}
            </button>
          ) : (
            `.${displayName(n.name)}`
          )}
        </td>
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
            {isPendingPhaseOpen ||
            nextLabel === "Wait for Bidding" ||
            nextLabel === "Owned" ||
            // Reveal in-flight / done-waiting are passive: the row just opens
            // the modal to show the pending/done card, no inline action.
            summary?.taskState === "revealBroadcastPending" ||
            summary?.taskState === "revealDoneWaitingForClose"
              ? "View"
              : nextLabel}
          </Button>
        </td>
      </tr>
    );
  };

  const totalActiveCount = allTasks.length;

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
