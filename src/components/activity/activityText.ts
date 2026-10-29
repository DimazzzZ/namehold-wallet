/**
 * Activity's labels and sentences, kept out of the page so its row
 * components can import them without importing the page.
 */
import type { MergedRow } from "../../lib/activity";
import { displayName } from "../../lib/idn";
import { formatDateShort, formatHns, formatHnsShort } from "../../lib/utils";

// Action label + badge variant mapping.
export const ACTION_META: Record<
  string,
  { label: string; variant: "default" | "success" | "warning" | "error" | "info" }
> = {
  send: { label: "Send", variant: "warning" },
  receive: { label: "Receive", variant: "success" },
  open: { label: "Open", variant: "info" },
  bid: { label: "Bid", variant: "info" },
  reveal: { label: "Reveal", variant: "info" },
  redeem: { label: "Redeem", variant: "success" },
  register: { label: "Register", variant: "success" },
  update: { label: "Update", variant: "default" },
  renew: { label: "Renew", variant: "default" },
  transfer: { label: "Transfer", variant: "warning" },
  finalize: { label: "Finalize", variant: "success" },
  revoke: { label: "Revoke", variant: "error" },
  claim: { label: "Claim", variant: "success" },
  // Shakedex: draft actions, kept as the row's label once the chain row of the
  // same tx is seen (`mergeActivity`; the chain's covenant label stays in
  // `covenantAction`) (PURCHASE_ACTION / PURCHASE_FINALIZE_ACTION in
  // shakedex/purchase.rs).
  shakedex_purchase: { label: "Buy", variant: "warning" },
  shakedex_purchase_finalize: { label: "Finalize purchase", variant: "success" },
  // Selling: LOCK_ACTION / LOCK_FINALIZE_ACTION in shakedex/sell.rs and
  // CANCEL_ACTION / CANCEL_FINALIZE_ACTION in shakedex/cancel.rs.
  shakedex_lock: { label: "Lock for sale", variant: "warning" },
  shakedex_lock_finalize: { label: "Finalize into lock", variant: "default" },
  shakedex_cancel: { label: "Cancel listing", variant: "warning" },
  shakedex_cancel_finalize: { label: "Finalize cancel", variant: "success" },
  // A Speed up child (speed_up.rs::SPEED_UP_ACTION).
  speed_up: { label: "Speed up", variant: "info" },
  other: { label: "Other", variant: "default" },
};

export const FALLBACK_META = { label: "Other", variant: "default" as const };

// Actions whose covenant output re-homes the name's locked value onto the
// wallet's own new coin. For these, the Amount cell shows the locked value as
// an informational "⤷ N" (not a spend) with a tooltip; net flow stays 0.
//
// Deliberately a subset of the action labels the backend emits — "open",
// "revoke" and "claim" move no locked value — so which actions belong here is
// this screen's decision, not the backend's. What the backend does own is the
// spelling: a renamed label would silently stop matching and quietly change
// what the Amount cell shows. A Rust test reads this list and fails if any
// entry is not a label `classify_tx` can produce.
export const NAME_COVENANT_ACTIONS = new Set([
  "bid",
  "reveal",
  "redeem",
  "register",
  "update",
  "renew",
  "transfer",
  "finalize",
]);

export type BadgeVariant = "default" | "success" | "warning" | "error" | "info";

/**
 * Map a MergedRow status to a badge variant and display label.
 */
