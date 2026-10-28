import { useMemo } from "react";
import type { NameActionCapabilities } from "../types";
import { needsYouItems, type NeedsYouItem } from "../lib/needsYou";
import { finalizeSentIds } from "../components/market/usePurchase";
import { useActiveProfile, useTxDrafts } from "./wallet";
import {
  useAuctionPositions,
  useNamesActionCapabilities,
  useReadNames,
  useReadRenewals,
} from "./read";
import { useNodeLive } from "./node";

/**
 * The active wallet's Needs you list, plus the ONE batch capability fetch it
 * is built from — WalletView reuses `capsByName` for the Owned Names table so
 * the page never fetches capabilities twice.
 */
export function useNeedsYou(): {
  items: NeedsYouItem[];
  capsByName: Map<string, NameActionCapabilities>;
  isLoading: boolean;
  isDegraded: boolean;
} {
  const { data: profile } = useActiveProfile();
  const { data: names = [] } = useReadNames();
  const { data: positionNames = [] } = useAuctionPositions(profile?.id ?? null);
  const { data: renewals } = useReadRenewals();
  const { data: drafts = [] } = useTxDrafts();
  const nodeLive = useNodeLive();
  const isWatchOnly = profile?.watchOnly ?? false;

  // Positions ride along so a reveal due on a name we bid on but don't own is
  // a task too (the AuctionsView dedupe). Purchases are not ours yet: no caps.
  const owned = new Set(names.map((n) => n.name));
  const capNames = isWatchOnly
    ? []
    : [
        ...names.filter((n) => !n.shakedex).map((n) => n.name),
        ...positionNames.filter((n) => !owned.has(n)),
      ];
  const capsQuery = useNamesActionCapabilities(capNames, profile?.id ?? null);
  const caps = capsQuery.data;

  const capsByName = useMemo(
    () => new Map<string, NameActionCapabilities>((caps ?? []).map((c) => [c.name, c])),
    [caps],
  );

  const items = useMemo(
    () =>
      needsYouItems({
        isWatchOnly: isWatchOnly || !profile,
        network: profile?.network ?? null,
        names,
        caps: caps ?? [],
        renewals: renewals?.names ?? [],
        finalizeSent: finalizeSentIds(drafts),
      }),
    [isWatchOnly, profile, names, caps, renewals, drafts],
  );

  return {
    items,
    capsByName,
    isLoading: capNames.length > 0 && caps === undefined && !capsQuery.isError,
    isDegraded: capsQuery.isError && !nodeLive,
  };
}
