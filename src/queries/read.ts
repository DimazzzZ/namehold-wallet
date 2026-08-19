import { useQuery, useMutation, useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { normalizeTransaction } from "../lib/providerMode";
import { useActiveProfile } from "./wallet";
import { useNodeLive } from "./node";
import type { ActionRow } from "../lib/zod";
import type {
  BlockInfo,
  HsdBalance,
  HsdName,
  NameActionCapabilities,
  NameBids,
  NameResource,
  ReceiveAddressRow,
  RecoveredBidCommitment,
  RenewalsResponse,
  TxInfo,
  TxInfoError,
  WalletTransactionRow,
} from "../types";

/**
 * Read query layer (explorer-backed, node-free). Balance + names come from the
 * HNSFans explorer over the active profile's addresses; transactions from the
 * local cache. Writes are never routed through here.
 */

const STALE_TIME = 15_000;

/**
 * Per-wallet balance, keyed by the active profile id so wallet B never
 * momentarily shows wallet A's number.
 *
 * Freshness: refetches on mount and — while the node is live — polls every 20s
 * so the displayed balance tracks the local chain cache (kept current by the
 * background sync). When the node is not live, polling is disabled and the
 * last-known value (persisted in the chain cache, survives restart) is shown
 * until the next sync/Refresh.
 */
export function useReadBalance(): UseQueryResult<HsdBalance | null> {
  const { data: profile } = useActiveProfile();
  const profileId = profile?.id ?? null;
  const nodeLive = useNodeLive();
  return useQuery<HsdBalance | null>({
    queryKey: ["read", "balance", profileId],
    enabled: profileId != null,
    queryFn: async () => {
      // Pin the read to THIS wallet so a fetch can never return another
      // profile's balance (the active profile may flip mid-switch).
      const raw = await invoke<HsdBalance | null>("read_balance", {
        walletProfileId: profileId,
      });
      return raw ?? null;
    },
    staleTime: STALE_TIME,
    refetchInterval: nodeLive ? 20_000 : false,
  });
}

/** Provider-aware list of owned / watched names, pinned to the active wallet. */
export function useReadNames(): UseQueryResult<HsdName[]> {
  const profileId = useActiveProfile().data?.id ?? null;
  return useQuery<HsdName[]>({
    queryKey: ["read", "names", profileId],
    enabled: profileId != null,
    queryFn: async () => {
      const raw = await invoke<HsdName[] | null>("read_names", {
        walletProfileId: profileId,
      });
      return Array.isArray(raw) ? raw : [];
    },
    staleTime: STALE_TIME,
  });
}

/**
 * Full wallet action history — every tx touching any derived address of the
 * active wallet, classified into Send / Receive / OPEN / BID / REVEAL /
 * REDEEM / REGISTER / UPDATE / RENEW / TRANSFER / FINALIZE / REVOKE.
 *
 * Requires a synced hsd node with `--index-tx` and `--index-address`. When the
 * node lacks the address index the backend rejects with an error message
 * containing "address index not enabled" — the caller can surface a
 * dedicated banner (see `ActivityView`).
 */
export function useActionHistory(): UseQueryResult<ActionRow[]> {
  const profileId = useActiveProfile().data?.id ?? null;
  return useQuery<ActionRow[]>({
    queryKey: ["read", "action_history", profileId],
    enabled: profileId != null,
    queryFn: async () => {
      const raw = await invoke<ActionRow[] | null>("read_action_history", {
        walletProfileId: profileId,
      });
      return Array.isArray(raw) ? raw : [];
    },
    // Don't hammer the node on every mount — the History view has an explicit
    // Refresh button and query-key invalidation from Send/broadcast covers the
    // "I just did a thing" case.
    staleTime: STALE_TIME,
  });
}

/**
 * Names with an open auction position for this wallet (open/bid/reveal draft
 * in signed/broadcast_pending/broadcasted/confirmed status, OR a bid
 * commitment) that are NOT already owned (no unspent owner coin) — i.e. the
 * backend's `read_auction_position_names`. A pure name list, no phase: the
 * caller pairs it with `useNamesActionCapabilities` to get live phase/task
 * state per name. Parameterized on `walletProfileId` (rather than resolving
 * the active profile internally like `useReadNames`) so callers that already
 * pin a profile id (e.g. AuctionsView) can share the exact same id across
 * both queries.
 */
export function useAuctionPositions(
  walletProfileId: string | null,
): UseQueryResult<string[]> {
  const nodeLive = useNodeLive();
  return useQuery<string[]>({
    queryKey: ["read", "auctionPositions", walletProfileId],
    enabled: walletProfileId != null,
    queryFn: async () => {
      const raw = await invoke<string[] | null>("read_auction_position_names", {
        walletProfileId,
      });
      return Array.isArray(raw) ? raw : [];
    },
    staleTime: STALE_TIME,
    // Poll while the node is live so an auction advancing through its phases
    // (reveal broadcast → confirmed → CLOSED → won/lost) surfaces without a
    // manual refresh — directly answering "when will I know if I won".
    refetchInterval: nodeLive ? 30_000 : false,
  });
}

/**
 * Chain-driven renewal/expiry data, pinned to the active wallet. Days until
 * expiry are computed live by the backend (`read_renewals`) from tracked chain
 * state + the current height — the stale CSV-imported columns are only a
 * per-row fallback, honestly marked `source: "csv-import"`.
 */
export function useReadRenewals(): UseQueryResult<RenewalsResponse | null> {
  const profileId = useActiveProfile().data?.id ?? null;
  return useQuery<RenewalsResponse | null>({
    queryKey: ["read", "renewals", profileId],
    enabled: profileId != null,
    queryFn: async () => {
      const raw = await invoke<RenewalsResponse | null>("read_renewals", {
        walletProfileId: profileId,
      });
      return raw && Array.isArray(raw.names) ? raw : null;
    },
    staleTime: STALE_TIME,
  });
}

/** Provider-aware single-name lookup. */
export function useReadNameInfo(
  name: string | null | undefined,
): UseQueryResult<HsdName | null> {
  return useQuery<HsdName | null>({
    queryKey: ["read", "name", name ?? ""],
    enabled: Boolean(name && name.trim().length > 0),
    queryFn: async () => {
      const raw = await invoke<HsdName | null>("read_name_info", {
        name: name!.trim(),
      });
      return raw ?? null;
    },
    staleTime: STALE_TIME,
  });
}

/**
 * Backend-driven name action capabilities for a specific name.
 * Fetches `get_name_action_capabilities` to evaluate what actions are
 * available right now for the active wallet.
 */
export function useNameActionCapabilities(
  name: string | null | undefined,
  walletProfileId?: string | null,
): UseQueryResult<NameActionCapabilities | null> {
  const profileId = walletProfileId ?? null;
  return useQuery<NameActionCapabilities | null>({
    queryKey: ["read", "nameCapabilities", profileId, name ?? ""],
    enabled: Boolean(name && name.trim().length > 0),
    queryFn: async () => {
      // Pin the evaluation to THIS wallet so capabilities can never reflect
      // another profile's owned-name evidence (the active profile may flip
      // mid-switch).
      const raw = await invoke<NameActionCapabilities | null>(
        "get_name_action_capabilities",
        { name: name!.trim(), walletProfileId: profileId },
      );
      return raw ?? null;
    },
    staleTime: STALE_TIME,
  });
}

/**
 * Explorer-backed per-bid detail for a single name (Task 2), joined against
 * this wallet's own bid_commitments so `mine`/`myValue` are trustworthy.
 * Modeled exactly on `useNameActionCapabilities`: pinned to `walletProfileId`
 * (not the internally-resolved active profile) so a fast profile switch can
 * never attribute another wallet's bids as "mine". Degrades to `null` on
 * explorer-down / name-not-found (the backend never errors for this read).
 */
export function useNameBids(
  name: string | null | undefined,
  walletProfileId: string | null,
): UseQueryResult<NameBids | null> {
  const profileId = walletProfileId ?? null;
  return useQuery<NameBids | null>({
    queryKey: ["read", "nameBids", profileId, name ?? ""],
    enabled: Boolean(name && name.trim().length > 0),
    queryFn: async () => {
      const raw = await invoke<NameBids | null>("read_name_bids", {
        name: name!.trim(),
        walletProfileId: profileId,
      });
      return raw ?? null;
    },
    staleTime: STALE_TIME,
  });
}

/**
* Current DNS records for a name, read from the local hsd node
* (`read_name_records` → `getnameresource`). Node-only: the explorer doesn't
* expose resource records, so this returns `[]` whenever no synced node is
* reachable (the backend degrades gracefully and never errors). The name
* actions modal seeds its DNS editor from this once per open so the user can
* see, edit, and delete the name's existing records. Pinned to a specific
* wallet the same way as `useNameBids`.
*/
export function useNameRecords(
  name: string | null | undefined,
  walletProfileId: string | null,
  opts?: { forceFresh?: boolean },
): UseQueryResult<NameResource> {
  const profileId = walletProfileId ?? null;
  const forceFresh = opts?.forceFresh ?? false;
  return useQuery<NameResource>({
    queryKey: ["read", "nameRecords", profileId, name ?? ""],
    enabled: Boolean(name && name.trim().length > 0),
    queryFn: async () => {
      const raw = await invoke<NameResource | null>("read_name_records", {
        name: name!.trim(),
        walletProfileId: profileId,
      });
      // The backend guarantees `{records:[]}` on every degrade path, but
      // belt-and-braces in case of null/unexpected shapes.
      if (!raw || typeof raw !== "object" || !Array.isArray(raw.records)) {
        return { records: [] };
      }
      return raw;
    },
    // The DNS editor MUST seed from a guaranteed-fresh read: the resource is
    // rewritten wholesale by UPDATE, so seeding from a cached pre-UPDATE
    // snapshot lets the user overwrite their on-chain records from a stale
    // base. `forceFresh` disables the 15s cache and forces a refetch on every
    // mount (i.e. every modal open). Other callers keep the 15s cache.
    staleTime: forceFresh ? 0 : STALE_TIME,
    refetchOnMount: forceFresh ? "always" : true,
  });
}

/**
 * Compact block details for the in-app Block Info modal. Node-only: the
 * backend `read_block_info` command soft-degrades to `null` when no synced
 * node is reachable, so this hook is nullable. A mined block is immutable, so
 * results never go stale (`staleTime: Infinity`).
 */
export function useReadBlockInfo(
  height: number | null,
): UseQueryResult<BlockInfo | null> {
  return useQuery<BlockInfo | null>({
    queryKey: ["read", "block", height ?? 0],
    enabled: height != null && height > 0,
    queryFn: async () => {
      const raw = await invoke<BlockInfo | null>("read_block_info", {
        height: height!,
      });
      return raw ?? null;
    },
    staleTime: Infinity,
  });
}

/**
 * Compact transaction details for the in-app Transaction Info modal. Node-only:
 * the backend `read_tx_info` command returns tri-state:
 *   • a full `TxInfo` object (normal case);
 *   • `{ error: "tx_index_disabled" }` when the node lacks `--index-tx`
 *     (modal shows a distinct hint — narrow with `isTxInfoError`);
 *   • `null` for any other soft-degrade (no synced node, unknown tx, etc.).
 * Pending txs gain confirmations over time, so a short stale time keeps
 * the data fresh on re-open.
 */
export function useReadTxInfo(
  txid: string | null,
): UseQueryResult<TxInfo | TxInfoError | null> {
  return useQuery<TxInfo | TxInfoError | null>({
    queryKey: ["read", "tx", txid ?? ""],
    enabled: Boolean(txid && txid.trim().length > 0),
    queryFn: async () => {
      const raw = await invoke<TxInfo | TxInfoError | null>("read_tx_info", {
        txid: txid!.trim(),
      });
      return raw ?? null;
    },
    staleTime: 15_000,
  });
}

/**
 * Batch form of `useNameActionCapabilities` — one invoke for a whole list of
 * names instead of one per name (F5 fix: AuctionsView used to spawn N+1
 * capability fetches, one per row). Pinned to a specific wallet the same way
 * as the single-name hook, and re-fetches whenever the name list changes.
 *
 * Returns `[]` while disabled/loading so callers can `.map` unconditionally.
 */
export function useNamesActionCapabilities(
  names: string[],
  walletProfileId?: string | null,
): UseQueryResult<NameActionCapabilities[]> {
  const profileId = walletProfileId ?? null;
  const nodeLive = useNodeLive();
  // A stable, order-independent key so an unrelated re-render (e.g. the same
  // names in a new array instance) doesn't retrigger a refetch.
  const namesKey = [...names].sort().join(",");
  return useQuery<NameActionCapabilities[]>({
    queryKey: ["read", "namesCapabilities", profileId, namesKey],
    enabled: names.length > 0,
    queryFn: async () => {
      const raw = await invoke<NameActionCapabilities[] | null>(
        "get_names_action_capabilities",
        { names, walletProfileId: profileId },
      );
      return Array.isArray(raw) ? raw : [];
    },
    staleTime: STALE_TIME,
    // Poll capabilities while the node is live so the auctions row advances
    // through pending → confirmed → closed → won/lost without a manual refresh.
    refetchInterval: nodeLive ? 30_000 : false,
  });
}

/**
 * Recover a lost `bid_commitments` row: given a candidate bid value (in
 * doos), the backend recomputes the nonce/blind from the account xpub and
 * compares it against the on-chain blind of the profile's unspent BID coins.
 * On success it invalidates the `["read"]` prefix (so `nameCapabilities`
 * re-fetches and the REVEAL flow unlocks) and `["wallet"]`.
 */
export function useRecoverBidCommitment() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: {
      walletProfileId: string | null;
      name: string;
      bidValueDoos: number;
    }) => invoke<RecoveredBidCommitment>("recover_bid_commitment", args as Record<string, unknown>),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["read"] });
      qc.invalidateQueries({ queryKey: ["wallet"] });
    },
  });
}

