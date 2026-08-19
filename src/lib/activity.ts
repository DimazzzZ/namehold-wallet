/**
 * Client-side merge of on-chain history (ActionRow) and local drafts
 * (TxDraftSummary) into a single recency-sorted list, deduped by txid.
 *
 * Pure function, no React dependency — unit-testable in isolation.
 */
import type { ActionRow } from "./zod";
import type { TxDraftSummary } from "../types";
import { netSpendDoos } from "./utils";

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
  confirmed: boolean;
  height: number | null;
  /** Unix-seconds sort key (ActionRow.time when present, else createdAt parsed). */
  sortTs: number;
  counterparty: string | null;
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
export function mergeActivity(
  rows: ActionRow[],
  drafts: TxDraftSummary[],
): MergedRow[] {
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
      action: row.action,
      name: row.name ?? null,
      nameList: draft?.summary?.nameList ?? null,
      valueDoos: row.valueDoos,
      direction: row.direction,
      feeDoos: draft?.summary?.feeDoos ?? null,
      status: draft?.status ?? "onchain",
      confirmed: row.confirmed,
      height: row.height ?? null,
      sortTs: row.time ?? 0,
      counterparty: row.counterparty ?? null,
      // For a self-homed covenant matched to a draft, expose the locked
      // name value so the UI can render its "222 HNS carried" tooltip.
      nameValueDoos:
        draft?.summary?.recipientAddress == null &&
        (draft?.summary?.sendTotalDoos ?? 0) > 0
          ? draft!.summary!.sendTotalDoos
          : null,
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

  // Step 4: sort newest-first.
  merged.sort((a, b) => b.sortTs - a.sortTs);
  return merged;
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
    confirmed: d.status === "confirmed",
    height: d.confirmationHeight ?? null,
    sortTs: parseCreatedAt(d.createdAt),
    counterparty: summary?.recipientAddress ?? null,
  };
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
