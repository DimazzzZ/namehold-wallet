/**
 * Client-side merge of on-chain history (ActionRow) and local drafts
 * (TxDraftSummary) into a single recency-sorted list, deduped by txid.
 *
 * Pure function, no React dependency — unit-testable in isolation.
 */
import type { ActionRow } from "./zod";
import type { TxDraftSummary } from "../types";
import { netSpendDoos } from "./utils";

/** `speed_up.rs::SPEED_UP_ACTION`. */
const SPEED_UP_ACTION = "speed_up";
/** The statuses of a child still in play (`find_live_speed_up_child`). */
const LIVE_CHILD = new Set<string>([
  "draft",
  "signed",
  "broadcast_pending",
  "broadcasted",
  "confirmed",
]);

/** Draft lifecycle status values. */
export type DraftStatus = TxDraftSummary["status"];

/**
 * A unified row combining on-chain facts (from ActionRow) with local-draft
 * enrichment (fee, lifecycle status). Rows that only exist on one side
 * (inbound receives; unbroadcast drafts) are represented with null fields
 * for the missing side.
 */
export interface MergedRow {
  /** Stable key: txid when known, else `draft:${id}`. */
  key: string;
  txid: string | null;
  /** Draft ID when this row has a local draft record; null for node-only rows. */
  draftId: string | null;
  action: string;
  /**
   * The covenant label the chain gives this transaction (`classify_tx`'s
   * action): the chain row's own for a row seen on chain, else the one the
   * draft builds (a Shakedex draft's TRANSFER or FINALIZE, another draft's
   * action as it is); null when unknown. `action` may be a Shakedex draft's
   * own label; what the Amount cell shows follows this one.
   */
  covenantAction: string | null;
  name: string | null;
  /**
   * For batch actions (batch-bid, batch-renew, etc.), the real list of
   * individual names. When non-null, `name` is a synthetic composite label
   * (e.g. "js + 1 more") that MUST NOT be treated as a real name for
   * info-modal or explorer-link purposes.
   */
  nameList: string[] | null;
  /**
   * Net external flow in doos (signed: negative = outflow, positive = inflow).
   * Matches ActionRow.valueDoos sign convention. Self-homed covenants = 0.
   */
  valueDoos: number;
  /**
   * For self-homed covenants (recipientAddress null, sendTotalDoos > 0),
   * this is the name's locked value being re-homed. Used to display an
   * explanatory tooltip on the Amount cell. Null when unknown (no draft
   * summary available for this row).
   */
  nameValueDoos: number | null;
  /** receive | send | internal */
  direction: string;
  /** Fee in doos from the draft; null for receives / external txs with no local draft. */
  feeDoos: number | null;
  /** Draft lifecycle status, or "onchain" for node-only rows with no draft. */
  status: DraftStatus | "onchain";
  /**
   * Why a draft ended `dropped` or `failed` (its stored error message), e.g.
   * a lost purchase's "someone else bought the name first, or the seller
   * cancelled the listing — nothing was paid" (R13); null otherwise.
   */
  statusNote: string | null;
  /** Why the purchase this row sent was lost, paid or not (R13); null otherwise. */
  lostReason: string | null;
  confirmed: boolean;
  height: number | null;
  /** Unix-seconds sort key (ActionRow.time when present, else createdAt parsed). */
  sortTs: number;
  counterparty: string | null;
  /** For a Speed up child's row, the txid of the transaction it speeds up. */
  speedsUp: string | null;
  /** For a row a live Speed up child speeds up, that child's txid. */
  spedUpBy: string | null;
}

/**
 * Parse a naive-UTC SQLite datetime string (or ISO) into unix seconds.
 * Mirrors the private `normalizeTimestamp` in utils.ts without importing it.
 */
function parseCreatedAt(s: string): number {
  const trimmed = s.trim();
  const hasTz = /[zZ]$/.test(trimmed) || /[+-]\d{2}:?\d{2}$/.test(trimmed);
  const iso = hasTz ? trimmed : `${trimmed.replace(" ", "T")}Z`;
  const ms = Date.parse(iso);
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : 0;
}