/**
 * Brute-force bid-commitment recovery — no value input needed.
 *
 * Iterates candidate bid values (Tier 1: round increments; Tier 2: full
 * integer sweep up to the coin's lockup) and matches against the on-chain
 * blind. Recovers bids made in ANY hsd-compatible wallet (Bob, hsd-cli, etc.)
 * because the derivation is the hsd standard.
 *
 * This is a blocking call — for typical bids (< 100 HNS lockup) it finishes
 * in seconds. Larger lockups return an error asking for the known value.
 */
export function useBruteForceRecoverBid() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: {
      walletProfileId: string | null;
      name: string;
    }) =>
      invoke<import("../types").BruteForcedBidCommitment>(
        "brute_force_recover_bid",
        args as Record<string, unknown>,
      ),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["read"] });
      qc.invalidateQueries({ queryKey: ["wallet"] });
    },
  });
}

/** Provider-aware, normalized transaction history, pinned to the active wallet. */
export function useReadTransactions(): UseQueryResult<WalletTransactionRow[]> {
  const profileId = useActiveProfile().data?.id ?? null;
  return useQuery<WalletTransactionRow[]>({
    queryKey: ["read", "transactions", profileId],
    enabled: profileId != null,
    queryFn: async () => {
      const raw = await invoke<unknown>("read_transactions", {
        walletProfileId: profileId,
      });
      const arr = Array.isArray(raw) ? (raw as unknown[]) : [];
      return arr.map((tx, i) =>
        normalizeTransaction(tx as Record<string, unknown>, i),
      );
    },
    staleTime: STALE_TIME,
  });
}

