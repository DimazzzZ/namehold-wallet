/**
 * The HNS ⇄ BTC exchange's commands (spec §9), one hook each. Every read is
 * keyed by the active profile, since every command acts on it: another
 * profile's trades are never shown. The order book and the trades poll while
 * the Exchange page is mounted, so a counterparty's step shows up by itself.
 *
 * The argument names on the wire are camelCase: Tauri 2 maps them onto the
 * commands' snake_case parameters (`trade_id` ← `tradeId`), see
 * `invoke-arg-names.contract.test.ts`.
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { useActiveProfile } from "./wallet";
import type {
  BtcBalance,
  BtcWithdrawResult,
  ExchangeOverview,
  MarketOffers,
  OfferRow,
  OfferSide,
  TradeRow,
} from "../types/exchange";

/** How often the order book and the trades are read again while the page is open. */
export const EXCHANGE_POLL_MS = 15_000;

/** Every exchange read of one profile sits under this key. */
/** The first part of every exchange query key: invalidate it to reread the page for every profile. */
export const EXCHANGE_QUERY_ROOT = "exchange";
export const exchangeKey = (profileId: string | null) => [EXCHANGE_QUERY_ROOT, profileId] as const;

function useProfileId(): string | null {
  return useActiveProfile().data?.id ?? null;
}

export function useExchangeOverview() {
  const profileId = useProfileId();
  return useQuery({
    queryKey: [...exchangeKey(profileId), "overview"],
    enabled: profileId != null,
    queryFn: () => invoke<ExchangeOverview>("exchange_overview"),
    retry: false,
  });
}

/** The BTC account's balance. Idle until the exchange is enabled (no account before). */
export function useBtcBalance(enabled: boolean) {
  const profileId = useProfileId();
  return useQuery({
    queryKey: [...exchangeKey(profileId), "btcBalance"],
    enabled: profileId != null && enabled,
    queryFn: () => invoke<BtcBalance>("exchange_btc_balance"),
    retry: false,
  });
}

/** The relay's order book. Read whenever the page is open: browsing needs no keys. */
export function useMarketOffers(enabled: boolean) {
  const profileId = useProfileId();
  return useQuery({
    queryKey: [...exchangeKey(profileId), "offers"],
    enabled: profileId != null && enabled,
    queryFn: () => invoke<MarketOffers>("exchange_list_offers"),
    refetchInterval: EXCHANGE_POLL_MS,
    retry: false,
  });
}

export function useMyOffers(enabled: boolean) {
  const profileId = useProfileId();
  return useQuery({
    queryKey: [...exchangeKey(profileId), "myOffers"],
    enabled: profileId != null && enabled,
    queryFn: () => invoke<OfferRow[]>("exchange_my_offers"),
    retry: false,
  });
}

export function useTrades(enabled: boolean) {
  const profileId = useProfileId();
  return useQuery({
    queryKey: [...exchangeKey(profileId), "trades"],
    enabled: profileId != null && enabled,
    queryFn: () => invoke<TradeRow[]>("exchange_list_trades"),
    refetchInterval: EXCHANGE_POLL_MS,
    retry: false,
  });
}

/** Invalidate some of the active profile's exchange reads. */
function useInvalidate() {
  const qc = useQueryClient();
  const profileId = useProfileId();
  return (...parts: string[]) => {
    for (const part of parts) {
      void qc.invalidateQueries({ queryKey: [...exchangeKey(profileId), part] });
    }
  };
}

/** Store the BTC account's and the swap branch's public keys. */
export function useEnableExchange() {
  const qc = useQueryClient();
  const profileId = useProfileId();
  return useMutation({
    mutationFn: () => invoke<ExchangeOverview>("exchange_enable"),
    onSuccess: (overview) => {
      qc.setQueryData([...exchangeKey(profileId), "overview"], overview);
      void qc.invalidateQueries({ queryKey: exchangeKey(profileId) });
    },
  });
}

export interface ExchangeSettingsArgs {
  relayUrl: string;
  /** "" = no watchtower. */
  watchtowerUrl: string;
  btcSources: string[];
  /** null = keep the stored one, "" = clear it. */
  bitcoindAuth: string | null;
}

/** Store the relay, watchtower and Bitcoin sources (the secure window confirms it). */
export function useUpdateExchangeSettings() {
  const qc = useQueryClient();
  const profileId = useProfileId();
  return useMutation({
    mutationFn: ({ relayUrl, watchtowerUrl, btcSources, bitcoindAuth }: ExchangeSettingsArgs) =>
      invoke<ExchangeOverview>("exchange_update_settings", {
        relayUrl,
        watchtowerUrl,
        btcSources,
        bitcoindAuth,
      }),
    onSuccess: (overview) => {
      qc.setQueryData([...exchangeKey(profileId), "overview"], overview);
      // A new relay or new sources answer differently.
      void qc.invalidateQueries({ queryKey: exchangeKey(profileId) });
    },
  });
}

export interface BtcWithdrawArgs {
  toAddress: string;
  /** null = everything. */
  amountSats: number | null;
}

/** Send BTC out of the BTC account (the secure window confirms it). */
export function useBtcWithdraw() {
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: ({ toAddress, amountSats }: BtcWithdrawArgs) =>
      invoke<BtcWithdrawResult>("exchange_btc_withdraw", { toAddress, amountSats }),
    onSuccess: () => invalidate("btcBalance"),
  });
}

export interface CreateOfferArgs {
  side: OfferSide;
  price: string;
  minHns: string;
  maxHns: string;
}

/** Publish an offer (the secure window confirms it). */
export function useCreateOffer() {
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: ({ side, price, minHns, maxHns }: CreateOfferArgs) =>
      invoke<OfferRow>("exchange_create_offer", { side, price, minHns, maxHns }),
    onSuccess: () => invalidate("myOffers", "offers"),
  });
}

export function useWithdrawOffer() {
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: (offerId: string) => invoke<null>("exchange_withdraw_offer", { offerId }),
    onSuccess: () => invalidate("myOffers", "offers"),
  });
}

export interface TakeOfferArgs {
  offerId: string;
  hnsAmount: string;
}

/** Take an offer: nothing is signed; the Commit asks later. */
export function useTakeOffer() {
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: ({ offerId, hnsAmount }: TakeOfferArgs) =>
      invoke<TradeRow>("exchange_take_offer", { offerId, hnsAmount }),
    onSuccess: () => invalidate("trades", "offers"),
  });
}

/** This party's one Commit (the secure window confirms it). */
export function useCommitTrade() {
  const qc = useQueryClient();
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: (tradeId: string) => invoke<TradeRow>("exchange_commit", { tradeId }),
    onSuccess: () => {
      invalidate("trades", "btcBalance");
      // The Seller's Commit spends HNS from the wallet.
      void qc.invalidateQueries({ queryKey: ["wallet"] });
    },
  });
}

export function useCancelTrade() {
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: (tradeId: string) => invoke<TradeRow>("exchange_cancel_trade", { tradeId }),
    onSuccess: () => invalidate("trades"),
  });
}

/** Run one engine step for every trade now; its answer is the trade list. */
export function useRefreshTrades() {
  const qc = useQueryClient();
  const profileId = useProfileId();
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: () => invoke<TradeRow[]>("exchange_refresh"),
    onSuccess: (trades) => {
      qc.setQueryData([...exchangeKey(profileId), "trades"], trades);
      invalidate("offers", "myOffers", "btcBalance");
    },
  });
}