/**
 * Merge on-chain history rows with local drafts, deduping by txid.
 *
 * - An ActionRow and a draft sharing the same txid collapse into one MergedRow
 *   (direction/time/name from ActionRow; fee + status from draft).
 * - Inbound receives (no draft) → MergedRow with feeDoos=null, status="onchain".
 * - Unbroadcast drafts (txid null) → MergedRow with direction inferred from
 *   summary, sorted by createdAt.
 * - Dropped drafts (txid set, node forgot) → MergedRow from draft alone.
 *
 * Result is sorted newest-first by sortTs.
 */
export function mergeActivity(rows: ActionRow[], drafts: TxDraftSummary[]): MergedRow[] {
  // Be defensive: either source may arrive as null/undefined from a
  // backend that returns null, or before a query resolves.
  const safeRows = Array.isArray(rows) ? rows : [];
  const safeDrafts = Array.isArray(drafts) ? drafts : [];
  // Step 1: index drafts by txid (only those with a non-null txid).
  const draftByTxid = new Map<string, TxDraftSummary>();
  const localOnlyDrafts: TxDraftSummary[] = [];

  for (const d of safeDrafts) {
    if (d.txid) {
      const existing = draftByTxid.get(d.txid);
      if (!existing || statusRank(d.status) > statusRank(existing.status)) {
        draftByTxid.set(d.txid, d);
      }
    } else {
      localOnlyDrafts.push(d);
    }
  }

  const consumedTxids = new Set<string>();
  const merged: MergedRow[] = [];

  // Step 2: for each ActionRow, build a MergedRow; overlay draft if matched.
  for (const row of safeRows) {
    const draft = draftByTxid.get(row.txid);
    if (draft) consumedTxids.add(row.txid);

    merged.push({
      key: row.txid,
      txid: row.txid,
      draftId: draft?.id ?? null,
      // A Shakedex draft's covenant is a plain TRANSFER, FINALIZE or UPDATE
      // on chain, and a Speed up child a plain send of our own change; the
      // draft knows it was a lock, a purchase, a cancel or a Speed up.
      action:
        draft && (draft.action.startsWith("shakedex_") || draft.action === SPEED_UP_ACTION)
          ? draft.action
          : row.action,
      covenantAction: row.action,
      // The chain row's name comes from the covenant, which only carries a raw
      // name for OPEN, BID and FINALIZE. For every other name action the draft
      // this wallet built is the only place the name exists — without this
      // fallback a confirmed REVEAL or REGISTER listed itself as "—".
      name: row.name ?? draft?.summary?.name ?? null,
      nameList: draft?.summary?.nameList ?? null,
      valueDoos: row.valueDoos,
      direction: row.direction,
      feeDoos: draft?.summary?.feeDoos ?? null,
      status: draft?.status ?? "onchain",
      statusNote: draft ? statusNote(draft) : null,
      lostReason: draft?.purchaseLostReason ?? null,
      confirmed: row.confirmed,
      height: row.height ?? null,
      sortTs: row.time ?? 0,
      counterparty: row.counterparty ?? null,
      speedsUp: null,
      spedUpBy: null,
      // For a self-homed covenant matched to a draft, expose the locked
      // name value so the UI can render its "222 HNS carried" tooltip.
      nameValueDoos:
        // Prefer the backend value taken from the covenant output's own
        // `value` (reliable, covers on-chain rows with no local draft). Fall
        // back to the matched draft's self-homed send total.
        row.nameValueDoos ??
        (draft?.summary?.recipientAddress == null && (draft?.summary?.sendTotalDoos ?? 0) > 0
          ? draft!.summary!.sendTotalDoos
          : null),
    });
  }

  // Step 3: drafts NOT consumed (local-only or dropped/evicted from node).
  for (const d of localOnlyDrafts) {
    merged.push(draftToMergedRow(d));
  }
  for (const [txid, d] of draftByTxid) {
    if (!consumedTxids.has(txid)) {
      merged.push(draftToMergedRow(d));
    }
  }

  // Step 4: link Speed up children and the transactions they speed up. A
  // dropped or failed child speeds up nothing.
  const draftById = new Map(safeDrafts.map((d) => [d.id, d]));
  const childByParent = new Map<string, string>();
  for (const d of safeDrafts) {
    const parent = d.summary?.parentTxid;
    const child = d.txid ?? d.summary?.txid;
    if (d.action === SPEED_UP_ACTION && parent && child && LIVE_CHILD.has(d.status)) {
      childByParent.set(parent, child);
    }
  }
  for (const r of merged) {
    const d = r.draftId ? draftById.get(r.draftId) : undefined;
    if (d?.action === SPEED_UP_ACTION) r.speedsUp = d.summary?.parentTxid ?? null;
    if (r.txid) r.spedUpBy = childByParent.get(r.txid) ?? null;
  }

  // Step 5: sort newest-first.
  merged.sort((a, b) => b.sortTs - a.sortTs);
  return merged;
}

