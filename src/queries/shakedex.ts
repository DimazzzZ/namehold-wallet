import { keepPreviousData, useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import type {
  ImportSource,
  MarketPage,
  MarketRow,
  PurchasePreview,
  TxDraftSummary,
} from "../types";

/**
 * One page (from 1) of the LearnHNS Market listings, each verified against
 * the chain. The previous page stays on screen while the next one loads.
 */
export function useMarketPage(page: number) {
  return useQuery<MarketPage>({
    queryKey: ["shakedex", "market", page],
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

/**
 * Price breakdown of buying a listing. Idle until a listing is given.
 * `feeRate` is doos per byte (as `parseFeeRateArg` returns it); null lets the
 * backend pick its default.
 */
export function usePurchasePreview(
  listingJson: string | null,
  payMarketFee: boolean,
  fromMarket: boolean,
  feeRate: number | null,
) {
  return useQuery<PurchasePreview>({
    queryKey: ["shakedex", "preview", listingJson, payMarketFee, fromMarket, feeRate],
    enabled: listingJson !== null,
    // Toggling the fee changes the key; keep the rows on screen meanwhile.
    placeholderData: keepPreviousData,
    queryFn: () =>
      invoke<PurchasePreview>("shakedex_preview_purchase", {
        listingJson,
        payMarketFee,
        fromMarket,
        feeRate,
      }),
    retry: false,
  });
}

export interface BuildPurchaseArgs {
  listingJson: string;
  /** The market fee the user accepted, in doos; null = do not pay it. */
  acceptedMarketFeeDoos: number | null;
  fromMarket: boolean;
  feeRate: number | null;
}

/** Stage the purchase as a transaction draft. */
export function useBuildPurchase() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: BuildPurchaseArgs) =>
      invoke<TxDraftSummary>("shakedex_build_purchase_draft", { ...args }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["wallet"] }),
  });
}

export interface BuildPurchaseFinalizeArgs {
  purchaseId: string;
  feeRate: number | null;
}

/** Stage the finalize of a purchase whose transfer lockup is over. */
export function useBuildPurchaseFinalize() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: BuildPurchaseFinalizeArgs) =>
      invoke<TxDraftSummary>("shakedex_build_purchase_finalize_draft", { ...args }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
    },
  });
}