export function statusBadge(
  status: string,
  /**
   * Whether a block has included the row. Authoritative over the draft's own
   * status, which can lag behind the chain.
   */
  onchain: { confirmed: boolean; height: number | null },
): {
  variant: BadgeVariant;
  label: string;
  /** What the status means and what it is waiting on. */
  hint: string;
} {
  if (status === "onchain") {
    return onchain.confirmed
      ? {
          variant: "success",
          label: "Confirmed",
          hint: `Mined into a block${onchain.height != null ? ` (#${onchain.height})` : ""}.`,
        }
      : {
          variant: "warning",
          label: "Pending",
          hint: "Seen by the node but not in a block yet.",
        };
  }
  if (status === "confirmed") {
    return { variant: "success", label: "Confirmed", hint: "Mined into a block. Done." };
  }
  if (status === "broadcasted" || status === "broadcast_pending") {
    // The draft hears about its block on the next confirmation refresh; the
    // chain row already knows.
    if (onchain.confirmed) {
      return { variant: "success", label: "Confirmed", hint: "Mined into a block. Done." };
    }
    return {
      variant: "warning",
      label: "Pending",
      hint: "Sent to the network and waiting in the mempool. Nothing changes on-chain until a block includes it.",
    };
  }
  if (status === "dropped") {
    return {
      variant: "error",
      label: "Not confirmed",
      hint: "The network no longer has it. It was never mined, so nothing happened on-chain.",
    };
  }
  if (status === "failed") {
    return {
      variant: "error",
      label: "Failed",
      hint: "It could not be sent. Nothing left this wallet.",
    };
  }
  if (status === "signed") {
    return { variant: "default", label: status, hint: "Signed but not sent yet." };
  }
  if (status === "draft") {
    return { variant: "default", label: status, hint: "Built but neither signed nor sent." };
  }
  return { variant: "default", label: status, hint: "" };
}

/** The row's badge: "Lost" for a lost purchase whatever its draft status. */
export function rowBadge(row: MergedRow): { variant: BadgeVariant; label: string; hint: string } {
  const badge = row.lostReason
    ? { variant: "error" as const, label: "Lost", hint: row.lostReason }
    : statusBadge(row.status, { confirmed: row.confirmed, height: row.height ?? null });
  return { ...badge, hint: row.statusNote ?? badge.hint };
}

/** The name a row is about: ".a", ".a + 2 more" for a batch, "" for none. */
export function nameLabel(row: MergedRow): string {
  // A batch row's `name` is a synthetic label: never print it.
  if (row.nameList && row.nameList.length > 1) {
    return `.${displayName(row.nameList[0]!)} + ${row.nameList.length - 1} more`;
  }
  return row.name ? `.${displayName(row.name)}` : "";
}

/** A covenant row that re-homes the name's locked value: only the fee left. */
export function showsNameValue(row: MergedRow): boolean {
  return (
    row.covenantAction != null &&
    NAME_COVENANT_ACTIONS.has(row.covenantAction) &&
    row.valueDoos === 0 &&
    row.nameValueDoos != null
  );
}

/** The row's time as an ISO string for the date formatters; null when unknown. */
export function rowIso(row: MergedRow): string | null {
  return row.sortTs > 0 ? new Date(row.sortTs * 1000).toISOString() : null;
}

export interface RowSentence {
  lead: string;
  name: string;
  tail: string;
}

/** Trim the space a lead keeps for the name when there is no name. */
function sentence(lead: string, name: string, tail: string): RowSentence {
  return { lead: name ? lead : lead.trimEnd(), name, tail };
}

function verb(row: MergedRow): string {
  if (row.action === "send") return `Send ${formatHnsShort(Math.abs(row.valueDoos))} HNS`;
  if (row.action === "shakedex_purchase") return "Buying";
  return (ACTION_META[row.action] ?? FALLBACK_META).label;
}

/** To finish: what was left undone, and the line under it. */
export function finishSentence(row: MergedRow): RowSentence & {
  detail: string;
  detailTone: "muted" | "error";
} {
  const name = nameLabel(row);
  const when = formatDateShort(rowIso(row), { year: false });
  if (row.lostReason) {
    const amount = Math.abs(row.valueDoos);
    return {
      ...sentence(
        "Buying ",
        name,
        amount > 0
          ? ` for ${formatHnsShort(amount)} HNS didn't go through.`
          : " didn't go through.",
      ),
      detail: `${when}. ${row.lostReason}`,
      detailTone: "error",
    };
  }
  const note = row.statusNote ? ` ${row.statusNote}` : "";
  const lead = `${verb(row)} `;
  switch (row.status) {
    case "draft":
      return {
        ...sentence(lead, name, " was built but not sent."),
        detail:
          row.feeDoos == null
            ? `Draft from ${when}. Its coins stay reserved until you send or discard it.`
            : `Draft from ${when} · fee ${formatHns(row.feeDoos)} HNS. Its coins stay reserved until you send or discard it.`,
        detailTone: "muted",
      };
    case "signed":
      return {
        ...sentence(lead, name, " was signed but not sent."),
        detail: `Built ${when}. Broadcast it, or discard it to free its coins.`,
        detailTone: "muted",
      };
    case "failed":
      return {
        ...sentence(lead, name, " could not be sent."),
        detail: `${when}. Nothing left this wallet.${note}`,
        detailTone: "muted",
      };
    default:
      return {
        ...sentence(lead, name, " was not confirmed."),
        detail: `Sent ${when}. The network dropped it; nothing happened on-chain.${note}`,
        detailTone: "muted",
      };
  }
}

/** Waiting for a block: what is on its way. */
export function waitingSentence(row: MergedRow): RowSentence {
  const name = nameLabel(row);
  const short = formatHnsShort(Math.abs(row.valueDoos));
  const covenant = showsNameValue(row);
  const nameValue = covenant ? formatHnsShort(row.nameValueDoos!) : "";
  switch (row.action) {
    case "send":
      return sentence(`Send ${short} HNS`, "", "");
    case "bid":
      return sentence("Bid on ", name, covenant ? ` · ${nameValue} HNS lockup` : "");
    case "shakedex_purchase":
      return sentence("Buy ", name, ` · ${short} HNS`);
    default:
      return sentence(
        `${(ACTION_META[row.action] ?? FALLBACK_META).label} `,
        name,
        covenant ? ` · ${nameValue} HNS` : row.valueDoos !== 0 ? ` · ${short} HNS` : "",
      );
  }
}