/**
 * The covenant each Shakedex draft action builds on chain: the lock, cancel
 * and purchase are TRANSFERs, their finalizes FINALIZEs (`shakedex/sell.rs`,
 * `cancel.rs`, `purchase.rs`).
 */
const SHAKEDEX_DRAFT_COVENANT: Record<string, string> = {
  shakedex_lock: "transfer",
  shakedex_lock_finalize: "finalize",
  shakedex_cancel: "transfer",
  shakedex_cancel_finalize: "finalize",
  shakedex_purchase: "transfer",
  shakedex_purchase_finalize: "finalize",
};

/** The covenant label of a draft not seen on chain yet; null when unknown. */
function draftCovenant(action: string): string | null {
  if (!action.startsWith("shakedex_")) return action;
  return SHAKEDEX_DRAFT_COVENANT[action] ?? null;
}

/** Convert a draft-only entry into a MergedRow. */
function draftToMergedRow(d: TxDraftSummary): MergedRow {
  const summary = d.summary;
  const spend = summary ? netSpendDoos(summary) : 0;
  const direction = summary?.recipientAddress != null ? "send" : "internal";
  return {
    key: d.txid ?? `draft:${d.id}`,
    txid: d.txid ?? null,
    draftId: d.id,
    action: d.action,
    covenantAction: draftCovenant(d.action),
    name: summary?.name ?? null,
    nameList: summary?.nameList ?? null,
    // Negate: netSpendDoos is a positive magnitude; ActionRow convention is
    // negative for outflow.
    valueDoos: direction === "send" ? -spend : 0,
    nameValueDoos:
      summary && summary.recipientAddress == null && summary.sendTotalDoos > 0
        ? summary.sendTotalDoos
        : null,
    direction,
    feeDoos: summary?.feeDoos ?? null,
    status: d.status,
    statusNote: statusNote(d),
    lostReason: d.purchaseLostReason ?? null,
    confirmed: d.status === "confirmed",
    height: d.confirmationHeight ?? null,
    sortTs: parseCreatedAt(d.createdAt),
    counterparty: summary?.recipientAddress ?? null,
    speedsUp: null,
    spedUpBy: null,
  };
}

function statusNote(d: TxDraftSummary): string | null {
  return d.status === "dropped" || d.status === "failed" ? (d.errorMessage ?? null) : null;
}

/** Rank draft statuses so that when two drafts share a txid we keep the most advanced. */
const STATUS_ORDER: Record<string, number> = {
  draft: 0,
  signed: 1,
  failed: 2,
  broadcast_pending: 3,
  broadcasted: 4,
  confirmed: 5,
  dropped: 6,
};
function statusRank(s: string): number {
  return STATUS_ORDER[s] ?? -1;
}