/**
 * Every derived receive-branch address for the active wallet, tagged with a
 * `used` flag (address is referenced by a tracked UTXO or bid commitment).
 * Change-branch addresses are wallet-internal and are intentionally omitted.
 *
 * Keyed on the profile id so switching wallets never shows stale rows.
 */
export function useReceiveAddresses(): UseQueryResult<ReceiveAddressRow[]> {
  const profileId = useActiveProfile().data?.id ?? null;
  return useQuery<ReceiveAddressRow[]>({
    queryKey: ["read", "receive_addresses", profileId],
    enabled: profileId != null,
    queryFn: async () => {
      const rows = await invoke<ReceiveAddressRow[] | null>(
        "list_receive_addresses",
        { walletProfileId: profileId },
      );
      return rows ?? [];
    },
    staleTime: STALE_TIME,
  });
}

/**
 * Allocate the next unused receive-branch address, persist it, and refresh the
 * address list. Also invalidates the balance/name queries so the sync engine's
 * next pass finds the new address without a manual refresh.
 */
export function useRevealNextReceiveAddress() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (args: { walletProfileId: string | null }) =>
      invoke<string>("reveal_next_receive_address", args as Record<string, unknown>),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["read", "receive_addresses"] });
      // Deliberately NOT invalidating ["wallet"] — the spec pinned
      // `profile.receiveAddress` as unchanged by this feature, so a broader
      // wallet refetch would exceed the intended blast radius.
    },
  });
}
