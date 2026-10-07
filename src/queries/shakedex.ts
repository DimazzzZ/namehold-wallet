import { keepPreviousData, useQuery, useMutation } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type { ImportSource, MarketPage, MarketRow } from "../types";

/**
 * One page (from 1) of the LearnHNS Market listings, each verified against
 * the chain. `enabled` lets the caller hold the request back until the market
 * view is open. The previous page stays on screen while the next one loads.
 */
export function useMarketPage(enabled: boolean, page: number) {
  return useQuery<MarketPage>({
    queryKey: ["shakedex", "market", page],
    enabled,
    queryFn: () => invoke<MarketPage>("shakedex_list_market", { page }),
    placeholderData: keepPreviousData,
    retry: false,
  });
}

/** Import a listing from a file, pasted text or a market link. */
export function useImportListing() {
  return useMutation({
    mutationFn: (source: ImportSource) => invoke<MarketRow>("shakedex_import_listing", { source }),
  });
}
