/**
 * Adapter to convert imported Namebase history rows into the ActionRow format
 * used by the Activity page (feat/wallet-action-history). This lets the
 * Activity page display imported Namebase events alongside on-chain
 * transactions in a unified table, with a source filter to distinguish them.
 *
 * The adapter is designed to merge cleanly: the Activity page imports this
 * function and calls it on Namebase rows to get ActionRow-compatible objects
 * with extra optional Namebase-specific fields (feeDoos, bidDoos, etc.).
 */

import type { NamebaseHistoryRow } from "../queries/namebase";

/**
 * ActionRow shape (from feat/wallet-action-history) — the wire format returned
 * by `read_action_history`.
 */
export interface ActionRow {
  txid: string;
  action: string;
  name: string | null;
  nameHash: string | null;
  valueDoos: number;
  direction: string;
  height: number | null;
  time: number | null;
  confirmed: boolean;
  counterparty: string | null;
}

/**
 * Extended ActionRow with Namebase-specific fields and a `source` marker, used
 * by the Activity page when displaying imported history.
 */
export interface NamebaseActionRow extends ActionRow {
  source: "namebase";
  feeDoos: number | null;
  bidDoos: number | null;
  stakeDoos: number | null;
  usdCents: number | null;
  hnsDoos: number | null;
}

/**
 * Convert a Namebase history row to an ActionRow-compatible object.
 *
 * - `txid` → synthetic `nb:{id}` (namespaced so it can't collide with a real
 *   64-hex txid).
 * - `verb` → `action` via {@link verbToAction}.
 * - `valueDoos` → 0 for name-covenant actions (the locked value re-homes to the
 *   wallet's own coin), or the HNS amount for deposits/gifts.
 * - `height` → `null` (Namebase events are off-chain).
 * - `time` → Unix seconds from `createdAt`.
 * - `confirmed` → `true` (historical, so effectively confirmed).
 *
 * Extra Namebase fields ride along as optional properties so the UI can render
 * extra columns when the source is "namebase".
 */
export function namebaseEventToActionRow(row: NamebaseHistoryRow): NamebaseActionRow {
  const txid = `nb:${row.id}`;
  const action = verbToAction(row.verb);
  const time = Math.floor(new Date(row.createdAt).getTime() / 1000);

  // Value math: most name-covenant actions don't spend HNS (the locked value
  // re-homes to the wallet's own coin). Deposits/gifts carry real HNS flow.
  let valueDoos = 0;
  let direction = "internal";

  if (
    row.family === "auctions" ||
    row.family === "subdomains" ||
    row.family === "marketplace"
  ) {
    direction = "internal";
    valueDoos = 0;
  } else if (row.family === "wallet" && row.hnsDoos != null) {
    direction = "receive";
    valueDoos = row.hnsDoos;
  } else if (row.family === "misc" && row.hnsDoos != null) {
    direction = row.hnsDoos > 0 ? "receive" : "send";
    valueDoos = row.hnsDoos;
  }

  return {
    txid,
    action,
    name: row.name,
    nameHash: null,
    valueDoos,
    direction,
    height: null,
    time: Number.isFinite(time) ? time : null,
    confirmed: true,
    counterparty: null,
    source: "namebase",
    feeDoos: row.feeDoos,
    bidDoos: row.bidDoos,
    stakeDoos: row.stakeDoos,
    usdCents: row.usdCents,
    hnsDoos: row.hnsDoos,
  };
}

/**
 * Map a Namebase event verb to the ACTION_META vocabulary used by the Activity
 * page. Unknown verbs map to "other".
 */
export function verbToAction(verb: string): string {
  const mapping: Record<string, string> = {
    "place-bid": "bid",
    "reveal-bid": "reveal",
    "redeem-bid": "redeem",
    "register-bid": "register",
    "charge-fee": "bid",
    "charge-renewal-fee": "renew",
    "update-domain": "update",
    "confirm-transfer": "transfer",
    "initialize-transfer": "transfer",
    "buy-now": "transfer",
    deposit: "receive",
    "admin-gift": "receive",
    "rollback-place-bid": "bid",
    "refund-unrevealed-mined-bid": "bid",
    "revert-redeem-charge-now": "redeem",
    "market-buy-lock-quote": "other",
    "market-buy-return-quote": "other",
    "limit-lock-funds": "other",
    "cancel-return-funds": "other",
    "stake-domain": "other",
    "change-custodian": "other",
  };
  return mapping[verb] ?? "other";
}
