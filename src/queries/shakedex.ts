import {
  keepPreviousData,
  useQuery,
  useMutation,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { useActiveProfile } from "./wallet";
import type {
  ImportSource,
  MarketPage,
  MarketRow,
  PurchasePreview,
  ShakedexListingMode,
  ShakedexListingSummary,
  ShakedexMyListing,
  ShakedexPendingSend,
  ShakedexScheduleInput,
  TxDraftSummary,
} from "../types";

/**
 * One page (from 1) of the LearnHNS Market listings, each verified against
 * the chain of the active profile's network and node, so the key carries the
 * profile (R7, per-wallet read isolation). The previous page of the same
 * profile stays on screen while the next one loads; another profile's never.
 */
export function useMarketPage(page: number) {
  const profileId = useActiveProfile().data?.id ?? null;
  return useQuery<MarketPage>({
    queryKey: ["shakedex", "market", profileId, page],
    enabled: profileId != null,
    queryFn: () => invoke<MarketPage>("shakedex_list_market", { page, walletProfileId: profileId }),
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[2] === profileId ? keepPreviousData(previous) : undefined,
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

// --- Selling (T7) ---

/** `shakedex_list_listings` of one profile: its listings, pinned to it. */
const listingsQuery = (profileId: string | null) => ({
  queryKey: ["shakedex", "listings", profileId],
  queryFn: () =>
    invoke<ShakedexMyListing[]>("shakedex_list_listings", { walletProfileId: profileId }),
});

/**
 * Every listing of the active profile, with what each can do now
 * (`shakedex_list_listings`). The read names the profile and the key carries
 * it (R7, per-wallet read isolation): after a switch another profile's rows
 * are never shown, even from a read that lands late.
 */
export function useMyListings(enabled = true) {
  const profileId = useActiveProfile().data?.id ?? null;
  return useQuery<ShakedexMyListing[]>({
    ...listingsQuery(profileId),
    enabled: enabled && profileId != null,
    retry: false,
    refetchInterval: 30_000,
  });
}

/**
 * After a failed broadcast: the My listings row that still offers draft
 * `draftId` to send, from a fresh read of the listings (which also brings
 * every row up to date), or null when none does — the node refused it, or
 * the backend deleted it with its listing (R31).
 */
export function useKeptDraft() {
  const qc = useQueryClient();
  const profileId = useActiveProfile().data?.id ?? null;
  return async (draftId: string): Promise<ShakedexPendingSend | null> => {
    try {
      const rows = await qc.fetchQuery({ ...listingsQuery(profileId), staleTime: 0 });
      return rows?.find((r) => r.pendingSend?.draftId === draftId)?.pendingSend ?? null;
    } catch {
      return null;
    }
  };
}

/** `wallet_tx_drafts.action` of a purchase draft (`purchase.rs::PURCHASE_ACTION`). */
export const PURCHASE_ACTION = "shakedex_purchase";

/**
 * What a purchase draft's send or discard changes besides the drafts: its
 * purchase record, which Owned Names ("purchase pending") and the market
 * page read. Called however the send or discard ended — a failed send may
 * have deleted the draft and its record, or kept them broadcast_pending.
 */
export function invalidatePurchaseViews(qc: QueryClient) {
  qc.invalidateQueries({ queryKey: ["read"] });
  qc.invalidateQueries({ queryKey: ["shakedex"] });
}

/** A selling command: listings, drafts and names change together. */
function useSellingMutation<A, R>(fn: (a: A) => Promise<R>) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: fn,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["shakedex"] });
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
    },
  });
}

export interface BuildLockArgs {
  name: string;
  mode: ShakedexListingMode;
  publish: boolean;
  feeRate: number | null;
}

/** Stage the day-0 lock TRANSFER of a name (R19: the mode and the publish box). */
export const useBuildLock = () =>
  useSellingMutation((a: BuildLockArgs) =>
    invoke<TxDraftSummary>("shakedex_build_lock_draft", { ...a }),
  );

export interface FinalizeAndSignArgs {
  listingId: string;
  /** A Buy Now's one price; empty for a reverse auction. */
  prices: { price: string }[];
  /** A reverse auction's schedule; null for a Buy Now. */
  schedule: ShakedexScheduleInput | null;
  feeRate: number | null;
}

/** Finalize into the lock and sign the prices (R20); the secure window asks first. */
export const useFinalizeAndSign = () =>
  useSellingMutation((a: FinalizeAndSignArgs) =>
    invoke<ShakedexListingSummary>("shakedex_finalize_and_sign", { ...a }),
  );

/** Stage and sign the cancel of a listing (R28); the secure window asks first. */
export const useCancelListing = () =>
  useSellingMutation((a: { listingId: string; feeRate: number | null }) =>
    invoke<TxDraftSummary>("shakedex_cancel_listing", { ...a }),
  );

/** Stage the FINALIZE that brings a cancelled listing's name home (R28). */
export const useFinalizeCancel = () =>
  useSellingMutation((a: { listingId: string; feeRate: number | null }) =>
    invoke<TxDraftSummary>("shakedex_finalize_cancel", { ...a }),
  );

/** Sign one more, lower price (R26); the price as typed, the secure window asks first. */
export const useLowerPrice = () =>
  useSellingMutation((a: { listingId: string; price: string }) =>
    invoke<ShakedexListingSummary>("shakedex_lower_price", { ...a }),
  );

/** Restore a lock of ours by the name's chain state (R32). */
export const useRestoreLock = () =>
  useSellingMutation((name: string) =>
    invoke<ShakedexListingSummary>("shakedex_restore_lock", { name }),
  );

/** Import our own listing file, pasted or by its path (the backend reads the file). */
export const useImportOwnListing = () =>
  useSellingMutation((s: { kind: "text"; text: string } | { kind: "file"; path: string }) =>
    s.kind === "text"
      ? invoke<ShakedexListingSummary>("shakedex_import_own_listing_file", { text: s.text })
      : invoke<ShakedexListingSummary>("shakedex_import_own_listing_from_file", { path: s.path }),
  );

/** Save a listing's stored file at the path the save dialog gave. Changes nothing stored. */
export const useSaveListingFile = () =>
  useMutation({
    mutationFn: (a: { listingId: string; path: string }) =>
      invoke<void>("shakedex_save_listing_file", { ...a }),
  });

/** Turn publishing on for a Listed listing (carry 1; never off). */
export const usePublishListing = () =>
  useSellingMutation((listingId: string) =>
    invoke<ShakedexListingSummary>("shakedex_publish_listing", { listingId }),
  );
